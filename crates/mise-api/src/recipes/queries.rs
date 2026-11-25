//! The recipe editor's SQL, each statement checked against the schema when mise is built. Everything here runs inside
//! the caller's transaction, so a recipe is written whole or not at all.

use serde_json::Value;
use sqlx::{PgConnection, PgExecutor};
use uuid::Uuid;

use super::{Checked, SAVING_LOCK};
use crate::sharing::Visibility;

/// Waits for any other save to finish; held until the transaction ends.
pub async fn take_turn(db: impl PgExecutor<'_>) -> sqlx::Result<()> {
    sqlx::query!("SELECT pg_advisory_xact_lock($1)", SAVING_LOCK)
        .execute(db)
        .await?;
    Ok(())
}

/// Which of `foods` the household may know of (`known_food`, in the database beside `visible_to`).
pub async fn known_foods(
    db: impl PgExecutor<'_>,
    household: Uuid,
    foods: &[Uuid],
) -> sqlx::Result<Vec<Uuid>> {
    sqlx::query_scalar!(
        "SELECT f.id FROM foods f WHERE f.id = ANY($2) AND known_food(f, $1)",
        household,
        foods
    )
    .fetch_all(db)
    .await
}

/// A food a name in an ingredient line finds.
pub struct AliasRow {
    /// The name, as the matcher keys it.
    pub alias: String,
    /// The food.
    pub id: Uuid,
    /// The food's name.
    pub name: String,
}

/// The foods `keys` name, of those the household may know of.
pub async fn foods_by_alias(
    db: impl PgExecutor<'_>,
    household: Uuid,
    keys: &[String],
) -> sqlx::Result<Vec<AliasRow>> {
    sqlx::query_as!(
        AliasRow,
        "SELECT a.alias, f.id, f.name FROM food_aliases a JOIN foods f ON f.id = a.food_id
         WHERE a.alias = ANY($2) AND known_food(f, $1)",
        household,
        keys
    )
    .fetch_all(db)
    .await
}

/// The addresses taken by `base` and its numbered forms ("base-2"), now or before, by dishes other than `dish`. A
/// slug holds letters, digits and hyphens only, so it is safe as a LIKE pattern.
pub async fn taken_slugs(
    db: impl PgExecutor<'_>,
    base: &str,
    dish: Option<Uuid>,
) -> sqlx::Result<Vec<String>> {
    sqlx::query_scalar!(
        r#"SELECT slug AS "slug!" FROM dishes WHERE (slug = $1 OR slug LIKE $1 || '-%') AND id IS DISTINCT FROM $2
           UNION
           SELECT slug FROM retired_slugs WHERE (slug = $1 OR slug LIKE $1 || '-%') AND dish_id IS DISTINCT FROM $2"#,
        base,
        dish
    )
    .fetch_all(db)
    .await
}

/// Adds the dish a new recipe opens on.
pub async fn add_dish(
    db: impl PgExecutor<'_>,
    dish: Uuid,
    recipe: &Checked,
    slug: &str,
    variant: Uuid,
) -> sqlx::Result<()> {
    sqlx::query!(
        "INSERT INTO dishes (id, title, subtitle, slug, content_type, default_variant_id, origin)
         VALUES ($1, $2, $3, $4, 'recipe', $5, 'user')",
        dish,
        &recipe.title,
        recipe.subtitle.as_deref(),
        slug,
        variant
    )
    .execute(db)
    .await?;
    Ok(())
}

/// Adds recipe `variant`, written by the household, on `dish`.
pub async fn add_recipe(
    db: impl PgExecutor<'_>,
    variant: Uuid,
    dish: Uuid,
    household: Uuid,
    recipe: &Checked,
    visibility: Visibility,
) -> sqlx::Result<()> {
    sqlx::query!(
        "INSERT INTO variants (id, dish_id, dish_pinned, origin, household_id, title, subtitle, description,
                               published_at, has_steps, meta, content_hash, visibility)
         VALUES ($1, $2, true, 'user', $3, $4, $5, $6, now(), $7, '{\"content_type\": \"recipe\"}', $8, $9)",
        variant,
        dish,
        household,
        &recipe.title,
        recipe.subtitle.as_deref(),
        recipe.description.as_deref(),
        !recipe.steps.is_empty(),
        recipe.content_hash(),
        visibility.as_str()
    )
    .execute(db)
    .await?;
    Ok(())
}

