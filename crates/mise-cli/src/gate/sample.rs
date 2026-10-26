//! Stratified, seeded sample of dishes for the hand check.
//!
//! A uniform sample would be mostly single-version dishes, where grouping can hardly go wrong. Instead each
//! stratum (a kind of risk) gets a share of the sample. Strata are taken in priority order and a dish lands in
//! the first one it qualifies for, so the sample has no repeats; a stratum with too few dishes passes its
//! shortfall on to the others. Within a stratum dishes are ordered by a hash of the seed and the dish id, so a
//! given seed always picks the same dishes from the same data.

use uuid::Uuid;

/// What the sampler knows about each dish.
#[derive(Debug, Clone, Default)]
pub struct Features {
    pub id: Uuid,
    pub content_type: String,
    pub versions: usize,
    /// Distinct cleaned titles among its versions.
    pub titles: usize,
    pub protein_choice: bool,
    pub merge_review: bool,
    /// Other dishes holding versions copied from this one's, or that this one's were copied from.
    pub lineage_links: usize,
    /// Other dishes with the same cleaned title.
    pub title_twins: usize,
    /// Other dishes with the same title apart from the protein whose default versions ship much the same.
    pub protein_twins: usize,
}

pub struct Stratum {
    pub key: &'static str,
    pub label: &'static str,
    pub about: &'static str,
    /// Share of the sample, relative to the other weights.
    pub weight: u32,
    pub test: fn(&Features) -> bool,
}

pub const STRATA: &[Stratum] = &[
    Stratum {
        key: "merge_review",
        label: "Open merge review",
        about: "Involved in an open dish_merge review item: a copy that grouping kept apart from its source.",
        weight: 15,
        test: |f| f.merge_review,
    },
    Stratum {
        key: "lineage",
        label: "Shared lineage",
        about: "Shares a lineage with another dish: split from drift, or a copy reused for a different dish.",
        weight: 12,
        test: |f| f.lineage_links > 0,
    },
    Stratum {
        key: "protein_twin",
        label: "Protein twin",
        about: "Another dish has the same title apart from the protein (or 2x/format words) and its default version \
                ships at least half the same foods: a possible missed merge.",
        weight: 8,
        test: |f| f.protein_twins > 0,
    },
    Stratum {
        key: "title_twin",
        label: "Title twin",
        about: "Another dish has the same cleaned title (its subtitle or content type differs).",
        weight: 6,
        test: |f| f.title_twins > 0,
    },
    Stratum {
        key: "many_titles",
        label: "Many titles",
        about: "Three or more distinct titles among its versions.",
        weight: 10,
        test: |f| f.titles >= 3,
    },
    Stratum { key: "large", label: "Large", about: "25 or more versions.", weight: 9, test: |f| f.versions >= 25 },
    Stratum {
        key: "protein",
        label: "Protein choice",
        about: "Offers a choice of protein.",
        weight: 9,
        test: |f| f.protein_choice,
    },
    Stratum {
        key: "not_recipe",
        label: "Ready meal / product",
        about: "Content type other than recipe.",
        weight: 9,
        test: |f| f.content_type != "recipe",
    },
    Stratum {
        key: "medium",
        label: "4–24 versions",
        about: "A mid-sized dish not caught above.",
        weight: 7,
        test: |f| (4..25).contains(&f.versions),
    },
    Stratum { key: "small", label: "2–3 versions", about: "A small dish.", weight: 7, test: |f| (2..4).contains(&f.versions) },
    Stratum { key: "single", label: "Single version", about: "One version only.", weight: 8, test: |f| f.versions == 1 },
];

/// One sampled dish: its index into the features slice and the stratum it was drawn for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pick {
    pub dish: usize,
    pub stratum: usize,
}

