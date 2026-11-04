//! Loading dish decisions (docs/curation.md): which recipe records belong in one dish, and which pairs a person
//! looked at and kept apart. The file is the source of truth: applying it replaces the table.

use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use sqlx::PgPool;

/// One record: the source it came from and its id there.
#[derive(Deserialize)]
pub struct Record(pub String, pub String);

#[derive(Deserialize)]
pub struct Decision {
    pub decision: String,
    pub a: Record,
    pub b: Record,
    #[serde(default = "user")]
    pub by: String,
    pub note: Option<String>,
}

fn user() -> String {
    "user".into()
}

#[derive(Debug, Default)]
pub struct ApplyStats {
    pub merge: usize,
    pub keep_apart: usize,
    /// Decisions naming a record that is not in the database (pruned, or not imported yet). Kept, harmless.
    pub unknown_records: usize,
}

/// How often a household has had one version.
#[derive(Deserialize)]
pub struct Preferred {
    pub record: Record,
    pub times: i32,
}

/// Load a file of preferred versions, replacing those recorded before. Returns how many were loaded.
pub async fn apply_preferred(pool: &PgPool, path: &Path) -> Result<usize> {
    let list: Vec<Preferred> = serde_json::from_reader(
        std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?,
    )
    .with_context(|| format!("reading {}", path.display()))?;
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM preferred_versions")
        .execute(&mut *tx)
        .await?;
    for p in &list {
        sqlx::query(
            "INSERT INTO preferred_versions (source, external_id, times) VALUES ($1, $2, $3)",
        )
        .bind(&p.record.0)
        .bind(&p.record.1)
        .bind(p.times)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(list.len())
}

pub async fn apply_file(pool: &PgPool, path: &Path) -> Result<ApplyStats> {
    let decisions: Vec<Decision> = serde_json::from_reader(
        std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?,
    )
    .with_context(|| format!("reading {}", path.display()))?;
    let mut stats = ApplyStats::default();
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM dish_decisions")
        .execute(&mut *tx)
        .await?;
    for d in &decisions {
        match d.decision.as_str() {
            "merge" => stats.merge += 1,
            "keep_apart" => stats.keep_apart += 1,
            other => bail!("unknown decision {other:?} (expected merge or keep_apart)"),
        }
        // One row per unordered pair, so the same pair decided twice keeps the later decision.
        let (a, b) = if (&d.a.0, &d.a.1) <= (&d.b.0, &d.b.1) {
            (&d.a, &d.b)
        } else {
            (&d.b, &d.a)
        };
        sqlx::query(
            "INSERT INTO dish_decisions (a_source, a_external, b_source, b_external, decision, decided_by, note)
             VALUES ($1, $2, $3, $4, $5, $6, $7)
             ON CONFLICT (a_source, a_external, b_source, b_external)
             DO UPDATE SET decision = EXCLUDED.decision, decided_by = EXCLUDED.decided_by, note = EXCLUDED.note",
        )
        .bind(&a.0)
        .bind(&a.1)
        .bind(&b.0)
        .bind(&b.1)
        .bind(&d.decision)
        .bind(&d.by)
        .bind(&d.note)
        .execute(&mut *tx)
        .await?;
    }
    stats.unknown_records = sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM dish_decisions dd WHERE NOT EXISTS (
             SELECT 1 FROM source_records sr JOIN sources s ON s.id = sr.source_id
             WHERE (s.key = dd.a_source AND sr.external_id = dd.a_external))
         OR NOT EXISTS (
             SELECT 1 FROM source_records sr JOIN sources s ON s.id = sr.source_id
             WHERE (s.key = dd.b_source AND sr.external_id = dd.b_external))",
    )
    .fetch_one(&mut *tx)
    .await? as usize;
    tx.commit().await?;
    Ok(stats)
}
