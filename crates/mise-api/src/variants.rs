//! One version of a dish, rendered for a serving count: ingredient amounts and step text at that yield.

use std::collections::HashMap;

use axum::{Extension, Json, extract::State};
use mise_core::servings;
use mise_core::step_amounts::{self, Context, Template};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::{
    AppState,
    auth::CurrentUser,
    error::{ApiError, ApiResult},
    extract::{Path, Query},
    media::Image,
};

/// The largest derived yield; any published yield is allowed, however large (a 40-serving roast turkey).
pub const MAX_SERVINGS: f64 = 24.0;

#[derive(Deserialize, IntoParams)]
pub struct RenderParams {
    /// Serving count (default: the smallest published yield).
    pub servings: Option<f64>,
}

#[derive(Serialize, ToSchema)]
pub struct Ingredient {
    pub name: String,
    pub food_id: Option<Uuid>,
    pub food: Option<String>,
    /// The amount at the requested yield, e.g. "1½ TBSP"; absent for "to taste" items like salt.
    pub amount: Option<String>,
    /// Derived rather than published for this yield.
    pub scaled: bool,
    /// Supplied by the source (meal kit) rather than from the pantry.
    pub shipped: Option<bool>,
    pub allergens: Vec<String>,
    pub note: Option<String>,
    pub image: Option<Image>,
}

#[derive(Serialize, ToSchema)]
pub struct Step {
    pub position: i32,
    pub text: String,
    /// Some amount in this step was derived for this yield.
    pub scaled: bool,
    /// Some wording came from the nearest published yield; worth a second look.
    pub check: bool,
    /// The step names a yield mise could not place, so it reads as published: check its amounts.
    pub check_amounts: bool,
    pub images: Vec<Image>,
}

#[derive(Serialize, ToSchema)]
pub struct Variant {
    pub id: Uuid,
    pub dish_slug: Option<String>,
    pub title: String,
    pub subtitle: Option<String>,
    pub description: Option<String>,
    /// Serving counts with published amounts.
    pub yields: Vec<f64>,
    pub servings: f64,
    /// The requested yield is one of the published ones.
    pub published_yield: bool,
    /// Every amount shown is as published (a published yield can still lack an amount for a line, or have
    /// step text written for another yield).
    pub official: bool,
    pub ingredients: Vec<Ingredient>,
    pub steps: Vec<Step>,
    pub image: Option<Image>,
    /// Source details kept with the version: prep and total time, difficulty, tags, cuisines, nutrition.
    #[schema(value_type = std::collections::HashMap<String, serde_json::Value>)]
    pub meta: serde_json::Value,
}

type LineRow = (
    Uuid,
    String,
    String,
    Option<Uuid>,
    Option<String>,
    Option<bool>,
    Vec<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
);

