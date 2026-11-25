//! Recipes a household writes itself: create one, read it back as it was written, change it, give it a photo,
//! delete it.
//!
//! A written recipe is a version like any other (`origin = 'user'`, owned by the household, seen as far as its
//! visibility says), so the dish page, search, the plan and the shopping list treat it as they treat a seed version.
//! Only its household changes it. It gets a dish of its own, marked as made by a person, which grouping leaves alone.
//! Ingredient lines arrive as text ("2 cups flour, sifted") and are read into an amount, a unit, a name and a note
//! (`mise_core::ingredient_line`); each name is matched to a food exactly as an import's is, unless the request names
//! the food. The line's text is kept, so reading the recipe back gives what was typed.
//!
//! A household can also make its own version of any version it can see (a fork): a written recipe like these, on
//! the original's dish rather than one of its own, naming the original as its parent.

use std::collections::HashMap;

use axum::{
    Extension, Json,
    body::Bytes,
    extract::{FromRequest, Request, State},
    http::{StatusCode, header},
};
use mise_core::{
    food_names::singular_key,
    ingredient_line::{self, Line},
    quantity::Unit,
    text::slugify,
};
use mise_ingest::media::{MAX_UPLOAD_PIXELS, UploadError};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use sqlx::{Postgres, Transaction};
use tokio::sync::Semaphore;
use utoipa::ToSchema;
use uuid::Uuid;

mod queries;

use crate::{
    AppState,
    auth::CurrentUser,
    error::{ApiError, ApiResult},
    extract::{JsonBody, Path},
    media::Image,
    sharing::{self, Visibility},
};

/// The longest title and subtitle, in characters.
const MAX_TITLE: usize = 200;
/// The longest description and the longest step, in characters.
const MAX_TEXT: usize = 4000;
/// The longest ingredient line, in characters.
const MAX_LINE: usize = 300;
/// The most ingredient lines a recipe can have.
const MAX_LINES: usize = 200;
/// The most steps a recipe can have.
const MAX_STEPS: usize = 100;
/// The most servings a recipe can be written for.
const MAX_SERVINGS: f64 = 1000.0;
/// The largest photo the server reads: a phone's full-size photo fits with room to spare.
pub const MAX_PHOTO_BYTES: usize = 20 * 1024 * 1024;

/// Photos being decoded at once. Decoding takes a few bytes of memory for every pixel, so uploads take turns
/// rather than letting a burst of them use it all.
static DECODING: Semaphore = Semaphore::const_new(2);

/// The lock a save holds until its transaction ends: saves take turns with each other and with grouping, so two
/// dishes can't both take the same address.
const SAVING_LOCK: i64 = mise_ingest::dishes::DISH_LOCK;

/// One ingredient line as typed.
#[derive(Deserialize, ToSchema)]
pub struct LineInput {
    /// The line as a cook writes it: "2 cups flour, sifted", "1 (14 oz) can chickpeas", "salt, to taste".
    pub text: String,
    /// The food this line is, when the cook picked it. Left out, the line is matched by its name, and a name no
    /// food has yet becomes a new food.
    pub food_id: Option<Uuid>,
}

/// A recipe as its writer sends it.
#[derive(Deserialize, ToSchema)]
pub struct RecipeInput {
    /// 1 to 200 characters.
    pub title: String,
    /// At most 200 characters: "with Garlic and Parmesan".
    pub subtitle: Option<String>,
    /// At most 4000 characters.
    pub description: Option<String>,
    /// How many servings the amounts are written for: more than 0, at most 1000. Any other serving count is
    /// worked out from these.
    pub servings: f64,
    /// At most 200 lines of at most 300 characters; blank lines are dropped.
    pub ingredients: Vec<LineInput>,
    /// At most 100 steps of at most 4000 characters, in order; blank ones are dropped.
    pub steps: Vec<String>,
    /// Who else sees it: left out, a new recipe takes the household's default and a changed one keeps what it had.
    /// No more open than the install allows.
    pub visibility: Option<Visibility>,
}

/// A food, by id and name.
#[derive(Serialize, ToSchema, Clone, Debug)]
pub struct FoodRef {
    /// The food's id.
    pub id: Uuid,
    /// The food's name.
    pub name: String,
}

