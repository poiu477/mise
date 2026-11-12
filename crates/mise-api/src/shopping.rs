//! The shopping list: worked out from the week's plan each time, with what people ticked, removed and added on top.

use std::collections::{BTreeMap, HashMap, HashSet};

use axum::{Extension, Json, extract::State};
use chrono::NaiveDate;
use mise_core::quantity::{Quantity, Unit};
use mise_core::servings::{self, CountUnit};
use mise_core::shopping::{self, CHECK, MarkLine, Measures, OTHER, TreeFood, WeightUnit};
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
    /// The key the device that added it sent, if it sent one.
    pub client_key: Option<String>,
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

/// One line of the week's list before marks: the foods it adds up and what it needs of them.
struct Line {
    /// Names the line when ticking or removing it: the line's food id, or the name of an unmatched line.
    key: String,
    /// The line's food: the parent its kinds add up at, or the one food.
    food_id: Option<Uuid>,
    /// The keys marks are kept under, one per food on the line.
    foods: Vec<String>,
    name: String,
    section: String,
    /// Every ingredient line for it is a pantry line.
    pantry: bool,
    count: Option<CountUnit>,
    measures: Measures,
    needs: Vec<Quantity>,
    recipes: Vec<String>,
}

/// A food as the list needs it: its place in the tree and what reads and converts its amounts.
#[derive(Default)]
struct FoodRow {
    name: String,
    tree: TreeFood,
    count: Option<CountUnit>,
    section: Option<String>,
    measures: Measures,
}

/// The week's lines, worked out from the plan, and the food tree they came from.
struct Planned {
    entries: usize,
    lines: Vec<Line>,
    /// Foods the household always has that the plan uses: (food id, name).
    staples: Vec<(Uuid, String)>,
    tree: HashMap<Uuid, TreeFood>,
}

impl Planned {
    /// The foods a mark sent with `key` lands on (see `shopping::foods_to_mark`).
    fn foods_to_mark(&self, key: &str) -> Vec<String> {
        let lines: Vec<MarkLine> = self
            .lines
            .iter()
            .map(|l| MarkLine {
                key: l.key.clone(),
                foods: l.foods.clone(),
            })
            .collect();
        shopping::foods_to_mark(key, &lines, |food, of| {
            match (Uuid::parse_str(food), Uuid::parse_str(of)) {
                (Ok(food), Ok(of)) => shopping::is_kind_of(food, of, &self.tree),
                _ => false,
            }
        })
    }
}

type LineRow = (Uuid, Uuid, String, Option<Uuid>, Option<bool>);
type FoodTreeRow = (
    Uuid,
    Option<Uuid>,
    bool,
    bool,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
);