/// Draw up to `n` dishes. Picks come back grouped by stratum, in stratum order.
pub fn draw(features: &[Features], n: usize, seed: u64) -> Vec<Pick> {
    let total: u32 = STRATA.iter().map(|s| s.weight).sum();
    // Quotas by cumulative rounding, so they always add up to n.
    let mut quotas = Vec::with_capacity(STRATA.len());
    let mut cum = 0u32;
    for s in STRATA {
        let before = (n as u64 * cum as u64 + total as u64 / 2) / total as u64;
        cum += s.weight;
        let after = (n as u64 * cum as u64 + total as u64 / 2) / total as u64;
        quotas.push((after - before) as usize);
    }

    // Each stratum's candidates in seeded order.
    let candidates: Vec<Vec<usize>> = STRATA
        .iter()
        .map(|s| {
            let mut c: Vec<usize> = (0..features.len()).filter(|&i| (s.test)(&features[i])).collect();
            c.sort_by_key(|&i| (mix(seed, features[i].id), features[i].id));
            c
        })
        .collect();

    let mut taken = vec![false; features.len()];
    let mut picks: Vec<Vec<usize>> = vec![Vec::new(); STRATA.len()];
    let mut cursor = vec![0usize; STRATA.len()];
    let mut take = |s: usize, want: usize, taken: &mut Vec<bool>, picks: &mut Vec<Vec<usize>>| -> usize {
        let mut got = 0;
        while got < want && cursor[s] < candidates[s].len() {
            let i = candidates[s][cursor[s]];
            cursor[s] += 1;
            if !taken[i] {
                taken[i] = true;
                picks[s].push(i);
                got += 1;
            }
        }
        got
    };
    let mut shortfall = 0;
    for (s, &q) in quotas.iter().enumerate() {
        shortfall += q - take(s, q, &mut taken, &mut picks);
    }
    // Hand any shortfall to the strata that still have dishes, in priority order.
    for s in 0..STRATA.len() {
        if shortfall == 0 {
            break;
        }
        shortfall -= take(s, shortfall, &mut taken, &mut picks);
    }
    picks.into_iter().enumerate().flat_map(|(stratum, ds)| ds.into_iter().map(move |dish| Pick { dish, stratum })).collect()
}

/// Every stratum a dish qualifies for, not only the one it was drawn for.
pub fn tags(f: &Features) -> Vec<usize> {
    (0..STRATA.len()).filter(|&s| (STRATA[s].test)(f)).collect()
}

/// A well-mixed 64-bit hash of the seed and an id (splitmix64 finaliser over both halves).
fn mix(seed: u64, id: Uuid) -> u64 {
    let (hi, lo) = id.as_u64_pair();
    let mut x = seed ^ hi.rotate_left(17) ^ lo;
    for _ in 0..2 {
        x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
        x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        x ^= x >> 31;
    }
    x
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dishes() -> Vec<Features> {
        (0..2000u128)
            .map(|i| Features {
                id: Uuid::from_u128(i * 7919 + 1),
                content_type: if i % 13 == 0 { "product".into() } else { "recipe".into() },
                versions: [1, 1, 1, 2, 3, 5, 12, 30][i as usize % 8],
                titles: 1 + (i as usize % 5 == 0) as usize * 2,
                protein_choice: i % 9 == 0,
                merge_review: i % 50 == 0,
                lineage_links: (i % 37 == 0) as usize,
                title_twins: (i % 23 == 0) as usize,
                protein_twins: (i % 29 == 0) as usize,
            })
            .collect()
    }

    #[test]
    fn same_seed_same_sample_and_no_repeats() {
        let f = dishes();
        let a = draw(&f, 300, 7);
        assert_eq!(a, draw(&f, 300, 7));
        assert_ne!(a, draw(&f, 300, 8));
        assert_eq!(a.len(), 300);
        let mut ids: Vec<usize> = a.iter().map(|p| p.dish).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), 300);
        for p in &a {
            assert!((STRATA[p.stratum].test)(&f[p.dish]), "a dish is drawn only for a stratum it qualifies for");
        }
    }

    #[test]
    fn short_strata_pass_their_share_on() {
        let f = dishes();
        // Only 40 dishes have a merge review; the sample still reaches its size.
        let picks = draw(&f, 600, 1);
        assert_eq!(picks.len(), 600);
        assert_eq!(picks.iter().filter(|p| p.stratum == 0).count(), 40);
        // Asking for more than exists returns everything once.
        assert_eq!(draw(&f, 5000, 1).len(), 2000);
    }
}