/// One ingredient line: what was typed, and what mise read from it.
#[derive(Serialize, ToSchema)]
pub struct RecipeLine {
    /// The line as typed.
    pub text: String,
    /// The amount read from it, at the recipe's servings; absent for "salt, to taste".
    pub amount: Option<f64>,
    /// The unit read from it: "cup", "tablespoon", "ounce", "clove", or "unit" for a bare count ("2 eggs").
    pub unit: Option<String>,
    /// The name read from it, which is what gets matched to a food: "flour".
    pub name: String,
    /// The rest: "sifted", "14 oz; drained", "to taste".
    pub note: Option<String>,
    /// The food the line is matched to. Absent only when reading lines without saving, for a name no food has yet.
    pub food: Option<FoodRef>,
}

/// A recipe the household wrote, as it was written.
#[derive(Serialize, ToSchema)]
pub struct Recipe {
    /// The version's id: `/api/v1/variants/{id}` shows it at any serving count.
    pub id: Uuid,
    /// The slug of its dish, for `/api/v1/dishes/{slug}`.
    pub dish_slug: String,
    /// The recipe's title.
    pub title: String,
    /// What it comes with, when it says.
    pub subtitle: Option<String>,
    /// What the household wrote about it.
    pub description: Option<String>,
    /// The servings the amounts are written for.
    pub servings: f64,
    /// Its ingredient lines as typed, each with how mise read it.
    pub ingredients: Vec<RecipeLine>,
    /// Its method, one step to an entry.
    pub steps: Vec<String>,
    /// Its photo, when it has one.
    pub image: Option<Image>,
    /// Who else sees it.
    pub visibility: Visibility,
}

/// One ingredient line checked and read.
struct CheckedLine {
    text: String,
    food_id: Option<Uuid>,
    read: Line,
}

impl CheckedLine {
    /// The name to match to a food: the name read from the line, or the whole line when it has none ("2").
    fn name(&self) -> &str {
        if self.read.name.is_empty() {
            &self.text
        } else {
            &self.read.name
        }
    }
}

/// A recipe checked: every text trimmed, of a length and kind the database stores, and every line read.
struct Checked {
    title: String,
    subtitle: Option<String>,
    description: Option<String>,
    servings: f64,
    lines: Vec<CheckedLine>,
    steps: Vec<String>,
}

/// `text` trimmed, when it is at most `max` characters and has nothing Postgres can't store; `what` names it in the
/// refusal ("the title").
fn within(text: &str, max: usize, what: &str) -> ApiResult<String> {
    let t = text.trim();
    if t.chars().count() > max || t.contains('\0') {
        return Err(ApiError::BadRequest(format!(
            "{what} can have at most {max} characters"
        )));
    }
    Ok(t.to_string())
}

/// An optional text: absent when left out or blank.
fn optional(text: Option<&str>, max: usize, what: &str) -> ApiResult<Option<String>> {
    let t = within(text.unwrap_or_default(), max, what)?;
    Ok((!t.is_empty()).then_some(t))
}

/// Ingredient lines trimmed, blank ones dropped, each checked and read.
fn check_lines(lines: Vec<LineInput>) -> ApiResult<Vec<CheckedLine>> {
    let lines = lines
        .into_iter()
        .enumerate()
        .map(|(i, l)| {
            let text = within(&l.text, MAX_LINE, &format!("ingredient {}", i + 1))?;
            let read = ingredient_line::parse(&text);
            Ok(CheckedLine {
                text,
                food_id: l.food_id,
                read,
            })
        })
        .filter(|l| !matches!(l, Ok(l) if l.text.is_empty()))
        .collect::<ApiResult<Vec<_>>>()?;
    if lines.len() > MAX_LINES {
        return Err(ApiError::BadRequest(format!(
            "a recipe can have at most {MAX_LINES} ingredients"
        )));
    }
    Ok(lines)
}

