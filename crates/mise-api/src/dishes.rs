//! Dishes: search, and the dish page (its options and versions).
//!
//! A household sees the versions no household owns and its own (the database's `visible_to`), and a dish only
//! through them: a dish none of whose versions it can see doesn't exist for it, and one it shares with another
//! household shows only its own share of the versions, counts and option choices.

use axum::{Extension, Json, extract::State};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
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
/// The kinds of dish, as `content_type` holds them.
const KINDS: [&str; 4] = ["recipe", "ready_meal", "add_on", "product"];

/// A search of the dishes: by words, by ingredient, by kind, a page at a time.
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
    /// How many results to skip, for the next page (default 0).
    pub offset: Option<i64>,
}

/// A dish as search lists it.
#[derive(Serialize, ToSchema)]
pub struct DishSummary {
    /// The dish's id.
    pub id: Uuid,
    /// Its address: `/api/v1/dishes/{slug}`. It follows the title, and old slugs still find it.
    pub slug: String,
    /// The dish's title.
    pub title: String,
    /// What it comes with ("with Lemon Rice & Greens").
    pub subtitle: Option<String>,
    /// recipe, ready_meal, add_on or product.
    pub content_type: String,
    /// Number of published or forked versions this household can see.
    pub versions: i64,
    /// The version the dish opens on.
    pub default_variant: Option<Uuid>,
    /// The photo of the version the dish opens on.
    pub image: Option<Image>,
}

/// A page of search results.
#[derive(Serialize, ToSchema)]
pub struct DishPage {
    /// How many dishes match in all, across every page.
    pub total: i64,
    /// This page's dishes, the best matches first.
    pub items: Vec<DishSummary>,
}

/// Search and browse the dishes this household can see.
#[utoipa::path(get, path = "/api/v1/dishes", operation_id = "list_dishes", tag = "dishes", params(ListParams),
    responses(
        (status = 200, description = "Matching dishes, a page at a time", body = DishPage),
        (status = 400, description = "A query value can't be read (`limit` or `offset` isn't a number, say), or `type` isn't a kind of dish", body = crate::error::ErrorBody),
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
    // A type that isn't one would match nothing and say nothing; say so instead.
    if let Some(kind) = p.content_type.as_deref()
        && !KINDS.contains(&kind)
    {
        return Err(ApiError::BadRequest(format!(
            "type must be one of {}",
            KINDS.join(", ")
        )));
    }
    let q = p.q.as_deref().map(str::trim).filter(|q| !q.is_empty());
    let ingredient = p
        .ingredient
        .as_deref()
        .map(str::trim)
        .filter(|q| !q.is_empty());
    // A dish shows when the household can see one of its versions, and opens on the one `default_version` picks.
    let rows = sqlx::query!(
        r#"SELECT d.id, d.slug, d.title, d.subtitle, d.content_type::text AS "content_type!",
                  (SELECT count(*) FROM variants v WHERE v.dish_id = d.id AND visible_to(v, $1)) AS "versions!",
                  dv.id AS default_variant,
                  (SELECT vm.media_id FROM variant_media vm WHERE vm.variant_id = dv.id AND vm.role = 'hero'
                   LIMIT 1) AS hero,
                  count(*) OVER () AS "total!"
           FROM dishes d CROSS JOIN LATERAL (SELECT default_version(d, $1) AS id) dv
           WHERE dv.id IS NOT NULL
             AND ($2::text IS NULL OR d.title ILIKE '%' || $2 || '%' OR d.title % $2
                  OR d.subtitle ILIKE '%' || $2 || '%')
             AND ($3::text IS NULL OR EXISTS (
                   SELECT 1 FROM ingredient_lines il JOIN foods f ON f.id = il.food_id
                   WHERE il.variant_id = dv.id AND f.name ILIKE '%' || $3 || '%'))
             AND ($4::text IS NULL OR d.content_type::text = $4)
           ORDER BY CASE WHEN $2::text IS NULL THEN 0 ELSE similarity(d.title, $2) END DESC, "versions!" DESC, d.title
           LIMIT $5 OFFSET $6"#,
        me.household_id,
        q,
        ingredient,
        p.content_type.as_deref(),
        limit,
        offset
    )
    .fetch_all(&s.pool)
    .await?;
    let total = rows.first().map_or(0, |r| r.total);
    let items = rows
        .into_iter()
        .map(|d| DishSummary {
            id: d.id,
            slug: d.slug,
            title: d.title,
            subtitle: d.subtitle,
            content_type: d.content_type,
            versions: d.versions,
            default_variant: d.default_variant,
            image: d.hero.map(Image::new),
        })
        .collect();
    Ok(Json(DishPage { total, items }))
}

