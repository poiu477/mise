//! Import a seed bundle (docs/seed-bundle.md) into the data core.
//!
//! Each variant line becomes a source record plus a variant with its yields, ingredient lines, amounts and
//! steps. Re-importing the same bundle is cheap: unchanged records are skipped, and a record whose content
//! changed becomes a new variant whose parent is the previous one. Lineage (`parent_external_id`) is
//! resolved after all lines are in, since a parent can appear later in the file.

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use uuid::Uuid;

const FORMAT: &str = "mise-seed/1";
const BATCH: usize = 500;

#[derive(Deserialize)]
struct Manifest {
    format: String,
    source: ManifestSource,
}

#[derive(Deserialize)]
struct ManifestSource {
    key: String,
    name: String,
}

#[derive(Deserialize)]
struct BundleVariant {
    external_id: String,
    url: Option<String>,
    parent_external_id: Option<String>,
    title: String,
    subtitle: Option<String>,
    description: Option<String>,
    content_type: Option<String>,
    published_at: Option<String>,
    #[serde(default)]
    meta: Value,
    #[serde(default)]
    yields: Vec<f64>,
    ingredients: Vec<BundleIngredient>,
    #[serde(default)]
    amounts: Vec<BundleAmount>,
    #[serde(default)]
    steps: Vec<BundleStep>,
    hero_image: Option<String>,
    raw: Option<Value>,
}

#[derive(Deserialize, Serialize)]
struct BundleIngredient {
    key: String,
    name: String,
    shipped: Option<bool>,
    #[serde(default)]
    allergens: Vec<Option<String>>,
    note: Option<String>,
    source_food: Option<Value>,
}

#[derive(Deserialize, Serialize)]
struct BundleAmount {
    key: String,
    servings: Option<f64>,
    amount: Option<f64>,
    unit: Option<String>,
}

#[derive(Deserialize, Serialize)]
struct BundleStep {
    index: i32,
    text: String,
    #[serde(default)]
    images: Vec<String>,
}

#[derive(Debug, Default)]
pub struct ImportStats {
    pub read: u64,
    pub inserted: u64,
    pub new_versions: u64,
    pub unchanged: u64,
    pub lineage_links: u64,
}

impl BundleVariant {
    /// Hash of everything that makes this version different from another: text, ingredients, amounts, steps.
    fn content_hash(&self) -> String {
        let canonical = json!({
            "title": self.title, "subtitle": self.subtitle, "description": self.description,
            "yields": self.yields, "ingredients": self.ingredients, "amounts": self.amounts, "steps": self.steps,
        });
        hex::encode(Sha256::digest(canonical.to_string().as_bytes()))
    }
}

/// Columns for one batch, filled per variant and written with one UNNEST insert per table.
#[derive(Default)]
struct Batch {
    rec_id: Vec<Uuid>,
    rec_external: Vec<String>,
    rec_url: Vec<Option<String>>,
    rec_raw: Vec<String>,
    rec_hash: Vec<String>,

    var_id: Vec<Uuid>,
    var_parent: Vec<Option<Uuid>>,
    var_title: Vec<String>,
    var_subtitle: Vec<Option<String>>,
    var_description: Vec<Option<String>>,
    var_published: Vec<Option<String>>,
    var_has_steps: Vec<bool>,
    var_meta: Vec<String>,
    var_type: Vec<String>,

    yield_variant: Vec<Uuid>,
    yield_servings: Vec<f64>,

    line_id: Vec<Uuid>,
    line_variant: Vec<Uuid>,
    line_key: Vec<String>,
    line_position: Vec<i32>,
    line_name: Vec<String>,
    line_shipped: Vec<Option<bool>>,
    line_allergens: Vec<String>,
    line_note: Vec<Option<String>>,
    line_source_food: Vec<Option<String>>,

    amount_line: Vec<Uuid>,
    amount_servings: Vec<f64>,
    amount_value: Vec<Option<f64>>,
    amount_unit: Vec<Option<String>>,

    step_id: Vec<Uuid>,
    step_variant: Vec<Uuid>,
    step_position: Vec<i32>,
    step_text: Vec<String>,
}

impl Batch {
    fn len(&self) -> usize {
        self.var_id.len()
    }