/// Every field of a recipe checked, in the order a person fills them in.
fn check(input: RecipeInput) -> ApiResult<Checked> {
    let title = within(&input.title, MAX_TITLE, "the title")?;
    if title.is_empty() {
        return Err(ApiError::BadRequest("give the recipe a title".into()));
    }
    if !(input.servings > 0.0 && input.servings <= MAX_SERVINGS) {
        return Err(ApiError::BadRequest(format!(
            "servings must be more than 0 and at most {MAX_SERVINGS}"
        )));
    }
    let steps = input
        .steps
        .iter()
        .enumerate()
        .map(|(i, s)| within(s, MAX_TEXT, &format!("step {}", i + 1)))
        .filter(|s| !matches!(s, Ok(s) if s.is_empty()))
        .collect::<ApiResult<Vec<_>>>()?;
    if steps.len() > MAX_STEPS {
        return Err(ApiError::BadRequest(format!(
            "a recipe can have at most {MAX_STEPS} steps"
        )));
    }
    Ok(Checked {
        title,
        subtitle: optional(input.subtitle.as_deref(), MAX_TITLE, "the subtitle")?,
        description: optional(input.description.as_deref(), MAX_TEXT, "the description")?,
        servings: input.servings,
        lines: check_lines(input.ingredients)?,
        steps,
    })
}

impl Checked {
    /// A hash of everything that makes this recipe different from another, as imports keep for their versions.
    fn content_hash(&self) -> String {
        let canonical = json!({
            "title": self.title, "subtitle": self.subtitle, "description": self.description,
            "servings": self.servings, "steps": self.steps,
            "ingredients": self.lines.iter().map(|l| (&l.text, l.food_id)).collect::<Vec<_>>(),
        });
        hex::encode(Sha256::digest(canonical.to_string().as_bytes()))
    }
}

/// Refuses a recipe that names a food that doesn't exist, saying which line. A food the household couldn't have
/// found by searching (`known_food`, a database function) doesn't exist for it.
async fn check_foods(
    tx: &mut Transaction<'_, Postgres>,
    household: Uuid,
    lines: &[CheckedLine],
) -> ApiResult<()> {
    let chosen: Vec<Uuid> = lines.iter().filter_map(|l| l.food_id).collect();
    if chosen.is_empty() {
        return Ok(());
    }
    let known = queries::known_foods(&mut **tx, household, &chosen).await?;
    match lines
        .iter()
        .position(|l| l.food_id.is_some_and(|f| !known.contains(&f)))
    {
        Some(i) => Err(ApiError::BadRequest(format!(
            "ingredient {}: no such food",
            i + 1
        ))),
        None => Ok(()),
    }
}

/// An address no dish has or used to have, for a dish titled `title`: its slug, or that with "-2", "-3" and so on.
/// The addresses `dish` itself has or had count as free, so renaming a dish back gives it its old one.
async fn free_slug(
    tx: &mut Transaction<'_, Postgres>,
    title: &str,
    dish: Option<Uuid>,
) -> ApiResult<String> {
    let base = match slugify(title) {
        s if s.is_empty() => "recipe".to_string(),
        s => s,
    };
    let taken = queries::taken_slugs(&mut **tx, &base, dish).await?;
    let mut slug = base.clone();
    let mut n = 2;
    while taken.contains(&slug) {
        slug = format!("{base}-{n}");
        n += 1;
    }
    Ok(slug)
}

/// Writes a recipe's yield, lines, amounts and steps (after an edit removed the old ones), then matches the lines
/// that name no food.
async fn write_parts(
    tx: &mut Transaction<'_, Postgres>,
    variant: Uuid,
    recipe: &Checked,
) -> ApiResult<()> {
    queries::add_yield(&mut **tx, variant, recipe.servings).await?;

    // A line's key is its place from 1, as an imported page's lines are keyed.
    let keyed: Vec<(String, String)> = recipe
        .lines
        .iter()
        .enumerate()
        .map(|(i, l)| ((i + 1).to_string(), l.name().to_string()))
        .collect();
    for (i, (line, (key, name))) in recipe.lines.iter().zip(&keyed).enumerate() {
        let id = Uuid::now_v7();
        let new = queries::NewLine {
            id,
            variant,
            key,
            position: i as i32,
            name,
            food_id: line.food_id,
            note: line.read.note.as_deref(),
            text: &line.text,
        };
        queries::add_line(&mut **tx, new).await?;
        if let Some(amount) = line.read.amount {
            queries::add_amount(
                &mut **tx,
                id,
                recipe.servings,
                amount,
                line.read.unit_word(),
            )
            .await?;
        }
    }
    for (i, text) in recipe.steps.iter().enumerate() {
        let template = mise_ingest::steps::template(text, &recipe.title, &keyed);
        let template = serde_json::to_value(&template).map_err(|e| ApiError::Internal(e.into()))?;
        queries::add_step(&mut **tx, variant, i as i32, text, template).await?;
    }
    mise_ingest::foods::match_foods_in(tx).await?;
    Ok(())
}

