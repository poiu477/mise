//! `mise gate-report`: the phase 1 gate check, "dish grouping checked by hand on 500 dishes".
//!
//! Draws a stratified sample of dishes (see `sample`), gathers what a person needs to judge each one (its
//! versions grouped by title, options, the default version's shipped ingredients, open merge reviews and
//! look-alike dishes), and writes one self-contained HTML file where decisions are recorded in the browser
//! and exported as JSON. Reads the database only.

mod data;
mod html;
mod sample;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

use anyhow::{Context, Result};
use chrono::{DateTime, Datelike, Utc};
use mise_core::dish_titles::{clean_title, protein_core};
use mise_core::text::normalize_name;
use sqlx::PgPool;
use uuid::Uuid;

use data::{Dish, Line, Snapshot, Variant};
pub use sample::STRATA;

/// Look-alike dishes shown per card, closest first.
const MAX_RELATED: usize = 6;
/// Dishes whose titles match once proteins are removed count as twins only when their default versions ship
/// this much in common: "Chicken Tacos" and "Beef Tacos" are often unrelated recipes.
const CORE_TWIN_OVERLAP: f64 = 0.5;

pub struct Report {
    pub generated_at: DateTime<Utc>,
    pub seed: u64,
    pub requested: usize,
    pub summary: Summary,
    pub strata: Vec<StratumRow>,
    pub cards: Vec<Card>,
}