    fn push(&mut self, v: BundleVariant, hash: String, parent: Option<Uuid>) {
        let variant_id = Uuid::now_v7();
        self.rec_id.push(variant_id); // a record and its variant share an id; they are 1:1 for imports
        self.rec_external.push(v.external_id.clone());
        self.rec_url.push(v.url.clone());
        self.rec_raw.push(v.raw.as_ref().unwrap_or(&Value::Null).to_string());
        self.rec_hash.push(hash);

        let has_steps = v.steps.iter().any(|s| !s.text.trim().is_empty());
        let mut meta = if v.meta.is_object() { v.meta.clone() } else { json!({}) };
        meta["lineage"] = json!({ "parent_external_id": v.parent_external_id });
        meta["media"] = json!({
            "hero": v.hero_image,
            "steps": v.steps.iter().filter(|s| !s.images.is_empty())
                .map(|s| json!({ "index": s.index, "images": s.images })).collect::<Vec<_>>(),
        });

        self.var_id.push(variant_id);
        self.var_parent.push(parent);
        self.var_title.push(v.title.clone());
        self.var_subtitle.push(v.subtitle.clone());
        self.var_description.push(v.description.clone());
        self.var_published.push(v.published_at.clone());
        self.var_has_steps.push(has_steps);
        self.var_meta.push(meta.to_string());
        self.var_type.push(v.content_type.clone().unwrap_or_else(|| "recipe".into()));

        let mut seen_yields = HashSet::new();
        for s in v.yields.iter().copied().filter(|s| *s > 0.0) {
            if seen_yields.insert(s.to_bits()) {
                self.yield_variant.push(variant_id);
                self.yield_servings.push(s);
            }
        }

        // Sources occasionally list the same ingredient twice; keys must be unique within a variant.
        let mut line_by_key: HashMap<String, Uuid> = HashMap::new();
        let mut used_keys: HashSet<String> = HashSet::new();
        for (pos, ing) in v.ingredients.iter().enumerate() {
            let mut key = ing.key.clone();
            let mut n = 2;
            while used_keys.contains(&key) {
                key = format!("{}#{n}", ing.key);
                n += 1;
            }
            used_keys.insert(key.clone());
            let id = Uuid::now_v7();
            line_by_key.entry(ing.key.clone()).or_insert(id);
            self.line_id.push(id);
            self.line_variant.push(variant_id);
            self.line_key.push(key);
            self.line_position.push(pos as i32);
            self.line_name.push(ing.name.clone());
            self.line_shipped.push(ing.shipped);
            self.line_allergens.push(ing.allergens.iter().flatten().cloned().collect::<Vec<_>>().join("|"));
            self.line_note.push(ing.note.clone());
            self.line_source_food.push(ing.source_food.as_ref().map(|s| s.to_string()));
        }

        let mut seen_amounts: HashSet<(Uuid, u64)> = HashSet::new();
        for a in &v.amounts {
            let (Some(line), Some(servings)) = (line_by_key.get(&a.key), a.servings) else { continue };
            if servings <= 0.0 || !seen_amounts.insert((*line, servings.to_bits())) {
                continue;
            }
            self.amount_line.push(*line);
            self.amount_servings.push(servings);
            self.amount_value.push(a.amount);
            self.amount_unit.push(a.unit.clone());
        }

        for (pos, s) in v.steps.iter().enumerate() {
            self.step_id.push(Uuid::now_v7());
            self.step_variant.push(variant_id);
            self.step_position.push(pos as i32);
            self.step_text.push(s.text.clone());
        }
    }

    async fn write(&self, pool: &PgPool, source_id: Uuid) -> Result<()> {
        let mut tx = pool.begin().await?;

        sqlx::query(
            "INSERT INTO source_records (id, source_id, external_id, url, raw, content_hash)
             SELECT id, $1, ext, url, raw::jsonb, hash
             FROM UNNEST($2::uuid[], $3::text[], $4::text[], $5::text[], $6::text[]) AS t(id, ext, url, raw, hash)",
        )
        .bind(source_id)
        .bind(&self.rec_id)
        .bind(&self.rec_external)
        .bind(&self.rec_url)
        .bind(&self.rec_raw)
        .bind(&self.rec_hash)
        .execute(&mut *tx)
        .await
        .context("inserting source records")?;

        sqlx::query(
            "INSERT INTO variants (id, source_record_id, origin, parent_variant_id, title, subtitle, description,
                                   published_at, has_steps, meta, content_hash)
             SELECT t.id, t.id, 'source', parent, title, subtitle, descr, published::timestamptz, has_steps,
                    meta::jsonb || jsonb_build_object('content_type', ctype), sr.content_hash
             FROM UNNEST($1::uuid[], $2::uuid[], $3::text[], $4::text[], $5::text[], $6::text[], $7::bool[],
                         $8::text[], $9::text[])
                  AS t(id, parent, title, subtitle, descr, published, has_steps, meta, ctype)
             JOIN source_records sr ON sr.id = t.id",
        )
        .bind(&self.var_id)
        .bind(&self.var_parent)
        .bind(&self.var_title)
        .bind(&self.var_subtitle)
        .bind(&self.var_description)
        .bind(&self.var_published)
        .bind(&self.var_has_steps)
        .bind(&self.var_meta)
        .bind(&self.var_type)
        .execute(&mut *tx)
        .await
        .context("inserting variants")?;

        sqlx::query(
            "INSERT INTO variant_yields (variant_id, servings)
             SELECT v, s::numeric FROM UNNEST($1::uuid[], $2::float8[]) AS t(v, s)",
        )
        .bind(&self.yield_variant)
        .bind(&self.yield_servings)
        .execute(&mut *tx)
        .await
        .context("inserting yields")?;

