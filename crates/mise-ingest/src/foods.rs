//! Foods: load researched food knowledge, match ingredient lines to foods, and propose the food tree.
//!
//! Matching keys every name by [`singular_key`], so "Onion" and "Onions" share one food. Tree links follow
//! [`relate`]: narrowing words make a kind-of link (applied, still `proposed` until reviewed), transforming
//! words make a `related` link, and anything else becomes a review item.

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

use anyhow::{Context, Result};
use mise_core::food_names::{Relation, relate, singular_key, strip_brand};
use mise_core::text::slugify;
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::{PgPool, Postgres, Transaction};
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
    let reader = BufReader::new(File::open(path).with_context(|| format!("opening {}", path.display()))?);
    let mut tx = pool.begin().await?;
    let mut added = 0;
    for (n, line) in reader.lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let food: KnowledgeFood = serde_json::from_str(&line).with_context(|| format!("foods.ndjson line {}", n + 1))?;

        // Category path: each level is a 'category' food under the previous one.
        let mut parent: Option<Uuid> = None;
        for depth in 0..food.parent_path.len() {
            let path = &food.parent_path[..=depth];
            let slug = format!("category-{}", slugify(&path.join(" ")));
            parent = Some(upsert_food(&mut tx, path.last().unwrap(), &slug, parent, "category").await?.0);
        }

        let kind = food.kind.as_deref().unwrap_or("food");
        let (food_id, created) = upsert_food(&mut tx, &food.name, &slugify(&food.name), parent, kind).await?;
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
        "INSERT INTO foods (id, name, slug, parent_id, kind) VALUES ($1, $2, $3, $4, $5::food_kind)
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
        None => Ok((sqlx::query_scalar("SELECT id FROM foods WHERE slug = $1").bind(slug).fetch_one(&mut **tx).await?, false)),
    }
}

async fn add_alias(tx: &mut Transaction<'_, Postgres>, food_id: Uuid, name: &str) -> Result<()> {
    sqlx::query("INSERT INTO food_aliases (food_id, alias) VALUES ($1, $2) ON CONFLICT DO NOTHING")
        .bind(food_id)
        .bind(singular_key(name))
        .execute(&mut **tx)
        .await?;
    Ok(())
}

/// Names that mean different foods depending on context; matched normally but flagged for a person.
const AMBIGUOUS: &[(&str, &str)] = &[("pepper", "black pepper or bell pepper (as a pantry item it is usually black pepper)")];

#[derive(Debug, Default)]
pub struct MatchStats {
    pub names: usize,
    pub foods_created: usize,
    pub lines_matched: u64,
    pub kind_of_links: usize,
    pub related_links: usize,
    pub review_items: usize,
}

struct Food {
    id: Uuid,
    name: String,
    created: bool,
}

