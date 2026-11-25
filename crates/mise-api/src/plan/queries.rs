//! The plan's SQL, each statement checked against the schema when mise is built.

use sqlx::PgExecutor;
use time::Date;
use uuid::Uuid;

/// A planned dish as the plan shows it.
pub struct EntryRow {
    /// The entry's id.
    pub id: Uuid,
    /// The version planned.
    pub variant_id: Uuid,
    /// Its dish's slug.
    pub dish_slug: Option<String>,
    /// The version's title.
    pub title: String,
    /// The version's subtitle.
    pub subtitle: Option<String>,
    /// How many it is planned for.
    pub servings: f64,
    /// Serving counts with published amounts, smallest first.
    pub yields: Vec<f64>,
    /// The version's photo.
    pub hero: Option<Uuid>,
}

/// A week's entries in the order they were added, with only the versions the household can see.
pub async fn entries(
    db: impl PgExecutor<'_>,
    household: Uuid,
    week: Date,
) -> sqlx::Result<Vec<EntryRow>> {
    sqlx::query_as!(
        EntryRow,
        r#"SELECT e.id, e.variant_id, d.slug AS "dish_slug?", v.title, v.subtitle,
                  e.servings::float8 AS "servings!",
                  ARRAY(SELECT y.servings::float8 FROM variant_yields y WHERE y.variant_id = v.id
                        ORDER BY y.servings) AS "yields!",
                  (SELECT vm.media_id FROM variant_media vm WHERE vm.variant_id = v.id AND vm.role = 'hero'
                   LIMIT 1) AS hero
           FROM plan_entries e JOIN variants v ON v.id = e.variant_id LEFT JOIN dishes d ON d.id = v.dish_id
           WHERE e.household_id = $1 AND e.week = $2 AND visible_to(v, $1) ORDER BY e.created_at, e.id"#,
        household,
        week
    )
    .fetch_all(db)
    .await
}

/// The serving counts version `variant` has amounts for, smallest first; none when the household can't see it.
pub async fn version_yields(
    db: impl PgExecutor<'_>,
    variant: Uuid,
    household: Uuid,
) -> sqlx::Result<Option<Vec<f64>>> {
    sqlx::query_scalar!(
        r#"SELECT ARRAY(SELECT y.servings::float8 FROM variant_yields y WHERE y.variant_id = v.id
                        ORDER BY y.servings) AS "yields!"
           FROM variants v WHERE v.id = $1 AND visible_to(v, $2)"#,
        variant,
        household
    )
    .fetch_optional(db)
    .await
}

/// Plans `variant` in `week` for `servings`, added by account `by`.
pub async fn add(
    db: impl PgExecutor<'_>,
    household: Uuid,
    week: Date,
    variant: Uuid,
    servings: f64,
    by: Uuid,
) -> sqlx::Result<()> {
    sqlx::query!(
        "INSERT INTO plan_entries (id, household_id, week, variant_id, servings, added_by)
         VALUES ($1, $2, $3, $4, $5::float8::numeric, $6)",
        Uuid::now_v7(),
        household,
        week,
        variant,
        servings,
        by
    )
    .execute(db)
    .await?;
    Ok(())
}

/// The serving counts entry `id`'s version has amounts for; none when the entry isn't the household's.
pub async fn entry_yields(
    db: impl PgExecutor<'_>,
    id: Uuid,
    household: Uuid,
) -> sqlx::Result<Option<Vec<f64>>> {
    sqlx::query_scalar!(
        r#"SELECT ARRAY(SELECT y.servings::float8 FROM variant_yields y WHERE y.variant_id = e.variant_id)
                  AS "yields!"
           FROM plan_entries e WHERE e.id = $1 AND e.household_id = $2"#,
        id,
        household
    )
    .fetch_optional(db)
    .await
}

/// Plans entry `id` for `servings`; answers with its week, or none when the entry isn't the household's.
pub async fn set_servings(
    db: impl PgExecutor<'_>,
    id: Uuid,
    household: Uuid,
    servings: f64,
) -> sqlx::Result<Option<Date>> {
    sqlx::query_scalar!(
        "UPDATE plan_entries SET servings = $3::float8::numeric WHERE id = $1 AND household_id = $2 RETURNING week",
        id,
        household,
        servings
    )
    .fetch_optional(db)
    .await
}

/// Takes entry `id` off the plan; answers with its week, or none when the entry isn't the household's.
pub async fn remove(
    db: impl PgExecutor<'_>,
    id: Uuid,
    household: Uuid,
) -> sqlx::Result<Option<Date>> {
    sqlx::query_scalar!(
        "DELETE FROM plan_entries WHERE id = $1 AND household_id = $2 RETURNING week",
        id,
        household
    )
    .fetch_optional(db)
    .await
}
