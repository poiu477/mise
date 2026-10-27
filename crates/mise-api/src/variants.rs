//! One version of a dish, rendered for a serving count: ingredient amounts and step text at that yield.

use std::collections::HashMap;

use axum::{
    Json,
    extract::{Path, Query, State},
};
use mise_core::quantity::{Quantity, Unit, format_amount};
use mise_core::servings;
use mise_core::step_amounts::{self, Context, Template};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::{
    AppState,
    error::{ApiError, ApiResult},
    media::Image,
};

const MAX_SERVINGS: f64 = 24.0;

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
    /// The requested yield is published, so every amount is as published.
    pub official: bool,
    pub ingredients: Vec<Ingredient>,
    pub steps: Vec<Step>,
    pub image: Option<Image>,
    /// Source details kept with the version: prep and total time, difficulty, tags, cuisines, nutrition.
    #[schema(value_type = std::collections::HashMap<String, serde_json::Value>)]
    pub meta: serde_json::Value,
}

type LineRow = (Uuid, String, String, Option<Uuid>, Option<String>, Option<bool>, Vec<String>, Option<String>, Option<String>);

/// A version of a dish at a serving count.
#[utoipa::path(
    get, path = "/api/variants/{id}", operation_id = "get_variant", tag = "dishes", params(("id" = Uuid, Path), RenderParams),
    responses((status = 200, body = Variant), (status = 404, body = crate::error::ErrorBody))
)]
pub async fn get(State(s): State<AppState>, Path(id): Path<Uuid>, Query(p): Query<RenderParams>) -> ApiResult<Json<Variant>> {
    let (title, subtitle, description, meta, dish_slug): (String, Option<String>, Option<String>, serde_json::Value, Option<String>) =
        sqlx::query_as(
            "SELECT v.title, v.subtitle, v.description, v.meta, d.slug FROM variants v LEFT JOIN dishes d ON d.id = v.dish_id
             WHERE v.id = $1",
        )
        .bind(id)
        .fetch_optional(&s.pool)
        .await?
        .ok_or(ApiError::NotFound)?;
    let yields: Vec<f64> = sqlx::query_scalar("SELECT servings::float8 FROM variant_yields WHERE variant_id = $1 ORDER BY servings")
        .bind(id)
        .fetch_all(&s.pool)
        .await?;
    let target = p.servings.or_else(|| yields.first().copied()).unwrap_or(2.0);
    if !(target > 0.0 && target <= MAX_SERVINGS) {
        return Err(ApiError::BadRequest(format!("servings must be between 1 and {MAX_SERVINGS}")));
    }

    let lines: Vec<LineRow> = sqlx::query_as(
        "SELECT il.id, il.line_key, il.raw_name, il.food_id, f.name, il.shipped, il.allergens, il.note, f.props->>'image_media_id'
         FROM ingredient_lines il LEFT JOIN foods f ON f.id = il.food_id WHERE il.variant_id = $1 ORDER BY il.position",
    )
    .bind(id)
    .fetch_all(&s.pool)
    .await?;
    let mut amounts: HashMap<Uuid, Vec<(f64, f64, Option<String>)>> = HashMap::new();
    for (line, sv, a, u) in sqlx::query_as::<_, (Uuid, f64, f64, Option<String>)>(
        "SELECT la.line_id, la.servings::float8, la.amount::float8, la.unit FROM line_amounts la
         JOIN ingredient_lines il ON il.id = la.line_id WHERE il.variant_id = $1 AND la.amount IS NOT NULL",
    )
    .bind(id)
    .fetch_all(&s.pool)
    .await?
    {
        amounts.entry(line).or_default().push((sv, a, u));
    }
    let by_key: HashMap<&str, Uuid> = lines.iter().map(|l| (l.1.as_str(), l.0)).collect();

    let ingredients = lines
        .iter()
        .map(|(line, _, name, food_id, food, shipped, allergens, note, image)| {
            let (amount, scaled) = amounts.get(line).map_or((None, false), |a| line_text(a, target));
            Ingredient {
                name: name.clone(),
                food_id: *food_id,
                food: food.clone(),
                amount,
                scaled,
                shipped: *shipped,
                allergens: allergens.clone(),
                note: note.clone(),
                image: image.as_deref().and_then(|m| m.parse().ok()).map(Image::new),
            }
        })
        .collect();

    let step_rows: Vec<(Uuid, i32, String, Option<serde_json::Value>)> =
        sqlx::query_as("SELECT id, position, text, template FROM steps WHERE variant_id = $1 ORDER BY position")
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
    let hero: Option<Uuid> = sqlx::query_scalar("SELECT media_id FROM variant_media WHERE variant_id = $1 AND role = 'hero' LIMIT 1")
        .bind(id)
        .fetch_optional(&s.pool)
        .await?;

    // Everything below is synchronous: the renderer's line lookup is not Send.
    let templates: Vec<Option<Template>> = step_rows.iter().map(|r| r.3.clone().and_then(|t| serde_json::from_value(t).ok())).collect();
    let base = step_amounts::text_base(&yields, templates.iter().flatten());
    let any_alts = templates.iter().flatten().any(Template::has_alts);
    let line_amount = |key: &str, sv: f64| -> Option<f64> {
        amounts.get(by_key.get(key)?)?.iter().find(|(x, _, _)| (x - sv).abs() < 1e-9).map(|(_, a, _)| *a)
    };
    let ctx = Context { base, scale_plain: !any_alts, line_amount: &line_amount };
    let steps = step_rows
        .iter()
        .zip(&templates)
        .map(|((step, position, text, _), t)| {
            let r = match t {
                Some(t) => step_amounts::render(t, target, &ctx),
                None => step_amounts::Rendered { text: text.clone(), ..Default::default() },
            };
            Step { position: *position, text: r.text, scaled: r.scaled, check: r.check, images: step_images.remove(step).unwrap_or_default() }
        })
        .collect();

    let official = yields.iter().any(|y| (y - target).abs() < 1e-9);
    let meta = public_meta(meta);
    Ok(Json(Variant {
        id,
        dish_slug,
        title,
        subtitle,
        description,
        yields,
        servings: target,
        official,
        ingredients,
        steps,
        image: hero.map(Image::new),
        meta,
    }))
}

