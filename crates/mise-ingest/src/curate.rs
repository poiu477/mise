//! Hand edits to the food model: add foods and brands, set parents, add aliases, merge duplicates, set
//! properties, and record what a food's amounts convert by (its density, the size of one item). Every food an edit
//! touches becomes `origin = 'user'`, so re-running matching keeps it.
//!
//! The same operations run one at a time from the CLI or as a list from a JSON file, so curation can be
//! replayed after rebuilding a database. The web app's review page answers with them too (one decision, one
//! operation), and [`export`] writes the edits a database holds back out as such a file.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;

use anyhow::{Context, Result};
use mise_core::food_names::singular_key;
use mise_core::quantity::{Family, Quantity, Unit};
use mise_core::shopping;
use mise_core::text::slugify;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

/// An edit refused for a reason the person who asked can put right: a name that matches no food, or two; a parent
/// that would make a loop. Its text is a sentence to show them. Every other error from this module is a failure
/// (the database, a file). Tell them apart with `error.downcast_ref::<Refused>()`.
#[derive(Debug)]
pub struct Refused(pub String);

impl fmt::Display for Refused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Refused {}

/// Refuses the edit with this sentence.
fn refuse<T>(why: String) -> Result<T> {
    Err(Refused(why).into())
}

/// One hand edit. Foods are named as people write them ("green onions") and found by alias or slug; a name that
/// matches no food fails with the closest names, and one that matches several asks for a merge first. In a JSON
/// file each is an object tagged by `op`: `{"op": "alias", "food": "scallion", "alias": "green onion"}`.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Op {
    /// Create a food (or brand, variety, cut...). Does nothing if a food with this name already exists.
    Add {
        /// The new food's name: "smoked paprika".
        name: String,
        /// What it's a kind of, if anything: "paprika".
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parent: Option<String>,
        /// A `food_kind`: "food" (the default), "brand", "variety", "cut", "form" or "proprietary".
        #[serde(default = "default_kind", skip_serializing_if = "is_default_kind")]
        kind: String,
        /// Other names it goes by, besides its own.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        aliases: Vec<String>,
    },
    /// `food` is right as it stands: it becomes yours, and matching stops asking about it.
    Keep {
        /// The food to keep: "pepper".
        food: String,
    },
    /// Make `food` a kind of `parent` (replacing any related link between the two).
    SetParent {
        /// The food to move: "baby spinach".
        food: String,
        /// What it becomes a kind of: "spinach". Refused if it's `food` or one of its descendants.
        parent: String,
    },
    /// Make `food` top-level.
    Unparent {
        /// The food to move to the top.
        food: String,
    },
    /// Another name for the same food.
    Alias {
        /// The food that gains the name: "scallion".
        food: String,
        /// The other name: "green onion".
        alias: String,
    },
    /// Remove a `related` link between two foods.
    Unrelate {
        /// One food of the pair.
        food: String,
        /// The other; the order doesn't matter.
        other: String,
    },
    /// `from` and `into` are the same food: move everything to `into` and delete `from`.
    Merge {
        /// The duplicate: "cilantro leaves". Its lines, children, facts, aliases and links move to `into`, and
        /// then it is deleted.
        from: String,
        /// The food that stays: "cilantro".
        into: String,
    },
    /// Set a property, e.g. `default_flexibility` = "strict" or `store_section` = "Dairy".
    Set {
        /// The food to change.
        food: String,
        /// The property's name: "store_section".
        key: String,
        /// Its new value, any JSON: "Dairy".
        value: Value,
    },
    /// What a volume of the food weighs, so the shopping list can add cups and ounces of it up as one weight:
    /// `{"op": "density", "food": "Parmesan", "volume": "1 cup", "weight": "3 oz"}`. Replaces any set before.
    Density {
        /// The food: "Parmesan".
        food: String,
        /// A volume of it: "1 cup".
        volume: String,
        /// What that volume weighs: "3 oz".
        weight: String,
    },
    /// How much one item of the food is, so the shopping list can add counts and measures of it up:
    /// `{"op": "item_size", "food": "Garlic", "item": "clove", "size": "1 tsp"}`. A volume and a weight can both be
    /// given, in two ops; each replaces only its own kind.
    ItemSize {
        /// The food: "Garlic".
        food: String,
        /// The word recipes count it in: "clove" (plurals are folded).
        item: String,
        /// One item's volume or weight: "1 tsp", "8 oz".
        size: String,
    },
}

