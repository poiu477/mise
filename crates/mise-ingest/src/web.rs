//! Import recipes from a web page into a household: fetch the page ([`crate::fetch`]), read its schema.org Recipe
//! data ([`crate::schema_org`]), keep each recipe as a record of the `web` source, make a version of it, and run the
//! shared stages over it: match its foods, group it into a dish, derive the dish's options. Its steps are parsed as
//! they are stored.
//!
//! The record and the version belong to the importing household, which alone sees them. Importing a page whose
//! recipe hasn't changed changes nothing; one that has changed becomes a new version whose parent is the previous
//! one, as a re-exported bundle's records do.

use std::path::Path;

use anyhow::{Context, Result};
use mise_core::ingredient_line;
use serde_json::json;
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Postgres, Transaction};
use url::Url;
use uuid::Uuid;

use crate::fetch::{FetchError, Fetcher};
use crate::media::{UploadError, store_upload};
use crate::schema_org::{self, ReadError, Record};

/// The source every web page's records belong to.
pub const SOURCE_KEY: &str = "web";

/// The most recipes taken from one page: a page with more is a list of them, not a recipe.
pub const MAX_RECIPES: usize = 10;

/// Why a page wasn't imported.
#[derive(Debug)]
pub enum ImportError {
    /// The page couldn't be fetched, or the address was refused.
    Fetch(FetchError),
    /// The page was fetched but holds no recipe mise can read.
    Read(ReadError),
    /// The page holds more than [`MAX_RECIPES`] recipes.
    TooManyRecipes(usize),
    /// Storing it failed: the database or the disk.
    Failed(anyhow::Error),
}

impl std::fmt::Display for ImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ImportError::Fetch(e) => write!(f, "couldn't import that link: {e}"),
            ImportError::Read(e) => write!(f, "couldn't import that link: {e}"),
            ImportError::TooManyRecipes(n) => write!(
                f,
                "couldn't import that link: the page lists {n} recipes, more than the {MAX_RECIPES} mise takes from one page"
            ),
            ImportError::Failed(e) => write!(f, "importing failed: {e:#}"),
        }
    }
}

impl std::error::Error for ImportError {}

impl From<anyhow::Error> for ImportError {
    fn from(e: anyhow::Error) -> Self {
        ImportError::Failed(e)
    }
}

impl From<sqlx::Error> for ImportError {
    fn from(e: sqlx::Error) -> Self {
        ImportError::Failed(e.into())
    }
}

/// One recipe from the page, as imported.
#[derive(Debug, Clone, PartialEq)]
pub struct Imported {
    /// The version made of it, or the one made before when nothing changed.
    pub variant_id: Uuid,
    /// The recipe's title.
    pub title: String,
    /// The address of the dish it joined.
    pub dish_slug: String,
    /// The page held this recipe as it was imported before, so nothing was stored.
    pub unchanged: bool,
    /// Why the recipe's photo wasn't kept, when it has one that wasn't ("the site took too long to answer").
    pub photo_problem: Option<String>,
}

/// Fetches the page at `url` and imports every recipe on it into `household`, photos stored under `media_root`.
/// Answers with each recipe in page order.
pub async fn import_url(
    pool: &PgPool,
    media_root: &Path,
    fetcher: &Fetcher,
    household: Uuid,
    url: &Url,
) -> Result<Vec<Imported>, ImportError> {
    let page = fetcher.page(url).await.map_err(ImportError::Fetch)?;
    let records = schema_org::read_page(&page.html, Some(&page.url)).map_err(ImportError::Read)?;
    if records.len() > MAX_RECIPES {
        return Err(ImportError::TooManyRecipes(records.len()));
    }

    let mut stored: Vec<(Uuid, String, bool, Option<String>)> = Vec::new();
    let mut changed = false;
    for record in &records {
        let hash = content_hash(record);
        let (variant, fresh) = store(pool, household, record, &hash).await?;
        let mut photo_problem = None;
        if fresh {
            changed = true;
            if let Some(hero) = &record.hero_image {
                photo_problem = photo(pool, media_root, fetcher, variant, hero)
                    .await?
                    .map(|why| format!("its photo couldn't be kept: {why}"));
            }
        }
        stored.push((variant, record.title.clone(), !fresh, photo_problem));
    }
    if changed {
        shared_stages(pool).await?;
    }

    let mut imported = Vec::new();
    for (variant_id, title, unchanged, photo_problem) in stored {
        let dish_slug: Option<String> = sqlx::query_scalar(
            "SELECT d.slug FROM variants v JOIN dishes d ON d.id = v.dish_id WHERE v.id = $1",
        )
        .bind(variant_id)
        .fetch_one(pool)
        .await?;
        imported.push(Imported {
            variant_id,
            title,
            dish_slug: dish_slug.context("an imported version has no dish after grouping")?,
            unchanged,
            photo_problem,
        });
    }
    Ok(imported)
}