/// What a line knows that the editor doesn't show: whether the kit shipped it, and its allergens. Kept by food, and
/// by name for a line with no food yet.
struct LineFacts {
    food_id: Option<Uuid>,
    name: String,
    shipped: Option<bool>,
    allergens: Vec<String>,
}

/// The facts of a version's lines that have any.
async fn line_facts(
    tx: &mut Transaction<'_, Postgres>,
    variant: Uuid,
) -> ApiResult<Vec<LineFacts>> {
    Ok(queries::line_facts(&mut **tx, variant)
        .await?
        .into_iter()
        .map(|f| LineFacts {
            food_id: f.food_id,
            name: singular_key(&f.raw_name),
            shipped: f.shipped,
            allergens: f.allergens,
        })
        .collect())
}

/// Gives each of a version's lines the facts of the line it replaces: the one for the same food, or with the same
/// name when either has no food. Rewriting a recipe, or copying one, then keeps "Contains: milk" and which items
/// come from the pantry for every ingredient still in it.
async fn carry_facts(
    tx: &mut Transaction<'_, Postgres>,
    variant: Uuid,
    facts: &[LineFacts],
) -> ApiResult<()> {
    if facts.is_empty() {
        return Ok(());
    }
    for line in queries::lines(&mut **tx, variant).await? {
        let name = singular_key(&line.raw_name);
        let same = |f: &&LineFacts| match (line.food_id, f.food_id) {
            (Some(a), Some(b)) => a == b,
            _ => f.name == name,
        };
        if let Some(f) = facts.iter().find(same) {
            queries::set_line_facts(&mut **tx, line.id, f.shipped, &f.allergens).await?;
        }
    }
    Ok(())
}

/// The recipe `id` as it was written, for the household that wrote it; not found for anyone else, and for a
/// version that wasn't written in the app.
async fn load(tx: &mut Transaction<'_, Postgres>, id: Uuid, household: Uuid) -> ApiResult<Recipe> {
    let v = queries::recipe(&mut **tx, id, household)
        .await?
        .ok_or(ApiError::NotFound)?;
    let ingredients = queries::recipe_lines(&mut **tx, id)
        .await?
        .into_iter()
        .map(|l| RecipeLine {
            text: l.text.unwrap_or_else(|| l.raw_name.clone()),
            amount: l.amount,
            unit: l.unit,
            name: l.raw_name,
            note: l.note,
            food: l.food_id.zip(l.food).map(|(id, name)| FoodRef { id, name }),
        })
        .collect();
    let steps = queries::steps(&mut **tx, id).await?;
    Ok(Recipe {
        id,
        dish_slug: v.dish_slug.unwrap_or_default(),
        title: v.title,
        subtitle: v.subtitle,
        description: v.description,
        servings: v.servings.unwrap_or(1.0),
        ingredients,
        steps,
        image: v.hero.map(Image::new),
        visibility: Visibility::parse(&v.visibility).unwrap_or(Visibility::Private),
    })
}

