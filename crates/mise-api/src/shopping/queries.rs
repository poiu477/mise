//! The shopping list's SQL, each statement checked against the schema when mise is built.

use serde_json::Value;
use sqlx::PgExecutor;
use time::Date;
use uuid::Uuid;

use super::ShoppingExtra;

/// A dish planned for the week, as the list counts it.
pub struct PlannedRow {
    /// The version planned.
    pub variant_id: Uuid,
    /// How many it is planned for.
    pub servings: f64,
    /// The dish's title, or the version's when it has no dish.
    pub dish: String,
}

/// A week's planned dishes in the order they were added, with only the versions the household can see.
pub async fn planned(
    db: impl PgExecutor<'_>,
    household: Uuid,
    week: Date,
) -> sqlx::Result<Vec<PlannedRow>> {
    sqlx::query_as!(
        PlannedRow,
        r#"SELECT e.variant_id, e.servings::float8 AS "servings!", coalesce(d.title, v.title) AS "dish!"
           FROM plan_entries e
           JOIN variants v ON v.id = e.variant_id LEFT JOIN dishes d ON d.id = v.dish_id
           WHERE e.household_id = $1 AND e.week = $2 AND visible_to(v, $1) ORDER BY e.created_at, e.id"#,
        household,
        week
    )
    .fetch_all(db)
    .await
}

/// An ingredient line of a planned version.
pub struct LineRow {
    /// The line's id.
    pub id: Uuid,
    /// Its version.
    pub variant_id: Uuid,
    /// The ingredient's name as written.
    pub raw_name: String,
    /// The food it is matched to.
    pub food_id: Option<Uuid>,
    /// Whether the kit brought it; false for a pantry line.
    pub shipped: Option<bool>,
}

/// The lines of `variants`, in each version's order.
pub async fn lines(db: impl PgExecutor<'_>, variants: &[Uuid]) -> sqlx::Result<Vec<LineRow>> {
    sqlx::query_as!(
        LineRow,
        "SELECT il.id, il.variant_id, il.raw_name, il.food_id, il.shipped
         FROM ingredient_lines il WHERE il.variant_id = ANY($1) ORDER BY il.position",
        variants
    )
    .fetch_all(db)
    .await
}

/// A line's amount at one serving count.
pub struct AmountRow {
    /// The line.
    pub line_id: Uuid,
    /// The serving count.
    pub servings: f64,
    /// How much.
    pub amount: f64,
    /// Its unit, as written.
    pub unit: Option<String>,
}

/// The amounts of the lines of `variants`, leaving out those of nothing.
pub async fn amounts(db: impl PgExecutor<'_>, variants: &[Uuid]) -> sqlx::Result<Vec<AmountRow>> {
    sqlx::query_as!(
        AmountRow,
        r#"SELECT la.line_id, la.servings::float8 AS "servings!", la.amount::float8 AS "amount!", la.unit
           FROM line_amounts la JOIN ingredient_lines il ON il.id = la.line_id
           WHERE il.variant_id = ANY($1) AND la.amount > 0"#,
        variants
    )
    .fetch_all(db)
    .await
}

/// A serving count a version has amounts for.
pub struct YieldRow {
    /// The version.
    pub variant_id: Uuid,
    /// The serving count.
    pub servings: f64,
}

/// The serving counts `variants` have amounts for.
pub async fn yields(db: impl PgExecutor<'_>, variants: &[Uuid]) -> sqlx::Result<Vec<YieldRow>> {
    sqlx::query_as!(
        YieldRow,
        r#"SELECT variant_id, servings::float8 AS "servings!" FROM variant_yields WHERE variant_id = ANY($1)"#,
        variants
    )
    .fetch_all(db)
    .await
}

/// A step's amounts template.
pub struct TemplateRow {
    /// The step's version.
    pub variant_id: Uuid,
    /// The template, as stored.
    pub template: Value,
}

/// The steps of `variants` that have an amounts template.
pub async fn templates(
    db: impl PgExecutor<'_>,
    variants: &[Uuid],
) -> sqlx::Result<Vec<TemplateRow>> {
    sqlx::query_as!(
        TemplateRow,
        r#"SELECT variant_id, template AS "template!"
           FROM steps WHERE variant_id = ANY($1) AND template IS NOT NULL"#,
        variants
    )
    .fetch_all(db)
    .await
}

/// A food as the list needs it.
pub struct FoodRow {
    /// The food.
    pub id: Uuid,
    /// The food it is a kind of.
    pub parent_id: Option<Uuid>,
    /// A shelf rather than a food.
    pub category: bool,
    /// Its kinds are bought as one.
    pub shop_as_one: bool,
    /// Its name.
    pub name: String,
    /// The word it is counted in ("clove").
    pub count_unit: Option<String>,
    /// How much one of it is.
    pub count_size: Option<String>,
    /// The section of the shop it is in.
    pub section: Option<String>,
}