/// The week's planned versions and their lines; like the plan, only versions the household can see.
async fn plan_lines(s: &AppState, household: Uuid, week: NaiveDate) -> ApiResult<Planned> {
    let entries: Vec<(Uuid, f64, String)> = sqlx::query_as(
        "SELECT e.variant_id, e.servings::float8, coalesce(d.title, v.title) FROM plan_entries e
         JOIN variants v ON v.id = e.variant_id LEFT JOIN dishes d ON d.id = v.dish_id
         WHERE e.household_id = $1 AND e.week = $2 AND visible_to(v, $1) ORDER BY e.created_at, e.id",
    )
    .bind(household)
    .bind(week)
    .fetch_all(&s.pool)
    .await?;
    let variants: Vec<Uuid> = entries.iter().map(|e| e.0).collect();

    let lines: Vec<LineRow> = sqlx::query_as(
        "SELECT il.id, il.variant_id, il.raw_name, il.food_id, il.shipped FROM ingredient_lines il
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

    // The plan's foods and everything they are kinds of, for sections, merging and conversions.
    let used_ids: Vec<Uuid> = lines.iter().filter_map(|l| l.3).collect();
    let mut foods: HashMap<Uuid, FoodRow> = HashMap::new();
    for (id, parent, category, shop_as_one, name, count_unit, count_size, section) in
        sqlx::query_as::<_, FoodTreeRow>(
            "WITH RECURSIVE up AS (
                 SELECT id FROM foods WHERE id = ANY($1)
                 UNION SELECT f.parent_id FROM foods f JOIN up ON f.id = up.id WHERE f.parent_id IS NOT NULL
             )
             SELECT f.id, f.parent_id, f.kind = 'category', coalesce(f.props->'shop_as_one' = 'true'::jsonb, false),
                    f.name, f.props->>'count_unit', f.props->>'count_size', f.props->>'store_section'
             FROM foods f JOIN up ON up.id = f.id",
        )
        .bind(&used_ids)
        .fetch_all(&s.pool)
        .await?
    {
        foods.insert(
            id,
            FoodRow {
                name,
                tree: TreeFood {
                    parent,
                    category,
                    shop_as_one,
                },
                count: CountUnit::from_props(count_unit.as_deref(), count_size.as_deref()),
                section,
                measures: Measures::default(),
            },
        );
    }
    let known: Vec<Uuid> = foods.keys().copied().collect();
    for (food, kind, item, ml, grams) in sqlx::query_as::<_, (Uuid, String, String, Option<f64>, Option<f64>)>(
        "SELECT food_id, kind, item, ml::float8, grams::float8 FROM food_measures WHERE food_id = ANY($1) ORDER BY created_at, id",
    )
    .bind(&known)
    .fetch_all(&s.pool)
    .await?
    {
        let Some(f) = foods.get_mut(&food) else { continue };
        match (kind.as_str(), ml, grams) {
            ("density", Some(ml), Some(g)) => f.measures.add_density(ml, g),
            ("item", ml, g) => {
                if let Some(ml) = ml {
                    f.measures.add_item(&item, &Quantity::new(ml, Unit::Ml));
                }
                if let Some(g) = g {
                    f.measures.add_item(&item, &Quantity::new(g, Unit::G));
                }
            }
            _ => {}
        }
    }
    let tree: HashMap<Uuid, TreeFood> = foods.iter().map(|(id, f)| (*id, f.tree.clone())).collect();
    // A food's count words, section and measures are its own, or its nearest ancestor's.
    let nearest = |food: Uuid| {
        shopping::ancestors(food, &tree)
            .into_iter()
            .filter_map(|a| foods.get(&a))
    };
    let count_of = |food: Uuid| nearest(food).find_map(|f| f.count.clone());

    let staple_ids: Vec<Uuid> =
        sqlx::query_scalar("SELECT food_id FROM pantry_staples WHERE household_id = $1")
            .bind(household)
            .fetch_all(&s.pool)
            .await?;

    // Every food the plan uses, in the plan's order, with what it needs.
    struct Use {
        pantry: bool,
        needs: Vec<Quantity>,
        recipes: Vec<String>,
    }
    let mut order: Vec<String> = Vec::new();
    let mut uses: HashMap<String, Use> = HashMap::new();
    let mut raw_names: HashMap<String, String> = HashMap::new();
    for (variant, target, dish) in &entries {
        for (line, line_variant, raw, food_id, shipped) in &lines {
            if line_variant != variant {
                continue;
            }
            let key = food_id.map_or_else(|| raw.to_lowercase(), |f| f.to_string());
            raw_names.entry(key.clone()).or_insert_with(|| raw.clone());
            let u = uses.entry(key.clone()).or_insert_with(|| {
                order.push(key.clone());
                Use {
                    pantry: true,
                    needs: Vec::new(),
                    recipes: Vec::new(),
                }
            });
            u.pantry &= *shipped == Some(false);
            let count = food_id.and_then(count_of);
            // A bare "1" is counted in the food's own words before kinds of one food add up together.
            u.needs.extend(
                amounts
                    .get(line)
                    .and_then(|a| servings::line_quantity(a, *target))
                    .map(|q| match (&q.unit, &count) {
                        (Unit::Count(noun), Some(c)) if noun.is_empty() => {
                            Quantity::new(q.amount, Unit::Count(c.noun.clone()))
                        }
                        _ => q,
                    }),
            );
            if !u.recipes.contains(dish) {
                u.recipes.push(dish.clone());
            }
        }
    }

    // A line whose food the household always has is left off whole, kinds and all: "Always have" on the onion line
    // sends the onion. A kind that is a staple itself leaves only its own part off. The rest is gathered again
    // without them.
    let matched: Vec<Uuid> = order
        .iter()
        .filter_map(|k| Uuid::parse_str(k).ok())
        .filter(|f| foods.contains_key(f))
        .collect();
    let mut staples = Vec::new();
    let mut left_off: HashSet<Uuid> = HashSet::new();
    for g in shopping::gather(&matched, &tree) {
        if staple_ids.contains(&g.line) {
            staples.push((g.line, foods[&g.line].name.clone()));
            left_off.extend(g.foods);
        }
    }
    for f in &matched {
        if staple_ids.contains(f) && left_off.insert(*f) {
            staples.push((*f, foods[f].name.clone()));
        }
    }
    let bought: Vec<Uuid> = matched
        .into_iter()
        .filter(|f| !left_off.contains(f))
        .collect();
    let gathered: HashMap<Uuid, shopping::Gathered> = shopping::gather(&bought, &tree)
        .into_iter()
        .map(|g| (g.foods[0], g))
        .collect();

    let mut out = Vec::new();
    for key in &order {
        let food = Uuid::parse_str(key).ok().filter(|f| foods.contains_key(f));
        let Some(food) = food else {
            let u = &uses[key];
            out.push(Line {
                key: key.clone(),
                food_id: None,
                foods: vec![key.clone()],
                name: raw_names[key].clone(),
                section: OTHER.to_string(),
                pantry: u.pantry,
                count: None,
                measures: Measures::default(),
                needs: u.needs.clone(),
                recipes: u.recipes.clone(),
            });
            continue;
        };
        // Each line once, where its first food comes in the plan.
        let Some(g) = gathered.get(&food) else {
            continue;
        };
        let mut line = Line {
            key: g.line.to_string(),
            food_id: Some(g.line),
            foods: g.foods.iter().map(Uuid::to_string).collect(),
            name: foods[&g.named].name.clone(),
            section: nearest(g.named)
                .find_map(|f| f.section.clone())
                .unwrap_or_else(|| OTHER.to_string()),
            pantry: true,
            count: count_of(g.named)
                .or_else(|| count_of(g.line))
                .or_else(|| g.foods.iter().find_map(|f| count_of(*f))),
            measures: Measures::default(),
            needs: Vec::new(),
            recipes: Vec::new(),
        };
        // The named food's measures first, then the others', then what the line's food is a kind of.
        let mut by = vec![g.named];
        by.extend(g.foods.iter().copied().filter(|f| *f != g.named));
        by.extend(shopping::ancestors(g.line, &tree).into_iter().skip(1));
        for f in by {
            if let Some(f) = foods.get(&f) {
                line.measures.add(&f.measures);
            }
        }
        for f in &g.foods {
            let u = &uses[&f.to_string()];
            line.pantry &= u.pantry;
            line.needs.extend(u.needs.iter().cloned());
            for r in &u.recipes {
                if !line.recipes.contains(r) {
                    line.recipes.push(r.clone());
                }
            }
        }
        out.push(line);
    }
    Ok(Planned {
        entries: entries.len(),
        lines: out,
        staples,
        tree,
    })
}

