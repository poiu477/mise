//! Removing imported variants that should not be in the library: a whole content type (ready meals), or whatever
//! a seed bundle no longer contains after its converter was told to leave it out.

use std::collections::HashSet;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

use anyhow::{Context, Result};
use serde::Deserialize;
use sqlx::{PgPool, Postgres, Transaction};

/// What one prune deleted, for the command's summary.
#[derive(Debug, Default)]
pub struct PruneStats {
    /// Imported versions deleted, with their lines, amounts and steps.
    pub variants: u64,
    /// Source records left with no version, deleted too.
    pub source_records: u64,
    /// Dishes the grouping job made that were left empty, deleted too.
    pub dishes: u64,
}

/// Deletes imported variants of `content_type` with everything that hangs off them.
pub async fn prune_content_type(pool: &PgPool, content_type: &str) -> Result<PruneStats> {
    let mut tx = pool.begin().await?;
    sqlx::query(
        "CREATE TEMP TABLE pruned ON COMMIT DROP AS
         SELECT id, source_record_id FROM variants
         WHERE source_record_id IS NOT NULL AND coalesce(meta->>'content_type', 'recipe') = $1",
    )
    .bind(content_type)
    .execute(&mut *tx)
    .await?;
    delete_pruned(tx).await
}

#[derive(Deserialize)]
struct Manifest {
    source: ManifestSource,
}

#[derive(Deserialize)]
struct ManifestSource {
    key: String,
}

#[derive(Deserialize)]
struct Line {
    external_id: String,
}

/// Deletes imported variants of the bundle's source whose record the bundle no longer contains.
pub async fn prune_missing_from(pool: &PgPool, dir: &Path) -> Result<PruneStats> {
    let manifest: Manifest = serde_json::from_reader(
        File::open(dir.join("manifest.json")).context("opening manifest.json")?,
    )
    .context("reading manifest.json")?;
    let mut present = HashSet::new();
    for line in
        BufReader::new(File::open(dir.join("variants.ndjson")).context("opening variants.ndjson")?)
            .lines()
    {
        let line = line?;
        if !line.trim().is_empty() {
            present.insert(
                serde_json::from_str::<Line>(&line)
                    .context("reading variants.ndjson")?
                    .external_id,
            );
        }
    }
    let present: Vec<String> = present.into_iter().collect();

    let mut tx = pool.begin().await?;
    sqlx::query(
        "CREATE TEMP TABLE pruned ON COMMIT DROP AS
         SELECT v.id, v.source_record_id FROM variants v
         JOIN source_records sr ON sr.id = v.source_record_id JOIN sources s ON s.id = sr.source_id
         WHERE s.key = $1 AND sr.external_id <> ALL($2)",
    )
    .bind(&manifest.source.key)
    .bind(&present)
    .execute(&mut *tx)
    .await?;
    delete_pruned(tx).await
}

/// Deletes the variants in the `pruned` temp table. Versions written in the app are never in it. Kept variants
/// copied from a pruned one lose that lineage link; dishes the grouping job made and that end up empty are
/// removed. Re-run `group-dishes` and `derive-options` afterwards.
async fn delete_pruned(mut tx: Transaction<'_, Postgres>) -> Result<PruneStats> {
    let mut stats = PruneStats::default();
    sqlx::query("UPDATE variants SET parent_variant_id = NULL WHERE parent_variant_id IN (SELECT id FROM pruned) AND id NOT IN (SELECT id FROM pruned)")
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE dishes SET default_variant_id = NULL WHERE default_variant_id IN (SELECT id FROM pruned)")
        .execute(&mut *tx)
        .await?;
    stats.variants = sqlx::query("DELETE FROM variants WHERE id IN (SELECT id FROM pruned)")
        .execute(&mut *tx)
        .await?
        .rows_affected();
    stats.source_records = sqlx::query(
        "DELETE FROM source_records sr WHERE sr.id IN (SELECT source_record_id FROM pruned)
         AND NOT EXISTS (SELECT 1 FROM variants v WHERE v.source_record_id = sr.id)",
    )
    .execute(&mut *tx)
    .await?
    .rows_affected();
    stats.dishes = sqlx::query("DELETE FROM dishes d WHERE origin = 'grouped' AND NOT EXISTS (SELECT 1 FROM variants v WHERE v.dish_id = d.id)")
        .execute(&mut *tx)
        .await?
        .rows_affected();
    tx.commit().await?;
    Ok(stats)
}