pub struct Summary {
    pub dishes: usize,
    pub variants: usize,
    /// content type -> (dishes, variants)
    pub by_type: BTreeMap<String, (usize, usize)>,
    /// (bucket label, dishes)
    pub versions_hist: Vec<(&'static str, usize)>,
    pub titles_hist: Vec<(&'static str, usize)>,
    pub mean_versions: f64,
    pub max_versions: usize,
    /// option key -> dishes having it
    pub options: Vec<(String, usize)>,
    pub with_any_option: usize,
    pub multi_version_dishes: usize,
    pub lineage_linked: usize,
    pub title_twinned: usize,
    pub protein_twinned: usize,
    pub in_merge_review: usize,
    pub review_counts: Vec<(String, String, i64)>,
}

pub struct StratumRow {
    pub key: &'static str,
    pub label: &'static str,
    pub about: &'static str,
    /// Dishes in the whole data set that qualify.
    pub population: usize,
    pub sampled: usize,
}

pub struct Card {
    pub dish: Dish,
    pub stratum: usize,
    pub tags: Vec<usize>,
    pub versions: usize,
    pub first: Option<DateTime<Utc>>,
    pub last: Option<DateTime<Utc>>,
    pub options: Vec<OptionView>,
    pub title_groups: Vec<TitleGroup>,
    pub rows: Vec<VersionRow>,
    pub default_version: Option<VersionRow>,
    pub shipped: Vec<Food>,
    pub pantry: Vec<Food>,
    pub reviews: Vec<ReviewView>,
    pub related: Vec<Related>,
}

pub struct OptionView {
    pub label: String,
    pub default_label: Option<String>,
    /// (label, variants, years)
    pub choices: Vec<(String, u64, String)>,
}

pub struct TitleGroup {
    pub title: String,
    pub subtitle: Option<String>,
    pub count: usize,
    pub years: String,
    pub proteins: Vec<(String, usize)>,
    pub portions: Vec<(String, usize)>,
    pub versions: Vec<(String, usize)>,
    pub has_default: bool,
    /// Versions in this group copied from a version in another dish.
    pub copied_in: usize,
}

#[derive(Clone)]
pub struct VersionRow {
    pub published: Option<DateTime<Utc>>,
    pub title: String,
    pub subtitle: Option<String>,
    pub protein: String,
    pub portion: String,
    pub version: String,
    pub has_steps: bool,
    pub parent: Parent,
    pub is_default: bool,
}

#[derive(Clone)]
pub enum Parent {
    None,
    SameDish,
    /// Copied from a version in another dish: (that dish's title, slug).
    Other(String, String),
}

pub struct Food {
    pub name: String,
    pub matched: bool,
}

pub struct ReviewView {
    /// True when this dish holds the copy; false when the other dish copied from this one.
    pub this_is_copy: bool,
    pub copy_title: String,
    pub source_title: String,
    pub other: RelatedDish,
    pub title_similarity: Option<f64>,
    pub shipped_overlap: Option<f64>,
    pub confidence: Option<f32>,
}

pub struct Related {
    pub dish: RelatedDish,
    pub why: Vec<String>,
    /// Jaccard overlap of the two default versions' shipped foods.
    pub overlap: Option<f64>,
    pub shipped: Vec<String>,
}

#[derive(Clone)]
pub struct RelatedDish {
    pub title: String,
    pub subtitle: Option<String>,
    pub slug: String,
    pub content_type: String,
    pub versions: usize,
    pub years: String,
    /// Card number, when this dish is in the sample too.
    pub in_sample: Option<usize>,
}

pub async fn write_report(pool: &PgPool, sample_size: usize, seed: u64, out: &Path) -> Result<Report> {
    let snap = data::load_snapshot(pool).await?;
    let report = build(pool, &snap, sample_size, seed).await?;
    if let Some(dir) = out.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    std::fs::write(out, html::render(&report)).with_context(|| format!("writing {}", out.display()))?;
    Ok(report)
}

/// Per-dish indexes over the snapshot.
struct Index<'a> {
    dish_at: HashMap<Uuid, usize>,
    variants_of: Vec<Vec<&'a Variant>>,
    variant: HashMap<Uuid, &'a Variant>,
    /// Other dishes sharing lineage, with the number of copy links between them.
    lineage: Vec<HashMap<usize, usize>>,
    /// Other dishes with the same cleaned title, and with the same protein core.
    same_title: Vec<Vec<usize>>,
    same_core: Vec<Vec<usize>>,
    reviews_of: Vec<Vec<usize>>,
}

fn index(snap: &Snapshot) -> Index<'_> {
    let dish_at: HashMap<Uuid, usize> = snap.dishes.iter().enumerate().map(|(i, d)| (d.id, i)).collect();
    let mut variants_of: Vec<Vec<&Variant>> = vec![Vec::new(); snap.dishes.len()];
    let variant: HashMap<Uuid, &Variant> = snap.variants.iter().map(|v| (v.id, v)).collect();
    let mut lineage: Vec<HashMap<usize, usize>> = vec![HashMap::new(); snap.dishes.len()];
    for v in &snap.variants {
        let Some(&d) = dish_at.get(&v.dish_id) else { continue };
        variants_of[d].push(v);
        if let Some(p) = v.parent_variant_id.and_then(|p| variant.get(&p)).and_then(|p| dish_at.get(&p.dish_id)) {
            if *p != d {
                *lineage[d].entry(*p).or_default() += 1;
                *lineage[*p].entry(d).or_default() += 1;
            }
        }
    }
    let group = |key: &dyn Fn(&Dish) -> String| -> Vec<Vec<usize>> {
        let mut by: HashMap<String, Vec<usize>> = HashMap::new();
        for (i, d) in snap.dishes.iter().enumerate() {
            let k = key(d);
            if !k.is_empty() {
                by.entry(k).or_default().push(i);
            }
        }
        let mut out = vec![Vec::new(); snap.dishes.len()];
        for members in by.values().filter(|m| m.len() > 1) {
            for &m in members {
                out[m] = members.iter().copied().filter(|&o| o != m).collect();
            }
        }
        out
    };
    let same_title = group(&|d| normalize_name(&clean_title(&d.title)));
    let mut same_core = group(&|d| protein_core(&d.title));
    for (d, twins) in same_core.iter_mut().enumerate() {
        twins.retain(|&o| !same_title[d].contains(&o) && overlap(snap, d, o).is_some_and(|x| x >= CORE_TWIN_OVERLAP));
    }
    let mut reviews_of = vec![Vec::new(); snap.dishes.len()];
    for (r, item) in snap.merge_reviews.iter().enumerate() {
        for id in [item.dish_id, item.other_dish_id] {
            if let Some(&d) = dish_at.get(&id) {
                if !reviews_of[d].contains(&r) {
                    reviews_of[d].push(r);
                }
            }
        }
    }
    Index { dish_at, variants_of, variant, lineage, same_title, same_core, reviews_of }
}