/// Write a recipe. It belongs to the household, which alone can change it, and gets a dish of its own; it is seen as
/// far as `visibility` says, the household's default when left out. Answers with the recipe as stored: each line
/// with what was read from it and the food it is matched to.
#[utoipa::path(
    post, path = "/api/v1/recipes", operation_id = "create_recipe", tag = "recipes", request_body = RecipeInput,
    responses(
        (status = 200, description = "The recipe, stored", body = Recipe),
        (status = 400, description = "A field is missing, empty, too long or out of range, a line names a food that doesn't exist, or the body isn't JSON of the right shape", body = crate::error::ErrorBody),
        crate::error::SignedIn,
        crate::error::ReadOnlyToken,
        crate::error::TooLarge
    )
)]
pub async fn create(
    State(s): State<AppState>,
    Extension(me): Extension<CurrentUser>,
    JsonBody(input): JsonBody<RecipeInput>,
) -> ApiResult<Json<Recipe>> {
    let asked = input.visibility;
    let recipe = check(input)?;
    let mut tx = s.pool.begin().await?;
    let visibility = match asked {
        Some(v) => sharing::allowed(&mut tx, v).await.map(|()| v)?,
        None => sharing::default_for(&mut tx, me.household_id).await?,
    };
    check_foods(&mut tx, me.household_id, &recipe.lines).await?;
    queries::take_turn(&mut *tx).await?;
    let (dish, variant) = (Uuid::now_v7(), Uuid::now_v7());
    let slug = free_slug(&mut tx, &recipe.title, None).await?;
    // The dish names its version and the version its dish; the first of the two checks waits for the commit.
    queries::add_dish(&mut *tx, dish, &recipe, &slug, variant).await?;
    queries::add_recipe(
        &mut *tx,
        variant,
        dish,
        me.household_id,
        &recipe,
        visibility,
    )
    .await?;
    write_parts(&mut tx, variant, &recipe).await?;
    let stored = load(&mut tx, variant, me.household_id).await?;
    tx.commit().await?;
    Ok(Json(stored))
}

/// A recipe the household wrote, as it was written, for editing. Another household's recipe is not found, as if it
/// didn't exist, and so is a version that wasn't written in the app.
#[utoipa::path(
    get, path = "/api/v1/recipes/{id}", operation_id = "get_recipe", tag = "recipes", params(("id" = Uuid, Path)),
    responses(
        (status = 200, description = "The recipe as written", body = Recipe),
        (status = 400, description = "The id isn't a UUID", body = crate::error::ErrorBody),
        (status = 404, description = "No such recipe, or not one this household wrote", body = crate::error::ErrorBody),
        crate::error::SignedIn,
        crate::error::KitchenDevice
    )
)]
pub async fn get(
    State(s): State<AppState>,
    Extension(me): Extension<CurrentUser>,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Recipe>> {
    let mut tx = s.pool.begin().await?;
    Ok(Json(load(&mut tx, id, me.household_id).await?))
}

/// Change a recipe the household wrote: the whole recipe is replaced by the one sent. Its dish takes the new title,
/// and a new address when the title changed (the old address still finds it). Answers with the recipe as stored.
#[utoipa::path(
    put, path = "/api/v1/recipes/{id}", operation_id = "update_recipe", tag = "recipes", params(("id" = Uuid, Path)),
    request_body = RecipeInput,
    responses(
        (status = 200, description = "The recipe, changed", body = Recipe),
        (status = 400, description = "The id isn't a UUID, a field is missing, empty, too long or out of range, a line names a food that doesn't exist, or the body isn't JSON of the right shape", body = crate::error::ErrorBody),
        (status = 404, description = "No such recipe, or not one this household wrote", body = crate::error::ErrorBody),
        crate::error::SignedIn,
        crate::error::ReadOnlyToken,
        crate::error::TooLarge
    )
)]
pub async fn update(
    State(s): State<AppState>,
    Extension(me): Extension<CurrentUser>,
    Path(id): Path<Uuid>,
    JsonBody(input): JsonBody<RecipeInput>,
) -> ApiResult<Json<Recipe>> {
    let asked = input.visibility;
    let recipe = check(input)?;
    let mut tx = s.pool.begin().await?;
    if let Some(v) = asked {
        sharing::allowed(&mut tx, v).await?;
    }
    queries::take_turn(&mut *tx).await?;
    // The dish follows the recipe only when it is the recipe's own: one a person made that opens on this version.
    let own_dish = queries::lock_recipe(&mut *tx, id, me.household_id)
        .await?
        .ok_or(ApiError::NotFound)?;
    let own_dish = own_dish.id.zip(own_dish.slug);
    check_foods(&mut tx, me.household_id, &recipe.lines).await?;

    queries::update_recipe(&mut *tx, id, &recipe, asked).await?;
    let facts = line_facts(&mut tx, id).await?;
    queries::clear_parts(&mut tx, id).await?;
    if let Some((dish, old_slug)) = own_dish {
        let slug = free_slug(&mut tx, &recipe.title, Some(dish)).await?;
        queries::rename_dish(&mut *tx, dish, &recipe, &slug).await?;
        if slug != old_slug {
            // The old address still finds the dish; the one it holds again is no longer a former one.
            queries::retire_slug(&mut tx, dish, &old_slug, &slug).await?;
        }
    }
    write_parts(&mut tx, id, &recipe).await?;
    carry_facts(&mut tx, id, &facts).await?;
    let stored = load(&mut tx, id, me.household_id).await?;
    tx.commit().await?;
    Ok(Json(stored))
}