/// Adds `copy`, the household's own copy of version `original`, on `dish`. Pinned to the dish, so grouping leaves it
/// where it was made, as it leaves a written recipe.
#[allow(clippy::too_many_arguments)] // one per column the copy takes from the caller
pub async fn add_copy(
    db: impl PgExecutor<'_>,
    copy: Uuid,
    dish: Uuid,
    original: Uuid,
    household: Uuid,
    recipe: &Checked,
    meta: Value,
    visibility: Visibility,
) -> sqlx::Result<()> {
    sqlx::query!(
        "INSERT INTO variants (id, dish_id, dish_pinned, origin, parent_variant_id, household_id, title, subtitle,
                               description, published_at, has_steps, meta, content_hash, visibility)
         VALUES ($1, $2, true, 'user', $3, $4, $5, $6, $7, now(), $8, $9, $10, $11)",
        copy,
        dish,
        original,
        household,
        &recipe.title,
        recipe.subtitle.as_deref(),
        recipe.description.as_deref(),
        !recipe.steps.is_empty(),
        meta,
        recipe.content_hash(),
        visibility.as_str()
    )
    .execute(db)
    .await?;
    Ok(())
}

/// Copies version `from`'s option values and photo to `to`.
pub async fn copy_attributes_and_photo(
    conn: &mut PgConnection,
    from: Uuid,
    to: Uuid,
) -> sqlx::Result<()> {
    sqlx::query!(
        "INSERT INTO variant_attributes (variant_id, key, value)
         SELECT $2, key, value FROM variant_attributes WHERE variant_id = $1",
        from,
        to
    )
    .execute(&mut *conn)
    .await?;
    sqlx::query!(
        "INSERT INTO variant_media (variant_id, media_id, role, position)
         SELECT $2, media_id, 'hero', 0 FROM variant_media WHERE variant_id = $1 AND role = 'hero' LIMIT 1",
        from,
        to
    )
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// Replaces recipe `id`'s own fields; its visibility only when one is given.
pub async fn update_recipe(
    db: impl PgExecutor<'_>,
    id: Uuid,
    recipe: &Checked,
    visibility: Option<Visibility>,
) -> sqlx::Result<()> {
    sqlx::query!(
        "UPDATE variants SET title = $2, subtitle = $3, description = $4, has_steps = $5, content_hash = $6,
                            visibility = coalesce($7, visibility)
         WHERE id = $1",
        id,
        &recipe.title,
        recipe.subtitle.as_deref(),
        recipe.description.as_deref(),
        !recipe.steps.is_empty(),
        recipe.content_hash(),
        visibility.map(Visibility::as_str)
    )
    .execute(db)
    .await?;
    Ok(())
}

/// Takes recipe `id`'s yields, lines and steps away, to be written again. Amounts go with their lines, and a step's
/// photos with the step.
pub async fn clear_parts(conn: &mut PgConnection, id: Uuid) -> sqlx::Result<()> {
    sqlx::query!("DELETE FROM variant_yields WHERE variant_id = $1", id)
        .execute(&mut *conn)
        .await?;
    sqlx::query!("DELETE FROM ingredient_lines WHERE variant_id = $1", id)
        .execute(&mut *conn)
        .await?;
    sqlx::query!("DELETE FROM steps WHERE variant_id = $1", id)
        .execute(&mut *conn)
        .await?;
    Ok(())
}

/// Adds that a version has amounts for `servings`.
pub async fn add_yield(db: impl PgExecutor<'_>, variant: Uuid, servings: f64) -> sqlx::Result<()> {
    sqlx::query!(
        "INSERT INTO variant_yields (variant_id, servings) VALUES ($1, $2::float8::numeric)",
        variant,
        servings
    )
    .execute(db)
    .await?;
    Ok(())
}

/// An ingredient line to add.
pub struct NewLine<'a> {
    /// The line's id.
    pub id: Uuid,
    /// Its version.
    pub variant: Uuid,
    /// Its key, its place from 1.
    pub key: &'a str,
    /// Its place, from 0.
    pub position: i32,
    /// The ingredient's name.
    pub name: &'a str,
    /// The food chosen for it.
    pub food_id: Option<Uuid>,
    /// What follows the name ("finely chopped").
    pub note: Option<&'a str>,
    /// The line as typed.
    pub text: &'a str,
}

/// Adds an ingredient line.
pub async fn add_line(db: impl PgExecutor<'_>, line: NewLine<'_>) -> sqlx::Result<()> {
    sqlx::query!(
        "INSERT INTO ingredient_lines (id, variant_id, line_key, position, raw_name, food_id, note, text)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        line.id,
        line.variant,
        line.key,
        line.position,
        line.name,
        line.food_id,
        line.note,
        line.text
    )
    .execute(db)
    .await?;
    Ok(())
}

