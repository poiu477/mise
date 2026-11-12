//! Group variants into dishes.
//!
//! Variants are joined (union-find) when one was copied from the other and they are recognisably the same
//! dish, or when separate lineages share the same cleaned title and subtitle. Copies that look like a recipe
//! reused as a template for a different dish are left apart and queued for review. Decisions recorded in
//! `dish_decisions` override all of this: decided merges are applied last, and decided pairs are not queued
//! again. Variants pinned by a person, and dishes a person made, are never touched.
//!
//! A household's own versions (imports, forks, recipes written in the app) group like any others, with one limit:
//! they may join a dish everyone sees, but never a dish only another household can see. A dish everyone sees takes its
//! name and the version it opens on from the versions everyone sees, so nothing of a household's own shows through it.

use std::collections::{BTreeMap, HashMap, HashSet};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use mise_core::dish_titles::{clean_title, core_similarity, protein_core, title_key};
use mise_core::proteins::protein_of;
use mise_core::text::{normalize_name, slugify};
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

/// Title similarity (Sørensen–Dice on letter pairs) and shipped-ingredient overlap a renamed copy needs to
/// count as the same dish.
const MIN_TITLE_SIMILARITY: f64 = 0.6;
const MIN_SHIPPED_OVERLAP: f64 = 0.5;
/// Near-identical shipped ingredients alone mean the same dish (a renamed protein swap such as "Cajun-Spiced
/// Chicken" ↔ "Cajun Pork Filet"), provided both versions ship enough items for the overlap to mean something.
const STRONG_SHIPPED_OVERLAP: f64 = 0.8;
const MIN_SHIPPED_ITEMS: usize = 4;
/// Within a dish every title must share this much of its protein-neutral core with the dish's anchor title,
/// or at least `WEAK_CORE_SIMILARITY` plus near-identical non-protein shipped ingredients.
const ANCHOR_CORE_SIMILARITY: f64 = 0.67;
const WEAK_CORE_SIMILARITY: f64 = 0.34;
/// Protein twins from separate lineages ("Gochujang Salmon" / "Gochujang Cod") merge when their non-protein
/// shipped ingredients overlap this much, or their subtitles are this similar.
const TWIN_OVERLAP: f64 = 0.5;
const TWIN_SUBTITLE_SIMILARITY: f64 = 0.6;

/// What one grouping run did, step by step, for the command's summary. The `merged_*` counts are joins that
/// brought two groups together; a join between versions already in one group isn't counted.
#[derive(Debug, Default)]
pub struct GroupStats {
    /// Versions the run grouped: those not pinned by a person and not in a dish a person made.
    pub variants: usize,
    /// Dishes the run ended with.
    pub dishes: usize,
    /// Copies joined to their parent because the cleaned titles match.
    pub merged_by_title: usize,
    /// Copies joined to their parent with only the protein changed: "Chicken Tacos" → "Pork Tacos".
    pub merged_as_protein_swap: usize,
    /// Copies joined to their parent on a similar title and shared shipped ingredients, or on near-identical
    /// shipped ingredients alone.
    pub merged_by_similarity: usize,
    /// Separate lineages joined because their cleaned title and subtitle match.
    pub merged_by_title_key: usize,
    /// Copies too unlike their parent to join, queued for a person to review.
    pub kept_apart_for_review: usize,
    /// New dishes split off a family that drifted, one small change at a time, into a different dish.
    pub split_from_drift: usize,
    /// Protein twins from separate lineages joined: "Lemon Herb Salmon" and "Lemon Herb Cod".
    pub merged_as_cross_lineage_twin: usize,
    /// Joins made because a person decided the pair is one dish (`dish_decisions`).
    pub merged_by_decision: usize,
    /// Dishes that kept their id from the previous run, so links to them still work.
    pub dishes_kept: usize,
    /// Kept dishes whose slug changed: their name changed, or "beef-ragu-2" became "beef-ragu" once that was free.
    pub slugs_changed: usize,
}