/// One value of a dish's option: a protein, a portion, a version.
#[derive(Serialize, Deserialize, ToSchema)]
pub struct Choice {
    /// The value as the versions' attributes hold it.
    pub value: String,
    /// The value as people read it ("Chicken", "Double portion").
    pub label: String,
    /// How many versions this household can see offer it.
    pub variants: i64,
    /// Years it was published, e.g. "2022–2024".
    pub years: Option<String>,
}

/// A way the dish's versions differ (protein, portion, version), and the choices it offers.
#[derive(Serialize, ToSchema)]
pub struct DishOption {
    /// protein, portion or version: the attribute key on each version.
    pub key: String,
    /// The option as people read it ("Protein").
    pub label: String,
    /// Its values, as the dish's versions offer them.
    pub choices: Vec<Choice>,
    /// The value the dish opens on, when it has one.
    pub default: Option<String>,
}

/// One version of a dish, as its page lists it.
#[derive(Serialize, ToSchema)]
pub struct VariantSummary {
    /// The version's id: `/api/v1/variants/{id}` shows it at any serving count.
    pub id: Uuid,
    /// The version's title.
    pub title: String,
    /// The version's subtitle.
    pub subtitle: Option<String>,
    /// When its source published it, RFC 3339; absent for one written or forked in mise.
    #[serde(with = "time::serde::rfc3339::option")]
    pub published_at: Option<OffsetDateTime>,
    /// Serving counts with published amounts.
    pub yields: Vec<f64>,
    /// It has a method, not only ingredients.
    pub has_steps: bool,
    /// This version's value for each of the dish's options.
    #[schema(value_type = std::collections::HashMap<String, String>)]
    pub attributes: serde_json::Value,
    /// The version's photo.
    pub image: Option<Image>,
    /// This household wrote the version in the app, so it can change it at `/api/v1/recipes/{id}`.
    pub editable: bool,
    /// For the household's own version of another (a fork), the version it was made from, while the household can
    /// still see it.
    pub original: Option<Original>,
    /// Another household's own version, shared with this one: that household's name. It can be read and forked, not
    /// changed.
    pub shared_by: Option<String>,
}

/// The version a fork was made from.
#[derive(Serialize, ToSchema)]
pub struct Original {
    /// The original version's id.
    pub id: Uuid,
    /// The original version's title.
    pub title: String,
    /// Who it is by: the source it was published or imported from, or the household that wrote it.
    pub by: Option<String>,
}

