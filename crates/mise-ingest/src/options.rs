//! Derive each dish's options from the differences between its variants, and mine ingredient swaps.
//!
//! Every variant gets attributes: its main protein, its portion (regular or double protein), and its version.
//! Versions compare shipped, non-protein ingredients after folding each food up the tree to its most generic
//! kind ("Roma Tomato" counts as Tomato, a brand as its generic), stopping at foods marked strict, and leaving
//! out foods that are usually pantry items. The most common ingredient set is "Standard"; others are labelled
//! by what differs from it. A dish's options are the attributes with more than one value. Brand swaps are not
//! stored per dish: they come from the food tree at display time.
//!
//! Versions that differ from Standard by exactly one ingredient swapped for another are mined across all
//! dishes: a swap seen in several dishes becomes a review item proposing an alias (same food, another name)
//! or a substitution rule.

use std::collections::{BTreeSet, HashMap, HashSet};

use anyhow::{Context, Result};
use chrono::{DateTime, Datelike, Utc};
use mise_core::proteins::{ounces, protein_of};
use mise_core::text::normalize_name;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

/// A protein amount this many times the dish's usual amount counts as a double portion.
const DOUBLE_RATIO: f64 = 1.8;
/// A food shipped in fewer than this share of the lines that use it is a pantry item.
const PANTRY_SHIPPED_SHARE: f64 = 0.5;
/// A one-for-one swap needs to appear in this many dishes before it is proposed for review.
const MIN_SWAP_DISHES: usize = 2;

#[derive(Debug, Default)]
pub struct OptionStats {
    pub dishes: usize,
    pub with_protein_choice: usize,
    pub with_double_portion: usize,
    pub with_versions: usize,
    pub attributes: usize,
    pub swaps_proposed: usize,
}

#[derive(sqlx::FromRow)]
struct VariantRow {
    id: Uuid,
    dish_id: Uuid,
    title: String,
    published: Option<DateTime<Utc>>,
    is_default: bool,
}

#[derive(sqlx::FromRow)]
struct LineRow {
    variant_id: Uuid,
    food_id: Uuid,
    shipped: Option<bool>,
    amount: Option<f64>,
    unit: Option<String>,
}

#[derive(sqlx::FromRow)]
struct FoodRow {
    id: Uuid,
    name: String,
    parent_id: Option<Uuid>,
    kind: String,
    strict: bool,
}

struct Facts {
    protein: Option<(&'static str, Option<f64>)>, // label, ounces
    titled_double: bool,
    signature: BTreeSet<Uuid>,
}

pub async fn derive_options(pool: &PgPool) -> Result<OptionStats> {
    let mut stats = OptionStats::default();
    let variants: Vec<VariantRow> = sqlx::query_as(
        "SELECT v.id, v.dish_id, v.title, v.published_at AS published, d.default_variant_id = v.id AS is_default
         FROM variants v JOIN dishes d ON d.id = v.dish_id",
    )
    .fetch_all(pool)
    .await?;
    let lines: Vec<LineRow> = sqlx::query_as(
        "SELECT il.variant_id, il.food_id, il.shipped, la.amount::float8 AS amount, la.unit
         FROM ingredient_lines il
         LEFT JOIN LATERAL (SELECT amount, unit FROM line_amounts WHERE line_id = il.id ORDER BY servings LIMIT 1) la ON true
         WHERE il.food_id IS NOT NULL",
    )
    .fetch_all(pool)
    .await
    .context("loading lines")?;
    let foods: HashMap<Uuid, FoodRow> = sqlx::query_as::<_, FoodRow>(
        "SELECT id, name, parent_id, kind::text AS kind, coalesce(props->>'default_flexibility', '') = 'strict' AS strict FROM foods",
    )
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|f| (f.id, f))
    .collect();