/// The key of the lock imports hold while grouping and deriving options ("mise imp" in ASCII): both rewrite every
/// dish from what they read, so two imports finishing together take turns.
const STAGES_LOCK: i64 = 0x6d69_7365_2069_6d70;

/// Groups dishes and derives their options, one import at a time. The lock is held by one connection for the session
/// and given back however the stages end.
async fn shared_stages(pool: &PgPool) -> Result<()> {
    let mut conn = pool.acquire().await?;
    sqlx::query("SELECT pg_advisory_lock($1)")
        .bind(STAGES_LOCK)
        .execute(&mut *conn)
        .await
        .context("waiting for another import")?;
    let ran = async {
        crate::dishes::group_dishes(pool).await?;
        crate::options::derive_options(pool).await?;
        anyhow::Ok(())
    }
    .await;
    let unlocked = sqlx::query("SELECT pg_advisory_unlock($1)")
        .bind(STAGES_LOCK)
        .execute(&mut *conn)
        .await;
    if unlocked.is_err() {
        // A connection that may still hold the lock never goes back to the pool.
        conn.detach();
    }
    ran
}

/// A hash of everything that makes this recipe different from another: what it says, not when it was read.
fn content_hash(r: &Record) -> String {
    let canonical = json!({
        "title": r.title, "description": r.description, "meta": r.meta, "yields": r.yields,
        "ingredients": r.ingredients, "steps": r.steps, "hero": r.hero_image,
    });
    hex::encode(Sha256::digest(canonical.to_string().as_bytes()))
}

/// The `web` source's id, made on first use.
async fn web_source(tx: &mut Transaction<'_, Postgres>) -> Result<Uuid> {
    Ok(sqlx::query_scalar(
        "INSERT INTO sources (id, key, name, kind) VALUES ($1, $2, 'Web pages', 'web')
         ON CONFLICT (key) DO UPDATE SET key = EXCLUDED.key RETURNING id",
    )
    .bind(Uuid::now_v7())
    .bind(SOURCE_KEY)
    .fetch_one(&mut **tx)
    .await?)
}

/// Stores the record and its version, with lines, amounts and steps, then matches the lines' foods, all in one
/// transaction. Answers with the version and whether it is new; a record the household already has, unchanged,
/// answers with the version made of it before.
async fn store(
    pool: &PgPool,
    household: Uuid,
    record: &Record,
    hash: &str,
) -> Result<(Uuid, bool)> {
    let mut tx = pool.begin().await?;
    let source = web_source(&mut tx).await?;
    let latest: Option<(String, Uuid)> = sqlx::query_as(
        "SELECT sr.content_hash, v.id FROM source_records sr JOIN variants v ON v.source_record_id = sr.id
         WHERE sr.source_id = $1 AND sr.external_id = $2 AND sr.household_id = $3
         ORDER BY sr.imported_at DESC, sr.id DESC LIMIT 1",
    )
    .bind(source)
    .bind(&record.external_id)
    .bind(household)
    .fetch_optional(&mut *tx)
    .await?;
    if let Some((known, variant)) = &latest
        && known == hash
    {
        return Ok((*variant, false));
    }

    let id = Uuid::now_v7();
    // The same page imported twice at once: the second finds the first's record and stores nothing.
    let inserted: Option<Uuid> = sqlx::query_scalar(
        "INSERT INTO source_records (id, source_id, external_id, url, raw, content_hash, household_id)
         VALUES ($1, $2, $3, $4, $5, $6, $7) ON CONFLICT DO NOTHING RETURNING id",
    )
    .bind(id)
    .bind(source)
    .bind(&record.external_id)
    .bind(&record.url)
    .bind(&record.raw)
    .bind(hash)
    .bind(household)
    .fetch_optional(&mut *tx)
    .await
    .context("storing the page's record")?;
    if inserted.is_none() {
        tx.rollback().await?;
        let variant: Uuid = sqlx::query_scalar(
            "SELECT v.id FROM source_records sr JOIN variants v ON v.source_record_id = sr.id
             WHERE sr.source_id = $1 AND sr.external_id = $2 AND sr.content_hash = $3 AND sr.household_id = $4",
        )
        .bind(source)
        .bind(&record.external_id)
        .bind(hash)
        .bind(household)
        .fetch_one(pool)
        .await?;
        return Ok((variant, false));
    }

    let mut meta = serde_json::to_value(&record.meta).context("writing the recipe's details")?;
    meta["content_type"] = json!("recipe");
    meta["source_url"] = json!(record.url);
    sqlx::query(
        "INSERT INTO variants (id, source_record_id, origin, parent_variant_id, household_id, title, description,
                               published_at, has_steps, meta, content_hash)
         VALUES ($1, $1, 'source', $2, $3, $4, $5, now(), $6, $7, $8)",
    )
    .bind(id)
    .bind(latest.map(|(_, v)| v))
    .bind(household)
    .bind(&record.title)
    .bind(&record.description)
    .bind(!record.steps.is_empty())
    .bind(&meta)
    .bind(hash)
    .execute(&mut *tx)
    .await
    .context("storing the version")?;
    write_parts(&mut tx, id, record).await?;
    crate::foods::match_foods_in(&mut tx).await?;
    tx.commit().await?;
    Ok((id, true))
}

