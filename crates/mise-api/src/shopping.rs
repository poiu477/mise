//! The shopping list: worked out from the week's plan each time, with what people ticked, removed and added on top.

use std::collections::{BTreeMap, HashMap};

use axum::{Extension, Json, extract::State};
use chrono::NaiveDate;
use mise_core::quantity::{Family, Quantity};
use mise_core::servings::{self, CountUnit};
use mise_core::shopping::{self, CHECK, OTHER, WeightUnit};
use mise_core::step_amounts::{self, Template};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::{
    AppState,
    auth::CurrentUser,
    error::{ApiError, ApiResult},
    extract::{JsonBody, Path, Query},
    plan::{WeekParams, monday},
};

/// How weights read: "1 lb 14 oz", "30 oz", "1⅞ lb" or "850 g".
#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Weights {
    LbOz,
    Oz,
    Lb,
    Metric,
}

impl Weights {
    fn core(self) -> WeightUnit {
        match self {
            Weights::LbOz => WeightUnit::LbOz,
            Weights::Oz => WeightUnit::Oz,
            Weights::Lb => WeightUnit::Lb,
            Weights::Metric => WeightUnit::Metric,
        }
    }

    fn stored(s: &str) -> Weights {
        match WeightUnit::parse(s).unwrap_or_default() {
            WeightUnit::LbOz => Weights::LbOz,
            WeightUnit::Oz => Weights::Oz,
            WeightUnit::Lb => Weights::Lb,
            WeightUnit::Metric => Weights::Metric,
        }
    }
}

#[derive(Serialize, ToSchema)]
pub struct ShoppingItem {
    /// Names the item when ticking or removing it.
    pub key: String,
    pub food_id: Option<Uuid>,
    pub name: String,
    /// Added up across recipes, exactly, e.g. "3 packets (4½ oz)"; absent for "to taste" items like salt.
    pub amount: Option<String>,
    /// Some of it is a weight, so the household can choose how it reads.
    pub weighed: bool,
    /// How this food's weight reads here, when not the household's usual way.
    pub weight_unit: Option<Weights>,
    pub checked: bool,
    /// The dishes that need it.
    pub recipes: Vec<String>,
}

#[derive(Serialize, ToSchema)]
pub struct ShoppingSection {
    pub name: String,
    pub items: Vec<ShoppingItem>,
}

#[derive(Serialize, ToSchema)]
pub struct ShoppingExtra {
    pub id: Uuid,
    pub name: String,
    pub checked: bool,
}

/// Something left off the list, to put back.
#[derive(Serialize, ToSchema)]
pub struct LeftOut {
    pub key: String,
    pub name: String,
}

#[derive(Serialize, ToSchema)]
pub struct ShoppingList {
    /// The Monday.
    pub week: NaiveDate,
    /// Dishes planned this week.
    pub meals: usize,
    pub extras: Vec<ShoppingExtra>,
    /// In the order a shop is walked.
    pub sections: Vec<ShoppingSection>,
    /// Taken off this week's list.
    pub removed: Vec<LeftOut>,
    /// Left off because the household always has them; `key` is the food id.
    pub staples: Vec<LeftOut>,
    /// How the household reads weights, unless a food says otherwise.
    pub weight_unit: Weights,
}

/// One food across the week.
struct Group {
    food_id: Option<Uuid>,
    name: String,
    section: String,
    /// Every line for it is a pantry line.
    pantry: bool,
    count: Option<CountUnit>,
    needs: Vec<Quantity>,
    recipes: Vec<String>,
}

type LineRow = (
    Uuid,
    Uuid,
    String,
    Option<Uuid>,
    Option<String>,
    Option<bool>,
    Option<String>,
    Option<String>,
    Option<String>,
);

