//! Storing imported recipes, shared by import from a link ([`crate::web`]) and from files ([`crate::files`]): keep
//! each recipe as a record of its source, make a version of it that the importing household alone sees, and run the
//! shared stages over it: match its foods, group it into a dish, derive the dish's options. Its steps are parsed as
//! they are stored.
//!
//! A recipe the household already has, unchanged, changes nothing; one that has changed becomes a new version whose
//! parent is the previous one, as a re-exported bundle's records do.

use std::path::Path;

use anyhow::{Context, Result};
use mise_core::ingredient_line;
use serde_json::json;
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::media::{UploadError, store_upload};
use crate::schema_org::Record;

/// Where imported records come from: a row of `sources`, made on first use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Source {
    /// The source's stable key ("web", "file:paprika").
    pub key: &'static str,
    /// Its name as people read it ("Web pages").
    pub name: &'static str,
    /// What kind of source it is: `web` or `file`.
    pub kind: &'static str,
}

/// One recipe, as imported.
#[derive(Debug, Clone, PartialEq)]
pub struct Imported {
    /// The version made of it, or the one made before when nothing changed.
    pub variant_id: Uuid,
    /// The recipe's title.
    pub title: String,
    /// The address of the dish it joined.
    pub dish_slug: String,
    /// The household had this recipe as it is already, so nothing was stored.
    pub unchanged: bool,
    /// Why the recipe's photo wasn't kept, when it has one that wasn't ("the site took too long to answer").
    pub photo_problem: Option<String>,
}

/// A recipe stored, before the shared stages give it a dish.
pub(crate) struct Stored {
    pub variant_id: Uuid,
    pub title: String,
    pub unchanged: bool,
    pub photo_problem: Option<String>,
}

/// Runs the shared stages when anything was stored, then answers with each recipe and the dish it joined.
pub(crate) async fn finish(pool: &PgPool, stored: Vec<Stored>) -> Result<Vec<Imported>> {
    if stored.iter().any(|s| !s.unchanged) {
        shared_stages(pool).await?;
    }
    let mut imported = Vec::with_capacity(stored.len());
    for s in stored {
        let dish_slug: Option<String> = sqlx::query_scalar(
            "SELECT d.slug FROM variants v JOIN dishes d ON d.id = v.dish_id WHERE v.id = $1",
        )
        .bind(s.variant_id)
        .fetch_one(pool)
        .await?;
        imported.push(Imported {
            variant_id: s.variant_id,
            title: s.title,
            dish_slug: dish_slug.context("an imported version has no dish after grouping")?,
            unchanged: s.unchanged,
            photo_problem: s.photo_problem,
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

/// A hash of everything that makes this recipe different from another: what it says, not when it was read. A photo
/// that came with the recipe (from a file) counts too, so a new photo is a change.
pub(crate) fn content_hash(r: &Record, photo: Option<&[u8]>) -> String {
    let mut canonical = json!({
        "title": r.title, "description": r.description, "meta": r.meta, "yields": r.yields,
        "ingredients": r.ingredients, "steps": r.steps, "hero": r.hero_image,
    });
    if let Some(photo) = photo {
        canonical["photo"] = json!(hex::encode(Sha256::digest(photo)));
    }
    hex::encode(Sha256::digest(canonical.to_string().as_bytes()))
}

/// The source's id, made on first use.
async fn source_id(tx: &mut Transaction<'_, Postgres>, source: &Source) -> Result<Uuid> {
    Ok(sqlx::query_scalar(
        "INSERT INTO sources (id, key, name, kind) VALUES ($1, $2, $3, $4)
         ON CONFLICT (key) DO UPDATE SET key = EXCLUDED.key RETURNING id",
    )
    .bind(Uuid::now_v7())
    .bind(source.key)
    .bind(source.name)
    .bind(source.kind)
    .fetch_one(&mut **tx)
    .await?)
}

/// Stores the record and its version, with lines, amounts and steps, then matches the lines' foods, all in one
/// transaction. Answers with the version and whether it is new; a record the household already has, unchanged,
/// answers with the version made of it before.
pub(crate) async fn store(
    pool: &PgPool,
    source: &Source,
    household: Uuid,
    record: &Record,
    hash: &str,
) -> Result<(Uuid, bool)> {
    let mut tx = pool.begin().await?;
    let source = source_id(&mut tx, source).await?;
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
    // The same recipe imported twice at once: the second finds the first's record and stores nothing.
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
    .context("storing the recipe's record")?;
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

/// Keeps only the picture of a recipe's photo (as an uploaded photo is kept) and makes it the version's. Answers
/// with why it wasn't kept, when it wasn't: a recipe is worth having without its photo.
pub(crate) async fn attach_photo(
    pool: &PgPool,
    media_root: &Path,
    variant: Uuid,
    bytes: Vec<u8>,
) -> Result<Option<String>> {
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
