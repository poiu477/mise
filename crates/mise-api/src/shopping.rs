//! The shopping list: worked out from the week's plan each time, with what people ticked, removed and added on top.

use std::collections::{BTreeMap, HashMap, HashSet};

use axum::{Extension, Json, extract::State};
use mise_core::quantity::{Quantity, Unit};
use mise_core::servings::{self, CountUnit};
use mise_core::shopping::{self, CHECK, MarkLine, Measures, OTHER, TreeFood, WeightUnit};
use mise_core::step_amounts::{self, Template};
use serde::{Deserialize, Serialize};
use time::Date;
use utoipa::ToSchema;
use uuid::Uuid;

mod queries;

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
    /// Pounds and ounces: "1 lb 14 oz".
    LbOz,
    /// Ounces: "30 oz".
    Oz,
    /// Pounds: "1⅞ lb".
    Lb,
    /// Grams and kilograms: "850 g".
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

/// One thing to buy: a food, with what the week's dishes need of it added up.
#[derive(Serialize, ToSchema)]
pub struct ShoppingItem {
    /// Names the item when ticking or removing it.
    pub key: String,
    /// The food it is, when the lines were matched to one.
    pub food_id: Option<Uuid>,
    /// What to buy, as the list names it.
    pub name: String,
    /// Added up across recipes, exactly, e.g. "3 packets (4½ oz)"; absent for "to taste" items like salt.
    pub amount: Option<String>,
    /// Some of it is a weight, so the household can choose how it reads.
    pub weighed: bool,
    /// How this food's weight reads here, when not the household's usual way.
    pub weight_unit: Option<Weights>,
    /// Ticked off this week.
    pub checked: bool,
    /// The dishes that need it.
    pub recipes: Vec<String>,
}

/// A section of the shop, and what to buy there.
#[derive(Serialize, ToSchema)]
pub struct ShoppingSection {
    /// The section ("Produce"), or "Check you have" for pantry items.
    pub name: String,
    /// What to buy in it, by name.
    pub items: Vec<ShoppingItem>,
}

/// Something added to the week's list by hand.
#[derive(Serialize, ToSchema)]
pub struct ShoppingExtra {
    /// The extra's id, to tick or remove it.
    pub id: Uuid,
    /// What was written ("foil").
    pub name: String,
    /// Ticked off.
    pub checked: bool,
    /// The key the device that added it sent, if it sent one.
    pub client_key: Option<String>,
}

/// Something left off the list, to put back.
#[derive(Serialize, ToSchema)]
pub struct LeftOut {
    /// The key to put it back with: the item's key, or for a staple the food's id.
    pub key: String,
    /// What it is.
    pub name: String,
}

