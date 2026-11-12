//! Dishes: search, and the dish page (its options and versions).
//!
//! A household sees the versions no household owns and its own (the database's `visible_to`), and a dish only
//! through them: a dish none of whose versions it can see doesn't exist for it, and one it shares with another
//! household shows only its own share of the versions, counts and option choices.

use axum::{Extension, Json, extract::State};
use chrono::{DateTime, Datelike, Utc};
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

const MAX_LIMIT: i64 = 100;

/// SQL for the version dish `d` opens on for the household in `$1`: the dish's default when the household can see it,
/// otherwise its most complete, newest version the household can see. Grouping names a shared dish after versions
/// everyone sees and opens it on one of them, so the second choice only matters for a dish a person made. No row when
/// the household can see none of the dish's versions: joined laterally, it leaves such dishes out.
macro_rules! visible_default {
    () => {
        "SELECT v.id FROM variants v WHERE v.dish_id = d.id AND visible_to(v, $1)
         ORDER BY v.id IS NOT DISTINCT FROM d.default_variant_id DESC, v.has_steps DESC, v.published_at DESC NULLS LAST, v.id
         LIMIT 1"
    };
}

#[derive(Deserialize, IntoParams)]
pub struct ListParams {
    /// Words in the title or subtitle; ranked by similarity, so small typos still match.
    pub q: Option<String>,
    /// Only dishes using a food whose name contains this ("chicken", "gochujang").
    pub ingredient: Option<String>,
    /// recipe, ready_meal, add_on or product.
    #[serde(rename = "type")]
    #[param(rename = "type")]
    pub content_type: Option<String>,
    /// Page size (default 24, at most 100).
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

#[derive(Serialize, ToSchema)]
pub struct DishSummary {
    pub id: Uuid,
    pub slug: String,
    pub title: String,
    pub subtitle: Option<String>,
    pub content_type: String,
    /// Number of published or forked versions this household can see.
    pub versions: i64,
    /// The version the dish opens on.
    pub default_variant: Option<Uuid>,
    pub image: Option<Image>,
}

#[derive(Serialize, ToSchema)]
pub struct DishPage {
    pub total: i64,
    pub items: Vec<DishSummary>,
}

/// id, slug, title, subtitle, content type, versions, default version, image, total matching.
type SummaryRow = (
    Uuid,
    String,
    String,
    Option<String>,
    String,
    i64,
    Option<Uuid>,
    Option<Uuid>,
    i64,
);

/// Search and browse the dishes this household can see.
#[utoipa::path(get, path = "/api/v1/dishes", operation_id = "list_dishes", tag = "dishes", params(ListParams),
    responses(
        (status = 200, description = "Matching dishes, a page at a time", body = DishPage),
        (status = 400, description = "A query value can't be read: `limit` or `offset` isn't a number, say", body = crate::error::ErrorBody),
        crate::error::SignedIn
    )
)]
pub async fn list(
    State(s): State<AppState>,
    Extension(me): Extension<CurrentUser>,
    Query(p): Query<ListParams>,
) -> ApiResult<Json<DishPage>> {
    let limit = p.limit.unwrap_or(24).clamp(1, MAX_LIMIT);
    let offset = p.offset.unwrap_or(0).max(0);
    let q = p.q.as_deref().map(str::trim).filter(|q| !q.is_empty());
    let ingredient = p
        .ingredient
        .as_deref()
        .map(str::trim)
        .filter(|q| !q.is_empty());
    let rows: Vec<SummaryRow> = sqlx::query_as(concat!(
        "SELECT d.id, d.slug, d.title, d.subtitle, d.content_type::text,
                (SELECT count(*) FROM variants v WHERE v.dish_id = d.id AND visible_to(v, $1)) AS versions, dv.id,
                (SELECT vm.media_id FROM variant_media vm WHERE vm.variant_id = dv.id AND vm.role = 'hero' LIMIT 1) AS hero,
                count(*) OVER () AS total
         FROM dishes d CROSS JOIN LATERAL (",
        visible_default!(),
        ") dv
         WHERE ($2::text IS NULL OR d.title ILIKE '%' || $2 || '%' OR d.title % $2 OR d.subtitle ILIKE '%' || $2 || '%')
           AND ($3::text IS NULL OR EXISTS (
                 SELECT 1 FROM ingredient_lines il JOIN foods f ON f.id = il.food_id
                 WHERE il.variant_id = dv.id AND f.name ILIKE '%' || $3 || '%'))
           AND ($4::text IS NULL OR d.content_type::text = $4)
         ORDER BY CASE WHEN $2::text IS NULL THEN 0 ELSE similarity(d.title, $2) END DESC, versions DESC, d.title
         LIMIT $5 OFFSET $6",
    ))
    .bind(me.household_id)
    .bind(q)
    .bind(ingredient)
    .bind(p.content_type.as_deref())
    .bind(limit)
    .bind(offset)
    .fetch_all(&s.pool)
    .await?;
    let total = rows.first().map_or(0, |r| r.8);
    let items = rows
        .into_iter()
        .map(
            |(id, slug, title, subtitle, content_type, versions, default_variant, hero, _)| {
                DishSummary {
                    id,
                    slug,
                    title,
                    subtitle,
                    content_type,
                    versions,
                    default_variant,
                    image: hero.map(Image::new),
                }
            },
        )
        .collect();
    Ok(Json(DishPage { total, items }))
}

#[derive(Serialize, Deserialize, ToSchema)]
pub struct Choice {
    pub value: String,
    pub label: String,
    /// How many versions this household can see offer it.
    pub variants: i64,
    /// Years it was published, e.g. "2022–2024".
    pub years: Option<String>,
}

#[derive(Serialize, ToSchema)]
pub struct DishOption {
    /// protein, portion or version: the attribute key on each version.
    pub key: String,
    pub label: String,
    pub choices: Vec<Choice>,
    pub default: Option<String>,
}

#[derive(Serialize, ToSchema)]
pub struct VariantSummary {
    pub id: Uuid,
    pub title: String,
    pub subtitle: Option<String>,
    pub published_at: Option<DateTime<Utc>>,
    /// Serving counts with published amounts.
    pub yields: Vec<f64>,
    pub has_steps: bool,
    /// This version's value for each of the dish's options.
    #[schema(value_type = std::collections::HashMap<String, String>)]
    pub attributes: serde_json::Value,
    pub image: Option<Image>,
}

#[derive(Serialize, ToSchema)]
pub struct Dish {
    pub id: Uuid,
    pub slug: String,
    pub title: String,
    pub subtitle: Option<String>,
    pub content_type: String,
    pub default_variant: Option<Uuid>,
    /// Only combinations some version actually has are offered: pick the version whose attributes match.
    pub options: Vec<DishOption>,
    /// The versions this household can see, newest first.
    pub variants: Vec<VariantSummary>,
}

/// id, slug, title, subtitle, content type, default version.
type DishRow = (Uuid, String, String, Option<String>, String, Option<Uuid>);

/// A dish with its options and versions, by its slug or one it used to have (its `slug` is always the current one).
/// A dish the household can see no version of is not found, as if it didn't exist.
#[utoipa::path(
    get, path = "/api/v1/dishes/{slug}", operation_id = "get_dish", tag = "dishes",
    params(("slug" = String, Path, description = "The dish's slug, or one it had before it was renamed")),
    responses(
        (status = 200, description = "The dish with its versions and options", body = Dish),
        (status = 400, description = "The slug has a NUL character", body = crate::error::ErrorBody),
        (status = 404, description = "No such dish, or not one this household can see", body = crate::error::ErrorBody),
        crate::error::SignedIn
    )
)]
pub async fn get(
    State(s): State<AppState>,
    Extension(me): Extension<CurrentUser>,
    Path(slug): Path<String>,
) -> ApiResult<Json<Dish>> {
    // A live slug wins over a retired one; grouping keeps the two apart, so this only guards a stray row.
    let (id, slug, title, subtitle, content_type, default_variant): DishRow = sqlx::query_as(concat!(
        "SELECT d.id, d.slug, d.title, d.subtitle, d.content_type::text, dv.id FROM dishes d CROSS JOIN LATERAL (",
        visible_default!(),
        ") dv
         WHERE d.slug = $2 OR d.id = (SELECT r.dish_id FROM retired_slugs r WHERE r.slug = $2)
         ORDER BY d.slug = $2 DESC LIMIT 1",
    ))
    .bind(me.household_id)
    .bind(&slug)
    .fetch_optional(&s.pool)
    .await?
    .ok_or(ApiError::NotFound)?;
    let mut options = sqlx::query_as::<_, (String, String, serde_json::Value, Option<String>)>(
        "SELECT key, label, choices, default_value FROM dish_options WHERE dish_id = $1 ORDER BY position",
    )
    .bind(id)
    .fetch_all(&s.pool)
    .await?
    .into_iter()
    .map(|(key, label, choices, default)| {
        let choices = serde_json::from_value(choices).map_err(|e| ApiError::Internal(e.into()))?;
        Ok(DishOption { key, label, choices, default })
    })
    .collect::<ApiResult<Vec<_>>>()?;
    let variants = sqlx::query_as::<_, (Uuid, String, Option<String>, Option<DateTime<Utc>>, bool, Option<Vec<f64>>, serde_json::Value, Option<Uuid>)>(
        "SELECT v.id, v.title, v.subtitle, v.published_at, v.has_steps,
                (SELECT array_agg(y.servings::float8 ORDER BY y.servings) FROM variant_yields y WHERE y.variant_id = v.id),
                coalesce((SELECT jsonb_object_agg(a.key, a.value) FROM variant_attributes a WHERE a.variant_id = v.id), '{}'),
                (SELECT vm.media_id FROM variant_media vm WHERE vm.variant_id = v.id AND vm.role = 'hero' LIMIT 1)
         FROM variants v WHERE v.dish_id = $1 AND visible_to(v, $2) ORDER BY v.published_at DESC NULLS LAST",
    )
    .bind(id)
    .bind(me.household_id)
    .fetch_all(&s.pool)
    .await?
    .into_iter()
    .map(|(id, title, subtitle, published_at, has_steps, yields, attributes, hero)| VariantSummary {
        id,
        title,
        subtitle,
        published_at,
        yields: yields.unwrap_or_default(),
        has_steps,
        attributes,
        image: hero.map(Image::new),
    })
    .collect::<Vec<_>>();
    // Options are derived across all of a dish's versions; where some are another household's, they are recounted
    // over this household's share, so they neither count nor offer what it can't open.
    let hidden: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM variants v WHERE v.dish_id = $1 AND NOT visible_to(v, $2)",
    )
    .bind(id)
    .bind(me.household_id)
    .fetch_one(&s.pool)
    .await?;
    if hidden > 0 {
        let opens_on = variants.iter().find(|v| Some(v.id) == default_variant);
        options = recount(options, &variants, opens_on);
    }
    Ok(Json(Dish {
        id,
        slug,
        title,
        subtitle,
        content_type,
        default_variant,
        options,
        variants,
    }))
}