/// `foods` and every food they are kinds of.
pub async fn food_tree(db: impl PgExecutor<'_>, foods: &[Uuid]) -> sqlx::Result<Vec<FoodRow>> {
    sqlx::query_as!(
        FoodRow,
        r#"WITH RECURSIVE up AS (
               SELECT id FROM foods WHERE id = ANY($1)
               UNION SELECT f.parent_id FROM foods f JOIN up ON f.id = up.id WHERE f.parent_id IS NOT NULL
           )
           SELECT f.id AS "id!", f.parent_id,
                  f.kind = 'category' AS "category!",
                  coalesce(f.props->'shop_as_one' = 'true'::jsonb, false) AS "shop_as_one!",
                  f.name AS "name!",
                  f.props->>'count_unit' AS count_unit,
                  f.props->>'count_size' AS count_size,
                  f.props->>'store_section' AS section
           FROM foods f JOIN up ON up.id = f.id"#,
        foods
    )
    .fetch_all(db)
    .await
}

/// A food's density, or the size of one of it.
pub struct MeasureRow {
    /// The food.
    pub food_id: Uuid,
    /// "density" or "item".
    pub kind: String,
    /// What one is ("clove"), for an item.
    pub item: String,
    /// Its volume.
    pub ml: Option<f64>,
    /// Its weight.
    pub grams: Option<f64>,
}

/// The measures of `foods`, oldest first.
pub async fn measures(db: impl PgExecutor<'_>, foods: &[Uuid]) -> sqlx::Result<Vec<MeasureRow>> {
    sqlx::query_as!(
        MeasureRow,
        r#"SELECT food_id, kind, item, ml::float8 AS ml, grams::float8 AS grams
           FROM food_measures WHERE food_id = ANY($1) ORDER BY created_at, id"#,
        foods
    )
    .fetch_all(db)
    .await
}

/// The foods the household always has.
pub async fn staples(db: impl PgExecutor<'_>, household: Uuid) -> sqlx::Result<Vec<Uuid>> {
    sqlx::query_scalar!(
        "SELECT food_id FROM pantry_staples WHERE household_id = $1",
        household
    )
    .fetch_all(db)
    .await
}

/// A food ticked or taken off a week's list.
pub struct MarkRow {
    /// The food's key.
    pub item_key: String,
    /// Ticked off.
    pub checked: bool,
    /// Taken off the list.
    pub hidden: bool,
}

/// The week's marks.
pub async fn marks(
    db: impl PgExecutor<'_>,
    household: Uuid,
    week: Date,
) -> sqlx::Result<Vec<MarkRow>> {
    sqlx::query_as!(
        MarkRow,
        "SELECT item_key, checked, hidden FROM shopping_marks WHERE household_id = $1 AND week = $2",
        household,
        week
    )
    .fetch_all(db)
    .await
}

/// Marks each of `foods` in `week`: ticked, or taken off; a mark left out keeps what it was.
pub async fn mark(
    db: impl PgExecutor<'_>,
    household: Uuid,
    week: Date,
    foods: &[String],
    checked: Option<bool>,
    hidden: Option<bool>,
) -> sqlx::Result<()> {
    sqlx::query!(
        "INSERT INTO shopping_marks (household_id, week, item_key, checked, hidden)
         SELECT $1, $2, k, coalesce($4, false), coalesce($5, false) FROM UNNEST($3::text[]) AS k
         ON CONFLICT (household_id, week, item_key)
         DO UPDATE SET checked = coalesce($4, shopping_marks.checked), hidden = coalesce($5, shopping_marks.hidden)",
        household,
        week,
        foods,
        checked,
        hidden
    )
    .execute(db)
    .await?;
    Ok(())
}

/// How the household reads weights, as stored.
pub async fn weight_unit(db: impl PgExecutor<'_>, household: Uuid) -> sqlx::Result<String> {
    sqlx::query_scalar!(
        "SELECT weight_unit FROM households WHERE id = $1",
        household
    )
    .fetch_one(db)
    .await
}

/// Sets how the household reads weights.
pub async fn set_weight_unit(
    db: impl PgExecutor<'_>,
    household: Uuid,
    unit: &str,
) -> sqlx::Result<()> {
    sqlx::query!(
        "UPDATE households SET weight_unit = $2 WHERE id = $1",
        household,
        unit
    )
    .execute(db)
    .await?;
    Ok(())
}

/// How one food's weight reads for a household, where it isn't the household's usual way.
pub struct FoodWeightRow {
    /// The food.
    pub food_id: Uuid,
    /// The weight unit, as stored.
    pub weight_unit: String,
}

