//! Everything the gate report reads from the database. Read-only: no query here writes.

use std::collections::{HashMap, HashSet};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

#[derive(sqlx::FromRow, Clone)]
pub struct Dish {
    pub id: Uuid,
    pub title: String,
    pub subtitle: Option<String>,
    pub slug: String,
    pub content_type: String,
    pub default_variant_id: Option<Uuid>,
}

#[derive(sqlx::FromRow, Clone)]
pub struct Variant {
    pub id: Uuid,
    pub dish_id: Uuid,
    pub title: String,
    pub subtitle: Option<String>,
    pub published: Option<DateTime<Utc>>,
    pub has_steps: bool,
    pub parent_variant_id: Option<Uuid>,
}

#[derive(sqlx::FromRow, Clone)]
pub struct DishOption {
    pub dish_id: Uuid,
    pub key: String,
    pub label: String,
    pub choices: serde_json::Value,
    pub default_value: Option<String>,
}

/// An open "these two dishes might be one" review item.
#[derive(sqlx::FromRow, Clone)]
pub struct MergeReview {
    /// The dish holding the copy that was kept apart.
    pub dish_id: Uuid,
    pub variant_title: String,
    /// The dish holding the version it was copied from.
    pub other_dish_id: Uuid,
    pub copied_from_title: String,
    pub title_similarity: Option<f64>,
    pub shipped_overlap: Option<f64>,
    pub confidence: Option<f32>,
}

#[derive(sqlx::FromRow, Clone)]
pub struct Line {
    pub variant_id: Uuid,
    pub raw_name: String,
    pub food_name: Option<String>,
    pub shipped: bool,
}

/// The whole dish layer, loaded once: small enough (tens of thousands of rows) to hold in memory.
pub struct Snapshot {
    pub dishes: Vec<Dish>,
    pub variants: Vec<Variant>,
    pub options: Vec<DishOption>,
    pub merge_reviews: Vec<MergeReview>,
    /// (kind, status, count) over the whole review queue.
    pub review_counts: Vec<(String, String, i64)>,
    /// dish id -> the foods its default version ships, for comparing look-alike dishes.
    pub default_shipped: HashMap<Uuid, HashSet<Uuid>>,
}

pub async fn load_snapshot(pool: &PgPool) -> Result<Snapshot> {
    let dishes: Vec<Dish> = sqlx::query_as(
        "SELECT id, title, subtitle, slug, content_type::text AS content_type, default_variant_id FROM dishes ORDER BY id",
    )
    .fetch_all(pool)
    .await
    .context("loading dishes")?;
    let variants: Vec<Variant> = sqlx::query_as(
        "SELECT id, dish_id, title, subtitle, published_at AS published, has_steps, parent_variant_id
         FROM variants WHERE dish_id IS NOT NULL ORDER BY published_at NULLS LAST, id",
    )
    .fetch_all(pool)
    .await
    .context("loading variants")?;
    let options: Vec<DishOption> =
        sqlx::query_as("SELECT dish_id, key, label, choices, default_value FROM dish_options ORDER BY dish_id, position")
            .fetch_all(pool)
            .await
            .context("loading options")?;
    let merge_reviews: Vec<MergeReview> = sqlx::query_as(
        "SELECT (subject->>'dish_id')::uuid AS dish_id, coalesce(subject->>'title', '') AS variant_title,
                (proposal->>'merge_into_dish')::uuid AS other_dish_id,
                coalesce(proposal->>'copied_from_title', '') AS copied_from_title,
                (proposal->>'title_similarity')::float8 AS title_similarity,
                (proposal->>'shipped_overlap')::float8 AS shipped_overlap, confidence
         FROM review_items WHERE kind = 'dish_merge' AND status = 'open'
           AND subject ? 'dish_id' AND proposal ? 'merge_into_dish'",
    )
    .fetch_all(pool)
    .await
    .context("loading dish merge reviews")?;
    let review_counts: Vec<(String, String, i64)> =
        sqlx::query_as("SELECT kind, status, count(*) FROM review_items GROUP BY kind, status ORDER BY kind, status")
            .fetch_all(pool)
            .await?;
    let rows: Vec<(Uuid, Uuid)> = sqlx::query_as(
        "SELECT d.id, il.food_id FROM dishes d JOIN ingredient_lines il ON il.variant_id = d.default_variant_id
         WHERE il.shipped AND il.food_id IS NOT NULL",
    )
    .fetch_all(pool)
    .await
    .context("loading default versions' shipped foods")?;
    let mut default_shipped: HashMap<Uuid, HashSet<Uuid>> = HashMap::new();
    for (dish, food) in rows {
        default_shipped.entry(dish).or_default().insert(food);
    }
    Ok(Snapshot { dishes, variants, options, merge_reviews, review_counts, default_shipped })
}

/// variant id -> key -> value, for the given dishes' variants.
pub async fn load_attributes(pool: &PgPool, dish_ids: &[Uuid]) -> Result<HashMap<Uuid, HashMap<String, String>>> {
    let rows: Vec<(Uuid, String, String)> = sqlx::query_as(
        "SELECT va.variant_id, va.key, va.value FROM variant_attributes va
         JOIN variants v ON v.id = va.variant_id WHERE v.dish_id = ANY($1)",
    )
    .bind(dish_ids)
    .fetch_all(pool)
    .await
    .context("loading variant attributes")?;
    let mut out: HashMap<Uuid, HashMap<String, String>> = HashMap::new();
    for (variant, key, value) in rows {
        out.entry(variant).or_default().insert(key, value);
    }
    Ok(out)
}

/// Ingredient lines of the given variants, in recipe order.
pub async fn load_lines(pool: &PgPool, variant_ids: &[Uuid]) -> Result<HashMap<Uuid, Vec<Line>>> {
    let rows: Vec<Line> = sqlx::query_as(
        "SELECT il.variant_id, il.raw_name, f.name AS food_name, coalesce(il.shipped, false) AS shipped
         FROM ingredient_lines il LEFT JOIN foods f ON f.id = il.food_id
         WHERE il.variant_id = ANY($1) ORDER BY il.variant_id, il.position",
    )
    .bind(variant_ids)
    .fetch_all(pool)
    .await
    .context("loading ingredient lines")?;
    let mut out: HashMap<Uuid, Vec<Line>> = HashMap::new();
    for l in rows {
        out.entry(l.variant_id).or_default().push(l);
    }
    Ok(out)
}