        sqlx::query(
            "INSERT INTO ingredient_lines (id, variant_id, line_key, position, raw_name, shipped, allergens, note, source_food)
             SELECT id, v, k, p, n, sh, COALESCE(string_to_array(NULLIF(al, ''), '|'), '{}'), note, sf::jsonb
             FROM UNNEST($1::uuid[], $2::uuid[], $3::text[], $4::int4[], $5::text[], $6::bool[], $7::text[],
                         $8::text[], $9::text[]) AS t(id, v, k, p, n, sh, al, note, sf)",
        )
        .bind(&self.line_id)
        .bind(&self.line_variant)
        .bind(&self.line_key)
        .bind(&self.line_position)
        .bind(&self.line_name)
        .bind(&self.line_shipped)
        .bind(&self.line_allergens)
        .bind(&self.line_note)
        .bind(&self.line_source_food)
        .execute(&mut *tx)
        .await
        .context("inserting ingredient lines")?;

        sqlx::query(
            "INSERT INTO line_amounts (line_id, servings, amount, unit)
             SELECT l, s::numeric, a::numeric, u FROM UNNEST($1::uuid[], $2::float8[], $3::float8[], $4::text[]) AS t(l, s, a, u)",
        )
        .bind(&self.amount_line)
        .bind(&self.amount_servings)
        .bind(&self.amount_value)
        .bind(&self.amount_unit)
        .execute(&mut *tx)
        .await
        .context("inserting amounts")?;

        sqlx::query(
            "INSERT INTO steps (id, variant_id, position, text)
             SELECT id, v, p, txt FROM UNNEST($1::uuid[], $2::uuid[], $3::int4[], $4::text[]) AS t(id, v, p, txt)",
        )
        .bind(&self.step_id)
        .bind(&self.step_variant)
        .bind(&self.step_position)
        .bind(&self.step_text)
        .execute(&mut *tx)
        .await
        .context("inserting steps")?;

        tx.commit().await?;
        Ok(())
    }
}

pub async fn import_bundle(pool: &PgPool, dir: &Path) -> Result<ImportStats> {
    let manifest: Manifest = serde_json::from_reader(File::open(dir.join("manifest.json")).context("opening manifest.json")?)
        .context("reading manifest.json")?;
    if manifest.format != FORMAT {
        bail!("unsupported bundle format {:?} (expected {FORMAT})", manifest.format);
    }

    let source_id: Uuid = sqlx::query_scalar(
        "INSERT INTO sources (id, key, name, kind) VALUES ($1, $2, $3, 'bundle')
         ON CONFLICT (key) DO UPDATE SET name = EXCLUDED.name RETURNING id",
    )
    .bind(Uuid::now_v7())
    .bind(&manifest.source.key)
    .bind(&manifest.source.name)
    .fetch_one(pool)
    .await?;

    // Latest known version of each external record: (hash, variant id).
    let mut latest: HashMap<String, (String, Uuid)> = HashMap::new();
    let rows: Vec<(String, String, Uuid)> = sqlx::query_as(
        "SELECT DISTINCT ON (sr.external_id) sr.external_id, sr.content_hash, v.id
         FROM source_records sr JOIN variants v ON v.source_record_id = sr.id
         WHERE sr.source_id = $1 ORDER BY sr.external_id, sr.imported_at DESC",
    )
    .bind(source_id)
    .fetch_all(pool)
    .await?;
    for (ext, hash, id) in rows {
        latest.insert(ext, (hash, id));
    }

    let reader = BufReader::new(File::open(dir.join("variants.ndjson")).context("opening variants.ndjson")?);
    let mut stats = ImportStats::default();
    let mut batch = Batch::default();
    for (n, line) in reader.lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let v: BundleVariant =
            serde_json::from_str(&line).with_context(|| format!("variants.ndjson line {}", n + 1))?;
        stats.read += 1;
        let hash = v.content_hash();
        let parent = match latest.get(&v.external_id) {
            Some((h, _)) if *h == hash => {
                stats.unchanged += 1;
                continue;
            }
            Some((_, previous)) => {
                stats.new_versions += 1;
                Some(*previous)
            }
            None => {
                stats.inserted += 1;
                None
            }
        };
        batch.push(v, hash, parent);
        if batch.len() >= BATCH {
            batch.write(pool, source_id).await?;
            batch = Batch::default();
            tracing::info!(read = stats.read, "importing");
        }
    }
    if batch.len() > 0 {
        batch.write(pool, source_id).await?;
    }

    // Lineage: point each variant at the latest version of the record it was copied from.
    stats.lineage_links = sqlx::query(
        "WITH latest AS (
             SELECT DISTINCT ON (sr.external_id) sr.external_id, v.id
             FROM source_records sr JOIN variants v ON v.source_record_id = sr.id
             WHERE sr.source_id = $1 ORDER BY sr.external_id, sr.imported_at DESC)
         UPDATE variants v SET parent_variant_id = latest.id
         FROM source_records sr, latest
         WHERE v.source_record_id = sr.id AND sr.source_id = $1 AND v.parent_variant_id IS NULL
           AND latest.external_id = v.meta->'lineage'->>'parent_external_id' AND latest.id <> v.id",
    )
    .bind(source_id)
    .execute(pool)
    .await?
    .rows_affected();

    Ok(stats)
}