/// Adds a line's amount at `servings`.
pub async fn add_amount(
    db: impl PgExecutor<'_>,
    line: Uuid,
    servings: f64,
    amount: f64,
    unit: Option<String>,
) -> sqlx::Result<()> {
    sqlx::query!(
        "INSERT INTO line_amounts (line_id, servings, amount, unit)
         VALUES ($1, $2::float8::numeric, $3::float8::numeric, $4)",
        line,
        servings,
        amount,
        unit
    )
    .execute(db)
    .await?;
    Ok(())
}

/// Adds a step, with the template its amounts scale by.
pub async fn add_step(
    db: impl PgExecutor<'_>,
    variant: Uuid,
    position: i32,
    text: &str,
    template: Value,
) -> sqlx::Result<()> {
    sqlx::query!(
        "INSERT INTO steps (id, variant_id, position, text, template) VALUES ($1, $2, $3, $4, $5)",
        Uuid::now_v7(),
        variant,
        position,
        text,
        template
    )
    .execute(db)
    .await?;
    Ok(())
}

/// A line's facts the editor doesn't show.
pub struct FactsRow {
    /// The line's food.
    pub food_id: Option<Uuid>,
    /// The ingredient's name as written.
    pub raw_name: String,
    /// Whether the kit shipped it.
    pub shipped: Option<bool>,
    /// What it contains that some can't eat.
    pub allergens: Vec<String>,
}

/// The facts of version `variant`'s lines that have any, in order.
pub async fn line_facts(db: impl PgExecutor<'_>, variant: Uuid) -> sqlx::Result<Vec<FactsRow>> {
    sqlx::query_as!(
        FactsRow,
        "SELECT food_id, raw_name, shipped, allergens FROM ingredient_lines
         WHERE variant_id = $1 AND (shipped IS NOT NULL OR allergens <> '{}') ORDER BY position",
        variant
    )
    .fetch_all(db)
    .await
}

/// A line, by what it names.
pub struct NamedLine {
    /// The line's id.
    pub id: Uuid,
    /// Its food.
    pub food_id: Option<Uuid>,
    /// The ingredient's name as written.
    pub raw_name: String,
}

/// Version `variant`'s lines.
pub async fn lines(db: impl PgExecutor<'_>, variant: Uuid) -> sqlx::Result<Vec<NamedLine>> {
    sqlx::query_as!(
        NamedLine,
        "SELECT id, food_id, raw_name FROM ingredient_lines WHERE variant_id = $1",
        variant
    )
    .fetch_all(db)
    .await
}

/// Sets line `id`'s facts.
pub async fn set_line_facts(
    db: impl PgExecutor<'_>,
    id: Uuid,
    shipped: Option<bool>,
    allergens: &[String],
) -> sqlx::Result<()> {
    sqlx::query!(
        "UPDATE ingredient_lines SET shipped = $2, allergens = $3 WHERE id = $1",
        id,
        shipped,
        allergens
    )
    .execute(db)
    .await?;
    Ok(())
}

/// A recipe's own fields.
pub struct RecipeRow {
    /// Its title.
    pub title: String,
    /// Its subtitle.
    pub subtitle: Option<String>,
    /// Its description.
    pub description: Option<String>,
    /// Its dish's slug.
    pub dish_slug: Option<String>,
    /// The serving count it was written for.
    pub servings: Option<f64>,
    /// Its photo.
    pub hero: Option<Uuid>,
    /// Who may see it, as stored.
    pub visibility: String,
}

/// Recipe `id`, when the household wrote it in the app.
pub async fn recipe(
    db: impl PgExecutor<'_>,
    id: Uuid,
    household: Uuid,
) -> sqlx::Result<Option<RecipeRow>> {
    sqlx::query_as!(
        RecipeRow,
        r#"SELECT v.title, v.subtitle, v.description, d.slug AS "dish_slug?",
                  (SELECT min(y.servings)::float8 FROM variant_yields y WHERE y.variant_id = v.id) AS servings,
                  (SELECT vm.media_id FROM variant_media vm WHERE vm.variant_id = v.id AND vm.role = 'hero'
                   LIMIT 1) AS hero,
                  v.visibility
           FROM variants v LEFT JOIN dishes d ON d.id = v.dish_id
           WHERE v.id = $1 AND v.origin = 'user' AND v.household_id = $2"#,
        id,
        household
    )
    .fetch_optional(db)
    .await
}

/// One of a recipe's lines, as the editor shows it.
pub struct RecipeLineRow {
    /// The line as typed.
    pub text: Option<String>,
    /// The ingredient's name.
    pub raw_name: String,
    /// What follows the name.
    pub note: Option<String>,
    /// How much.
    pub amount: Option<f64>,
    /// Its unit.
    pub unit: Option<String>,
    /// The food it is matched to.
    pub food_id: Option<Uuid>,
    /// The food's name.
    pub food: Option<String>,
}