    // Fold each food up to its most generic kind, stopping at strict foods and at categories.
    let fold = |mut id: Uuid| -> Uuid {
        while let Some(f) = foods.get(&id) {
            match f.parent_id.and_then(|p| foods.get(&p)) {
                Some(p) if !f.strict && p.kind != "category" => id = p.id,
                _ => break,
            }
        }
        id
    };
    let mut folded: HashMap<Uuid, Uuid> = HashMap::new();
    let mut shipped_share: HashMap<Uuid, (u32, u32)> = HashMap::new();
    for l in &lines {
        let root = *folded.entry(l.food_id).or_insert_with(|| fold(l.food_id));
        let e = shipped_share.entry(root).or_default();
        e.1 += 1;
        e.0 += l.shipped.unwrap_or(false) as u32;
    }
    let pantry: HashSet<Uuid> = shipped_share
        .iter()
        .filter(|(_, (s, n))| (*s as f64) < PANTRY_SHIPPED_SHARE * *n as f64)
        .map(|(id, _)| *id)
        .collect();

    let mut facts: HashMap<Uuid, Facts> = variants
        .iter()
        .map(|v| {
            let t = v.title.to_lowercase();
            let titled_double = t
                .split(|c: char| !c.is_alphanumeric())
                .any(|w| w == "2x" || w == "double");
            (
                v.id,
                Facts {
                    protein: None,
                    titled_double,
                    signature: BTreeSet::new(),
                },
            )
        })
        .collect();
    for l in lines.iter().filter(|l| l.shipped.unwrap_or(false)) {
        let Some(f) = facts.get_mut(&l.variant_id) else {
            continue;
        };
        let name = &foods[&l.food_id].name;
        match protein_of(name) {
            Some(label) => {
                let oz = l
                    .amount
                    .zip(l.unit.as_deref())
                    .and_then(|(a, u)| ounces(a, u));
                // The main protein is the one with the largest amount (bacon on top of chicken stays a garnish).
                let bigger = match (&f.protein, oz) {
                    (None, _) => true,
                    (Some((_, Some(cur))), Some(new)) => new > *cur,
                    (Some((_, None)), Some(_)) => true,
                    _ => false,
                };
                if bigger {
                    f.protein = Some((label, oz));
                }
            }
            None => {
                let root = folded[&l.food_id];
                if !pantry.contains(&root) {
                    f.signature.insert(root);
                }
            }
        }
    }
    let name_of = |id: &Uuid| foods.get(id).map(|f| f.name.clone()).unwrap_or_default();

    let mut by_dish: HashMap<Uuid, Vec<&VariantRow>> = HashMap::new();
    for v in &variants {
        by_dish.entry(v.dish_id).or_default().push(v);
    }
    stats.dishes = by_dish.len();

    let (mut a_variant, mut a_key, mut a_value) = (vec![], vec![], vec![]);
    let (mut o_dish, mut o_key, mut o_label, mut o_pos, mut o_choices, mut o_default) =
        (vec![], vec![], vec![], vec![], vec![], vec![]);
    // (removed, added) -> (dishes, variants)
    let mut swaps: HashMap<(Uuid, Uuid), (HashSet<Uuid>, usize)> = HashMap::new();

