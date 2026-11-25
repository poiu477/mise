//! Foods: load researched food knowledge, match ingredient lines to foods, and propose the food tree.
//!
//! Matching keys every name by [`singular_key`], so "Onion" and "Onions" share one food. Tree links follow
//! [`relate`]: narrowing words make a kind-of link (applied, still `proposed` until reviewed), transforming
//! words make a `related` link, and anything else becomes a review item. Brands come from the `brands` table, which
//! curation edits ([`crate::curate`]), read once a run.
//!
//! Matching can't tell that two names it doesn't know are one food ("Kale" and "Chopped Kale" both arrive as
//! names of their own), so afterwards [`duplicate_key`] pairs the foods it made whose names look like one food
//! written differently, and each pair becomes a `food_duplicate` review item for a person.

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

use anyhow::{Context, Result};
use mise_core::food_names::{Brands, Relation, duplicate_key, relate, singular_key, strip_brand};
use mise_core::text::slugify;
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::{PgExecutor, PgPool, Postgres, Transaction};
use uuid::Uuid;

#[derive(Deserialize)]
struct KnowledgeFood {
    name: String,
    #[serde(default)]
    parent_path: Vec<String>,
    kind: Option<String>,
    #[serde(default)]
    aliases: Vec<String>,
    #[serde(default)]
    facts: Vec<KnowledgeFact>,
}

#[derive(Deserialize)]
struct KnowledgeFact {
    kind: String,
    value: Value,
    tier: String,
    source_ref: Option<String>,
    observed_at: Option<String>,
}

/// Load `foods.ndjson` from a bundle. Foods that already exist (by slug) are left alone, so re-running is safe.
pub async fn import_food_knowledge(pool: &PgPool, path: &Path) -> Result<u64> {
    let reader =
        BufReader::new(File::open(path).with_context(|| format!("opening {}", path.display()))?);
    let mut tx = pool.begin().await?;
    let mut added = 0;
    for (n, line) in reader.lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let food: KnowledgeFood =
            serde_json::from_str(&line).with_context(|| format!("foods.ndjson line {}", n + 1))?;

        // Category path: each level is a 'category' food under the previous one.
        let mut parent: Option<Uuid> = None;
        for depth in 0..food.parent_path.len() {
            let path = &food.parent_path[..=depth];
            let slug = format!("category-{}", slugify(&path.join(" ")));
            parent = Some(
                upsert_food(&mut tx, path.last().unwrap(), &slug, parent, "category")
                    .await?
                    .0,
            );
        }

        let kind = food.kind.as_deref().unwrap_or("food");
        let (food_id, created) =
            upsert_food(&mut tx, &food.name, &slugify(&food.name), parent, kind).await?;
        add_alias(&mut tx, food_id, &food.name).await?;
        for alias in &food.aliases {
            add_alias(&mut tx, food_id, alias).await?;
        }
        if created {
            added += 1;
            for fact in &food.facts {
                sqlx::query(
                    "INSERT INTO food_facts (id, food_id, kind, value, tier, source_ref, observed_at)
                     VALUES ($1, $2, $3, $4, $5::evidence_tier, $6, $7::date)",
                )
                .bind(Uuid::now_v7())
                .bind(food_id)
                .bind(&fact.kind)
                .bind(&fact.value)
                .bind(&fact.tier)
                .bind(&fact.source_ref)
                .bind(&fact.observed_at)
                .execute(&mut *tx)
                .await
                .with_context(|| format!("fact for {}", food.name))?;
            }
        }
    }
    tx.commit().await?;
    Ok(added)
}