async fn build(s: &AppState, household: Uuid, week: NaiveDate) -> ApiResult<ShoppingList> {
    let entries: Vec<(Uuid, f64, String)> = sqlx::query_as(
        "SELECT e.variant_id, e.servings::float8, coalesce(d.title, v.title) FROM plan_entries e
         JOIN variants v ON v.id = e.variant_id LEFT JOIN dishes d ON d.id = v.dish_id
         WHERE e.household_id = $1 AND e.week = $2 ORDER BY e.created_at, e.id",
    )
    .bind(household)
    .bind(week)
    .fetch_all(&s.pool)
    .await?;
    let variants: Vec<Uuid> = entries.iter().map(|e| e.0).collect();

    // A food's section is its own, or the nearest ancestor's.
    let lines: Vec<LineRow> = sqlx::query_as(
        "SELECT il.id, il.variant_id, il.raw_name, il.food_id, f.name, il.shipped, f.props->>'count_unit', f.props->>'count_size', section.name
         FROM ingredient_lines il LEFT JOIN foods f ON f.id = il.food_id
         LEFT JOIN LATERAL (
             WITH RECURSIVE up AS (
                 SELECT f.parent_id, f.props->>'store_section' AS name, 0 AS depth
                 UNION ALL
                 SELECT p.parent_id, p.props->>'store_section', up.depth + 1 FROM foods p JOIN up ON p.id = up.parent_id
             )
             SELECT up.name FROM up WHERE up.name IS NOT NULL ORDER BY up.depth LIMIT 1
         ) section ON true
         WHERE il.variant_id = ANY($1) ORDER BY il.position",
    )
    .bind(&variants)
    .fetch_all(&s.pool)
    .await?;
    let mut amounts: HashMap<Uuid, Vec<(f64, f64, Option<String>)>> = HashMap::new();
    for (line, sv, a, u) in sqlx::query_as::<_, (Uuid, f64, f64, Option<String>)>(
        "SELECT la.line_id, la.servings::float8, la.amount::float8, la.unit FROM line_amounts la
         JOIN ingredient_lines il ON il.id = la.line_id WHERE il.variant_id = ANY($1) AND la.amount > 0",
    )
    .bind(&variants)
    .fetch_all(&s.pool)
    .await?
    {
        amounts.entry(line).or_default().push((sv, a, u));
    }

    // As the dish page does: published amounts that would shrink as servings grow are derived instead.
    let mut yields: HashMap<Uuid, Vec<f64>> = HashMap::new();
    for (v, y) in sqlx::query_as::<_, (Uuid, f64)>(
        "SELECT variant_id, servings::float8 FROM variant_yields WHERE variant_id = ANY($1)",
    )
    .bind(&variants)
    .fetch_all(&s.pool)
    .await?
    {
        yields.entry(v).or_default().push(y);
    }
    let mut templates: HashMap<Uuid, Vec<Template>> = HashMap::new();
    for (v, t) in sqlx::query_as::<_, (Uuid, serde_json::Value)>("SELECT variant_id, template FROM steps WHERE variant_id = ANY($1) AND template IS NOT NULL")
        .bind(&variants)
        .fetch_all(&s.pool)
        .await?
    {
        if let Ok(t) = serde_json::from_value(t) {
            templates.entry(v).or_default().push(t);
        }
    }
    for (line, variant, ..) in &lines {
        if let Some(list) = amounts.get_mut(line) {
            let ys = yields.get(variant).map(Vec::as_slice).unwrap_or(&[]);
            let base = step_amounts::text_base(ys, templates.get(variant).into_iter().flatten());
            servings::keep_consistent(list, &[base, base * 2.0]);
        }
    }

    let mut groups: BTreeMap<String, Group> = BTreeMap::new();
    for (variant, target, dish) in &entries {
        for (line, line_variant, raw, food_id, food, shipped, count_unit, count_size, section) in
            &lines
        {
            if line_variant != variant {
                continue;
            }
            let key = food_id.map_or_else(|| raw.to_lowercase(), |f| f.to_string());
            let g = groups.entry(key).or_insert_with(|| Group {
                food_id: *food_id,
                name: food.clone().unwrap_or_else(|| raw.clone()),
                section: section.clone().unwrap_or_else(|| OTHER.to_string()),
                pantry: true,
                count: CountUnit::from_props(count_unit.as_deref(), count_size.as_deref()),
                needs: Vec::new(),
                recipes: Vec::new(),
            });
            g.pantry &= *shipped == Some(false);
            g.needs.extend(
                amounts
                    .get(line)
                    .and_then(|a| servings::line_quantity(a, *target)),
            );
            if !g.recipes.contains(dish) {
                g.recipes.push(dish.clone());
            }
        }
    }

    let marks: HashMap<String, (bool, bool)> =
        sqlx::query_as::<_, (String, bool, bool)>("SELECT item_key, checked, hidden FROM shopping_marks WHERE household_id = $1 AND week = $2")
            .bind(household)
            .bind(week)
            .fetch_all(&s.pool)
            .await?
            .into_iter()
            .map(|(k, c, h)| (k, (c, h)))
            .collect();
    let staples: Vec<Uuid> =
        sqlx::query_scalar("SELECT food_id FROM pantry_staples WHERE household_id = $1")
            .bind(household)
            .fetch_all(&s.pool)
            .await?;
    let usual: String = sqlx::query_scalar("SELECT weight_unit FROM households WHERE id = $1")
        .bind(household)
        .fetch_one(&s.pool)
        .await?;
    let usual = Weights::stored(&usual);
    let per_food: HashMap<Uuid, Weights> = sqlx::query_as::<_, (Uuid, String)>(
        "SELECT food_id, weight_unit FROM food_weight_units WHERE household_id = $1",
    )
    .bind(household)
    .fetch_all(&s.pool)
    .await?
    .into_iter()
    .map(|(f, w)| (f, Weights::stored(&w)))
    .collect();

    let mut by_section: BTreeMap<(usize, String), Vec<ShoppingItem>> = BTreeMap::new();
    let (mut removed, mut left_out) = (Vec::new(), Vec::new());
    for (key, g) in groups {
        if g.food_id.is_some_and(|f| staples.contains(&f)) {
            left_out.push(LeftOut { key, name: g.name });
            continue;
        }
        let (checked, hidden) = marks.get(&key).copied().unwrap_or((false, false));
        if hidden {
            removed.push(LeftOut { key, name: g.name });
            continue;
        }
        let section = if g.pantry {
            CHECK.to_string()
        } else {
            g.section
        };
        let weight_unit = g.food_id.and_then(|f| per_food.get(&f).copied());
        let weighed = g.needs.iter().any(|q| q.unit.family() == Family::Weight)
            || g.count
                .as_ref()
                .and_then(|c| c.size.as_ref())
                .is_some_and(|s| s.unit.family() == Family::Weight);
        by_section
            .entry((shopping::section_order(&section), section))
            .or_default()
            .push(ShoppingItem {
                amount: shopping::total(
                    &g.needs,
                    g.count.as_ref(),
                    weight_unit.unwrap_or(usual).core(),
                ),
                weighed,
                weight_unit,
                key,
                food_id: g.food_id,
                name: g.name,
                checked,
                recipes: g.recipes,
            });
    }
    let sections = by_section
        .into_iter()
        .map(|((_, name), mut items)| {
            items.sort_by_key(|i| i.name.to_lowercase());
            ShoppingSection { name, items }
        })
        .collect();
    removed.sort_by_key(|l: &LeftOut| l.name.to_lowercase());
    left_out.sort_by_key(|l: &LeftOut| l.name.to_lowercase());

    let extras = sqlx::query_as::<_, (Uuid, String, bool)>("SELECT id, name, checked FROM shopping_extras WHERE household_id = $1 AND week = $2 ORDER BY created_at, id")
        .bind(household)
        .bind(week)
        .fetch_all(&s.pool)
        .await?
        .into_iter()
        .map(|(id, name, checked)| ShoppingExtra { id, name, checked })
        .collect();
    Ok(ShoppingList {
        week,
        meals: entries.len(),
        extras,
        sections,
        removed,
        staples: left_out,
        weight_unit: usual,
    })
}