async fn build(s: &AppState, household: Uuid, week: NaiveDate) -> ApiResult<ShoppingList> {
    let planned = plan_lines(s, household, week).await?;
    // Marks are kept per food, so a line shows ticked (or removed) when every food on it is.
    let marks: HashMap<String, (bool, bool)> =
        sqlx::query_as::<_, (String, bool, bool)>("SELECT item_key, checked, hidden FROM shopping_marks WHERE household_id = $1 AND week = $2")
            .bind(household)
            .bind(week)
            .fetch_all(&s.pool)
            .await?
            .into_iter()
            .map(|(k, c, h)| (k, (c, h)))
            .collect();
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
    let mut removed = Vec::new();
    let mut left_out: Vec<LeftOut> = planned
        .staples
        .into_iter()
        .map(|(id, name)| LeftOut {
            key: id.to_string(),
            name,
        })
        .collect();
    for l in planned.lines {
        let marked = |which: fn(&(bool, bool)) -> bool| {
            shopping::all_marked(&l.foods, |f| marks.get(f).is_some_and(which))
        };
        if marked(|m| m.1) {
            removed.push(LeftOut {
                key: l.key,
                name: l.name,
            });
            continue;
        }
        let checked = marked(|m| m.0);
        let section = if l.pantry {
            CHECK.to_string()
        } else {
            l.section
        };
        let weight_unit = l.food_id.and_then(|f| per_food.get(&f).copied());
        by_section
            .entry((shopping::section_order(&section), section))
            .or_default()
            .push(ShoppingItem {
                amount: l.measures.total(
                    &l.needs,
                    l.count.as_ref(),
                    weight_unit.unwrap_or(usual).core(),
                ),
                weighed: l.measures.weighed(&l.needs, l.count.as_ref()),
                weight_unit,
                key: l.key,
                food_id: l.food_id,
                name: l.name,
                checked,
                recipes: l.recipes,
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
    left_out.dedup_by(|a, b| a.key == b.key);

    let extras = sqlx::query_as::<_, (Uuid, String, bool, Option<String>)>("SELECT id, name, checked, client_key FROM shopping_extras WHERE household_id = $1 AND week = $2 ORDER BY created_at, id")
        .bind(household)
        .bind(week)
        .fetch_all(&s.pool)
        .await?
        .into_iter()
        .map(|(id, name, checked, client_key)| ShoppingExtra { id, name, checked, client_key })
        .collect();
    Ok(ShoppingList {
        week,
        meals: planned.entries,
        extras,
        sections,
        removed,
        staples: left_out,
        weight_unit: usual,
    })
}

/// A week's shopping list.
#[utoipa::path(get, path = "/api/v1/shopping", operation_id = "get_shopping", tag = "shopping", params(WeekParams), responses((status = 200, description = "The week's shopping list", body = ShoppingList)))]
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
#[utoipa::path(put, path = "/api/v1/shopping/marks", operation_id = "mark_shopping_item", tag = "shopping", request_body = Mark, responses((status = 200, description = "The updated list", body = ShoppingList)))]
pub async fn mark(
    State(s): State<AppState>,
    Extension(me): Extension<CurrentUser>,
    JsonBody(b): JsonBody<Mark>,
) -> ApiResult<Json<ShoppingList>> {
    clean(&b.key)?;
    let week = monday(b.week);
    // Marks are kept per food. The key names a line as the device saw it, which may have merged or split since,
    // so it marks every food on the lines that now cover it.
    let foods = plan_lines(&s, me.household_id, week)
        .await?
        .foods_to_mark(&b.key);
    sqlx::query(
        "INSERT INTO shopping_marks (household_id, week, item_key, checked, hidden)
         SELECT $1, $2, k, coalesce($4, false), coalesce($5, false) FROM UNNEST($3::text[]) AS k
         ON CONFLICT (household_id, week, item_key)
         DO UPDATE SET checked = coalesce($4, shopping_marks.checked), hidden = coalesce($5, shopping_marks.hidden)",
    )
    .bind(me.household_id)
    .bind(week)
    .bind(&foods)
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
    /// A key the device made for this addition (1 to 100 characters), unique in the household. Sending the same
    /// key again adds nothing, so a device can resend an addition whose answer it never got.
    pub client_key: Option<String>,
}

/// Add something to a week's list by hand. Answers with the list. With a `client_key` already used in this
/// household, nothing is added and the answer is the list as it stands.
#[utoipa::path(
    post, path = "/api/v1/shopping/extras", operation_id = "add_shopping_extra", tag = "shopping", request_body = NewExtra,
    responses(
        (status = 200, description = "The list with the item on it, added now or by an earlier request with the same key", body = ShoppingList),
        (status = 400, description = "The name or the key is empty or too long", body = crate::error::ErrorBody)
    )
)]
pub async fn add_extra(
    State(s): State<AppState>,
    Extension(me): Extension<CurrentUser>,
    JsonBody(b): JsonBody<NewExtra>,
) -> ApiResult<Json<ShoppingList>> {
    let week = monday(b.week);
    let key = b.client_key.as_deref();
    if key.is_some_and(|k| k.is_empty() || k.chars().count() > 100 || k.contains('\0')) {
        return Err(ApiError::BadRequest(
            "client_key must be between 1 and 100 characters".into(),
        ));
    }
    // Two missing keys never conflict, so additions without one always go in.
    sqlx::query(
        "INSERT INTO shopping_extras (id, household_id, week, name, client_key) VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT (household_id, client_key) DO NOTHING",
    )
    .bind(Uuid::now_v7())
    .bind(me.household_id)
    .bind(week)
    .bind(clean(&b.name)?)
    .bind(key)
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
    patch, path = "/api/v1/shopping/extras/{id}", operation_id = "update_shopping_extra", tag = "shopping", params(("id" = Uuid, Path)), request_body = ExtraChange,
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
    delete, path = "/api/v1/shopping/extras/{id}", operation_id = "delete_shopping_extra", tag = "shopping", params(("id" = Uuid, Path)),
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
    post, path = "/api/v1/pantry-staples", operation_id = "add_pantry_staple", tag = "shopping", request_body = NewStaple,
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
    delete, path = "/api/v1/pantry-staples/{food_id}", operation_id = "delete_pantry_staple", tag = "shopping",
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
#[utoipa::path(get, path = "/api/v1/shopping/settings", operation_id = "get_shopping_settings", tag = "shopping", responses((status = 200, description = "The household's shopping settings", body = ShoppingSettings)))]
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
#[utoipa::path(put, path = "/api/v1/shopping/settings", operation_id = "update_shopping_settings", tag = "shopping", request_body = ShoppingSettings, responses((status = 200, description = "The saved settings", body = ShoppingSettings)))]
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
    put, path = "/api/v1/shopping/food-weights", operation_id = "set_food_weights", tag = "shopping", request_body = FoodWeights,
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