/// Recipe `id`'s lines, in order.
pub async fn recipe_lines(db: impl PgExecutor<'_>, id: Uuid) -> sqlx::Result<Vec<RecipeLineRow>> {
    sqlx::query_as!(
        RecipeLineRow,
        r#"SELECT il.text, il.raw_name, il.note, la.amount::float8 AS "amount?", la.unit AS "unit?",
                  f.id AS "food_id?", f.name AS "food?"
           FROM ingredient_lines il
           LEFT JOIN line_amounts la ON la.line_id = il.id
           LEFT JOIN foods f ON f.id = il.food_id
           WHERE il.variant_id = $1 ORDER BY il.position"#,
        id
    )
    .fetch_all(db)
    .await
}

/// Version `id`'s steps, in order.
pub async fn steps(db: impl PgExecutor<'_>, id: Uuid) -> sqlx::Result<Vec<String>> {
    sqlx::query_scalar!(
        "SELECT text FROM steps WHERE variant_id = $1 ORDER BY position",
        id
    )
    .fetch_all(db)
    .await
}

/// The dish that follows a recipe: one a person made that opens on it.
pub struct OwnDish {
    /// The dish.
    pub id: Option<Uuid>,
    /// Its slug.
    pub slug: Option<String>,
}

/// Locks recipe `id` when the household wrote it, and answers with its own dish, if it has one.
pub async fn lock_recipe(
    db: impl PgExecutor<'_>,
    id: Uuid,
    household: Uuid,
) -> sqlx::Result<Option<OwnDish>> {
    sqlx::query_as!(
        OwnDish,
        r#"SELECT d.id AS "id?", d.slug AS "slug?"
           FROM variants v
           LEFT JOIN dishes d ON d.id = v.dish_id AND d.origin = 'user' AND d.default_variant_id = v.id
           WHERE v.id = $1 AND v.origin = 'user' AND v.household_id = $2 FOR UPDATE OF v"#,
        id,
        household
    )
    .fetch_optional(db)
    .await
}

/// Gives `dish` the recipe's title and the address `slug`.
pub async fn rename_dish(
    db: impl PgExecutor<'_>,
    dish: Uuid,
    recipe: &Checked,
    slug: &str,
) -> sqlx::Result<()> {
    sqlx::query!(
        "UPDATE dishes SET title = $2, subtitle = $3, slug = $4, updated_at = now() WHERE id = $1",
        dish,
        &recipe.title,
        recipe.subtitle.as_deref(),
        slug
    )
    .execute(db)
    .await?;
    Ok(())
}

/// Keeps `old` as a former address of `dish`, and `new`, the one it holds again, as no longer a former one.
pub async fn retire_slug(
    conn: &mut PgConnection,
    dish: Uuid,
    old: &str,
    new: &str,
) -> sqlx::Result<()> {
    sqlx::query!(
        "INSERT INTO retired_slugs (slug, dish_id) VALUES ($1, $2)
         ON CONFLICT (slug) DO UPDATE SET dish_id = excluded.dish_id, retired_at = now()",
        old,
        dish
    )
    .execute(&mut *conn)
    .await?;
    sqlx::query!("DELETE FROM retired_slugs WHERE slug = $1", new)
        .execute(&mut *conn)
        .await?;
    Ok(())
}

/// The version a copy is made from.
pub struct SourceRow {
    /// Its dish.
    pub dish_id: Uuid,
    /// Its title.
    pub title: String,
    /// Its subtitle.
    pub subtitle: Option<String>,
    /// Its description.
    pub description: Option<String>,
    /// What else it says about itself, without nutrition, which stops being true at the first change.
    pub meta: Value,
    /// The smallest serving count it has amounts for.
    pub servings: Option<f64>,
    /// The household it belongs to; none for a published one.
    pub household_id: Option<Uuid>,
}

/// Version `id`, when the household can see it and it is on a dish.
pub async fn source(
    db: impl PgExecutor<'_>,
    id: Uuid,
    household: Uuid,
) -> sqlx::Result<Option<SourceRow>> {
    sqlx::query_as!(
        SourceRow,
        r#"SELECT v.dish_id AS "dish_id!", v.title, v.subtitle, v.description,
                  v.meta - 'nutrition' AS "meta!",
                  (SELECT min(y.servings)::float8 FROM variant_yields y WHERE y.variant_id = v.id) AS servings,
                  v.household_id
           FROM variants v WHERE v.id = $1 AND v.dish_id IS NOT NULL AND visible_to(v, $2)"#,
        id,
        household
    )
    .fetch_optional(db)
    .await
}