fn default_kind() -> String {
    "food".into()
}

fn is_default_kind(kind: &str) -> bool {
    kind == "food"
}

/// Applies one operation in its own transaction and says what it did: "added smoked paprika",
/// "scallion: already exists".
pub async fn apply(pool: &PgPool, op: &Op) -> Result<String> {
    let mut tx = pool.begin().await?;
    let done = apply_in(&mut tx, op)
        .await
        .with_context(|| format!("{op:?}"))?;
    tx.commit().await?;
    Ok(done)
}

/// Apply a list of operations from a JSON file (an array of ops), all or nothing.
pub async fn apply_file(pool: &PgPool, path: &std::path::Path) -> Result<Vec<String>> {
    let ops: Vec<Op> = serde_json::from_reader(
        std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?,
    )
    .with_context(|| format!("reading {}", path.display()))?;
    let mut tx = pool.begin().await?;
    let mut log = Vec::new();
    for (i, op) in ops.iter().enumerate() {
        log.push(
            apply_in(&mut tx, op)
                .await
                .with_context(|| format!("op {} {op:?}", i + 1))?,
        );
    }
    tx.commit().await?;
    Ok(log)
}

/// Applies one operation inside the caller's transaction, so it commits or rolls back with whatever else the
/// caller does there (the review page marks its item decided in the same one). Says what it did, as [`apply`]. An
/// edit the person can put right fails with [`Refused`].
pub async fn apply_in(tx: &mut Transaction<'_, Postgres>, op: &Op) -> Result<String> {
    match op {
        Op::Add {
            name,
            parent,
            kind,
            aliases,
        } => {
            if let Some(existing) = try_find(tx, name).await? {
                adopt(tx, existing).await?;
                return Ok(format!("{name}: already exists"));
            }
            let parent_id = match parent {
                Some(p) => Some(find(tx, p).await?),
                None => None,
            };
            let id = Uuid::now_v7();
            sqlx::query(
                "INSERT INTO foods (id, name, slug, parent_id, kind, origin) VALUES ($1, $2, $3, $4, $5::food_kind, 'user')",
            )
            .bind(id)
            .bind(name)
            .bind(unique_slug(tx, name).await?)
            .bind(parent_id)
            .bind(kind)
            .execute(&mut **tx)
            .await?;
            add_alias(tx, id, name).await?;
            for a in aliases {
                add_alias(tx, id, a).await?;
            }
            if let Some(p) = parent_id {
                adopt(tx, p).await?;
            }
            Ok(format!("added {name}"))
        }
        Op::Keep { food } => {
            let id = find(tx, food).await?;
            close_reviews(tx, id, "keep", About::Food).await?;
            adopt(tx, id).await?;
            Ok(format!("{food} stays as it is"))
        }
        Op::SetParent { food, parent } => {
            let (child, parent_id) = (find(tx, food).await?, find(tx, parent).await?);
            if child == parent_id || is_ancestor(tx, child, parent_id).await? {
                return refuse(format!(
                    "{parent} is {food} or one of its descendants; that would make a loop"
                ));
            }
            sqlx::query("UPDATE foods SET parent_id = $2 WHERE id = $1")
                .bind(child)
                .bind(parent_id)
                .execute(&mut **tx)
                .await?;
            sqlx::query(
                "DELETE FROM food_relations WHERE kind = 'related'
                   AND ((from_food = $1 AND to_food = $2) OR (from_food = $2 AND to_food = $1))",
            )
            .bind(child)
            .bind(parent_id)
            .execute(&mut **tx)
            .await?;
            close_reviews(tx, child, "set_parent", About::Food).await?;
            adopt(tx, child).await?;
            adopt(tx, parent_id).await?;
            Ok(format!("{food} is now a kind of {parent}"))
        }
        Op::Unparent { food } => {
            let id = find(tx, food).await?;
            sqlx::query("UPDATE foods SET parent_id = NULL WHERE id = $1")
                .bind(id)
                .execute(&mut **tx)
                .await?;
            close_reviews(tx, id, "unparent", About::Food).await?;
            adopt(tx, id).await?;
            Ok(format!("{food} is now top-level"))
        }
        Op::Alias { food, alias } => {
            let id = find(tx, food).await?;
            if let Some(other) = try_find(tx, alias).await?.filter(|o| *o != id) {
                return refuse(format!(
                    "'{alias}' already names another food ({other}); use merge if they are the same food"
                ));
            }
            add_alias(tx, id, alias).await?;
            adopt(tx, id).await?;
            Ok(format!("'{alias}' now also means {food}"))
        }
        Op::Merge { from, into } => {
            let (from_id, into_id) = (find(tx, from).await?, find(tx, into).await?);
            if from_id == into_id {
                return Ok(format!("{from} is already merged into {into}"));
            }
            for sql in [
                "UPDATE ingredient_lines SET food_id = $2 WHERE food_id = $1",
                // `into` takes over `from`'s place in the tree if it sat under it; nothing becomes its own parent.
                "UPDATE foods SET parent_id = (SELECT parent_id FROM foods WHERE id = $1) WHERE id = $2 AND parent_id = $1",
                "UPDATE foods SET parent_id = $2 WHERE parent_id = $1 AND id <> $2",
                "UPDATE food_facts SET food_id = $2 WHERE food_id = $1",
                // `into`'s own measures win; `from`'s fill in what it lacks and the rest go with it.
                "UPDATE food_measures m SET food_id = $2 WHERE m.food_id = $1
                   AND NOT EXISTS (SELECT 1 FROM food_measures o WHERE o.food_id = $2 AND o.kind = m.kind AND o.item = m.item)",
                "INSERT INTO food_aliases (food_id, alias, origin) SELECT $2, alias, 'user' FROM food_aliases WHERE food_id = $1 ON CONFLICT DO NOTHING",
                "UPDATE food_relations SET from_food = $2 WHERE from_food = $1 AND to_food <> $2",
                "UPDATE food_relations SET to_food = $2 WHERE to_food = $1 AND from_food <> $2",
                "DELETE FROM food_relations WHERE from_food = $1 OR to_food = $1",
            ] {
                sqlx::query(sql)
                    .bind(from_id)
                    .bind(into_id)
                    .execute(&mut **tx)
                    .await?;
            }
            close_reviews(tx, from_id, "merged", About::FoodOrParent).await?;
            sqlx::query("DELETE FROM foods WHERE id = $1")
                .bind(from_id)
                .execute(&mut **tx)
                .await?;
            adopt(tx, into_id).await?;
            Ok(format!("merged {from} into {into}"))
        }
        Op::Unrelate { food, other } => {
            let (a, b) = (find(tx, food).await?, find(tx, other).await?);
            let removed = sqlx::query(
                "DELETE FROM food_relations WHERE kind = 'related' AND ((from_food = $1 AND to_food = $2) OR (from_food = $2 AND to_food = $1))",
            )
            .bind(a)
            .bind(b)
            .execute(&mut **tx)
            .await?
            .rows_affected();
            adopt(tx, a).await?;
            Ok(format!(
                "{food} and {other}: {removed} related link(s) removed"
            ))
        }
        Op::Set { food, key, value } => {
            let id = find(tx, food).await?;
            sqlx::query("UPDATE foods SET props = props || jsonb_build_object($2::text, $3::jsonb) WHERE id = $1")
                .bind(id)
                .bind(key)
                .bind(value)
                .execute(&mut **tx)
                .await?;
            adopt(tx, id).await?;
            Ok(format!("{food}: {key} = {value}"))
        }
        Op::Density {
            food,
            volume,
            weight,
        } => {
            let id = find(tx, food).await?;
            let ml = parse_size(volume, Family::Volume)?
                .in_unit(&Unit::Ml)
                .unwrap_or_default();
            let grams = parse_size(weight, Family::Weight)?
                .in_unit(&Unit::G)
                .unwrap_or_default();
            sqlx::query(
                "INSERT INTO food_measures (id, food_id, kind, item, ml, grams) VALUES ($1, $2, 'density', '', $3::float8::numeric, $4::float8::numeric)
                 ON CONFLICT (food_id, kind, item) DO UPDATE SET ml = excluded.ml, grams = excluded.grams",
            )
            .bind(Uuid::now_v7())
            .bind(id)
            .bind(ml)
            .bind(grams)
            .execute(&mut **tx)
            .await?;
            adopt(tx, id).await?;
            Ok(format!("{food}: {volume} weighs {weight}"))
        }
        Op::ItemSize { food, item, size } => {
            let id = find(tx, food).await?;
            let word = item_word(item)?;
            let Some(q) = shopping::parse_size(size) else {
                return refuse(format!(
                    "'{size}' isn't a volume or a weight, such as \"1 tsp\" or \"8 oz\""
                ));
            };
            let (ml, grams) = match q.unit.family() {
                Family::Volume => (q.in_unit(&Unit::Ml), None),
                _ => (None, q.in_unit(&Unit::G)),
            };
            sqlx::query(
                "INSERT INTO food_measures (id, food_id, kind, item, ml, grams) VALUES ($1, $2, 'item', $3, $4::float8::numeric, $5::float8::numeric)
                 ON CONFLICT (food_id, kind, item)
                 DO UPDATE SET ml = coalesce(excluded.ml, food_measures.ml), grams = coalesce(excluded.grams, food_measures.grams)",
            )
            .bind(Uuid::now_v7())
            .bind(id)
            .bind(&word)
            .bind(ml)
            .bind(grams)
            .execute(&mut **tx)
            .await?;
            adopt(tx, id).await?;
            Ok(format!("{food}: one {word} is {size}"))
        }
    }
}