/// A dish's page: its versions this household can see, and the options that pick between them.
#[derive(Serialize, ToSchema)]
pub struct Dish {
    /// The dish's id.
    pub id: Uuid,
    /// Its address now; an old slug asked for answers with this one.
    pub slug: String,
    /// The dish's title.
    pub title: String,
    /// What it comes with.
    pub subtitle: Option<String>,
    /// recipe, ready_meal, add_on or product.
    pub content_type: String,
    /// The version the dish opens on: its default when this household can see it, else the one with steps published
    /// last.
    pub default_variant: Option<Uuid>,
    /// Only combinations some version actually has are offered: pick the version whose attributes match.
    pub options: Vec<DishOption>,
    /// The versions this household can see, newest first.
    pub variants: Vec<VariantSummary>,
}

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
    let d = sqlx::query!(
        r#"SELECT d.id, d.slug, d.title, d.subtitle, d.content_type::text AS "content_type!",
                  dv.id AS default_variant
           FROM dishes d CROSS JOIN LATERAL (SELECT default_version(d, $1) AS id) dv
           WHERE dv.id IS NOT NULL
             AND (d.slug = $2 OR d.id = (SELECT r.dish_id FROM retired_slugs r WHERE r.slug = $2))
           ORDER BY d.slug = $2 DESC LIMIT 1"#,
        me.household_id,
        &slug
    )
    .fetch_optional(&s.pool)
    .await?
    .ok_or(ApiError::NotFound)?;
    let (id, slug, title, subtitle, content_type, default_variant) = (
        d.id,
        d.slug,
        d.title,
        d.subtitle,
        d.content_type,
        d.default_variant,
    );
    let mut options = sqlx::query!(
        "SELECT key, label, choices, default_value FROM dish_options WHERE dish_id = $1 ORDER BY position",
        id
    )
    .fetch_all(&s.pool)
    .await?
    .into_iter()
    .map(|o| {
        let choices = serde_json::from_value(o.choices).map_err(|e| ApiError::Internal(e.into()))?;
        Ok(DishOption {
            key: o.key,
            label: o.label,
            choices,
            default: o.default_value,
        })
    })
    .collect::<ApiResult<Vec<_>>>()?;
    // A version written in the app names its parent as its original; an imported one's parent is the issue before
    // it, which isn't an original in that sense.
    let variants = sqlx::query!(
        r#"SELECT v.id, v.title, v.subtitle, v.published_at, v.has_steps,
                  (SELECT array_agg(y.servings::float8 ORDER BY y.servings) FROM variant_yields y
                   WHERE y.variant_id = v.id) AS yields,
                  coalesce((SELECT jsonb_object_agg(a.key, a.value) FROM variant_attributes a
                            WHERE a.variant_id = v.id), '{}') AS "attributes!",
                  (SELECT vm.media_id FROM variant_media vm WHERE vm.variant_id = v.id AND vm.role = 'hero'
                   LIMIT 1) AS hero,
                  v.origin = 'user' AND v.household_id IS NOT DISTINCT FROM $2 AS "editable!",
                  p.id AS "parent?", p.title AS "parent_title?", coalesce(src.name, h.name) AS by,
                  (SELECT o.name FROM households o WHERE o.id = v.household_id AND o.id <> $2) AS shared_by
           FROM variants v
           LEFT JOIN variants p ON p.id = v.parent_variant_id AND v.origin = 'user' AND visible_to(p, $2)
           LEFT JOIN source_records sr ON sr.id = p.source_record_id
           LEFT JOIN sources src ON src.id = sr.source_id
           LEFT JOIN households h ON h.id = p.household_id AND p.origin = 'user'
           WHERE v.dish_id = $1 AND visible_to(v, $2) ORDER BY v.published_at DESC NULLS LAST"#,
        id,
        me.household_id
    )
    .fetch_all(&s.pool)
    .await?
    .into_iter()
    .map(|v| VariantSummary {
        id: v.id,
        title: v.title,
        subtitle: v.subtitle,
        published_at: v.published_at,
        yields: v.yields.unwrap_or_default(),
        has_steps: v.has_steps,
        attributes: v.attributes,
        image: v.hero.map(Image::new),
        editable: v.editable,
        original: v.parent.zip(v.parent_title).map(|(id, title)| Original { id, title, by: v.by }),
        shared_by: v.shared_by,
    })
    .collect::<Vec<_>>();
    // Options are derived across all of a dish's versions; where some are another household's, they are recounted
    // over this household's share, so they neither count nor offer what it can't open.
    let hidden: i64 = sqlx::query_scalar!(
        r#"SELECT count(*) AS "hidden!" FROM variants v WHERE v.dish_id = $1 AND NOT visible_to(v, $2)"#,
        id,
        me.household_id
    )
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