/// A line of the version a copy is made from, at the serving count the copy is written for.
pub struct SourceLine {
    /// The ingredient's name.
    pub raw_name: String,
    /// What follows the name.
    pub note: Option<String>,
    /// The food it is matched to.
    pub food_id: Option<Uuid>,
    /// The line as typed or read, when it was.
    pub text: Option<String>,
    /// How much, at that serving count.
    pub amount: Option<f64>,
    /// Its unit.
    pub unit: Option<String>,
}

/// Version `id`'s lines, with their amounts at `servings`.
pub async fn source_lines(
    db: impl PgExecutor<'_>,
    id: Uuid,
    servings: f64,
) -> sqlx::Result<Vec<SourceLine>> {
    sqlx::query_as!(
        SourceLine,
        r#"SELECT il.raw_name, il.note, il.food_id, il.text, la.amount::float8 AS "amount?", la.unit AS "unit?"
           FROM ingredient_lines il
           LEFT JOIN line_amounts la ON la.line_id = il.id AND la.servings = $2::float8::numeric
           WHERE il.variant_id = $1 ORDER BY il.position"#,
        id,
        servings
    )
    .fetch_all(db)
    .await
}

/// Locks recipe `id` when the household wrote it, and answers with its dish: none for no such recipe, `Some(None)`
/// for one on no dish.
pub async fn lock_for_delete(
    db: impl PgExecutor<'_>,
    id: Uuid,
    household: Uuid,
) -> sqlx::Result<Option<Option<Uuid>>> {
    sqlx::query_scalar!(
        "SELECT dish_id FROM variants WHERE id = $1 AND origin = 'user' AND household_id = $2 FOR UPDATE",
        id,
        household
    )
    .fetch_optional(db)
    .await
}

/// Deletes recipe `id`, on `dish`. Its yields, lines, amounts, steps, photo links and plan entries go with it; a
/// version copied from it stays, no longer naming it. A dish a person made goes with its last version. Any other
/// dish that opened on this version opens on the best of what is left, as the dish page would choose for a household
/// that couldn't see its default.
pub async fn delete_recipe(
    conn: &mut PgConnection,
    id: Uuid,
    dish: Option<Uuid>,
) -> sqlx::Result<()> {
    sqlx::query!(
        "UPDATE variants SET parent_variant_id = NULL WHERE parent_variant_id = $1",
        id
    )
    .execute(&mut *conn)
    .await?;
    sqlx::query!("DELETE FROM variants WHERE id = $1", id)
        .execute(&mut *conn)
        .await?;
    let Some(dish) = dish else {
        return Ok(());
    };
    sqlx::query!(
        "DELETE FROM dishes d WHERE d.id = $1 AND d.origin = 'user'
           AND NOT EXISTS (SELECT 1 FROM variants v WHERE v.dish_id = d.id)",
        dish
    )
    .execute(&mut *conn)
    .await?;
    sqlx::query!(
        "UPDATE dishes d SET default_variant_id = (
             SELECT v.id FROM variants v WHERE v.dish_id = d.id
             ORDER BY v.household_id IS NULL DESC, v.has_steps DESC, v.published_at DESC NULLS LAST, v.id LIMIT 1)
         WHERE d.id = $1 AND d.default_variant_id = $2",
        dish,
        id
    )
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// Whether the household wrote recipe `id` in the app.
pub async fn is_own(db: impl PgExecutor<'_>, id: Uuid, household: Uuid) -> sqlx::Result<bool> {
    sqlx::query_scalar!(
        r#"SELECT EXISTS (SELECT 1 FROM variants WHERE id = $1 AND origin = 'user' AND household_id = $2)
                  AS "own!""#,
        id,
        household
    )
    .fetch_one(db)
    .await
}

/// Makes `media` recipe `id`'s photo, in place of any it had.
pub async fn set_photo(conn: &mut PgConnection, id: Uuid, media: Uuid) -> sqlx::Result<()> {
    remove_photo(&mut *conn, id).await?;
    sqlx::query!(
        "INSERT INTO variant_media (variant_id, media_id, role, position) VALUES ($1, $2, 'hero', 0)",
        id,
        media
    )
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// Takes recipe `id`'s photo off.
pub async fn remove_photo(db: impl PgExecutor<'_>, id: Uuid) -> sqlx::Result<()> {
    sqlx::query!(
        "DELETE FROM variant_media WHERE variant_id = $1 AND role = 'hero'",
        id
    )
    .execute(db)
    .await?;
    Ok(())
}