/// A size of the kind asked for, or an error that says what was expected.
fn parse_size(text: &str, family: Family) -> Result<Quantity> {
    let word = if family == Family::Volume {
        "volume, such as \"1 cup\""
    } else {
        "weight, such as \"3 oz\""
    };
    match shopping::parse_size(text).filter(|q| q.unit.family() == family) {
        Some(q) => Ok(q),
        None => refuse(format!("'{text}' isn't a {word}")),
    }
}

/// The singular word an item is counted in: "Cloves" is "clove".
fn item_word(item: &str) -> Result<String> {
    let w = item.trim().to_lowercase();
    if w.is_empty() || w.chars().count() > 50 {
        return refuse("an item is a word of 1 to 50 characters, such as \"clove\"".into());
    }
    Ok(match Unit::parse(&w) {
        Some(Unit::Count(noun)) if !noun.is_empty() => noun,
        _ => singular_key(&w),
    })
}

/// One food as [`export`] reads it.
#[derive(sqlx::FromRow)]
struct Exported {
    id: Uuid,
    name: String,
    kind: String,
    origin: String,
    parent_id: Option<Uuid>,
    /// Whether the parent is a category, which no operation can name.
    parent_is_category: Option<bool>,
    props: Value,
}

/// The hand edits this database holds, as operations: applied (`mise food apply`) to a database rebuilt from the
/// same sources and matched, they bring the food model back to where it is now. Applying them here changes nothing.
///
/// The database keeps what an edit left, not the edit, so the list is worked out from it, in an order that
/// applies cleanly: every food that is someone's own (`add`, which only adopts a food matching makes anyway), the
/// names given by hand (`merge` where recipes use the name, since a rebuilt database has a food for it again;
/// `alias` otherwise), where each of those foods sits (`set_parent` or `unparent`, parents before their kinds),
/// properties (`set`) and measures (`density`, `item_size`). It reads the same every time for the same edits.
///
/// Two edits leave nothing to read back, so they stay in a hand-written file: `unrelate` (a link that is gone),
/// and `unparent` on a food from a bundle's food knowledge (it reads like one that never had a parent).
pub async fn export(pool: &PgPool) -> Result<Vec<Op>> {
    let foods: Vec<Exported> = sqlx::query_as(
        "SELECT f.id, f.name, f.kind::text AS kind, f.origin, f.parent_id, p.kind = 'category' AS parent_is_category, f.props
         FROM foods f LEFT JOIN foods p ON p.id = f.parent_id
         WHERE f.kind <> 'category' ORDER BY lower(f.name), f.name, f.id",
    )
    .fetch_all(pool)
    .await
    .context("reading foods")?;
    let by_id: HashMap<Uuid, &Exported> = foods.iter().map(|f| (f.id, f)).collect();
    let mut ops = Vec::new();

    for f in foods.iter().filter(|f| f.origin == "user") {
        ops.push(Op::Add {
            name: f.name.clone(),
            parent: None,
            kind: f.kind.clone(),
            aliases: Vec::new(),
        });
    }

    // Names given by hand. One that recipes use came from a merge, or has drawn lines since: a rebuilt database
    // makes a food for it again, which only `merge` folds back in. `alias` would refuse it as another food's name.
    let aliases: Vec<(Uuid, String)> = sqlx::query_as(
        "SELECT food_id, alias FROM food_aliases WHERE origin = 'user' ORDER BY alias, food_id",
    )
    .fetch_all(pool)
    .await
    .context("reading aliases")?;
    let owners: Vec<Uuid> = aliases.iter().map(|a| a.0).collect();
    // Each food's line names, the most used spelling of a name first, so the merge reads as recipes write it.
    let written: Vec<(Uuid, String)> = sqlx::query_as(
        "SELECT food_id, raw_name FROM ingredient_lines WHERE food_id = ANY($1)
         GROUP BY food_id, raw_name ORDER BY count(*) DESC, raw_name",
    )
    .bind(&owners)
    .fetch_all(pool)
    .await
    .context("reading ingredient names")?;
    let mut spelled: HashMap<(Uuid, String), String> = HashMap::new();
    for (food, raw) in written {
        spelled
            .entry((food, singular_key(&raw)))
            .or_insert_with(|| raw.trim().to_string());
    }
    for (food, alias) in aliases {
        let Some(f) = by_id.get(&food) else { continue };
        if alias == singular_key(&f.name) {
            continue;
        }
        ops.push(match spelled.get(&(food, alias.clone())) {
            Some(raw) => Op::Merge {
                from: raw.clone(),
                into: f.name.clone(),
            },
            None => Op::Alias {
                food: f.name.clone(),
                alias,
            },
        });
    }

    // Where each food sits. A person's own food is pinned either way, so a parent matching proposes for it on a
    // rebuilt database doesn't stand; a knowledge food only under a parent someone gave it (its bundle gives it
    // categories). Parents go before their kinds, so no step passes through a loop.
    let depth = |f: &Exported| {
        let (mut at, mut seen) = (f.parent_id, HashSet::from([f.id]));
        let mut n = 0;
        while let Some(p) = at.filter(|p| seen.insert(*p)) {
            n += 1;
            at = by_id.get(&p).and_then(|p| p.parent_id);
        }
        n
    };
    let mut placed: Vec<(usize, Op)> = Vec::new();
    for f in &foods {
        let parent = f
            .parent_id
            .filter(|_| f.parent_is_category != Some(true))
            .and_then(|p| by_id.get(&p));
        match (parent, f.origin.as_str()) {
            (Some(p), "user" | "knowledge") => placed.push((
                depth(f),
                Op::SetParent {
                    food: f.name.clone(),
                    parent: p.name.clone(),
                },
            )),
            (None, "user") => placed.push((
                0,
                Op::Unparent {
                    food: f.name.clone(),
                },
            )),
            _ => {}
        }
    }
    placed.sort_by_key(|(depth, _)| *depth);
    ops.extend(placed.into_iter().map(|(_, op)| op));

    for f in &foods {
        // Sorted by key: a JSON object's own order isn't promised.
        let props: BTreeMap<&String, &Value> = f.props.as_object().into_iter().flatten().collect();
        for (key, value) in props {
            ops.push(Op::Set {
                food: f.name.clone(),
                key: key.clone(),
                value: value.clone(),
            });
        }
    }

    // food, kind, item, milliliters, grams
    type MeasureRow = (Uuid, String, String, Option<f64>, Option<f64>);
    let measures: Vec<MeasureRow> = sqlx::query_as(
        "SELECT m.food_id, m.kind, m.item, m.ml::float8, m.grams::float8 FROM food_measures m
         JOIN foods f ON f.id = m.food_id ORDER BY lower(f.name), f.name, f.id, m.kind, m.item",
    )
    .fetch_all(pool)
    .await
    .context("reading measures")?;
    for (food, kind, item, ml, grams) in measures {
        let Some(f) = by_id.get(&food) else { continue };
        let food = f.name.clone();
        match (kind.as_str(), ml, grams) {
            ("density", Some(ml), Some(grams)) => ops.push(Op::Density {
                food,
                volume: size_text(ml, Family::Volume),
                weight: size_text(grams, Family::Weight),
            }),
            ("item", ml, grams) => {
                let sizes = [(ml, Family::Volume), (grams, Family::Weight)];
                for (amount, family) in sizes {
                    if let Some(amount) = amount {
                        ops.push(Op::ItemSize {
                            food: food.clone(),
                            item: item.clone(),
                            size: size_text(amount, family),
                        });
                    }
                }
            }
            _ => {}
        }
    }
    Ok(ops)
}