/// Options recounted over `variants`, the versions a household can see: each choice's count and years come from
/// them, a choice none of them offers is dropped, and so is an option left with fewer than two choices (derive-options
/// makes an option only where versions differ). Choices keep their order, by count where counts now differ; where the
/// derived default is gone, the default is the choice of `opens_on`, the version the dish opens on.
fn recount(
    options: Vec<DishOption>,
    variants: &[VariantSummary],
    opens_on: Option<&VariantSummary>,
) -> Vec<DishOption> {
    options
        .into_iter()
        .filter_map(|mut o| {
            let value_of = |v: &VariantSummary| {
                v.attributes
                    .get(&o.key)
                    .and_then(|a| a.as_str())
                    .map(str::to_string)
            };
            for c in &mut o.choices {
                let offering: Vec<&VariantSummary> = variants
                    .iter()
                    .filter(|v| value_of(v).as_deref() == Some(c.value.as_str()))
                    .collect();
                c.variants = offering.len() as i64;
                c.years = Some(years(
                    offering
                        .iter()
                        .filter_map(|v| v.published_at.map(|p| p.year())),
                ));
            }
            o.choices.retain(|c| c.variants > 0);
            o.choices.sort_by_key(|c| std::cmp::Reverse(c.variants));
            if o.choices.len() < 2 {
                return None;
            }
            if !o
                .choices
                .iter()
                .any(|c| o.default.as_deref() == Some(c.value.as_str()))
            {
                o.default = opens_on.and_then(value_of);
            }
            Some(o)
        })
        .collect()
}

/// The years versions were published, as derive-options writes them: "2024", "2022–2024", or empty.
fn years(published: impl Iterator<Item = i32>) -> String {
    let (mut min, mut max) = (None::<i32>, None::<i32>);
    for y in published {
        min = Some(min.map_or(y, |m| m.min(y)));
        max = Some(max.map_or(y, |m| m.max(y)));
    }
    match (min, max) {
        (Some(a), Some(b)) if a == b => a.to_string(),
        (Some(a), Some(b)) => format!("{a}–{b}"),
        _ => String::new(),
    }
}