/// A household's shopping list for one week, worked out from its plan each time it is asked for.
#[derive(Serialize, ToSchema)]
pub struct ShoppingList {
    /// The Monday.
    pub week: Date,
    /// Dishes planned this week.
    pub meals: usize,
    /// Things added by hand.
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

/// The week's planned versions and their lines; like the plan, only versions the household can see.
async fn plan_lines(s: &AppState, household: Uuid, week: Date) -> ApiResult<Planned> {
    let entries = queries::planned(&s.pool, household, week).await?;
    let variants: Vec<Uuid> = entries.iter().map(|e| e.variant_id).collect();

    let lines = queries::lines(&s.pool, &variants).await?;
    let mut amounts: HashMap<Uuid, Vec<(f64, f64, Option<String>)>> = HashMap::new();
    for a in queries::amounts(&s.pool, &variants).await? {
        amounts
            .entry(a.line_id)
            .or_default()
            .push((a.servings, a.amount, a.unit));
    }

    // As the dish page does: published amounts that would shrink as servings grow are derived instead.
    let mut yields: HashMap<Uuid, Vec<f64>> = HashMap::new();
    for y in queries::yields(&s.pool, &variants).await? {
        yields.entry(y.variant_id).or_default().push(y.servings);
    }
    let mut templates: HashMap<Uuid, Vec<Template>> = HashMap::new();
    for t in queries::templates(&s.pool, &variants).await? {
        if let Ok(template) = serde_json::from_value(t.template) {
            templates.entry(t.variant_id).or_default().push(template);
        }
    }
    for l in &lines {
        if let Some(list) = amounts.get_mut(&l.id) {
            let variant = &l.variant_id;
            let ys = yields.get(variant).map(Vec::as_slice).unwrap_or(&[]);
            let base = step_amounts::text_base(ys, templates.get(variant).into_iter().flatten());
            servings::keep_consistent(list, &[base, base * 2.0]);
        }
    }

    // The plan's foods and everything they are kinds of, for sections, merging and conversions.
    let used_ids: Vec<Uuid> = lines.iter().filter_map(|l| l.food_id).collect();
    let mut foods: HashMap<Uuid, FoodRow> = HashMap::new();
    for f in queries::food_tree(&s.pool, &used_ids).await? {
        foods.insert(
            f.id,
            FoodRow {
                name: f.name,
                tree: TreeFood {
                    parent: f.parent_id,
                    category: f.category,
                    shop_as_one: f.shop_as_one,
                },
                count: CountUnit::from_props(f.count_unit.as_deref(), f.count_size.as_deref()),
                section: f.section,
                measures: Measures::default(),
            },
        );
    }
    let known: Vec<Uuid> = foods.keys().copied().collect();
    for m in queries::measures(&s.pool, &known).await? {
        let Some(f) = foods.get_mut(&m.food_id) else {
            continue;
        };
        match (m.kind.as_str(), m.ml, m.grams) {
            ("density", Some(ml), Some(g)) => f.measures.add_density(ml, g),
            ("item", ml, g) => {
                if let Some(ml) = ml {
                    f.measures.add_item(&m.item, &Quantity::new(ml, Unit::Ml));
                }
                if let Some(g) = g {
                    f.measures.add_item(&m.item, &Quantity::new(g, Unit::G));
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

    let staple_ids = queries::staples(&s.pool, household).await?;

    // Every food the plan uses, in the plan's order, with what it needs.
    struct Use {
        pantry: bool,
        needs: Vec<Quantity>,
        recipes: Vec<String>,
    }
    let mut order: Vec<String> = Vec::new();
    let mut uses: HashMap<String, Use> = HashMap::new();
    let mut raw_names: HashMap<String, String> = HashMap::new();
    for e in &entries {
        for l in lines.iter().filter(|l| l.variant_id == e.variant_id) {
            let key = l
                .food_id
                .map_or_else(|| l.raw_name.to_lowercase(), |f| f.to_string());
            raw_names
                .entry(key.clone())
                .or_insert_with(|| l.raw_name.clone());
            let u = uses.entry(key.clone()).or_insert_with(|| {
                order.push(key.clone());
                Use {
                    pantry: true,
                    needs: Vec::new(),
                    recipes: Vec::new(),
                }
            });
            u.pantry &= l.shipped == Some(false);
            let count = l.food_id.and_then(count_of);
            // A bare "1" is counted in the food's own words before kinds of one food add up together.
            u.needs.extend(
                amounts
                    .get(&l.id)
                    .and_then(|a| servings::line_quantity(a, e.servings))
                    .map(|q| match (&q.unit, &count) {
                        (Unit::Count(noun), Some(c)) if noun.is_empty() => {
                            Quantity::new(q.amount, Unit::Count(c.noun.clone()))
                        }
                        _ => q,
                    }),
            );
            if !u.recipes.contains(&e.dish) {
                u.recipes.push(e.dish.clone());
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

async fn build(s: &AppState, household: Uuid, week: Date) -> ApiResult<ShoppingList> {
    let planned = plan_lines(s, household, week).await?;
    // Marks are kept per food, so a line shows ticked (or removed) when every food on it is.
    let marks: HashMap<String, (bool, bool)> = queries::marks(&s.pool, household, week)
        .await?
        .into_iter()
        .map(|m| (m.item_key, (m.checked, m.hidden)))
        .collect();
    let usual = Weights::stored(&queries::weight_unit(&s.pool, household).await?);
    let per_food: HashMap<Uuid, Weights> = queries::food_weight_units(&s.pool, household)
        .await?
        .into_iter()
        .map(|f| (f.food_id, Weights::stored(&f.weight_unit)))
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

    let extras = queries::extras(&s.pool, household, week).await?;
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
#[utoipa::path(get, path = "/api/v1/shopping", operation_id = "get_shopping", tag = "shopping", params(WeekParams),
    responses(
        (status = 200, description = "The week's shopping list", body = ShoppingList),
        (status = 400, description = "`week` isn't a date", body = crate::error::ErrorBody),
        crate::error::SignedIn,
        crate::error::KitchenDevice
    )
)]
pub async fn get(
    State(s): State<AppState>,
    Extension(me): Extension<CurrentUser>,
    Query(p): Query<WeekParams>,
) -> ApiResult<Json<ShoppingList>> {
    Ok(Json(build(&s, me.household_id, monday(p.week)).await?))
}

/// A tick, or taking an item off this week's list, or putting it back.
#[derive(Deserialize, ToSchema)]
pub struct Mark {
    /// Any day in the week (default: today).
    pub week: Option<Date>,
    /// The item's `key`, as the list gave it.
    pub key: String,
    /// Ticked off.
    pub checked: Option<bool>,
    /// Taken off this week's list.
    pub hidden: Option<bool>,
}

/// Tick an item off, or take it off or put it back on this week's list. Answers with the list.
#[utoipa::path(put, path = "/api/v1/shopping/marks", operation_id = "mark_shopping_item", tag = "shopping", request_body = Mark,
    responses(
        (status = 200, description = "The updated list", body = ShoppingList),
        (status = 400, description = "The key is empty or too long, or the body isn't JSON of the right shape", body = crate::error::ErrorBody),
        crate::error::SignedIn,
        crate::error::ReadOnlyToken,
        crate::error::TooLarge
    )
)]
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
    queries::mark(&s.pool, me.household_id, week, &foods, b.checked, b.hidden).await?;
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

/// Something to add to a week's list by hand.
#[derive(Deserialize, ToSchema)]
pub struct NewExtra {
    /// Any day in the week (default: today).
    pub week: Option<Date>,
    /// What to add ("foil"), at most 200 characters.
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
        (status = 400, description = "The name or the key is empty or too long, or the body isn't JSON of the right shape", body = crate::error::ErrorBody),
        crate::error::SignedIn,
        crate::error::ReadOnlyToken,
        crate::error::TooLarge
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
    queries::add_extra(&s.pool, me.household_id, week, clean(&b.name)?, key).await?;
    Ok(Json(build(&s, me.household_id, week).await?))
}

/// A change to an extra.
#[derive(Deserialize, ToSchema)]
pub struct ExtraChange {
    /// Ticked off.
    pub checked: bool,
}

/// Tick a hand-added item off. Answers with the list.
#[utoipa::path(
    patch, path = "/api/v1/shopping/extras/{id}", operation_id = "update_shopping_extra", tag = "shopping", params(("id" = Uuid, Path)), request_body = ExtraChange,
    responses(
        (status = 200, description = "The updated list", body = ShoppingList),
        (status = 400, description = "The id isn't a UUID, or the body isn't JSON of the right shape", body = crate::error::ErrorBody),
        (status = 404, description = "No such item on this household's list", body = crate::error::ErrorBody),
        crate::error::SignedIn,
        crate::error::ReadOnlyToken,
        crate::error::TooLarge
    )
)]
pub async fn update_extra(
    State(s): State<AppState>,
    Extension(me): Extension<CurrentUser>,
    Path(id): Path<Uuid>,
    JsonBody(b): JsonBody<ExtraChange>,
) -> ApiResult<Json<ShoppingList>> {
    let week = queries::check_extra(&s.pool, id, me.household_id, b.checked)
        .await?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(build(&s, me.household_id, week).await?))
}

/// Delete a hand-added item. Answers with the list.
#[utoipa::path(
    delete, path = "/api/v1/shopping/extras/{id}", operation_id = "delete_shopping_extra", tag = "shopping", params(("id" = Uuid, Path)),
    responses(
        (status = 200, description = "The list without it", body = ShoppingList),
        (status = 400, description = "The id isn't a UUID", body = crate::error::ErrorBody),
        (status = 404, description = "No such item on this household's list", body = crate::error::ErrorBody),
        crate::error::SignedIn,
        crate::error::ReadOnlyToken
    )
)]
pub async fn remove_extra(
    State(s): State<AppState>,
    Extension(me): Extension<CurrentUser>,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<ShoppingList>> {
    let week = queries::remove_extra(&s.pool, id, me.household_id)
        .await?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(build(&s, me.household_id, week).await?))
}

/// A food the household always has, to leave off its lists.
#[derive(Deserialize, ToSchema)]
pub struct NewStaple {
    /// Any day in the week to answer with (default: today).
    pub week: Option<Date>,
    /// The food, by id.
    pub food_id: Uuid,
}

/// Mark a food as always on hand, leaving it off every list. Answers with the list.
#[utoipa::path(
    post, path = "/api/v1/pantry-staples", operation_id = "add_pantry_staple", tag = "shopping", request_body = NewStaple,
    responses(
        (status = 200, description = "The list without that food", body = ShoppingList),
        (status = 400, description = "The body isn't JSON of the right shape", body = crate::error::ErrorBody),
        (status = 404, description = "No such food", body = crate::error::ErrorBody),
        crate::error::SignedIn,
        crate::error::ReadOnlyToken,
        crate::error::TooLarge
    )
)]
pub async fn add_staple(
    State(s): State<AppState>,
    Extension(me): Extension<CurrentUser>,
    JsonBody(b): JsonBody<NewStaple>,
) -> ApiResult<Json<ShoppingList>> {
    // Nothing added is fine when it is already a staple; not when there is no such food.
    if !queries::add_staple(&s.pool, me.household_id, b.food_id).await?
        && !queries::food_exists(&s.pool, b.food_id).await?
    {
        return Err(ApiError::NotFound);
    }
    Ok(Json(build(&s, me.household_id, monday(b.week)).await?))
}