/// The foods whose weights read the household's own way.
pub async fn food_weight_units(
    db: impl PgExecutor<'_>,
    household: Uuid,
) -> sqlx::Result<Vec<FoodWeightRow>> {
    sqlx::query_as!(
        FoodWeightRow,
        "SELECT food_id, weight_unit FROM food_weight_units WHERE household_id = $1",
        household
    )
    .fetch_all(db)
    .await
}

/// Sets how `food`'s weight reads for the household; none goes back to its usual way.
pub async fn set_food_weight_unit(
    db: impl PgExecutor<'_>,
    household: Uuid,
    food: Uuid,
    unit: Option<&str>,
) -> sqlx::Result<()> {
    match unit {
        Some(unit) => sqlx::query!(
            "INSERT INTO food_weight_units (household_id, food_id, weight_unit) VALUES ($1, $2, $3)
             ON CONFLICT (household_id, food_id) DO UPDATE SET weight_unit = $3",
            household,
            food,
            unit
        ),
        None => sqlx::query!(
            "DELETE FROM food_weight_units WHERE household_id = $1 AND food_id = $2",
            household,
            food
        ),
    }
    .execute(db)
    .await?;
    Ok(())
}

/// The week's hand-added items, oldest first.
pub async fn extras(
    db: impl PgExecutor<'_>,
    household: Uuid,
    week: Date,
) -> sqlx::Result<Vec<ShoppingExtra>> {
    sqlx::query_as!(
        ShoppingExtra,
        "SELECT id, name, checked, client_key FROM shopping_extras
         WHERE household_id = $1 AND week = $2 ORDER BY created_at, id",
        household,
        week
    )
    .fetch_all(db)
    .await
}

/// Adds `name` to the week's list by hand; nothing when `key` was used before in the household. Two missing keys
/// never conflict, so additions without one always go in.
pub async fn add_extra(
    db: impl PgExecutor<'_>,
    household: Uuid,
    week: Date,
    name: &str,
    key: Option<&str>,
) -> sqlx::Result<()> {
    sqlx::query!(
        "INSERT INTO shopping_extras (id, household_id, week, name, client_key) VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT (household_id, client_key) DO NOTHING",
        Uuid::now_v7(),
        household,
        week,
        name,
        key
    )
    .execute(db)
    .await?;
    Ok(())
}

/// Ticks hand-added item `id` on or off; answers with its week, or none when it isn't the household's.
pub async fn check_extra(
    db: impl PgExecutor<'_>,
    id: Uuid,
    household: Uuid,
    checked: bool,
) -> sqlx::Result<Option<Date>> {
    sqlx::query_scalar!(
        "UPDATE shopping_extras SET checked = $3 WHERE id = $1 AND household_id = $2 RETURNING week",
        id,
        household,
        checked
    )
    .fetch_optional(db)
    .await
}

/// Deletes hand-added item `id`; answers with its week, or none when it isn't the household's.
pub async fn remove_extra(
    db: impl PgExecutor<'_>,
    id: Uuid,
    household: Uuid,
) -> sqlx::Result<Option<Date>> {
    sqlx::query_scalar!(
        "DELETE FROM shopping_extras WHERE id = $1 AND household_id = $2 RETURNING week",
        id,
        household
    )
    .fetch_optional(db)
    .await
}

/// Leaves `food` off the household's lists; answers whether a staple was added (not when the food doesn't exist or
/// already is one).
pub async fn add_staple(
    db: impl PgExecutor<'_>,
    household: Uuid,
    food: Uuid,
) -> sqlx::Result<bool> {
    let added = sqlx::query!(
        "INSERT INTO pantry_staples (household_id, food_id) SELECT $1, f.id FROM foods f WHERE f.id = $2
         ON CONFLICT DO NOTHING",
        household,
        food
    )
    .execute(db)
    .await?;
    Ok(added.rows_affected() > 0)
}

/// Puts `food` back on the household's lists.
pub async fn remove_staple(
    db: impl PgExecutor<'_>,
    household: Uuid,
    food: Uuid,
) -> sqlx::Result<()> {
    sqlx::query!(
        "DELETE FROM pantry_staples WHERE household_id = $1 AND food_id = $2",
        household,
        food
    )
    .execute(db)
    .await?;
    Ok(())
}

/// Whether food `id` exists.
pub async fn food_exists(db: impl PgExecutor<'_>, id: Uuid) -> sqlx::Result<bool> {
    sqlx::query_scalar!(
        r#"SELECT EXISTS (SELECT 1 FROM foods WHERE id = $1) AS "exists!""#,
        id
    )
    .fetch_one(db)
    .await
}