async fn upsert_food(
    tx: &mut Transaction<'_, Postgres>,
    name: &str,
    slug: &str,
    parent: Option<Uuid>,
    kind: &str,
) -> Result<(Uuid, bool)> {
    let inserted: Option<Uuid> = sqlx::query_scalar(
        "INSERT INTO foods (id, name, slug, parent_id, kind, origin) VALUES ($1, $2, $3, $4, $5::food_kind, 'knowledge')
         ON CONFLICT (slug) DO NOTHING RETURNING id",
    )
    .bind(Uuid::now_v7())
    .bind(name)
    .bind(slug)
    .bind(parent)
    .bind(kind)
    .fetch_optional(&mut **tx)
    .await?;
    match inserted {
        Some(id) => Ok((id, true)),
        None => Ok((
            sqlx::query_scalar("SELECT id FROM foods WHERE slug = $1")
                .bind(slug)
                .fetch_one(&mut **tx)
                .await?,
            false,
        )),
    }
}

async fn add_alias(tx: &mut Transaction<'_, Postgres>, food_id: Uuid, name: &str) -> Result<()> {
    sqlx::query("INSERT INTO food_aliases (food_id, alias, origin) VALUES ($1, $2, 'knowledge') ON CONFLICT DO NOTHING")
        .bind(food_id)
        .bind(singular_key(name))
        .execute(&mut **tx)
        .await?;
    Ok(())
}

/// The brands matching strips from names: every one in the `brands` table that curation hasn't removed.
pub async fn brands(db: impl PgExecutor<'_>) -> Result<Brands> {
    let names: Vec<String> = sqlx::query_scalar("SELECT name FROM brands WHERE NOT removed")
        .fetch_all(db)
        .await
        .context("reading brands")?;
    Ok(Brands::new(names))
}

/// Names that mean different foods depending on context; matched normally but flagged for a person.
const AMBIGUOUS: &[(&str, &str)] = &[(
    "pepper",
    "black pepper or bell pepper (as a pantry item it is usually black pepper)",
)];

/// What one matching run did, for the command's summary.
#[derive(Debug, Default)]
pub struct MatchStats {
    /// Distinct spellings on unmatched lines at the start ("Onion" and "onions" count as two).
    pub names: usize,
    /// Foods made for names no existing food had.
    pub foods_created: usize,
    /// Ingredient lines now pointing at a food.
    pub lines_matched: u64,
    /// New foods placed under another as a kind of it: "red onion" under "onion".
    pub kind_of_links: usize,
    /// New foods linked to one they're made from: "onion powder" to "onion".
    pub related_links: usize,
    /// Links and names left for a person to decide, such as "pepper", which could be black or bell.
    pub review_items: usize,
    /// Pairs of foods that look like one food written two ways, raised for a person: "Kale", "Chopped Kale".
    pub duplicate_items: usize,
    /// Foods for every 100 ingredient lines, to one decimal. Matching makes a food for each name it doesn't know,
    /// so a jump means the sources are spelling things differently and the duplicates are worth a look.
    pub foods_per_100_lines: f64,
}

struct Food {
    id: Uuid,
    name: String,
    created: bool,
}