/// A stored size (milliliters or grams) as a curation file writes one, reading back to the same number: "1 cup",
/// "2 tbsp", "8 oz", "250 g". The first unit that gives a plain amount (eighths, at least a quarter) is used;
/// failing all of them, the stored number itself, "236.5 ml".
fn size_text(stored: f64, family: Family) -> String {
    let (base, units): (Unit, &[(Unit, &str)]) = if family == Family::Volume {
        (
            Unit::Ml,
            &[
                (Unit::Cup, "cup"),
                (Unit::Tbsp, "tbsp"),
                (Unit::Tsp, "tsp"),
                (Unit::FlOz, "fl oz"),
                (Unit::Ml, "ml"),
                (Unit::L, "l"),
            ],
        )
    } else {
        (
            Unit::G,
            &[
                (Unit::Oz, "oz"),
                (Unit::G, "g"),
                (Unit::Lb, "lb"),
                (Unit::Kg, "kg"),
            ],
        )
    };
    let whole = Quantity::new(stored, base.clone());
    // Postgres keeps 15 digits of what an edit stores, so a size reads back the same when it agrees that far.
    let same = |text: &str| {
        shopping::parse_size(text)
            .and_then(|q| q.in_unit(&base))
            .is_some_and(|back| (back - stored).abs() <= stored.abs() * 1e-13)
    };
    for (unit, word) in units {
        let Some(amount) = whole.in_unit(unit) else {
            continue;
        };
        let plain = (amount * 8.0).round() / 8.0;
        let text = format!("{plain} {word}");
        if plain >= 0.25 && same(&text) {
            return text;
        }
    }
    let word = if family == Family::Volume { "ml" } else { "g" };
    format!("{stored} {word}")
}