    for (dish_id, members) in &by_dish {
        let default = members
            .iter()
            .find(|v| v.is_default)
            .copied()
            .unwrap_or(members[0]);

        // Protein and its usual amount (the median) for spotting double portions.
        let mut usual: HashMap<&str, Vec<f64>> = HashMap::new();
        for v in members {
            if let Some((label, Some(oz))) = facts[&v.id].protein {
                usual.entry(label).or_default().push(oz);
            }
        }
        let usual: HashMap<&str, f64> = usual
            .into_iter()
            .map(|(k, mut xs)| {
                xs.sort_by(|a, b| a.total_cmp(b));
                (k, xs[xs.len() / 2])
            })
            .collect();

        // Versions: the most common ingredient set is Standard; others are labelled by their difference.
        let mut sig_count: HashMap<&BTreeSet<Uuid>, usize> = HashMap::new();
        for v in members {
            *sig_count.entry(&facts[&v.id].signature).or_default() += 1;
        }
        let standard = *sig_count
            .iter()
            .max_by_key(|(sig, n)| (**n, **sig == &facts[&default.id].signature))
            .map(|(sig, _)| sig)
            .unwrap();
        let mut version_of: HashMap<&BTreeSet<Uuid>, String> =
            HashMap::from([(standard, "standard".to_string())]);
        let mut version_labels: HashMap<String, String> =
            HashMap::from([("standard".into(), "Standard".into())]);
        for (sig, n) in &sig_count {
            if version_of.contains_key(sig) {
                continue;
            }
            let value = format!("v{}", version_of.len() + 1);
            let added: Vec<Uuid> = sig.difference(standard).copied().collect();
            let removed: Vec<Uuid> = standard.difference(sig).copied().collect();
            if let ([a], [r]) = (added.as_slice(), removed.as_slice()) {
                let e = swaps.entry((*r, *a)).or_default();
                e.0.insert(*dish_id);
                e.1 += n;
            }
            version_labels.insert(
                value.clone(),
                describe_difference(&added, &removed, &name_of),
            );
            version_of.insert(sig, value);
        }

        let mut values: HashMap<&str, HashMap<String, Vec<&VariantRow>>> = HashMap::new();
        for v in members {
            let f = &facts[&v.id];
            let protein = f
                .protein
                .map(|(p, _)| p.to_string())
                .unwrap_or_else(|| "none".into());
            let double = f.titled_double
                || f.protein
                    .and_then(|(p, oz)| oz.zip(usual.get(p)))
                    .is_some_and(|(oz, u)| oz >= u * DOUBLE_RATIO);
            let portion = if double { "double" } else { "regular" };
            let version = version_of[&f.signature].clone();
            for (key, value) in [
                ("protein", protein),
                ("portion", portion.to_string()),
                ("version", version),
            ] {
                a_variant.push(v.id);
                a_key.push(key.to_string());
                a_value.push(value.clone());
                values
                    .entry(key)
                    .or_default()
                    .entry(value)
                    .or_default()
                    .push(v);
            }
        }

        for (pos, (key, label)) in [
            ("protein", "Protein"),
            ("portion", "Portion"),
            ("version", "Version"),
        ]
        .into_iter()
        .enumerate()
        {
            let Some(vals) = values.get(key).filter(|vals| vals.len() > 1) else {
                continue;
            };
            match key {
                "protein" => stats.with_protein_choice += 1,
                "portion" => stats.with_double_portion += 1,
                _ => stats.with_versions += 1,
            }
            let mut choices: Vec<_> = vals
                .iter()
                .map(|(value, vs)| {
                    let shown = match key {
                        "protein" if value == "none" => "No meat".to_string(),
                        "protein" => value.clone(),
                        "portion" if value == "double" => "Double protein".into(),
                        "portion" => "Regular".into(),
                        _ => version_labels[value].clone(),
                    };
                    json!({ "value": value, "label": shown, "variants": vs.len(), "years": years(vs) })
                })
                .collect();
            choices.sort_by_key(|c| std::cmp::Reverse(c["variants"].as_u64()));
            let default_value = vals
                .iter()
                .find(|(_, vs)| vs.iter().any(|v| v.id == default.id))
                .map(|(k, _)| k.clone());
            o_dish.push(*dish_id);
            o_key.push(key.to_string());
            o_label.push(label.to_string());
            o_pos.push(pos as i32);
            o_choices.push(serde_json::Value::Array(choices).to_string());
            o_default.push(default_value);
        }
    }
    stats.attributes = a_variant.len();

    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM variant_attributes")
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM dish_options")
        .execute(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO variant_attributes (variant_id, key, value) SELECT * FROM UNNEST($1::uuid[], $2::text[], $3::text[])")
        .bind(&a_variant)
        .bind(&a_key)
        .bind(&a_value)
        .execute(&mut *tx)
        .await
        .context("writing attributes")?;
    sqlx::query(
        "INSERT INTO dish_options (dish_id, key, label, position, choices, default_value)
         SELECT d, k, l, p, c::jsonb, dv FROM UNNEST($1::uuid[], $2::text[], $3::text[], $4::int4[], $5::text[], $6::text[]) AS t(d, k, l, p, c, dv)",
    )
    .bind(&o_dish)
    .bind(&o_key)
    .bind(&o_label)
    .bind(&o_pos)
    .bind(&o_choices)
    .bind(&o_default)
    .execute(&mut *tx)
    .await
    .context("writing options")?;

    // Swaps seen across several dishes, both directions combined: similar names suggest one food under two
    // names, others a substitution.
    let mut pairs: HashMap<(Uuid, Uuid), (HashSet<Uuid>, usize, usize)> = HashMap::new(); // dishes, a→b, b→a
    for ((removed, added), (dishes, variants)) in &swaps {
        let key = if removed < added {
            (*removed, *added)
        } else {
            (*added, *removed)
        };
        let e = pairs.entry(key).or_default();
        e.0.extend(dishes);
        if key.0 == *removed {
            e.1 += variants
        } else {
            e.2 += variants
        }
    }
    sqlx::query("DELETE FROM review_items WHERE kind = 'food_swap' AND status = 'open'")
        .execute(&mut *tx)
        .await?;
    for ((removed, added), (dishes, forward, backward)) in pairs
        .iter()
        .filter(|(_, (d, _, _))| d.len() >= MIN_SWAP_DISHES)
    {
        let variants = forward + backward;
        let (rn, an) = (name_of(removed), name_of(added));
        let (nr, na) = (normalize_name(&rn), normalize_name(&an));
        let alike = nr.contains(&na) || na.contains(&nr) || strsim::sorensen_dice(&nr, &na) >= 0.6;
        sqlx::query("INSERT INTO review_items (id, kind, subject, proposal, confidence) VALUES ($1, 'food_swap', $2, $3, $4)")
            .bind(Uuid::now_v7())
            .bind(json!({ "from_food": removed, "from_name": rn, "to_food": added, "to_name": an }))
            .bind(json!({ "suggestion": if alike { "alias" } else { "substitute" }, "dishes": dishes.len(), "variants": variants,
                          "from_to": forward, "to_from": backward }))
            .bind((dishes.len() as f32 / 10.0).min(1.0))
            .execute(&mut *tx)
            .await?;
        stats.swaps_proposed += 1;
    }
    tx.commit().await?;
    Ok(stats)
}

/// "with Squash instead of Zucchini", "with Broccoli added", "without Feta", or a count when it's many things.
fn describe_difference(
    added: &[Uuid],
    removed: &[Uuid],
    name_of: &impl Fn(&Uuid) -> String,
) -> String {
    let added: Vec<String> = added.iter().map(name_of).collect();
    let removed: Vec<String> = removed.iter().map(name_of).collect();
    match (added.as_slice(), removed.as_slice()) {
        ([a], [r]) => format!("with {a} instead of {r}"),
        ([], [r]) => format!("without {r}"),
        ([a], []) => format!("with {a} added"),
        ([], []) => "Same ingredients".into(),
        ([a, b], []) => format!("with {a} and {b} added"),
        ([], [a, b]) => format!("without {a} and {b}"),
        _ => {
            let mut sample: Vec<&str> = added.iter().map(String::as_str).take(2).collect();
            if sample.is_empty() {
                sample = removed.iter().map(String::as_str).take(2).collect();
            }
            format!(
                "{} ingredient changes, incl. {}",
                added.len() + removed.len(),
                sample.join(", ")
            )
        }
    }
}

fn years(vs: &[&VariantRow]) -> String {
    let ys: Vec<i32> = vs
        .iter()
        .filter_map(|v| v.published.map(|p| p.year()))
        .collect();
    match (ys.iter().min(), ys.iter().max()) {
        (Some(a), Some(b)) if a == b => a.to_string(),
        (Some(a), Some(b)) => format!("{a}–{b}"),
        _ => String::new(),
    }
}