/// Make the household's own version of any version it can see, a published one or one of its own: a copy it can
/// change, on the same dish, with the same option values, naming the version it came from. The copy is written for
/// one serving count, the smallest the original was published for, and every other count is worked out from it,
/// as for a recipe written in the app. Its lines open in the editor as text ("1¼ cups flour, sifted") and keep their
/// foods, allergens and pantry marks; its photo is the original's. Answers with the copy as `GET
/// /api/v1/recipes/{id}` would, ready to edit.
#[utoipa::path(
    post, path = "/api/v1/variants/{id}/fork", operation_id = "fork_variant", tag = "recipes", params(("id" = Uuid, Path)),
    responses(
        (status = 200, description = "The household's own copy, stored", body = Recipe),
        (status = 400, description = "The id isn't a UUID", body = crate::error::ErrorBody),
        (status = 404, description = "No such version, or not one this household can see", body = crate::error::ErrorBody),
        crate::error::SignedIn,
        crate::error::ReadOnlyToken
    )
)]
pub async fn fork(
    State(s): State<AppState>,
    Extension(me): Extension<CurrentUser>,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Recipe>> {
    let mut tx = s.pool.begin().await?;
    queries::take_turn(&mut *tx).await?;
    let source = queries::source(&mut *tx, id, me.household_id)
        .await?
        .ok_or(ApiError::NotFound)?;
    // A version without amounts has no yield; its copy is written for two, the size a dish page opens at.
    let servings = source.servings.unwrap_or(2.0);
    let lines = queries::source_lines(&mut *tx, id, servings)
        .await?
        .into_iter()
        .map(|l| {
            let read = Line {
                amount: l.amount,
                unit: l.amount.and(l.unit.as_deref()).and_then(Unit::parse),
                name: l.raw_name,
                note: l.note,
            };
            // A line typed into the app, or read from a page, keeps its words; any other is written out from its
            // parts.
            let text = l.text.unwrap_or_else(|| read.text());
            CheckedLine {
                text,
                food_id: l.food_id,
                read,
            }
        })
        .collect();
    let steps = queries::steps(&mut *tx, id).await?;
    let recipe = Checked {
        title: source.title,
        subtitle: source.subtitle,
        description: source.description,
        servings,
        lines,
        steps,
    };
    let facts = line_facts(&mut tx, id).await?;
    // A copy of another household's recipe starts private, whatever this household's default: that household chose
    // who sees it, and sharing the copy further is this household's choice to make in the editor, not a default's.
    let visibility = match source.household_id {
        Some(owner) if owner != me.household_id => Visibility::Private,
        _ => sharing::default_for(&mut tx, me.household_id).await?,
    };

    let copy = Uuid::now_v7();
    queries::add_copy(
        &mut *tx,
        copy,
        source.dish_id,
        id,
        me.household_id,
        &recipe,
        source.meta,
        visibility,
    )
    .await?;
    write_parts(&mut tx, copy, &recipe).await?;
    carry_facts(&mut tx, copy, &facts).await?;
    queries::copy_attributes_and_photo(&mut tx, id, copy).await?;
    let stored = load(&mut tx, copy, me.household_id).await?;
    tx.commit().await?;
    Ok(Json(stored))
}

/// Delete a recipe the household wrote, for good. It leaves every plan it was on, and its dish goes with it when
/// it was the dish's only version. A version copied from it stays, no longer naming it as its original.
#[utoipa::path(
    delete, path = "/api/v1/recipes/{id}", operation_id = "delete_recipe", tag = "recipes", params(("id" = Uuid, Path)),
    responses(
        (status = 204, description = "The recipe is gone"),
        (status = 400, description = "The id isn't a UUID", body = crate::error::ErrorBody),
        (status = 404, description = "No such recipe, or not one this household wrote", body = crate::error::ErrorBody),
        crate::error::SignedIn,
        crate::error::ReadOnlyToken
    )
)]
pub async fn delete(
    State(s): State<AppState>,
    Extension(me): Extension<CurrentUser>,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    let mut tx = s.pool.begin().await?;
    queries::take_turn(&mut *tx).await?;
    let dish = queries::lock_for_delete(&mut *tx, id, me.household_id)
        .await?
        .ok_or(ApiError::NotFound)?;
    queries::delete_recipe(&mut tx, id, dish).await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

/// An image file's bytes, as the API document describes a photo's request body.
#[derive(ToSchema)]
#[schema(value_type = String, format = Binary)]
pub struct PhotoFile(pub Vec<u8>);

/// A photo sent as a request's whole body, with its image type. Asking for an image type is also what keeps another
/// site from sending a signed-in visitor's browser here: a form can't set it, and a script that does is stopped by
/// the browser first.
pub struct PhotoBody(pub Bytes);

impl<S: Send + Sync> FromRequest<S> for PhotoBody {
    type Rejection = ApiError;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        let is_image = req
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.trim_start().to_ascii_lowercase().starts_with("image/"));
        if !is_image {
            return Err(ApiError::BadRequest(
                "send the photo as the body, with its image type as the Content-Type".into(),
            ));
        }
        Bytes::from_request(req, state)
            .await
            .map(PhotoBody)
            .map_err(|e| match e.status() {
                StatusCode::PAYLOAD_TOO_LARGE => ApiError::PayloadTooLarge,
                _ => ApiError::BadRequest(e.body_text()),
            })
    }
}