/// Undo a previous matching run: unlink ingredient lines and delete the foods, links and review items it
/// created. Foods loaded as knowledge (and their categories and facts) are kept.
pub async fn reset_matching(pool: &PgPool) -> Result<()> {
    let mut tx = pool.begin().await?;
    sqlx::query("UPDATE ingredient_lines SET food_id = NULL WHERE food_id IS NOT NULL").execute(&mut *tx).await?;
    sqlx::query("DELETE FROM review_items WHERE kind IN ('food_parent', 'food_match')").execute(&mut *tx).await?;
    sqlx::query("DELETE FROM food_relations").execute(&mut *tx).await?;
    // Matching-created foods have no facts and are not categories.
    sqlx::query(
        "DELETE FROM foods f WHERE f.kind <> 'category'
           AND NOT EXISTS (SELECT 1 FROM food_facts ff WHERE ff.food_id = f.id)
           AND NOT EXISTS (SELECT 1 FROM foods c WHERE c.parent_id = f.id AND EXISTS (SELECT 1 FROM food_facts x WHERE x.food_id = c.id))",
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

/// Match every unmatched ingredient line to a food, creating foods as needed, then propose tree links for
/// the foods created in this run.
pub async fn match_foods(pool: &PgPool) -> Result<MatchStats> {
    let mut stats = MatchStats::default();

    // Existing foods by key.
    let mut by_key: HashMap<String, Food> = HashMap::new();
    let rows: Vec<(String, Uuid, String)> =
        sqlx::query_as("SELECT a.alias, f.id, f.name FROM food_aliases a JOIN foods f ON f.id = a.food_id")
            .fetch_all(pool)
            .await?;
    for (alias, id, name) in rows {
        by_key.insert(alias, Food { id, name, created: false });
    }

    // Unmatched names, most frequent spelling first so it becomes the food's display name.
    let names: Vec<(String, i64)> = sqlx::query_as(
        "SELECT raw_name, count(*) FROM ingredient_lines WHERE food_id IS NULL AND raw_name <> ''
         GROUP BY raw_name ORDER BY count(*) DESC",
    )
    .fetch_all(pool)
    .await?;
    stats.names = names.len();

    let mut new_foods: Vec<(Uuid, String, String, &'static str)> = Vec::new(); // id, name, slug, kind
    let mut slugs_taken: std::collections::HashSet<String> =
        sqlx::query_scalar::<_, String>("SELECT slug FROM foods").fetch_all(pool).await?.into_iter().collect();
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
            let kind = if strip_brand(raw).is_some() { "brand" } else { "food" };
            new_foods.push((id, raw.trim().to_string(), slug, kind));
            Food { id, name: raw.trim().to_string(), created: true }
        });
        match_names.push(raw.clone());
        match_ids.push(food.id);
    }
    stats.foods_created = new_foods.len();

    let mut tx = pool.begin().await?;
    sqlx::query(
        "INSERT INTO foods (id, name, slug, kind)
         SELECT id, name, slug, kind::food_kind FROM UNNEST($1::uuid[], $2::text[], $3::text[], $4::text[]) AS t(id, name, slug, kind)",
    )
    .bind(new_foods.iter().map(|f| f.0).collect::<Vec<_>>())
    .bind(new_foods.iter().map(|f| f.1.clone()).collect::<Vec<_>>())
    .bind(new_foods.iter().map(|f| f.2.clone()).collect::<Vec<_>>())
    .bind(new_foods.iter().map(|f| f.3.to_string()).collect::<Vec<_>>())
    .execute(&mut *tx)
    .await
    .context("inserting foods")?;

    let (alias_keys, alias_ids): (Vec<String>, Vec<Uuid>) =
        by_key.iter().filter(|(_, f)| f.created).map(|(k, f)| (k.clone(), f.id)).unzip();
    sqlx::query(
        "INSERT INTO food_aliases (food_id, alias) SELECT f, a FROM UNNEST($1::uuid[], $2::text[]) AS t(f, a)
         ON CONFLICT DO NOTHING",
    )
    .bind(&alias_ids)
    .bind(&alias_keys)
    .execute(&mut *tx)
    .await
    .context("inserting aliases")?;

    stats.lines_matched = sqlx::query(
        "UPDATE ingredient_lines il SET food_id = t.f
         FROM UNNEST($1::text[], $2::uuid[]) AS t(n, f)
         WHERE il.raw_name = t.n AND il.food_id IS NULL",
    )
    .bind(&match_names)
    .bind(&match_ids)
    .execute(&mut *tx)
    .await
    .context("matching lines")?
    .rows_affected();

    // How many recipe lines (vs. product and ready-meal lines) use each food, so review can sort by impact.
    let usage: HashMap<Uuid, (i64, i64)> = sqlx::query_as::<_, (Uuid, i64, i64)>(
        "SELECT il.food_id, count(*) FILTER (WHERE v.meta->>'content_type' = 'recipe'), count(*)
         FROM ingredient_lines il JOIN variants v ON v.id = il.variant_id
         WHERE il.food_id IS NOT NULL GROUP BY il.food_id",
    )
    .fetch_all(&mut *tx)
    .await?
    .into_iter()
    .map(|(id, recipes, all)| (id, (recipes, all)))
    .collect();

    propose_tree(&mut tx, &by_key, &usage, &mut stats).await?;
    tx.commit().await?;
    Ok(stats)
}

/// For each food created in this run, find the longest other food name it contains as whole words at its
/// start or end, and link the two according to [`relate`].
async fn propose_tree(
    tx: &mut Transaction<'_, Postgres>,
    by_key: &HashMap<String, Food>,
    usage: &HashMap<Uuid, (i64, i64)>,
    stats: &mut MatchStats,
) -> Result<()> {
    // Review subjects carry usage; foods seen only in products and ready meals get low confidence so they sort last.
    let subject = |food: &Food| {
        let (recipe_uses, all_uses) = usage.get(&food.id).copied().unwrap_or((0, 0));
        json!({ "food_id": food.id, "name": food.name, "recipe_uses": recipe_uses, "all_uses": all_uses })
    };
    let confidence = |food: &Food, base: f32| if usage.get(&food.id).map_or(0, |u| u.0) == 0 { 0.1 } else { base };
    let mut parents: Vec<(Uuid, Uuid)> = Vec::new();
    let mut related: Vec<(Uuid, Uuid)> = Vec::new();
    let mut reviews: Vec<(String, Value, Value, f32)> = Vec::new();

    for (key, food) in by_key.iter().filter(|(_, f)| f.created) {
        // A branded name links to its unbranded form first.
        let base_key = strip_brand(&food.name).map(|(_, rest)| singular_key(&rest));
        if let Some(base) = base_key.as_ref().and_then(|k| by_key.get(k)).filter(|b| b.id != food.id) {
            parents.push((food.id, base.id));
            continue;
        }

        let words: Vec<&str> = key.split(' ').collect();
        let candidates = (1..words.len()).flat_map(|n| [words[..n].join(" "), words[words.len() - n..].join(" ")]);
        let best = candidates
            .filter_map(|c| by_key.get(&c).map(|b| (c, b)))
            .filter(|(_, b)| b.id != food.id)
            .max_by_key(|(c, _)| c.len());
        let Some((_, base)) = best else { continue };

        match relate(&food.name, &base.name) {
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
            reviews.push(("food_match".into(), subject(food), json!({ "note": note }), confidence(food, 0.3)));
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
