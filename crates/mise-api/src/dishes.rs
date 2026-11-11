//! Dishes: search, and the dish page (its options and versions).

use axum::{Json, extract::State};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::{
    AppState,
    error::{ApiError, ApiResult},
    extract::{Path, Query},
    media::Image,
};

const MAX_LIMIT: i64 = 100;

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
    /// Number of published or forked versions.
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

/// Search and browse dishes.
#[utoipa::path(get, path = "/api/v1/dishes", operation_id = "list_dishes", tag = "dishes", params(ListParams), responses((status = 200, description = "Matching dishes, a page at a time", body = DishPage)))]
pub async fn list(
    State(s): State<AppState>,
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
    let rows: Vec<SummaryRow> = sqlx::query_as(
        "SELECT d.id, d.slug, d.title, d.subtitle, d.content_type::text,
                (SELECT count(*) FROM variants v WHERE v.dish_id = d.id) AS versions, d.default_variant_id,
                (SELECT vm.media_id FROM variant_media vm
                 WHERE vm.variant_id = d.default_variant_id AND vm.role = 'hero' LIMIT 1) AS hero,
                count(*) OVER () AS total
         FROM dishes d
         WHERE ($1::text IS NULL OR d.title ILIKE '%' || $1 || '%' OR d.title % $1 OR d.subtitle ILIKE '%' || $1 || '%')
           AND ($2::text IS NULL OR EXISTS (
                 SELECT 1 FROM ingredient_lines il JOIN foods f ON f.id = il.food_id
                 WHERE il.variant_id = d.default_variant_id AND f.name ILIKE '%' || $2 || '%'))
           AND ($3::text IS NULL OR d.content_type::text = $3)
         ORDER BY CASE WHEN $1::text IS NULL THEN 0 ELSE similarity(d.title, $1) END DESC, versions DESC, d.title
         LIMIT $4 OFFSET $5",
    )
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
    /// How many versions offer it.
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
    /// Newest first.
    pub variants: Vec<VariantSummary>,
}

/// id, slug, title, subtitle, content type, default version.
type DishRow = (Uuid, String, String, Option<String>, String, Option<Uuid>);

/// A dish with its options and versions, by its slug or one it used to have (its `slug` is always the current one).
#[utoipa::path(
    get, path = "/api/v1/dishes/{slug}", operation_id = "get_dish", tag = "dishes",
    params(("slug" = String, Path, description = "The dish's slug, or one it had before it was renamed")),
    responses((status = 200, description = "The dish with its versions and options", body = Dish), (status = 404, description = "No such dish", body = crate::error::ErrorBody))
)]
pub async fn get(State(s): State<AppState>, Path(slug): Path<String>) -> ApiResult<Json<Dish>> {
    // A live slug wins over a retired one; grouping keeps the two apart, so this only guards a stray row.
    let (id, slug, title, subtitle, content_type, default_variant): DishRow = sqlx::query_as(
        "SELECT d.id, d.slug, d.title, d.subtitle, d.content_type::text, d.default_variant_id FROM dishes d
         WHERE d.slug = $1 OR d.id = (SELECT r.dish_id FROM retired_slugs r WHERE r.slug = $1)
         ORDER BY d.slug = $1 DESC LIMIT 1",
    )
    .bind(&slug)
    .fetch_optional(&s.pool)
    .await?
    .ok_or(ApiError::NotFound)?;
    let options = sqlx::query_as::<_, (String, String, serde_json::Value, Option<String>)>(
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
         FROM variants v WHERE v.dish_id = $1 ORDER BY v.published_at DESC NULLS LAST",
    )
    .bind(id)
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
    .collect();
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
