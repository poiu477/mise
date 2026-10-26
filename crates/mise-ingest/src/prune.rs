//! Removing imported variants of a content type (ready meals, store products) that should not be in the library.

use anyhow::Result;
use sqlx::PgPool;

#[derive(Debug, Default)]
pub struct PruneStats {
    pub variants: u64,
    pub source_records: u64,
    pub dishes: u64,
}

/// Deletes imported variants of `content_type` with everything that hangs off them. Versions written in the app
/// are never touched. Kept variants copied from a pruned one lose that lineage link; dishes the grouping job made
/// and that end up empty are removed. Re-run `group-dishes` and `derive-options` afterwards.
pub async fn prune_content_type(pool: &PgPool, content_type: &str) -> Result<PruneStats> {
    let mut stats = PruneStats::default();
    let mut tx = pool.begin().await?;

    sqlx::query(
        "CREATE TEMP TABLE pruned ON COMMIT DROP AS
         SELECT id, source_record_id FROM variants
         WHERE source_record_id IS NOT NULL AND coalesce(meta->>'content_type', 'recipe') = $1",
    )
    .bind(content_type)
    .execute(&mut *tx)
    .await?;

    sqlx::query("UPDATE variants SET parent_variant_id = NULL WHERE parent_variant_id IN (SELECT id FROM pruned) AND id NOT IN (SELECT id FROM pruned)")
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE dishes SET default_variant_id = NULL WHERE default_variant_id IN (SELECT id FROM pruned)")
        .execute(&mut *tx)
        .await?;
    stats.variants = sqlx::query("DELETE FROM variants WHERE id IN (SELECT id FROM pruned)").execute(&mut *tx).await?.rows_affected();
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
