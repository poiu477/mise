//! Hand edits to the food model: add foods and brands, set parents, add aliases, merge duplicates, set
//! properties, and record what a food's amounts convert by (its density, the size of one item). Every food an edit
//! touches becomes `origin = 'user'`, so re-running matching keeps it.
//!
//! The same operations run one at a time from the CLI or as a list from a JSON file, so curation can be
//! replayed after rebuilding a database.

use anyhow::{Context, Result, bail};
use mise_core::food_names::singular_key;
use mise_core::quantity::{Family, Quantity, Unit};
use mise_core::shopping;
use mise_core::text::slugify;
use serde::Deserialize;
use serde_json::Value;
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

/// One hand edit. Foods are named as people write them ("green onions") and found by alias or slug; a name that
/// matches no food fails with the closest names, and one that matches several asks for a merge first. In a JSON
/// file each is an object tagged by `op`: `{"op": "alias", "food": "scallion", "alias": "green onion"}`.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Op {
    /// Create a food (or brand, variety, cut...). Does nothing if a food with this name already exists.
    Add {
        /// The new food's name: "smoked paprika".
        name: String,
        /// What it's a kind of, if anything: "paprika".
        parent: Option<String>,
        /// A `food_kind`: "food" (the default), "brand", "variety", "cut", "form" or "proprietary".
        #[serde(default = "default_kind")]
        kind: String,
        /// Other names it goes by, besides its own.
        #[serde(default)]
        aliases: Vec<String>,
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

async fn apply_in(tx: &mut Transaction<'_, Postgres>, op: &Op) -> Result<String> {
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
        Op::SetParent { food, parent } => {
            let (child, parent_id) = (find(tx, food).await?, find(tx, parent).await?);
            if child == parent_id || is_ancestor(tx, child, parent_id).await? {
                bail!("{parent} is {food} or one of its descendants; that would make a loop");
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
            close_reviews(tx, child, "set_parent").await?;
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
            close_reviews(tx, id, "unparent").await?;
            adopt(tx, id).await?;
            Ok(format!("{food} is now top-level"))
        }
        Op::Alias { food, alias } => {
            let id = find(tx, food).await?;
            if let Some(other) = try_find(tx, alias).await?.filter(|o| *o != id) {
                bail!(
                    "'{alias}' already names another food ({other}); use merge if they are the same food"
                );
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
            close_reviews(tx, from_id, "merged").await?;
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
            let q = shopping::parse_size(size).with_context(|| {
                format!("'{size}' isn't a volume or a weight, such as \"1 tsp\" or \"8 oz\"")
            })?;
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
    shopping::parse_size(text)
        .filter(|q| q.unit.family() == family)
        .with_context(|| format!("'{text}' isn't a {word}"))
}

/// The singular word an item is counted in: "Cloves" is "clove".
fn item_word(item: &str) -> Result<String> {
    let w = item.trim().to_lowercase();
    if w.is_empty() || w.chars().count() > 50 {
        bail!("an item is a word of 1 to 50 characters, such as \"clove\"");
    }
    Ok(match Unit::parse(&w) {
        Some(Unit::Count(noun)) if !noun.is_empty() => noun,
        _ => singular_key(&w),
    })
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
        _ => bail!(
            "'{name}' matches {} foods; merge them first or use a more specific name",
            ids.len()
        ),
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
    bail!("no food named '{name}'. Closest: {}", close.join(", "))
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

/// Resolve open review items about this food; the edit is the decision.
async fn close_reviews(tx: &mut Transaction<'_, Postgres>, food: Uuid, how: &str) -> Result<()> {
    sqlx::query(
        "UPDATE review_items SET status = 'edited', decided_at = now(), decision = jsonb_build_object('by', 'curation', 'how', $2::text)
         WHERE status = 'open' AND kind IN ('food_parent', 'food_match')
           AND ((subject->>'food_id')::uuid = $1 OR (proposal->>'parent_id')::uuid = $1)",
    )
    .bind(food)
    .bind(how)
    .execute(&mut **tx)
    .await?;
    Ok(())
}