#[derive(sqlx::FromRow)]
struct V {
    id: Uuid,
    title: String,
    subtitle: Option<String>,
    parent_variant_id: Option<Uuid>,
    content_type: String,
    has_steps: bool,
    has_hero: bool,
    published: Option<chrono::DateTime<chrono::Utc>>,
    /// The household that owns the version; none for seed and bundle versions, which everyone sees.
    household_id: Option<Uuid>,
}

struct UnionFind(Vec<usize>);

impl UnionFind {
    fn find(&mut self, x: usize) -> usize {
        let mut root = x;
        while self.0[root] != root {
            root = self.0[root];
        }
        let mut x = x;
        while self.0[x] != root {
            let next = self.0[x];
            self.0[x] = root;
            x = next;
        }
        root
    }
    fn union(&mut self, a: usize, b: usize) -> bool {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra == rb {
            return false;
        }
        self.0[ra] = rb;
        true
    }
}

/// Regroups every version the job owns into dishes, in one transaction: joins them, sets each dish's default
/// version, name and slug, retires the slugs dishes no longer use, and queues doubtful copies for review. Running
/// it again on the same data gives the same dishes, ids and slugs.
pub async fn group_dishes(pool: &PgPool) -> Result<GroupStats> {
    let mut stats = GroupStats::default();
    let mut tx = pool.begin().await?;

    // Dishes keep their id across runs, and their slug while it still fits their name (links, plans and bookmarks
    // point at them): remember which dish each version was in before starting over.
    let mut old_dish: HashMap<Uuid, Uuid> = HashMap::new();
    let mut old_slug: HashMap<Uuid, String> = HashMap::new();
    for (v, d, slug) in sqlx::query_as::<_, (Uuid, Uuid, String)>(
        "SELECT v.id, d.id, d.slug FROM variants v JOIN dishes d ON d.id = v.dish_id WHERE d.origin = 'grouped' AND NOT v.dish_pinned",
    )
    .fetch_all(&mut *tx)
    .await?
    {
        old_dish.insert(v, d);
        old_slug.insert(d, slug);
    }
    // Deleting a dish below drops its retired slugs (they cascade), so they are written back at the end.
    let old_retired: Vec<(String, Uuid, DateTime<Utc>)> = sqlx::query_as(
        "SELECT r.slug, r.dish_id, r.retired_at FROM retired_slugs r JOIN dishes d ON d.id = r.dish_id WHERE d.origin = 'grouped'",
    )
    .fetch_all(&mut *tx)
    .await
    .context("loading retired slugs")?;

    // Start over for everything the job owns.
    sqlx::query("UPDATE variants v SET dish_id = NULL FROM dishes d WHERE d.id = v.dish_id AND d.origin = 'grouped' AND NOT v.dish_pinned")
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE dishes SET default_variant_id = NULL WHERE origin = 'grouped'")
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM dishes d WHERE origin = 'grouped' AND NOT EXISTS (SELECT 1 FROM variants v WHERE v.dish_id = d.id)")
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM review_items WHERE kind = 'dish_merge' AND status = 'open'")
        .execute(&mut *tx)
        .await?;

    let vs: Vec<V> = sqlx::query_as(
        "SELECT id, title, subtitle, parent_variant_id, coalesce(meta->>'content_type', 'recipe') AS content_type,
                has_steps, (meta->'media'->>'hero') IS NOT NULL AS has_hero, published_at AS published, household_id
         FROM variants WHERE dish_id IS NULL ORDER BY id",
    )
    .fetch_all(&mut *tx)
    .await?;
    stats.variants = vs.len();
    let index: HashMap<Uuid, usize> = vs.iter().enumerate().map(|(i, v)| (v.id, i)).collect();

    // Shipped ingredients only: pantry staples (salt, oil, butter) would make unrelated recipes look alike.
    // Non-protein shipped ingredients as well: protein swaps differ there by design, so sameness of a dish is
    // judged without them.
    let mut shipped: HashMap<Uuid, HashSet<Uuid>> = HashMap::new();
    let mut flavour: HashMap<Uuid, HashSet<Uuid>> = HashMap::new();
    let rows: Vec<(Uuid, Uuid, String)> = sqlx::query_as(
        "SELECT il.variant_id, il.food_id, f.name FROM ingredient_lines il
         JOIN variants v ON v.id = il.variant_id JOIN foods f ON f.id = il.food_id
         WHERE v.dish_id IS NULL AND il.shipped",
    )
    .fetch_all(&mut *tx)
    .await?;
    for (variant, food, name) in rows {
        shipped.entry(variant).or_default().insert(food);
        if protein_of(&name).is_none() {
            flavour.entry(variant).or_default().insert(food);
        }
    }
    let jaccard = |sets: &HashMap<Uuid, HashSet<Uuid>>, a: Uuid, b: Uuid| -> f64 {
        let empty = HashSet::new();
        let (sa, sb) = (
            sets.get(&a).unwrap_or(&empty),
            sets.get(&b).unwrap_or(&empty),
        );
        let union = sa.union(sb).count();
        if union == 0 {
            0.0
        } else {
            sa.intersection(sb).count() as f64 / union as f64
        }
    };
    let overlap = |a: Uuid, b: Uuid| jaccard(&shipped, a, b);
    let flavour_overlap = |a: Uuid, b: Uuid| jaccard(&flavour, a, b);

    // Recorded decisions, as pairs of variants still to be grouped (a record can have several versions).
    let decided: Vec<(String, Uuid, Uuid)> = sqlx::query_as(
        "SELECT dd.decision, va.id, vb.id FROM dish_decisions dd
         JOIN sources sa ON sa.key = dd.a_source
         JOIN source_records ra ON ra.source_id = sa.id AND ra.external_id = dd.a_external
         JOIN variants va ON va.source_record_id = ra.id
         JOIN sources sb ON sb.key = dd.b_source
         JOIN source_records rb ON rb.source_id = sb.id AND rb.external_id = dd.b_external
         JOIN variants vb ON vb.source_record_id = rb.id",
    )
    .fetch_all(&mut *tx)
    .await
    .context("loading dish decisions")?;
    let mut decided_pairs: HashSet<(usize, usize)> = HashSet::new();
    let mut decided_merges: Vec<(usize, usize)> = Vec::new();
    for (decision, a, b) in &decided {
        let (Some(&a), Some(&b)) = (index.get(a), index.get(b)) else {
            continue;
        };
        decided_pairs.insert((a.min(b), a.max(b)));
        if decision == "merge" {
            decided_merges.push((a, b));
        }
    }

    let clean: Vec<String> = vs.iter().map(|v| title_key(&v.title)).collect();
    let mut uf = UnionFind((0..vs.len()).collect());
    let mut reviews = Vec::new();

    // 1. Lineage: a copy joins its parent when it is recognisably the same dish.
    for (i, v) in vs.iter().enumerate() {
        let Some(&p) = v.parent_variant_id.and_then(|p| index.get(&p)) else {
            continue;
        };
        if vs[p].content_type != v.content_type {
            continue;
        }
        if clean[i] == clean[p] {
            stats.merged_by_title += uf.union(i, p) as usize;
            continue;
        }
        let core = protein_core(&v.title);
        if !core.is_empty() && core == protein_core(&vs[p].title) {
            stats.merged_as_protein_swap += uf.union(i, p) as usize;
            continue;
        }
        let (sim, ov) = (
            strsim::sorensen_dice(&clean[i], &clean[p]),
            overlap(v.id, vs[p].id),
        );
        let enough_items = [v.id, vs[p].id]
            .iter()
            .all(|id| shipped.get(id).map_or(0, HashSet::len) >= MIN_SHIPPED_ITEMS);
        if (sim >= MIN_TITLE_SIMILARITY && ov >= MIN_SHIPPED_OVERLAP)
            || (ov >= STRONG_SHIPPED_OVERLAP && enough_items)
        {
            stats.merged_by_similarity += uf.union(i, p) as usize;
        } else if !decided_pairs.contains(&(i.min(p), i.max(p))) {
            reviews.push((v, &vs[p], sim, ov));
        }
    }
    stats.kept_apart_for_review = reviews.len();

    // 2. Separate lineages with the same cleaned title and subtitle are the same dish.
    let mut by_key: HashMap<(String, String, &str), usize> = HashMap::new();
    for (i, v) in vs.iter().enumerate() {
        let sub = normalize_name(&clean_title(v.subtitle.as_deref().unwrap_or("")));
        match by_key.entry((clean[i].clone(), sub, v.content_type.as_str())) {
            std::collections::hash_map::Entry::Occupied(e) => {
                stats.merged_by_title_key += uf.union(i, *e.get()) as usize
            }
            std::collections::hash_map::Entry::Vacant(e) => {
                e.insert(i);
            }
        }
    }

    // 2b. Protein twins from separate lineages: the same protein-neutral core (two words or more, so a bare
    //     "Tacos" does not qualify) and shared non-protein ingredients or similar subtitles.
    let sub_key: Vec<String> = vs
        .iter()
        .map(|v| normalize_name(&clean_title(v.subtitle.as_deref().unwrap_or(""))))
        .collect();
    let mut twins: HashMap<(String, &str), Vec<usize>> = HashMap::new(); // representatives per core
    for (i, v) in vs.iter().enumerate() {
        let core = protein_core(&v.title);
        if core.split(' ').filter(|w| !w.is_empty()).count() < 2 {
            continue;
        }
        let reps = twins.entry((core, v.content_type.as_str())).or_default();
        let mut placed = false;
        for &r in reps.iter() {
            if uf.find(r) == uf.find(i) {
                placed = true;
                break;
            }
            let subtitles_alike = !sub_key[i].is_empty()
                && strsim::sorensen_dice(&sub_key[i], &sub_key[r]) >= TWIN_SUBTITLE_SIMILARITY;
            if flavour_overlap(v.id, vs[r].id) >= TWIN_OVERLAP || subtitles_alike {
                stats.merged_as_cross_lineage_twin += uf.union(i, r) as usize;
                placed = true;
                break;
            }
        }
        if !placed {
            reps.push(i);
        }
    }

    // 3. Split families that drifted along a chain of small changes ("Chicken Cobb Salad" → … → "Shrimp,
    //    Lentil & Spinach Salad"). Every title group must be the same dish as its cluster's anchor (the most
    //    common title), not merely as its neighbour in the chain.
    let mut by_root: HashMap<usize, Vec<usize>> = HashMap::new();
    for i in 0..vs.len() {
        by_root.entry(uf.find(i)).or_default().push(i);
    }
    // Compared on protein-neutral cores: shared protein words ("Chicken ... Salad") would make different dishes
    // look alike.
    let same_dish = |a: usize, b: usize| -> bool {
        let sim = core_similarity(&vs[a].title, &vs[b].title);
        sim >= ANCHOR_CORE_SIMILARITY
            || (sim >= WEAK_CORE_SIMILARITY
                && flavour_overlap(vs[a].id, vs[b].id) >= STRONG_SHIPPED_OVERLAP
                && [a, b]
                    .iter()
                    .all(|&x| flavour.get(&vs[x].id).map_or(0, HashSet::len) >= MIN_SHIPPED_ITEMS))
    };
    let mut components: Vec<Vec<usize>> = Vec::new();
    for members in by_root.into_values() {
        let mut groups: HashMap<&str, Vec<usize>> = HashMap::new();
        for &m in &members {
            groups.entry(clean[m].as_str()).or_default().push(m);
        }
        if groups.len() == 1 {
            components.push(members);
            continue;
        }
        // Largest title groups first, so the anchor is the dish's usual name; each group is represented by
        // its newest version.
        // Ties go to the title itself, so a run on the same data always picks the same anchor.
        let mut groups: Vec<(&str, Vec<usize>)> = groups.into_iter().collect();
        groups.sort_by(|(ta, a), (tb, b)| {
            (b.len(), b.iter().map(|&m| vs[m].published).max())
                .cmp(&(a.len(), a.iter().map(|&m| vs[m].published).max()))
                .then(ta.cmp(tb))
        });
        let groups: Vec<Vec<usize>> = groups.into_iter().map(|(_, g)| g).collect();
        let mut clusters: Vec<(usize, Vec<usize>)> = Vec::new(); // (anchor representative, members)
        for g in groups {
            let rep = *g.iter().max_by_key(|&&m| vs[m].published).unwrap();
            match clusters
                .iter_mut()
                .find(|(anchor, _)| same_dish(rep, *anchor))
            {
                Some((_, ms)) => ms.extend(g),
                None => {
                    stats.split_from_drift += 1;
                    clusters.push((rep, g));
                }
            }
        }
        stats.split_from_drift -= 1; // the first cluster is the original dish, not a split
        components.extend(clusters.into_iter().map(|(_, ms)| ms));
    }

    // 3b. Decided merges, after the drift split so that splitting cannot undo them.
    if !decided_merges.is_empty() {
        let mut comp_of = vec![0; vs.len()];
        for (c, members) in components.iter().enumerate() {
            for &m in members {
                comp_of[m] = c;
            }
        }
        let mut cuf = UnionFind((0..components.len()).collect());
        for &(a, b) in &decided_merges {
            stats.merged_by_decision += cuf.union(comp_of[a], comp_of[b]) as usize;
        }
        let mut joined: HashMap<usize, Vec<usize>> = HashMap::new();
        for (c, members) in components.into_iter().enumerate() {
            joined.entry(cuf.find(c)).or_default().extend(members);
        }
        components = joined.into_values().collect();
        for members in &mut components {
            members.sort_unstable();
        }
    }

    // 3c. Households: a component with a version everyone sees may hold any household's own versions too, but one
    //     without is split by owner, so one household's version never joins a dish only another can see. Decided
    //     merges included: a decision is about recipes, not about who may see them.
    components = components
        .into_iter()
        .flat_map(|members| {
            let mut by_owner: BTreeMap<Option<Uuid>, Vec<usize>> = BTreeMap::new();
            for &m in &members {
                by_owner.entry(vs[m].household_id).or_default().push(m);
            }
            if by_owner.contains_key(&None) || by_owner.len() == 1 {
                vec![members]
            } else {
                by_owner.into_values().collect()
            }
        })
        .collect();

    // 4. One dish per component; its default is the most complete, newest version. Each component takes over
    //    the previous dish it shares the most versions with (greedily, largest overlap first), keeping its id,
    //    and its slug while that still fits the name. Larger dishes choose slugs first, so a big family gets the
    //    plain slug ahead of a stray.
    let mut votes: Vec<(usize, usize, Uuid)> = Vec::new(); // (shared versions, component, previous dish)
    for (c, members) in components.iter().enumerate() {
        let mut shared: HashMap<Uuid, usize> = HashMap::new();
        for &m in members {
            if let Some(d) = old_dish.get(&vs[m].id) {
                *shared.entry(*d).or_default() += 1;
            }
        }
        votes.extend(shared.into_iter().map(|(d, n)| (n, c, d)));
    }
    votes.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
    let mut previous: Vec<Option<Uuid>> = vec![None; components.len()];
    let mut taken_over = HashSet::new();
    for &(_, c, d) in &votes {
        if previous[c].is_none() && taken_over.insert(d) {
            previous[c] = Some(d);
        }
    }
    // Where each previous dish's addresses lead now: the component that took it over, or for one whose versions
    // all joined other dishes, the component that took the most of them.
    let mut heir: HashMap<Uuid, usize> = previous
        .iter()
        .enumerate()
        .filter_map(|(c, d)| d.map(|d| (d, c)))
        .collect();
    for &(_, c, d) in &votes {
        heir.entry(d).or_insert(c);
    }
    // Versions the household has had count for more when choosing a dish's default and name.
    let times: HashMap<Uuid, usize> = sqlx::query_as::<_, (Uuid, i32)>(
        "SELECT v.id, p.times FROM preferred_versions p JOIN sources s ON s.key = p.source
         JOIN source_records sr ON sr.source_id = s.id AND sr.external_id = p.external_id
         JOIN variants v ON v.source_record_id = sr.id",
    )
    .fetch_all(&mut *tx)
    .await?
    .into_iter()
    .map(|(v, n)| (v, n as usize))
    .collect();
    let votes = |m: usize| 1 + times.get(&vs[m].id).copied().unwrap_or(0);

    let mut order: Vec<usize> = (0..components.len()).collect();
    // Variants are loaded in id order, so a component's first member is a stable tie-break.
    order.sort_by_key(|&c| {
        (
            std::cmp::Reverse(components[c].len()),
            components[c].iter().min().copied(),
        )
    });

    let mut slugs: HashSet<String> = sqlx::query_scalar::<_, String>("SELECT slug FROM dishes")
        .fetch_all(&mut *tx)
        .await?
        .into_iter()
        .collect();
    let (mut d_id, mut d_title, mut d_sub, mut d_slug, mut d_type, mut d_default) =
        (vec![], vec![], vec![], vec![], vec![], vec![]);
    let (mut a_variant, mut a_dish) = (vec![], vec![]);
    let mut made: Vec<(Uuid, String)> = vec![(Uuid::nil(), String::new()); components.len()]; // (id, slug)
    for &c in &order {
        let members = &components[c];
        // A dish everyone sees is named by, and opens on, versions everyone sees; a household's own dish by its own.
        let shared: Vec<usize> = members
            .iter()
            .copied()
            .filter(|&m| vs[m].household_id.is_none())
            .collect();
        let named_by: &[usize] = if shared.is_empty() { members } else { &shared };
        let best = *named_by
            .iter()
            .max_by_key(|&&i| (vs[i].has_steps, votes(i), vs[i].has_hero, vs[i].published))
            .expect("components are never empty");
        // The dish is named by its most common cleaned title (ties go to the most recent), not by whichever
        // version happens to be the default: that one may be a protein swap or carry a word like "Organic".
        let mut counts: HashMap<String, (usize, Option<chrono::DateTime<chrono::Utc>>)> =
            HashMap::new();
        for &m in named_by {
            let e = counts.entry(clean_title(&vs[m].title)).or_default();
            e.0 += votes(m);
            e.1 = e.1.max(vs[m].published);
        }
        let title = counts
            .into_iter()
            .max_by(|(ta, (na, la)), (tb, (nb, lb))| (na, la).cmp(&(nb, lb)).then(tb.cmp(ta)))
            .map(|(t, _)| t)
            .unwrap_or_default();
        let base = slugify(&title);
        let kept = previous[c].and_then(|d| old_slug.get(&d)).cloned();
        // The previous slug while it fits the name: as it was, or the plain one when the previous was a numbered
        // copy of it and that is now free. A renamed dish takes its new name's slug; the old one is retired below.
        let mut slug = match &kept {
            Some(k) if strip_number(k) == base && slugs.contains(&base) => k.clone(),
            _ => base.clone(),
        };
        let mut n = 2;
        while !slugs.insert(slug.clone()) {
            slug = format!("{base}-{n}");
            n += 1;
        }
        if kept.as_ref().is_some_and(|k| *k != slug) {
            stats.slugs_changed += 1;
        }
        let id = previous[c].unwrap_or_else(Uuid::now_v7);
        stats.dishes_kept += previous[c].is_some() as usize;
        made[c] = (id, slug.clone());
        d_id.push(id);
        d_title.push(title.clone());
        // Subtitle: the most common one among versions that carry the chosen title, so the two read together.
        let mut subs: HashMap<String, usize> = HashMap::new();
        for &m in named_by
            .iter()
            .filter(|&&m| clean_title(&vs[m].title) == title)
        {
            *subs
                .entry(
                    vs[m]
                        .subtitle
                        .as_deref()
                        .map(clean_title)
                        .unwrap_or_default(),
                )
                .or_default() += votes(m);
        }
        d_sub.push(
            subs.into_iter()
                .max_by_key(|(s, n)| (*n, s.clone()))
                .map(|(s, _)| s)
                .filter(|s| !s.is_empty()),
        );
        d_slug.push(slug);
        d_type.push(vs[best].content_type.clone());
        d_default.push(vs[best].id);
        for &m in members {
            a_variant.push(vs[m].id);
            a_dish.push(id);
        }
    }
    stats.dishes = d_id.len();

    sqlx::query(
        "INSERT INTO dishes (id, title, subtitle, slug, content_type, default_variant_id, origin)
         SELECT id, t, s, sl, ct::content_type, dv, 'grouped'
         FROM UNNEST($1::uuid[], $2::text[], $3::text[], $4::text[], $5::text[], $6::uuid[]) AS x(id, t, s, sl, ct, dv)",
    )
    .bind(&d_id)
    .bind(&d_title)
    .bind(&d_sub)
    .bind(&d_slug)
    .bind(&d_type)
    .bind(&d_default)
    .execute(&mut *tx)
    .await
    .context("inserting dishes")?;
    sqlx::query("UPDATE variants v SET dish_id = x.d FROM UNNEST($1::uuid[], $2::uuid[]) AS x(v, d) WHERE v.id = x.v")
        .bind(&a_variant)
        .bind(&a_dish)
        .execute(&mut *tx)
        .await
        .context("assigning variants")?;

    // 5. Retired slugs: those from before, following their dish to its heir, then the previous slug of every dish
    //    that changed its slug or joined another dish (a newer retirement replaces an older one). A slug a live
    //    dish holds is never kept retired, so the live dish always answers to it.
    let mut retired: HashMap<String, (Uuid, Option<DateTime<Utc>>)> = HashMap::new();
    for (slug, d, at) in old_retired {
        if let Some(&c) = heir.get(&d) {
            retired.insert(slug, (made[c].0, Some(at)));
        }
    }
    for (d, slug) in &old_slug {
        if let Some(&c) = heir.get(d)
            && made[c].1 != *slug
        {
            retired.insert(slug.clone(), (made[c].0, None));
        }
    }
    let (mut r_slug, mut r_dish, mut r_at) = (vec![], vec![], vec![]);
    for (slug, (dish, at)) in retired {
        r_slug.push(slug);
        r_dish.push(dish);
        r_at.push(at);
    }
    sqlx::query(
        "INSERT INTO retired_slugs (slug, dish_id, retired_at)
         SELECT s, d, coalesce(at, now()) FROM UNNEST($1::text[], $2::uuid[], $3::timestamptz[]) AS x(s, d, at)
         ON CONFLICT (slug) DO UPDATE SET dish_id = EXCLUDED.dish_id, retired_at = EXCLUDED.retired_at",
    )
    .bind(&r_slug)
    .bind(&r_dish)
    .bind(&r_at)
    .execute(&mut *tx)
    .await
    .context("retiring slugs")?;
    sqlx::query("DELETE FROM retired_slugs r USING dishes d WHERE d.slug = r.slug")
        .execute(&mut *tx)
        .await
        .context("dropping retired slugs a dish holds again")?;

    let dish_of: HashMap<Uuid, Uuid> = a_variant
        .iter()
        .copied()
        .zip(a_dish.iter().copied())
        .collect();
    for (v, p, sim, ov) in reviews {
        let (dv, dp) = (dish_of[&v.id], dish_of[&p.id]);
        if dv == dp {
            continue; // joined some other way
        }
        sqlx::query("INSERT INTO review_items (id, kind, subject, proposal, confidence) VALUES ($1, 'dish_merge', $2, $3, $4)")
            .bind(Uuid::now_v7())
            .bind(json!({ "variant_id": v.id, "dish_id": dv, "title": clean_title(&v.title) }))
            .bind(json!({ "merge_into_dish": dp, "copied_from_variant": p.id, "copied_from_title": clean_title(&p.title),
                          "title_similarity": (sim * 100.0).round() / 100.0, "shipped_overlap": (ov * 100.0).round() / 100.0 }))
            .bind(((sim + ov) / 2.0) as f32)
            .execute(&mut *tx)
            .await?;
    }

    tx.commit().await?;
    Ok(stats)
}

/// "beef-ragu-2" → "beef-ragu".
fn strip_number(slug: &str) -> &str {
    match slug.rsplit_once('-') {
        Some((head, tail))
            if !head.is_empty() && !tail.is_empty() && tail.bytes().all(|b| b.is_ascii_digit()) =>
        {
            head
        }
        _ => slug,
    }
}