/// A week's shopping list.
#[utoipa::path(get, path = "/api/shopping", operation_id = "get_shopping", tag = "shopping", params(WeekParams), responses((status = 200, description = "The week's shopping list", body = ShoppingList)))]
pub async fn get(
    State(s): State<AppState>,
    Extension(me): Extension<CurrentUser>,
    Query(p): Query<WeekParams>,
) -> ApiResult<Json<ShoppingList>> {
    Ok(Json(build(&s, me.household_id, monday(p.week)).await?))
}

#[derive(Deserialize, ToSchema)]
pub struct Mark {
    /// Any day in the week (default: today).
    pub week: Option<NaiveDate>,
    pub key: String,
    /// Ticked off.
    pub checked: Option<bool>,
    /// Taken off this week's list.
    pub hidden: Option<bool>,
}

/// Tick an item off, or take it off or put it back on this week's list. Answers with the list.
#[utoipa::path(put, path = "/api/shopping/marks", operation_id = "mark_shopping_item", tag = "shopping", request_body = Mark, responses((status = 200, description = "The updated list", body = ShoppingList)))]
pub async fn mark(
    State(s): State<AppState>,
    Extension(me): Extension<CurrentUser>,
    JsonBody(b): JsonBody<Mark>,
) -> ApiResult<Json<ShoppingList>> {
    clean(&b.key)?;
    let week = monday(b.week);
    sqlx::query(
        "INSERT INTO shopping_marks (household_id, week, item_key, checked, hidden) VALUES ($1, $2, $3, coalesce($4, false), coalesce($5, false))
         ON CONFLICT (household_id, week, item_key)
         DO UPDATE SET checked = coalesce($4, shopping_marks.checked), hidden = coalesce($5, shopping_marks.hidden)",
    )
    .bind(me.household_id)
    .bind(week)
    .bind(&b.key)
    .bind(b.checked)
    .bind(b.hidden)
    .execute(&s.pool)
    .await?;
    Ok(Json(build(&s, me.household_id, week).await?))
}