/// Refuses a recipe the household didn't write, before any work is done for it.
async fn require_own(s: &AppState, id: Uuid, household: Uuid) -> ApiResult<()> {
    if queries::is_own(&s.pool, id, household).await? {
        Ok(())
    } else {
        Err(ApiError::NotFound)
    }
}

/// Give a recipe the household wrote its photo, replacing any it had. The body is the image itself (JPEG, PNG,
/// WebP or GIF, at most 20 MB and 50 megapixels) with its type as the `Content-Type`. Only the picture is kept: it
/// is decoded and stored again without the camera's metadata, such as where it was taken. Answers with the recipe.
#[utoipa::path(
    put, path = "/api/v1/recipes/{id}/photo", operation_id = "set_recipe_photo", tag = "recipes", params(("id" = Uuid, Path)),
    request_body(content = PhotoFile, content_type = "image/*", description = "The image file"),
    responses(
        (status = 200, description = "The recipe with its new photo", body = Recipe),
        (status = 400, description = "The id isn't a UUID, the Content-Type isn't an image type, or the body isn't an image mise reads or has too many pixels", body = crate::error::ErrorBody),
        (status = 404, description = "No such recipe, or not one this household wrote", body = crate::error::ErrorBody),
        (status = 413, description = "The photo is over 20 MB", body = crate::error::ErrorBody),
        crate::error::SignedIn,
        crate::error::ReadOnlyToken
    )
)]
pub async fn set_photo(
    State(s): State<AppState>,
    Extension(me): Extension<CurrentUser>,
    Path(id): Path<Uuid>,
    PhotoBody(photo): PhotoBody,
) -> ApiResult<Json<Recipe>> {
    require_own(&s, id, me.household_id).await?;
    let media = {
        let _turn = DECODING
            .acquire()
            .await
            .map_err(|e| ApiError::Internal(e.into()))?;
        mise_ingest::media::store_upload(&s.pool, &s.media_root, photo.into()).await
    }
    .map_err(|e| match e {
        UploadError::NotAnImage => ApiError::BadRequest(
            "that isn't a photo mise can read: send a JPEG, PNG, WebP or GIF".into(),
        ),
        UploadError::TooManyPixels => ApiError::BadRequest(format!(
            "that photo is too large: at most {} megapixels",
            MAX_UPLOAD_PIXELS / 1_000_000
        )),
        UploadError::Failed(e) => ApiError::Internal(e),
    })?;
    let mut tx = s.pool.begin().await?;
    queries::set_photo(&mut tx, id, media).await?;
    let stored = load(&mut tx, id, me.household_id).await?;
    tx.commit().await?;
    Ok(Json(stored))
}