/// Undo a previous matching run: unlink ingredient lines and delete the foods, links and open review items it
/// created. Knowledge foods and anything a person created or edited (origin `user`) are kept.
pub async fn reset_matching(pool: &PgPool) -> Result<()> {
    let mut tx = pool.begin().await?;
    // Only lines pointing at matched foods are unlinked; lines a person pointed elsewhere stay.
    sqlx::query(
        "UPDATE ingredient_lines il SET food_id = NULL FROM foods f WHERE f.id = il.food_id AND f.origin = 'matched'",
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "DELETE FROM review_items WHERE kind IN ('food_parent', 'food_match', 'food_duplicate') AND status = 'open'",
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "DELETE FROM food_relations r USING foods a, foods b
         WHERE a.id = r.from_food AND b.id = r.to_food AND a.origin = 'matched' AND b.origin = 'matched'",
    )
    .execute(&mut *tx)
    .await?;
    // Knowledge and user foods may have been placed under a matched parent; detach before deleting it.
    sqlx::query(
        "UPDATE foods c SET parent_id = NULL FROM foods p
         WHERE p.id = c.parent_id AND p.origin = 'matched' AND c.origin <> 'matched'",
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query("DELETE FROM foods WHERE origin = 'matched'")
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

/// Match every unmatched ingredient line to a food, creating foods as needed, then propose tree links for
/// the foods created in this run.
pub async fn match_foods(pool: &PgPool) -> Result<MatchStats> {
    let mut tx = pool.begin().await?;
    let stats = match_foods_in(&mut tx).await?;
    tx.commit().await?;
    Ok(stats)
}

/// The key of the advisory lock one matching run holds until its transaction ends ("mise food" in ASCII).
const MATCHING_LOCK: i64 = 0x6d69_7365_666f_6f64;

/// [`match_foods`] inside the caller's transaction, so saving a recipe and matching its lines succeed or fail
/// together. Runs take turns (an advisory lock held to the end of the transaction): two at once would each make a
/// food for the same new name.
pub async fn match_foods_in(tx: &mut Transaction<'_, Postgres>) -> Result<MatchStats> {
    let mut stats = MatchStats::default();
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(MATCHING_LOCK)
        .execute(&mut **tx)
        .await
        .context("waiting for another matching run")?;

    // Unmatched names, most frequent spelling first so it becomes the food's display name.
    let names: Vec<(String, i64)> = sqlx::query_as(
        "SELECT raw_name, count(*) FROM ingredient_lines WHERE food_id IS NULL AND raw_name <> ''
         GROUP BY raw_name ORDER BY count(*) DESC",
    )
    .fetch_all(&mut **tx)
    .await?;
    stats.names = names.len();
    if names.is_empty() {
        return Ok(stats);
    }
    let brands = brands(&mut **tx).await?;

    // Existing foods by key.
    let mut by_key: HashMap<String, Food> = HashMap::new();
    let rows: Vec<(String, Uuid, String)> = sqlx::query_as(
        "SELECT a.alias, f.id, f.name FROM food_aliases a JOIN foods f ON f.id = a.food_id",
    )
    .fetch_all(&mut **tx)
    .await?;
    for (alias, id, name) in rows {
        by_key.insert(
            alias,
            Food {
                id,
                name,
                created: false,
            },
        );
    }

    let mut new_foods: Vec<(Uuid, String, String, &'static str)> = Vec::new(); // id, name, slug, kind
    let mut slugs_taken: std::collections::HashSet<String> =
        sqlx::query_scalar::<_, String>("SELECT slug FROM foods")
            .fetch_all(&mut **tx)
            .await?
            .into_iter()
            .collect();
    let mut match_names = Vec::new();
    let mut match_ids = Vec::new();
    for (raw, _) in &names {
        let key = singular_key(raw);
        if key.is_empty() {
            continue;
        }
        let food = by_key.entry(key).or_insert_with(|| {
            let id = Uuid::now_v7();
            let mut slug = slugify(raw);
            let mut n = 2;
            while slugs_taken.contains(&slug) {
                slug = format!("{}-{n}", slugify(raw));
                n += 1;
            }
            slugs_taken.insert(slug.clone());
            let kind = if strip_brand(raw, &brands).is_some() {
                "brand"
            } else {
                "food"
            };
            new_foods.push((id, raw.trim().to_string(), slug, kind));
            Food {
                id,
                name: raw.trim().to_string(),
                created: true,
            }
        });
        match_names.push(raw.clone());
        match_ids.push(food.id);
    }
    stats.foods_created = new_foods.len();

    sqlx::query(
        "INSERT INTO foods (id, name, slug, kind)
         SELECT id, name, slug, kind::food_kind FROM UNNEST($1::uuid[], $2::text[], $3::text[], $4::text[]) AS t(id, name, slug, kind)",
    )
    .bind(new_foods.iter().map(|f| f.0).collect::<Vec<_>>())
    .bind(new_foods.iter().map(|f| f.1.clone()).collect::<Vec<_>>())
    .bind(new_foods.iter().map(|f| f.2.clone()).collect::<Vec<_>>())
    .bind(new_foods.iter().map(|f| f.3.to_string()).collect::<Vec<_>>())
    .execute(&mut **tx)
    .await
    .context("inserting foods")?;

    let (alias_keys, alias_ids): (Vec<String>, Vec<Uuid>) = by_key
        .iter()
        .filter(|(_, f)| f.created)
        .map(|(k, f)| (k.clone(), f.id))
        .unzip();
    sqlx::query(
        "INSERT INTO food_aliases (food_id, alias) SELECT f, a FROM UNNEST($1::uuid[], $2::text[]) AS t(f, a)
         ON CONFLICT DO NOTHING",
    )
    .bind(&alias_ids)
    .bind(&alias_keys)
    .execute(&mut **tx)
    .await
    .context("inserting aliases")?;

    stats.lines_matched = sqlx::query(
        "UPDATE ingredient_lines il SET food_id = t.f
         FROM UNNEST($1::text[], $2::uuid[]) AS t(n, f)
         WHERE il.raw_name = t.n AND il.food_id IS NULL",
    )
    .bind(&match_names)
    .bind(&match_ids)
    .execute(&mut **tx)
    .await
    .context("matching lines")?
    .rows_affected();

    // How many recipe lines (vs. product and ready-meal lines) use each food, so review can sort by impact.
    let usage: HashMap<Uuid, (i64, i64)> = sqlx::query_as::<_, (Uuid, i64, i64)>(
        "SELECT il.food_id, count(*) FILTER (WHERE v.meta->>'content_type' = 'recipe'), count(*)
         FROM ingredient_lines il JOIN variants v ON v.id = il.variant_id
         WHERE il.food_id IS NOT NULL GROUP BY il.food_id",
    )
    .fetch_all(&mut **tx)
    .await?
    .into_iter()
    .map(|(id, recipes, all)| (id, (recipes, all)))
    .collect();

    propose_tree(tx, &by_key, &usage, &brands, &mut stats).await?;
    propose_duplicates(tx, &by_key, &usage, &mut stats).await?;

    // Foods per 100 ingredient lines: matching makes a food for every name it doesn't know, so a jump here means
    // the sources started spelling things differently, and the duplicates above are worth a look.
    let (foods, lines): (i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM foods WHERE kind <> 'category'),
                (SELECT count(*) FROM ingredient_lines)",
    )
    .fetch_one(&mut **tx)
    .await?;
    stats.foods_per_100_lines = if lines == 0 {
        0.0
    } else {
        // One decimal: enough to show a drift, not so much that a line or two rewrites the snapshot.
        ((foods as f64) * 1000.0 / (lines as f64)).round() / 10.0
    };
    Ok(stats)
}

/// Pair up the foods this run made whose names look like the same food written differently, and raise each pair
/// for a person ([`duplicate_key`] says which names are near). Only foods matching made are paired: one a person
/// added or curated is their answer already, and the `category` foods a bundle's knowledge brings are not foods
/// recipes name.
///
/// A pair matching has already answered itself is left alone: one is a kind of the other, the two are `related`,
/// or a person said they are different foods (`food_distinctions`, the `distinct` curation op).
async fn propose_duplicates(
    tx: &mut Transaction<'_, Postgres>,
    by_key: &HashMap<String, Food>,
    usage: &HashMap<Uuid, (i64, i64)>,
    stats: &mut MatchStats,
) -> Result<()> {
    // Every food recipes name, not only this run's: a name that arrives today may duplicate one matched last week.
    // Only the foods matching made are paired; a person's own name is their answer already.
    let matched: Vec<(Uuid, String)> = sqlx::query_as(
        "SELECT id, name FROM foods WHERE kind <> 'category' AND origin = 'matched'",
    )
    .fetch_all(&mut **tx)
    .await
    .context("reading foods to pair")?;
    let made_now: std::collections::HashSet<Uuid> = by_key
        .values()
        .filter(|f| f.created)
        .map(|f| f.id)
        .collect();

    // The pairs a person has already told apart, each side under its own name key (`singular_key`): the two sides
    // of a pair share a `duplicate_key` by definition, so that could not tell them apart.
    let told_apart: std::collections::HashSet<(String, String)> =
        sqlx::query_as::<_, (String, String)>("SELECT a_name, b_name FROM food_distinctions")
            .fetch_all(&mut **tx)
            .await
            .context("reading food distinctions")?
            .into_iter()
            .collect();
    // Group by the key the names of one food share; a group of one has nothing to ask about.
    let mut groups: HashMap<String, Vec<(Uuid, String)>> = HashMap::new();
    for (id, name) in matched {
        groups
            .entry(duplicate_key(&name))
            .or_default()
            .push((id, name));
    }

    let mut pairs: Vec<(Uuid, String, Uuid, String)> = Vec::new();
    for (_, mut group) in groups {
        if group.len() < 2 {
            continue;
        }
        // Most used first, and the shortest name to break a tie, so the pair reads as "fold the longer, rarer
        // spelling into the plain one everything else uses".
        group.sort_by_key(|(id, name)| {
            (
                std::cmp::Reverse(usage.get(id).map_or(0, |u| u.1)),
                name.chars().count(),
                name.clone(),
            )
        });
        for (i, (a, a_name)) in group.iter().enumerate() {
            for (b, b_name) in &group[i + 1..] {
                // At least one side must be new, or this pair was offered on an earlier run already.
                if !made_now.contains(a) && !made_now.contains(b) {
                    continue;
                }
                // A person who told this pair apart stored it by each side's own name key.
                let (x, y) = (singular_key(a_name), singular_key(b_name));
                let pair = if x < y { (x, y) } else { (y, x) };
                if told_apart.contains(&pair) {
                    continue;
                }
                pairs.push((*a, a_name.clone(), *b, b_name.clone()));
            }
        }
    }

    if pairs.is_empty() {
        return Ok(());
    }
    // Drop the pairs matching has already answered itself: one is a kind of the other, or the two are related.
    let linked: std::collections::HashSet<(Uuid, Uuid)> = sqlx::query_as::<_, (Uuid, Uuid)>(
        "SELECT c.id, p.id FROM foods c JOIN foods p ON p.id = c.parent_id
         UNION SELECT from_food, to_food FROM food_relations",
    )
    .fetch_all(&mut **tx)
    .await
    .context("reading links")?
    .into_iter()
    .collect();
    pairs.retain(|(a, _, b, _)| !linked.contains(&(*a, *b)) && !linked.contains(&(*b, *a)));

    stats.duplicate_items = pairs.len();
    for (a, a_name, b, b_name) in pairs {
        let lines = |id: &Uuid| usage.get(id).map_or(0, |u| u.1);
        // How alike the two read, as pg_trgm scores it: the pairing goes by words, but this sorts the surest first.
        let confidence: f32 = sqlx::query_scalar("SELECT similarity($1, $2)")
            .bind(&a_name)
            .bind(&b_name)
            .fetch_one(&mut **tx)
            .await?;
        sqlx::query(
            "INSERT INTO review_items (id, kind, subject, proposal, confidence)
             VALUES ($1, 'food_duplicate', $2, $3, $4)",
        )
        .bind(Uuid::now_v7())
        .bind(json!({ "food_id": a, "name": a_name, "recipe_uses": usage.get(&a).map_or(0, |u| u.0), "all_uses": lines(&a) }))
        .bind(json!({ "other_id": b, "other_name": b_name, "other_uses": lines(&b) }))
        .bind(confidence)
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

/// For each food created in this run, find the longest other food name it contains as whole words at its
/// start or end, and link the two according to [`relate`].
async fn propose_tree(
    tx: &mut Transaction<'_, Postgres>,
    by_key: &HashMap<String, Food>,
    usage: &HashMap<Uuid, (i64, i64)>,
    brands: &Brands,
    stats: &mut MatchStats,
) -> Result<()> {
    // Review subjects carry usage; foods seen only in products and ready meals get low confidence so they sort last.
    let subject = |food: &Food| {
        let (recipe_uses, all_uses) = usage.get(&food.id).copied().unwrap_or((0, 0));
        json!({ "food_id": food.id, "name": food.name, "recipe_uses": recipe_uses, "all_uses": all_uses })
    };
    let confidence = |food: &Food, base: f32| {
        if usage.get(&food.id).map_or(0, |u| u.0) == 0 {
            0.1
        } else {
            base
        }
    };
    let mut parents: Vec<(Uuid, Uuid)> = Vec::new();
    let mut related: Vec<(Uuid, Uuid)> = Vec::new();
    let mut reviews: Vec<(String, Value, Value, f32)> = Vec::new();

    for (key, food) in by_key.iter().filter(|(_, f)| f.created) {
        // A branded name links to its unbranded form first.
        let base_key = strip_brand(&food.name, brands).map(|(_, rest)| singular_key(&rest));
        if let Some(base) = base_key
            .as_ref()
            .and_then(|k| by_key.get(k))
            .filter(|b| b.id != food.id)
        {
            parents.push((food.id, base.id));
            continue;
        }

        let words: Vec<&str> = key.split(' ').collect();
        let candidates = (1..words.len())
            .flat_map(|n| [words[..n].join(" "), words[words.len() - n..].join(" ")]);
        let best = candidates
            .filter_map(|c| by_key.get(&c).map(|b| (c, b)))
            .filter(|(_, b)| b.id != food.id)
            .max_by_key(|(c, _)| c.len());
        let Some((_, base)) = best else { continue };

        match relate(&food.name, &base.name, brands) {
            Some(Relation::KindOf) => parents.push((food.id, base.id)),
            Some(Relation::Related) => related.push((food.id, base.id)),
            Some(Relation::Unsure) => reviews.push((
                "food_parent".into(),
                subject(food),
                json!({ "parent_id": base.id, "parent_name": base.name }),
                confidence(food, 0.5),
            )),
            None => {}
        }
    }

    for (name, note) in AMBIGUOUS {
        if let Some(food) = by_key.get(*name).filter(|f| f.created) {
            reviews.push((
                "food_match".into(),
                subject(food),
                json!({ "note": note }),
                confidence(food, 0.3),
            ));
        }
    }

    stats.kind_of_links = parents.len();
    stats.related_links = related.len();
    stats.review_items = reviews.len();

    sqlx::query("UPDATE foods f SET parent_id = t.p FROM UNNEST($1::uuid[], $2::uuid[]) AS t(c, p) WHERE f.id = t.c")
        .bind(parents.iter().map(|p| p.0).collect::<Vec<_>>())
        .bind(parents.iter().map(|p| p.1).collect::<Vec<_>>())
        .execute(&mut **tx)
        .await
        .context("setting parents")?;
    sqlx::query(
        "INSERT INTO food_relations (id, from_food, to_food, kind)
         SELECT gen_random_uuid(), a, b, 'related' FROM UNNEST($1::uuid[], $2::uuid[]) AS t(a, b)",
    )
    .bind(related.iter().map(|p| p.0).collect::<Vec<_>>())
    .bind(related.iter().map(|p| p.1).collect::<Vec<_>>())
    .execute(&mut **tx)
    .await
    .context("adding related links")?;
    for (kind, subject, proposal, confidence) in reviews {
        sqlx::query("INSERT INTO review_items (id, kind, subject, proposal, confidence) VALUES ($1, $2, $3, $4, $5)")
            .bind(Uuid::now_v7())
            .bind(kind)
            .bind(subject)
            .bind(proposal)
            .bind(confidence)
            .execute(&mut **tx)
            .await?;
    }
    Ok(())
}