/// Text Postgres can store, of a sensible length.
fn clean(text: &str) -> ApiResult<&str> {
    let t = text.trim();
    if t.is_empty() || t.len() > 200 || t.contains('\0') {
        return Err(ApiError::BadRequest(
            "enter between 1 and 200 characters".into(),
        ));
    }
    Ok(t)
}

#[derive(Deserialize, ToSchema)]
pub struct NewExtra {
    /// Any day in the week (default: today).
    pub week: Option<NaiveDate>,
    pub name: String,
}

/// Add something to a week's list by hand. Answers with the list.
#[utoipa::path(
    post, path = "/api/shopping/extras", operation_id = "add_shopping_extra", tag = "shopping", request_body = NewExtra,
    responses((status = 200, description = "The list with the item added", body = ShoppingList), (status = 400, description = "The name is empty or too long", body = crate::error::ErrorBody))
)]
pub async fn add_extra(
    State(s): State<AppState>,
    Extension(me): Extension<CurrentUser>,
    JsonBody(b): JsonBody<NewExtra>,
) -> ApiResult<Json<ShoppingList>> {
    let week = monday(b.week);
    sqlx::query(
        "INSERT INTO shopping_extras (id, household_id, week, name) VALUES ($1, $2, $3, $4)",
    )
    .bind(Uuid::now_v7())
    .bind(me.household_id)
    .bind(week)
    .bind(clean(&b.name)?)
    .execute(&s.pool)
    .await?;
    Ok(Json(build(&s, me.household_id, week).await?))
}

#[derive(Deserialize, ToSchema)]
pub struct ExtraChange {
    pub checked: bool,
}