/// A food by name, alias or slug.
async fn try_find(tx: &mut Transaction<'_, Postgres>, name: &str) -> Result<Option<Uuid>> {
    let ids: Vec<Uuid> = sqlx::query_scalar(
        "SELECT DISTINCT f.id FROM foods f LEFT JOIN food_aliases a ON a.food_id = f.id
         WHERE (a.alias = $1 OR f.slug = $2) AND f.kind <> 'category'",
    )
    .bind(singular_key(name))
    .bind(slugify(name))
    .fetch_all(&mut **tx)
    .await?;
    match ids.as_slice() {
        [] => Ok(None),
        [id] => Ok(Some(*id)),
        _ => refuse(format!(
            "'{name}' matches {} foods; merge them first or use a more specific name",
            ids.len()
        )),
    }
}

async fn find(tx: &mut Transaction<'_, Postgres>, name: &str) -> Result<Uuid> {
    if let Some(id) = try_find(tx, name).await? {
        return Ok(id);
    }
    let close: Vec<String> = sqlx::query_scalar(
        "SELECT name FROM foods WHERE kind <> 'category' ORDER BY similarity(name, $1) DESC LIMIT 5",
    )
    .bind(name)
    .fetch_all(&mut **tx)
    .await?;
    refuse(format!(
        "no food named '{name}'. Closest: {}",
        close.join(", ")
    ))
}