/// A version of a dish at a serving count. Another household's own version is not found, as if it didn't exist.
#[utoipa::path(
    get, path = "/api/v1/variants/{id}", operation_id = "get_variant", tag = "dishes", params(("id" = Uuid, Path), RenderParams),
    responses(
        (status = 200, description = "The version at the requested serving count", body = Variant),
        (status = 400, description = "The id isn't a UUID, or `servings` isn't a number or is out of range", body = crate::error::ErrorBody),
        (status = 404, description = "No such version, or not one this household can see", body = crate::error::ErrorBody),
        crate::error::SignedIn
    )
)]
pub async fn get(
    State(s): State<AppState>,
    Extension(me): Extension<CurrentUser>,
    Path(id): Path<Uuid>,
    Query(p): Query<RenderParams>,
) -> ApiResult<Json<Variant>> {
    let (title, subtitle, description, meta, dish_slug): (String, Option<String>, Option<String>, serde_json::Value, Option<String>) =
        sqlx::query_as(
            "SELECT v.title, v.subtitle, v.description, v.meta, d.slug FROM variants v LEFT JOIN dishes d ON d.id = v.dish_id
             WHERE v.id = $1 AND visible_to(v, $2)",
        )
        .bind(id)
        .bind(me.household_id)
        .fetch_optional(&s.pool)
        .await?
        .ok_or(ApiError::NotFound)?;
    let yields: Vec<f64> = sqlx::query_scalar(
        "SELECT servings::float8 FROM variant_yields WHERE variant_id = $1 ORDER BY servings",
    )
    .bind(id)
    .fetch_all(&s.pool)
    .await?;
    let target = p
        .servings
        .or_else(|| yields.first().copied())
        .unwrap_or(2.0);
    let max = yields.iter().copied().fold(MAX_SERVINGS, f64::max);
    if !(target > 0.0 && target <= max) {
        return Err(ApiError::BadRequest(format!(
            "servings must be more than 0 and at most {max}"
        )));
    }

    let lines: Vec<LineRow> = sqlx::query_as(
        "SELECT il.id, il.line_key, il.raw_name, il.food_id, f.name, il.shipped, il.allergens, il.note, f.props->>'image_media_id',
                f.props->>'count_unit', f.props->>'count_size'
         FROM ingredient_lines il LEFT JOIN foods f ON f.id = il.food_id WHERE il.variant_id = $1 ORDER BY il.position",
    )
    .bind(id)
    .fetch_all(&s.pool)
    .await?;
    let mut amounts: HashMap<Uuid, Vec<(f64, f64, Option<String>)>> = HashMap::new();
    for (line, sv, a, u) in sqlx::query_as::<_, (Uuid, f64, f64, Option<String>)>(
        "SELECT la.line_id, la.servings::float8, la.amount::float8, la.unit FROM line_amounts la
         JOIN ingredient_lines il ON il.id = la.line_id WHERE il.variant_id = $1 AND la.amount > 0",
    )
    .bind(id)
    .fetch_all(&s.pool)
    .await?
    {
        amounts.entry(line).or_default().push((sv, a, u));
    }
    let step_rows: Vec<(Uuid, i32, String, Option<serde_json::Value>)> = sqlx::query_as(
        "SELECT id, position, text, template FROM steps WHERE variant_id = $1 ORDER BY position",
    )
    .bind(id)
    .fetch_all(&s.pool)
    .await?;
    let mut step_images: HashMap<Uuid, Vec<Image>> = HashMap::new();
    for (step, media) in sqlx::query_as::<_, (Uuid, Uuid)>(
        "SELECT step_id, media_id FROM variant_media WHERE variant_id = $1 AND role = 'step' AND step_id IS NOT NULL ORDER BY position",
    )
    .bind(id)
    .fetch_all(&s.pool)
    .await?
    {
        step_images.entry(step).or_default().push(Image::new(media));
    }
    let hero: Option<Uuid> = sqlx::query_scalar(
        "SELECT media_id FROM variant_media WHERE variant_id = $1 AND role = 'hero' LIMIT 1",
    )
    .bind(id)
    .fetch_optional(&s.pool)
    .await?;

    // Everything below is synchronous: the renderer's line lookup is not Send.
    let templates: Vec<Option<Template>> = step_rows
        .iter()
        .map(|r| r.3.clone().and_then(|t| serde_json::from_value(t).ok()))
        .collect();
    let base = step_amounts::text_base(&yields, templates.iter().flatten());
    // The yield the steps are written for, and its double (the one their parentheses give), are trusted first;
    // a published amount that would shrink as servings grow is derived instead.
    for list in amounts.values_mut() {
        servings::keep_consistent(list, &[base, base * 2.0]);
    }
    let by_key: HashMap<&str, Uuid> = lines.iter().map(|l| (l.1.as_str(), l.0)).collect();

    let ingredients: Vec<Ingredient> = lines
        .iter()
        .map(
            |(
                line,
                _,
                name,
                food_id,
                food,
                shipped,
                allergens,
                note,
                image,
                count_unit,
                count_size,
            )| {
                let count =
                    servings::CountUnit::from_props(count_unit.as_deref(), count_size.as_deref());
                let (amount, scaled) = amounts.get(line).map_or((None, false), |a| {
                    servings::line_text(a, target, count.as_ref())
                });
                Ingredient {
                    name: name.clone(),
                    food_id: *food_id,
                    food: food.clone(),
                    amount,
                    scaled,
                    shipped: *shipped,
                    allergens: allergens.clone(),
                    note: note.clone(),
                    image: image
                        .as_deref()
                        .and_then(|m| m.parse().ok())
                        .map(Image::new),
                }
            },
        )
        .collect();

    let any_alts = templates.iter().flatten().any(Template::has_alts);
    // A line's amount at any yield: published, or interpolated along its (consistent) published amounts.
    let line_amount = |key: &str, sv: f64| -> Option<f64> {
        let list = amounts.get(by_key.get(key)?)?;
        let unit = &list.first()?.2;
        let points: Vec<(f64, f64)> = list
            .iter()
            .filter(|p| &p.2 == unit)
            .map(|p| (p.0, p.1))
            .collect();
        servings::amount_at(&points, sv).map(|(a, _)| a)
    };
    let ctx = Context {
        base,
        scale_plain: !any_alts,
        line_amount: &line_amount,
    };
    let steps: Vec<Step> = step_rows
        .iter()
        .zip(&templates)
        .map(|((step, position, text, _), t)| {
            let r = match t {
                Some(t) => step_amounts::render(t, target, &ctx),
                None => step_amounts::Rendered {
                    text: text.clone(),
                    ..Default::default()
                },
            };
            Step {
                position: *position,
                text: r.text,
                scaled: r.scaled,
                check: r.check,
                check_amounts: r.check_amounts,
                images: step_images.remove(step).unwrap_or_default(),
            }
        })
        .collect();

    let published_yield = yields.iter().any(|y| (y - target).abs() < 1e-9);
    let official = published_yield
        && !ingredients.iter().any(|i: &Ingredient| i.scaled)
        && !steps.iter().any(|s: &Step| s.scaled);
    let meta = public_meta(meta);
    Ok(Json(Variant {
        id,
        dish_slug,
        title,
        subtitle,
        description,
        yields,
        servings: target,
        published_yield,
        official,
        ingredients,
        steps,
        image: hero.map(Image::new),
        meta,
    }))
}

/// Source metadata worth showing; internal bookkeeping (lineage, media paths) stays out.
fn public_meta(mut meta: serde_json::Value) -> serde_json::Value {
    if let Some(m) = meta.as_object_mut() {
        m.remove("lineage");
        m.remove("media");
    }
    meta
}