/// Tick a hand-added item off. Answers with the list.
#[utoipa::path(
    patch, path = "/api/shopping/extras/{id}", operation_id = "update_shopping_extra", tag = "shopping", params(("id" = Uuid, Path)), request_body = ExtraChange,
    responses((status = 200, description = "The updated list", body = ShoppingList), (status = 404, description = "No such item on this household's list", body = crate::error::ErrorBody))
)]
pub async fn update_extra(
    State(s): State<AppState>,
    Extension(me): Extension<CurrentUser>,
    Path(id): Path<Uuid>,
    JsonBody(b): JsonBody<ExtraChange>,
) -> ApiResult<Json<ShoppingList>> {
    let week: NaiveDate = sqlx::query_scalar("UPDATE shopping_extras SET checked = $3 WHERE id = $1 AND household_id = $2 RETURNING week")
        .bind(id)
        .bind(me.household_id)
        .bind(b.checked)
        .fetch_optional(&s.pool)
        .await?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(build(&s, me.household_id, week).await?))
}

/// Delete a hand-added item. Answers with the list.
#[utoipa::path(
    delete, path = "/api/shopping/extras/{id}", operation_id = "delete_shopping_extra", tag = "shopping", params(("id" = Uuid, Path)),
    responses((status = 200, description = "The list without it", body = ShoppingList), (status = 404, description = "No such item on this household's list", body = crate::error::ErrorBody))
)]
pub async fn remove_extra(
    State(s): State<AppState>,
    Extension(me): Extension<CurrentUser>,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<ShoppingList>> {
    let week: NaiveDate = sqlx::query_scalar(
        "DELETE FROM shopping_extras WHERE id = $1 AND household_id = $2 RETURNING week",
    )
    .bind(id)
    .bind(me.household_id)
    .fetch_optional(&s.pool)
    .await?
    .ok_or(ApiError::NotFound)?;
    Ok(Json(build(&s, me.household_id, week).await?))
}

#[derive(Deserialize, ToSchema)]
pub struct NewStaple {
    /// Any day in the week to answer with (default: today).
    pub week: Option<NaiveDate>,
    pub food_id: Uuid,
}

/// Mark a food as always on hand, leaving it off every list. Answers with the list.
#[utoipa::path(
    post, path = "/api/pantry-staples", operation_id = "add_pantry_staple", tag = "shopping", request_body = NewStaple,
    responses((status = 200, description = "The list without that food", body = ShoppingList), (status = 404, description = "No such food", body = crate::error::ErrorBody))
)]
pub async fn add_staple(
    State(s): State<AppState>,
    Extension(me): Extension<CurrentUser>,
    JsonBody(b): JsonBody<NewStaple>,
) -> ApiResult<Json<ShoppingList>> {
    let added = sqlx::query("INSERT INTO pantry_staples (household_id, food_id) SELECT $1, f.id FROM foods f WHERE f.id = $2 ON CONFLICT DO NOTHING")
        .bind(me.household_id)
        .bind(b.food_id)
        .execute(&s.pool)
        .await?;
    if added.rows_affected() == 0
        && !sqlx::query_scalar::<_, bool>("SELECT EXISTS (SELECT 1 FROM foods WHERE id = $1)")
            .bind(b.food_id)
            .fetch_one(&s.pool)
            .await?
    {
        return Err(ApiError::NotFound);
    }
    Ok(Json(build(&s, me.household_id, monday(b.week)).await?))
}

/// Put a food back on the list. Answers with the list.
#[utoipa::path(
    delete, path = "/api/pantry-staples/{food_id}", operation_id = "delete_pantry_staple", tag = "shopping",
    params(("food_id" = Uuid, Path), WeekParams), responses((status = 200, description = "The list with that food back on it", body = ShoppingList))
)]
pub async fn remove_staple(
    State(s): State<AppState>,
    Extension(me): Extension<CurrentUser>,
    Path(food_id): Path<Uuid>,
    Query(p): Query<WeekParams>,
) -> ApiResult<Json<ShoppingList>> {
    sqlx::query("DELETE FROM pantry_staples WHERE household_id = $1 AND food_id = $2")
        .bind(me.household_id)
        .bind(food_id)
        .execute(&s.pool)
        .await?;
    Ok(Json(build(&s, me.household_id, monday(p.week)).await?))
}