async fn add_alias(tx: &mut Transaction<'_, Postgres>, food: Uuid, name: &str) -> Result<()> {
    sqlx::query("INSERT INTO food_aliases (food_id, alias, origin) VALUES ($1, $2, 'user') ON CONFLICT DO NOTHING")
        .bind(food)
        .bind(singular_key(name))
        .execute(&mut **tx)
        .await?;
    Ok(())
}

/// A food someone edited is theirs: matching resets leave it alone.
async fn adopt(tx: &mut Transaction<'_, Postgres>, food: Uuid) -> Result<()> {
    sqlx::query("UPDATE foods SET origin = 'user' WHERE id = $1 AND origin = 'matched'")
        .bind(food)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

async fn is_ancestor(tx: &mut Transaction<'_, Postgres>, ancestor: Uuid, of: Uuid) -> Result<bool> {
    Ok(sqlx::query_scalar(
        "WITH RECURSIVE up AS (SELECT parent_id FROM foods WHERE id = $2
                               UNION SELECT f.parent_id FROM foods f JOIN up ON f.id = up.parent_id)
         SELECT EXISTS (SELECT 1 FROM up WHERE parent_id = $1)",
    )
    .bind(ancestor)
    .bind(of)
    .fetch_one(&mut **tx)
    .await?)
}