/// Put a food back on the list. Answers with the list.
#[utoipa::path(
    delete, path = "/api/v1/pantry-staples/{food_id}", operation_id = "delete_pantry_staple", tag = "shopping",
    params(("food_id" = Uuid, Path), WeekParams),
    responses(
        (status = 200, description = "The list with that food back on it", body = ShoppingList),
        (status = 400, description = "The food id isn't a UUID, or `week` isn't a date", body = crate::error::ErrorBody),
        crate::error::SignedIn,
        crate::error::ReadOnlyToken
    )
)]
pub async fn remove_staple(
    State(s): State<AppState>,
    Extension(me): Extension<CurrentUser>,
    Path(food_id): Path<Uuid>,
    Query(p): Query<WeekParams>,
) -> ApiResult<Json<ShoppingList>> {
    queries::remove_staple(&s.pool, me.household_id, food_id).await?;
    Ok(Json(build(&s, me.household_id, monday(p.week)).await?))
}

/// How the household's lists read.
#[derive(Serialize, Deserialize, ToSchema)]
pub struct ShoppingSettings {
    /// How the household reads weights on its list.
    pub weight_unit: Weights,
}

/// The household's shopping list settings.
#[utoipa::path(get, path = "/api/v1/shopping/settings", operation_id = "get_shopping_settings", tag = "shopping",
    responses(
        (status = 200, description = "The household's shopping settings", body = ShoppingSettings),
        crate::error::SignedIn,
        crate::error::KitchenDevice
    )
)]
pub async fn settings(
    State(s): State<AppState>,
    Extension(me): Extension<CurrentUser>,
) -> ApiResult<Json<ShoppingSettings>> {
    let w = queries::weight_unit(&s.pool, me.household_id).await?;
    Ok(Json(ShoppingSettings {
        weight_unit: Weights::stored(&w),
    }))
}