/// Take the photo off a recipe the household wrote. Answers with the recipe; one without a photo stays as it is.
#[utoipa::path(
    delete, path = "/api/v1/recipes/{id}/photo", operation_id = "remove_recipe_photo", tag = "recipes", params(("id" = Uuid, Path)),
    responses(
        (status = 200, description = "The recipe without a photo", body = Recipe),
        (status = 400, description = "The id isn't a UUID", body = crate::error::ErrorBody),
        (status = 404, description = "No such recipe, or not one this household wrote", body = crate::error::ErrorBody),
        crate::error::SignedIn,
        crate::error::ReadOnlyToken
    )
)]
pub async fn remove_photo(
    State(s): State<AppState>,
    Extension(me): Extension<CurrentUser>,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Recipe>> {
    let mut tx = s.pool.begin().await?;
    let stored = load(&mut tx, id, me.household_id).await?;
    queries::remove_photo(&mut *tx, id).await?;
    tx.commit().await?;
    Ok(Json(Recipe {
        image: None,
        ..stored
    }))
}

/// Ingredient lines to read without saving anything.
#[derive(Deserialize, ToSchema)]
pub struct LinesInput {
    /// At most 200 lines of at most 300 characters; blank lines are dropped.
    pub lines: Vec<String>,
}

/// Ingredient lines, read.
#[derive(Serialize, ToSchema)]
pub struct LinesRead {
    /// Each line sent, in order, as mise reads it.
    pub lines: Vec<RecipeLine>,
}

/// Read ingredient lines as saving a recipe would, without saving: what each line's amount, unit, name and note
/// are, and the food its name matches. An editor shows these as the cook types. A line whose name no food has yet
/// comes back without a food; saving the recipe makes one.
#[utoipa::path(
    post, path = "/api/v1/recipes/lines", operation_id = "read_recipe_lines", tag = "recipes", request_body = LinesInput,
    responses(
        (status = 200, description = "Each line, read", body = LinesRead),
        (status = 400, description = "There are too many lines, one is too long, or the body isn't JSON of the right shape", body = crate::error::ErrorBody),
        crate::error::SignedIn,
        crate::error::ReadOnlyToken,
        crate::error::TooLarge
    )
)]
pub async fn read_lines(
    State(s): State<AppState>,
    Extension(me): Extension<CurrentUser>,
    JsonBody(input): JsonBody<LinesInput>,
) -> ApiResult<Json<LinesRead>> {
    let lines = check_lines(
        input
            .lines
            .into_iter()
            .map(|text| LineInput {
                text,
                food_id: None,
            })
            .collect(),
    )?;
    // Foods are found by the same key the matcher uses, so what shows here is what saving will choose; only those
    // the household could find by searching (`known_food`), so a food named only in another household's
    // private recipe doesn't show its name here.
    let keys: Vec<String> = lines.iter().map(|l| singular_key(l.name())).collect();
    let foods: HashMap<String, FoodRef> = queries::foods_by_alias(&s.pool, me.household_id, &keys)
        .await?
        .into_iter()
        .map(|f| {
            (
                f.alias,
                FoodRef {
                    id: f.id,
                    name: f.name,
                },
            )
        })
        .collect();
    let lines = lines
        .into_iter()
        .zip(keys)
        .map(|(l, key)| RecipeLine {
            name: l.name().to_string(),
            amount: l.read.amount,
            unit: l.read.unit_word(),
            note: l.read.note,
            food: foods.get(&key).cloned(),
            text: l.text,
        })
        .collect();
    Ok(Json(LinesRead { lines }))
}