#[derive(Serialize, Deserialize, ToSchema)]
pub struct ShoppingSettings {
    /// How the household reads weights on its list.
    pub weight_unit: Weights,
}

/// The household's shopping list settings.
#[utoipa::path(get, path = "/api/shopping/settings", operation_id = "get_shopping_settings", tag = "shopping", responses((status = 200, description = "The household's shopping settings", body = ShoppingSettings)))]
pub async fn settings(
    State(s): State<AppState>,
    Extension(me): Extension<CurrentUser>,
) -> ApiResult<Json<ShoppingSettings>> {
    let w: String = sqlx::query_scalar("SELECT weight_unit FROM households WHERE id = $1")
        .bind(me.household_id)
        .fetch_one(&s.pool)
        .await?;
    Ok(Json(ShoppingSettings {
        weight_unit: Weights::stored(&w),
    }))
}

/// Change the household's shopping list settings, for everyone in it.
#[utoipa::path(put, path = "/api/shopping/settings", operation_id = "update_shopping_settings", tag = "shopping", request_body = ShoppingSettings, responses((status = 200, description = "The saved settings", body = ShoppingSettings)))]
pub async fn update_settings(
    State(s): State<AppState>,
    Extension(me): Extension<CurrentUser>,
    JsonBody(b): JsonBody<ShoppingSettings>,
) -> ApiResult<Json<ShoppingSettings>> {
    sqlx::query("UPDATE households SET weight_unit = $2 WHERE id = $1")
        .bind(me.household_id)
        .bind(b.weight_unit.core().as_str())
        .execute(&s.pool)
        .await?;
    Ok(Json(b))
}

#[derive(Deserialize, ToSchema)]
pub struct FoodWeights {
    /// Any day in the week to answer with (default: today).
    pub week: Option<NaiveDate>,
    pub food_id: Uuid,
    /// How this food's weight reads; leave out to go back to the household's usual way.
    pub weight_unit: Option<Weights>,
}

/// Choose how one food's weight reads on the household's lists. Answers with the list.
#[utoipa::path(
    put, path = "/api/shopping/food-weights", operation_id = "set_food_weights", tag = "shopping", request_body = FoodWeights,
    responses((status = 200, description = "The updated list", body = ShoppingList), (status = 404, description = "No such food", body = crate::error::ErrorBody))
)]
pub async fn set_food_weights(
    State(s): State<AppState>,
    Extension(me): Extension<CurrentUser>,
    JsonBody(b): JsonBody<FoodWeights>,
) -> ApiResult<Json<ShoppingList>> {
    if !sqlx::query_scalar::<_, bool>("SELECT EXISTS (SELECT 1 FROM foods WHERE id = $1)")
        .bind(b.food_id)
        .fetch_one(&s.pool)
        .await?
    {
        return Err(ApiError::NotFound);
    }
    match b.weight_unit {
        Some(w) => sqlx::query(
            "INSERT INTO food_weight_units (household_id, food_id, weight_unit) VALUES ($1, $2, $3)
             ON CONFLICT (household_id, food_id) DO UPDATE SET weight_unit = $3",
        )
        .bind(me.household_id)
        .bind(b.food_id)
        .bind(w.core().as_str()),
        None => {
            sqlx::query("DELETE FROM food_weight_units WHERE household_id = $1 AND food_id = $2")
                .bind(me.household_id)
                .bind(b.food_id)
        }
    }
    .execute(&s.pool)
    .await?;
    Ok(Json(build(&s, me.household_id, monday(b.week)).await?))
}