async fn build(pool: &PgPool, snap: &Snapshot, sample_size: usize, seed: u64) -> Result<Report> {
    let ix = index(snap);
    let options_of: HashMap<Uuid, Vec<&data::DishOption>> = snap.options.iter().fold(HashMap::new(), |mut m, o| {
        m.entry(o.dish_id).or_default().push(o);
        m
    });

    let features: Vec<sample::Features> = snap
        .dishes
        .iter()
        .enumerate()
        .map(|(i, d)| {
            let titles: HashSet<String> = ix.variants_of[i].iter().map(|v| normalize_name(&clean_title(&v.title))).collect();
            sample::Features {
                id: d.id,
                content_type: d.content_type.clone(),
                versions: ix.variants_of[i].len(),
                titles: titles.len(),
                protein_choice: options_of.get(&d.id).is_some_and(|os| os.iter().any(|o| o.key == "protein")),
                merge_review: !ix.reviews_of[i].is_empty(),
                lineage_links: ix.lineage[i].len(),
                title_twins: ix.same_title[i].len(),
                protein_twins: ix.same_core[i].len(),
            }
        })
        .collect();
    let picks = sample::draw(&features, sample_size, seed);
    let card_no: HashMap<usize, usize> = picks.iter().enumerate().map(|(n, p)| (p.dish, n + 1)).collect();

    let strata = STRATA
        .iter()
        .enumerate()
        .map(|(s, st)| StratumRow {
            key: st.key,
            label: st.label,
            about: st.about,
            population: features.iter().filter(|f| (st.test)(f)).count(),
            sampled: picks.iter().filter(|p| p.stratum == s).count(),
        })
        .collect();

    // Related dishes per pick, so their default versions' ingredients load in the same query.
    let related_of: Vec<Vec<(usize, Vec<String>)>> = picks.iter().map(|p| related_dishes(&ix, snap, p.dish)).collect();
    let mut default_ids: Vec<Uuid> = picks.iter().filter_map(|p| snap.dishes[p.dish].default_variant_id).collect();
    for rel in &related_of {
        default_ids.extend(rel.iter().filter_map(|(d, _)| snap.dishes[*d].default_variant_id));
    }
    default_ids.sort();
    default_ids.dedup();
    let lines = data::load_lines(pool, &default_ids).await?;
    let picked_ids: Vec<Uuid> = picks.iter().map(|p| snap.dishes[p.dish].id).collect();
    let attributes = data::load_attributes(pool, &picked_ids).await?;

    let shipped_names = |d: usize| -> Vec<String> {
        snap.dishes[d]
            .default_variant_id
            .and_then(|v| lines.get(&v))
            .map(|ls| ls.iter().filter(|l| l.shipped).map(|l| food_of(l).name).collect())
            .unwrap_or_default()
    };
    let related_dish = |d: usize| -> RelatedDish {
        let dish = &snap.dishes[d];
        let vs = &ix.variants_of[d];
        RelatedDish {
            title: dish.title.clone(),
            subtitle: dish.subtitle.clone(),
            slug: dish.slug.clone(),
            content_type: dish.content_type.clone(),
            versions: vs.len(),
            years: years(vs.iter().map(|v| v.published)),
            in_sample: card_no.get(&d).copied(),
        }
    };

    let mut cards = Vec::with_capacity(picks.len());
    for (p, rel) in picks.iter().zip(related_of) {
        let d = p.dish;
        let dish = &snap.dishes[d];
        let vs = &ix.variants_of[d];
        let opts = options_of.get(&dish.id).cloned().unwrap_or_default();
        let version_label: HashMap<String, String> = opts
            .iter()
            .filter(|o| o.key == "version")
            .flat_map(|o| o.choices.as_array().cloned().unwrap_or_default())
            .filter_map(|c| Some((c["value"].as_str()?.to_string(), c["label"].as_str()?.to_string())))
            .collect();

        let rows: Vec<VersionRow> = vs
            .iter()
            .map(|v| {
                let attr = |k: &str| attributes.get(&v.id).and_then(|a| a.get(k)).cloned().unwrap_or_default();
                let version = attr("version");
                let parent = match v.parent_variant_id.and_then(|p| ix.variant.get(&p)) {
                    None => Parent::None,
                    Some(p) if p.dish_id == dish.id => Parent::SameDish,
                    Some(p) => match ix.dish_at.get(&p.dish_id) {
                        Some(&o) => Parent::Other(snap.dishes[o].title.clone(), snap.dishes[o].slug.clone()),
                        None => Parent::None,
                    },
                };
                VersionRow {
                    published: v.published,
                    title: v.title.clone(),
                    subtitle: v.subtitle.clone(),
                    protein: match attr("protein").as_str() {
                        "none" => "No meat".into(),
                        p => p.to_string(),
                    },
                    portion: attr("portion"),
                    version: version_label.get(&version).cloned().unwrap_or_else(|| if version == "standard" { "Standard".into() } else { version }),
                    has_steps: v.has_steps,
                    parent,
                    is_default: dish.default_variant_id == Some(v.id),
                }
            })
            .collect();

        let options = opts
            .iter()
            .map(|o| {
                let choices: Vec<(String, u64, String)> = o
                    .choices
                    .as_array()
                    .map(|cs| {
                        cs.iter()
                            .map(|c| {
                                (
                                    c["label"].as_str().unwrap_or("?").to_string(),
                                    c["variants"].as_u64().unwrap_or(0),
                                    c["years"].as_str().unwrap_or("").to_string(),
                                )
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let default_label = o.default_value.as_ref().and_then(|dv| {
                    o.choices.as_array()?.iter().find(|c| c["value"].as_str() == Some(dv))?["label"].as_str().map(str::to_string)
                });
                OptionView { label: o.label.clone(), default_label, choices }
            })
            .collect();

        let default_lines: &[Line] = dish.default_variant_id.and_then(|v| lines.get(&v)).map(Vec::as_slice).unwrap_or(&[]);
        let related = rel
            .into_iter()
            .map(|(o, why)| Related { dish: related_dish(o), why, overlap: overlap(snap, d, o), shipped: shipped_names(o) })
            .collect();

        let reviews = ix.reviews_of[d]
            .iter()
            .filter_map(|&r| {
                let item = &snap.merge_reviews[r];
                let this_is_copy = item.dish_id == dish.id;
                let other_id = if this_is_copy { item.other_dish_id } else { item.dish_id };
                let &o = ix.dish_at.get(&other_id)?;
                Some(ReviewView {
                    this_is_copy,
                    copy_title: item.variant_title.clone(),
                    source_title: item.copied_from_title.clone(),
                    other: related_dish(o),
                    title_similarity: item.title_similarity,
                    shipped_overlap: item.shipped_overlap,
                    confidence: item.confidence,
                })
            })
            .collect();

        cards.push(Card {
            dish: dish.clone(),
            stratum: p.stratum,
            tags: sample::tags(&features[d]),
            versions: vs.len(),
            first: vs.iter().filter_map(|v| v.published).min(),
            last: vs.iter().filter_map(|v| v.published).max(),
            options,
            title_groups: title_groups(&rows),
            default_version: rows.iter().find(|r| r.is_default).cloned(),
            rows,
            shipped: default_lines.iter().filter(|l| l.shipped).map(food_of).collect(),
            pantry: default_lines.iter().filter(|l| !l.shipped).map(food_of).collect(),
            reviews,
            related,
        });
    }

    Ok(Report {
        generated_at: Utc::now(),
        seed,
        requested: sample_size,
        summary: summarize(snap, &ix, &features, &options_of),
        strata,
        cards,
    })
}

/// Lineage partners first (most links), then same-title, then same-protein-core dishes.
fn related_dishes(ix: &Index, snap: &Snapshot, d: usize) -> Vec<(usize, Vec<String>)> {
    let mut why: BTreeMap<usize, Vec<String>> = BTreeMap::new();
    let mut lineage: Vec<(&usize, &usize)> = ix.lineage[d].iter().collect();
    lineage.sort_by_key(|(o, n)| (std::cmp::Reverse(**n), **o));
    let mut order: Vec<usize> = Vec::new();
    for (&o, &n) in lineage {
        why.entry(o).or_default().push(format!("shared lineage ×{n}"));
        order.push(o);
    }
    for (list, reason) in [(&ix.same_title[d], "same title"), (&ix.same_core[d], "same title apart from protein")] {
        // Closest ingredients first.
        let mut list = list.clone();
        list.sort_by(|&a, &b| overlap(snap, d, b).unwrap_or(0.0).total_cmp(&overlap(snap, d, a).unwrap_or(0.0)).then(a.cmp(&b)));
        for o in list {
            let e = why.entry(o).or_default();
            if e.is_empty() {
                order.push(o);
            }
            e.push(reason.to_string());
        }
    }
    order.into_iter().take(MAX_RELATED).map(|o| (o, why.remove(&o).unwrap_or_default())).collect()
}

fn title_groups(rows: &[VersionRow]) -> Vec<TitleGroup> {
    let mut groups: Vec<(String, Vec<&VersionRow>)> = Vec::new();
    let mut at: HashMap<String, usize> = HashMap::new();
    for r in rows {
        let key = normalize_name(&clean_title(&r.title));
        let i = *at.entry(key).or_insert_with(|| {
            groups.push((String::new(), Vec::new()));
            groups.len() - 1
        });
        groups[i].1.push(r);
    }
    let mut out: Vec<TitleGroup> = groups
        .into_iter()
        .map(|(_, rs)| TitleGroup {
            title: most_common(rs.iter().map(|r| clean_title(&r.title))).map(|(t, _)| t).unwrap_or_default(),
            subtitle: most_common(rs.iter().filter_map(|r| r.subtitle.as_deref().map(clean_title)).filter(|s| !s.is_empty()))
                .map(|(s, _)| s),
            count: rs.len(),
            years: years(rs.iter().map(|r| r.published)),
            proteins: counts(rs.iter().map(|r| r.protein.clone())),
            portions: counts(rs.iter().map(|r| r.portion.clone())),
            versions: counts(rs.iter().map(|r| r.version.clone())),
            has_default: rs.iter().any(|r| r.is_default),
            copied_in: rs.iter().filter(|r| matches!(r.parent, Parent::Other(..))).count(),
        })
        .collect();
    out.sort_by_key(|g| (std::cmp::Reverse(g.count), g.title.clone()));
    out
}

fn summarize(
    snap: &Snapshot,
    ix: &Index,
    features: &[sample::Features],
    options_of: &HashMap<Uuid, Vec<&data::DishOption>>,
) -> Summary {
    let mut by_type: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    for (i, d) in snap.dishes.iter().enumerate() {
        let e = by_type.entry(d.content_type.clone()).or_default();
        e.0 += 1;
        e.1 += ix.variants_of[i].len();
    }
    let bucket = |edges: &[(usize, usize, &'static str)], value: fn(&sample::Features) -> usize| {
        edges.iter().map(|&(lo, hi, label)| (label, features.iter().filter(|f| (lo..=hi).contains(&value(f))).count())).collect()
    };
    let mut options: BTreeMap<String, usize> = BTreeMap::new();
    for os in options_of.values() {
        for o in os {
            *options.entry(o.label.clone()).or_default() += 1;
        }
    }
    let variants: usize = features.iter().map(|f| f.versions).sum();
    Summary {
        dishes: snap.dishes.len(),
        variants,
        by_type,
        versions_hist: bucket(
            &[(1, 1, "1"), (2, 3, "2–3"), (4, 9, "4–9"), (10, 24, "10–24"), (25, 99, "25–99"), (100, usize::MAX, "100+")],
            |f| f.versions,
        ),
        titles_hist: bucket(&[(1, 1, "1"), (2, 2, "2"), (3, 5, "3–5"), (6, 10, "6–10"), (11, usize::MAX, "11+")], |f| f.titles),
        mean_versions: variants as f64 / snap.dishes.len().max(1) as f64,
        max_versions: features.iter().map(|f| f.versions).max().unwrap_or(0),
        options: options.into_iter().collect(),
        with_any_option: options_of.len(),
        multi_version_dishes: features.iter().filter(|f| f.versions > 1).count(),
        lineage_linked: features.iter().filter(|f| f.lineage_links > 0).count(),
        title_twinned: features.iter().filter(|f| f.title_twins > 0).count(),
        protein_twinned: features.iter().filter(|f| f.protein_twins > 0).count(),
        in_merge_review: features.iter().filter(|f| f.merge_review).count(),
        review_counts: snap.review_counts.clone(),
    }
}

/// Jaccard overlap of two dishes' default versions' shipped foods; None when neither ships anything.
fn overlap(snap: &Snapshot, a: usize, b: usize) -> Option<f64> {
    let empty = HashSet::new();
    let sa = snap.default_shipped.get(&snap.dishes[a].id).unwrap_or(&empty);
    let sb = snap.default_shipped.get(&snap.dishes[b].id).unwrap_or(&empty);
    let union = sa.union(sb).count();
    (union > 0).then(|| sa.intersection(sb).count() as f64 / union as f64)
}

fn food_of(l: &Line) -> Food {
    match &l.food_name {
        Some(name) => Food { name: name.clone(), matched: true },
        None => Food { name: l.raw_name.clone(), matched: false },
    }
}

/// "2019–2023", "2021", or "" when no version has a date.
pub fn years(dates: impl Iterator<Item = Option<DateTime<Utc>>>) -> String {
    let ys: Vec<i32> = dates.flatten().map(|d| d.year()).collect();
    match (ys.iter().min(), ys.iter().max()) {
        (Some(a), Some(b)) if a == b => a.to_string(),
        (Some(a), Some(b)) => format!("{a}–{b}"),
        _ => String::new(),
    }
}

/// Values with their counts, most common first, empty values left out.
fn counts(values: impl Iterator<Item = String>) -> Vec<(String, usize)> {
    let mut m: HashMap<String, usize> = HashMap::new();
    for v in values.filter(|v| !v.is_empty()) {
        *m.entry(v).or_default() += 1;
    }
    let mut out: Vec<(String, usize)> = m.into_iter().collect();
    out.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    out
}

fn most_common(values: impl Iterator<Item = String>) -> Option<(String, usize)> {
    counts(values).into_iter().next()
}