/// Change the household's shopping list settings, for everyone in it.
#[utoipa::path(put, path = "/api/v1/shopping/settings", operation_id = "update_shopping_settings", tag = "shopping", request_body = ShoppingSettings,
    responses(
        (status = 200, description = "The saved settings", body = ShoppingSettings),
        (status = 400, description = "The weight unit isn't one mise knows, or the body isn't JSON of the right shape", body = crate::error::ErrorBody),
        crate::error::SignedIn,
        crate::error::ReadOnlyToken,
        crate::error::TooLarge
    )
)]
pub async fn update_settings(
    State(s): State<AppState>,
    Extension(me): Extension<CurrentUser>,
    JsonBody(b): JsonBody<ShoppingSettings>,
) -> ApiResult<Json<ShoppingSettings>> {
    queries::set_weight_unit(&s.pool, me.household_id, b.weight_unit.core().as_str()).await?;
    Ok(Json(b))
}

/// How one food's weight reads on the household's lists.
#[derive(Deserialize, ToSchema)]
pub struct FoodWeights {
    /// Any day in the week to answer with (default: today).
    pub week: Option<Date>,
    /// The food, by id.
    pub food_id: Uuid,
    /// How this food's weight reads; leave out to go back to the household's usual way.
    pub weight_unit: Option<Weights>,
}

/// Choose how one food's weight reads on the household's lists. Answers with the list.
#[utoipa::path(
    put, path = "/api/v1/shopping/food-weights", operation_id = "set_food_weights", tag = "shopping", request_body = FoodWeights,
    responses(
        (status = 200, description = "The updated list", body = ShoppingList),
        (status = 400, description = "The weight unit isn't one mise knows, or the body isn't JSON of the right shape", body = crate::error::ErrorBody),
        (status = 404, description = "No such food", body = crate::error::ErrorBody),
        crate::error::SignedIn,
        crate::error::ReadOnlyToken,
        crate::error::TooLarge
    )
)]
pub async fn set_food_weights(
    State(s): State<AppState>,
    Extension(me): Extension<CurrentUser>,
    JsonBody(b): JsonBody<FoodWeights>,
) -> ApiResult<Json<ShoppingList>> {
    if !queries::food_exists(&s.pool, b.food_id).await? {
        return Err(ApiError::NotFound);
    }
    let unit = b.weight_unit.map(|w| w.core().as_str());
    queries::set_food_weight_unit(&s.pool, me.household_id, b.food_id, unit).await?;
    Ok(Json(build(&s, me.household_id, monday(b.week)).await?))
}
