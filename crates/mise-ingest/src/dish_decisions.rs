//! Dish decisions (docs/curation.md): which recipe records belong in one dish, and which pairs a person looked at
//! and kept apart. A decisions file is the source of truth when it is applied: applying it replaces the table. The
//! web app's review page records one decision at a time ([`record`]), and [`export`] writes the table back out as
//! such a file.

use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Postgres, Transaction};

/// One record: the source it came from and its id there.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
pub struct Record(pub String, pub String);

/// A decision about one pair of records: `{"decision": "merge", "a": [...], "b": [...], "note": "renamed"}`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct Decision {
    /// "merge" (the two belong in one dish) or "keep_apart" (looked at; don't propose the pair again).
    pub decision: String,
    /// One record of the pair; which is `a` and which `b` doesn't matter.
    pub a: Record,
    /// The other record.
    pub b: Record,
    /// Who decided: "user", or "claude" when decided on the person's behalf from their earlier decisions.
    #[serde(default = "user")]
    pub by: String,
    /// Why, in a few words: "renamed: same recipe".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

fn user() -> String {
    "user".into()
}

/// How many decisions of each kind a file held, for the command's summary.
#[derive(Debug, Default)]
pub struct ApplyStats {
    /// Pairs to join into one dish.
    pub merge: usize,
    /// Pairs kept apart.
    pub keep_apart: usize,
    /// Decisions naming a record that is not in the database (pruned, or not imported yet). Kept, harmless.
    pub unknown_records: usize,
}

/// How often a household has had one version.
#[derive(Deserialize)]
pub struct Preferred {
    /// The version, by its record.
    pub record: Record,
    /// How many times it's been had: 14.
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

/// Loads a file of dish decisions (an array of [`Decision`]), replacing those recorded before, all or nothing.
/// The same pair decided twice keeps the later decision; an unknown `decision` fails the whole file.
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
        record(&mut tx, d).await?;
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

/// Records one decision beside those already there, inside the caller's transaction; a pair decided before takes
/// the new decision. `mise group-dishes` honours it from its next run.
pub async fn record(tx: &mut Transaction<'_, Postgres>, d: &Decision) -> Result<()> {
    // One row per unordered pair, so the same pair decided twice keeps the later decision.
    let (a, b) = if d.a <= d.b {
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
    .execute(&mut **tx)
    .await
    .context("recording a dish decision")?;
    Ok(())
}

/// Every recorded decision, as a decisions file holds them (`mise dish apply` loads it back), in a fixed order.
pub async fn export(pool: &PgPool) -> Result<Vec<Decision>> {
    type Row = (
        String,
        String,
        String,
        String,
        String,
        String,
        Option<String>,
    );
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT decision, a_source, a_external, b_source, b_external, decided_by, note FROM dish_decisions
         ORDER BY a_source, a_external, b_source, b_external",
    )
    .fetch_all(pool)
    .await
    .context("reading dish decisions")?;
    Ok(rows
        .into_iter()
        .map(
            |(decision, a_source, a_external, b_source, b_external, by, note)| Decision {
                decision,
                a: Record(a_source, a_external),
                b: Record(b_source, b_external),
                by,
                note,
            },
        )
        .collect())
}