/// An ingredient line's amount at `target`: as published when it is, else derived from the published yields
/// in the unit of the nearest one and rounded to kitchen amounts.
fn line_text(published: &[(f64, f64, Option<String>)], target: f64) -> (Option<String>, bool) {
    let show = |a: f64, unit: &Option<String>, round: bool| match unit.as_deref().and_then(Unit::parse) {
        Some(u) => {
            let q = Quantity::new(a, u);
            if round { q.rounded() } else { q }.to_string()
        }
        None => format!("{} {}", format_amount(a), unit.as_deref().unwrap_or("")).trim().to_string(),
    };
    if let Some((_, a, u)) = published.iter().find(|(s, _, _)| (s - target).abs() < 1e-9) {
        return (Some(show(*a, u, false)), false);
    }
    let yields: Vec<f64> = published.iter().map(|p| p.0).collect();
    let Some(near) = servings::nearest(&yields, target) else { return (None, false) };
    let unit = published.iter().find(|p| p.0 == near).and_then(|p| p.2.clone());
    let same_unit: Vec<(f64, f64)> = published.iter().filter(|p| p.2 == unit).map(|p| (p.0, p.1)).collect();
    match servings::amount_at(&same_unit, target) {
        Some((a, _)) => (Some(show(a, &unit, true)), true),
        None => (None, false),
    }
}

/// Source metadata worth showing; internal bookkeeping (lineage, media paths) stays out.
fn public_meta(mut meta: serde_json::Value) -> serde_json::Value {
    if let Some(m) = meta.as_object_mut() {
        m.remove("lineage");
        m.remove("media");
    }
    meta
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn published_amounts_are_exact_and_others_rounded() {
        let a = vec![(2.0, 0.5, Some("cup".into())), (4.0, 1.0, Some("cup".into()))];
        assert_eq!(line_text(&a, 2.0), (Some("½ cup".into()), false));
        assert_eq!(line_text(&a, 3.0), (Some("¾ cup".into()), true));
        assert_eq!(line_text(&a, 6.0), (Some("1½ cups".into()), true));
        let counts = vec![(2.0, 1.0, Some("unit".into())), (4.0, 2.0, Some("unit".into()))];
        assert_eq!(line_text(&counts, 5.0), (Some("2½".into()), true));
        let odd = vec![(2.0, 4.0, Some("nach Geschmack".into()))];
        assert_eq!(line_text(&odd, 2.0), (Some("4 nach Geschmack".into()), false));
    }
}