/// The serving count a recipe's amounts are for. A yield that counts something else ("Makes 24 cookies"), or none,
/// is one serving: the whole batch, which scales as batches.
fn servings(record: &Record) -> f64 {
    record.yields.first().copied().unwrap_or(1.0)
}

/// Writes the version's yield, ingredient lines (each read into an amount, a unit, a name and a note, the line kept
/// as written) and steps (each with its template).
async fn write_parts(
    tx: &mut Transaction<'_, Postgres>,
    variant: Uuid,
    record: &Record,
) -> Result<()> {
    let servings = servings(record);
    sqlx::query(
        "INSERT INTO variant_yields (variant_id, servings) VALUES ($1, $2::float8::numeric)",
    )
    .bind(variant)
    .bind(servings)
    .execute(&mut **tx)
    .await?;
    let read: Vec<ingredient_line::Line> = record
        .ingredients
        .iter()
        .map(|i| ingredient_line::parse(&i.name))
        .collect();
    // A line's key is its place from 1, as the reader numbers them; a line with no name is matched by all of it.
    let keyed: Vec<(String, String)> = record
        .ingredients
        .iter()
        .zip(&read)
        .map(|(i, l)| {
            let name = if l.name.is_empty() { &i.name } else { &l.name };
            (i.key.clone(), name.clone())
        })
        .collect();
    for (position, ((ingredient, line), (key, name))) in
        record.ingredients.iter().zip(&read).zip(&keyed).enumerate()
    {
        let id = Uuid::now_v7();
        sqlx::query(
            "INSERT INTO ingredient_lines (id, variant_id, line_key, position, raw_name, note, text)
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(id)
        .bind(variant)
        .bind(key)
        .bind(position as i32)
        .bind(name)
        .bind(&line.note)
        .bind(&ingredient.name)
        .execute(&mut **tx)
        .await
        .context("storing an ingredient line")?;
        if let Some(amount) = line.amount {
            sqlx::query(
                "INSERT INTO line_amounts (line_id, servings, amount, unit)
                 VALUES ($1, $2::float8::numeric, $3::float8::numeric, $4)",
            )
            .bind(id)
            .bind(servings)
            .bind(amount)
            .bind(line.unit_word())
            .execute(&mut **tx)
            .await?;
        }
    }
    for (position, step) in record.steps.iter().enumerate() {
        let template = crate::steps::template(&step.text, &record.title, &keyed);
        sqlx::query(
            "INSERT INTO steps (id, variant_id, position, text, template) VALUES ($1, $2, $3, $4, $5::jsonb)",
        )
        .bind(Uuid::now_v7())
        .bind(variant)
        .bind(position as i32)
        .bind(&step.text)
        .bind(serde_json::to_string(&template)?)
        .execute(&mut **tx)
        .await
        .context("storing a step")?;
    }
    Ok(())
}

/// Fetches the recipe's photo through the same checks as the page, keeps only the picture (as an uploaded photo is
/// kept) and makes it the version's. Answers with why it wasn't kept, when it wasn't: a recipe is worth having
/// without its photo.
async fn photo(
    pool: &PgPool,
    media_root: &Path,
    fetcher: &Fetcher,
    variant: Uuid,
    hero: &str,
) -> Result<Option<String>> {
    let Ok(url) = Url::parse(hero) else {
        return Ok(Some("its address isn't a link".into()));
    };
    let bytes = match fetcher.image(&url).await {
        Ok(bytes) => bytes,
        Err(e) => return Ok(Some(e.to_string())),
    };
    let media = match store_upload(pool, media_root, bytes).await {
        Ok(media) => media,
        Err(UploadError::NotAnImage) => return Ok(Some("it isn't an image mise reads".into())),
        Err(UploadError::TooManyPixels) => return Ok(Some("it has too many pixels".into())),
        Err(UploadError::Failed(e)) => return Err(e),
    };
    sqlx::query(
        "INSERT INTO variant_media (variant_id, media_id, role, position) VALUES ($1, $2, 'hero', 0)
         ON CONFLICT DO NOTHING",
    )
    .bind(variant)
    .bind(media)
    .execute(pool)
    .await?;
    Ok(None)
}