async fn unique_slug(tx: &mut Transaction<'_, Postgres>, name: &str) -> Result<String> {
    let base = slugify(name);
    let mut slug = base.clone();
    let mut n = 2;
    while sqlx::query_scalar::<_, bool>("SELECT EXISTS (SELECT 1 FROM foods WHERE slug = $1)")
        .bind(&slug)
        .fetch_one(&mut **tx)
        .await?
    {
        slug = format!("{base}-{n}");
        n += 1;
    }
    Ok(slug)
}

/// Which open review items an edit to a food answers.
#[derive(Clone, Copy, PartialEq, Eq)]
enum About {
    /// Those asking about the food itself: what it is, what it's a kind of.
    Food,
    /// Those too that propose it as another food's parent: for a food that is going away.
    FoodOrParent,
}

/// Resolve the open review items this edit to `food` answers; the edit is the decision. Moving or keeping a food
/// answers the questions about it, and leaves "is X a kind of it?" for whoever decides about X.
async fn close_reviews(
    tx: &mut Transaction<'_, Postgres>,
    food: Uuid,
    how: &str,
    about: About,
) -> Result<()> {
    sqlx::query(
        "UPDATE review_items SET status = 'edited', decided_at = now(), decision = jsonb_build_object('by', 'curation', 'how', $2::text)
         WHERE status = 'open' AND kind IN ('food_parent', 'food_match')
           AND ((subject->>'food_id')::uuid = $1 OR ($3 AND (proposal->>'parent_id')::uuid = $1))",
    )
    .bind(food)
    .bind(how)
    .bind(about == About::FoodOrParent)
    .execute(&mut **tx)
    .await?;
    Ok(())
}
