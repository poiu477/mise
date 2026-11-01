//! Ingredient amounts at any serving count. A published yield is used exactly; any other is interpolated
//! between the nearest published yields on either side, or scaled in proportion to the nearest one when the
//! target lies outside them. Published amounts are not always linear (0.25 oz dill for both 2 and 4 servings),
//! which interpolation keeps.

use crate::quantity::{PINCH_TSP, Quantity, Unit, format_amount, parse_number};

/// What a food's bare counts mean: sources often publish only "1 unit", which reads here as "1 packet (1½ oz)".
#[derive(Debug, Clone, PartialEq)]
pub struct CountUnit {
    pub noun: String,
    /// How much one is, where the source publishes the same food both ways.
    pub size: Option<Quantity>,
}

impl CountUnit {
    /// From a food's `count_unit` and `count_size` ("1.5 oz") properties.
    pub fn from_props(noun: Option<&str>, size: Option<&str>) -> Option<CountUnit> {
        let size = size.and_then(|s| {
            let (a, used) = parse_number(s)?;
            Some(Quantity::new(a, Unit::parse(&s[used..])?))
        });
        Some(CountUnit { noun: noun?.to_string(), size })
    }
}

/// The amount at `target` from `(servings, amount)` pairs, and whether it was published.
pub fn amount_at(published: &[(f64, f64)], target: f64) -> Option<(f64, bool)> {
    if let Some(&(_, a)) = published.iter().find(|(s, _)| (s - target).abs() < 1e-9) {
        return Some((a, true));
    }
    let below = published.iter().filter(|(s, _)| *s < target).max_by(|a, b| a.0.total_cmp(&b.0));
    let above = published.iter().filter(|(s, _)| *s > target).min_by(|a, b| a.0.total_cmp(&b.0));
    match (below, above) {
        (Some(&(s0, a0)), Some(&(s1, a1))) => Some((a0 + (a1 - a0) * (target - s0) / (s1 - s0), false)),
        (Some(&(s, a)), None) | (None, Some(&(s, a))) if s > 0.0 => Some((a * target / s, false)),
        _ => None,
    }
}

/// Published amounts that make sense together: an amount never shrinks as the serving count grows. Some sources
/// publish extra yields by formula (HelloFresh's 3 and 6 servings are ¾ and 1½ times the 4-serving amount, even
/// where 2 and 4 servings share one amount, so 3 servings would get less than 2). Yields are accepted in order of
/// trust (`trusted` first, as given, then the rest from small to large); one that would break the order is left
/// out, to be derived from the others.
pub fn consistent(published: &[(f64, f64)], trusted: &[f64]) -> Vec<(f64, f64)> {
    let mut order: Vec<(f64, f64)> = Vec::with_capacity(published.len());
    for t in trusted {
        if let Some(p) = published.iter().find(|(s, _)| (s - t).abs() < 1e-9) {
            if !order.iter().any(|q| (q.0 - p.0).abs() < 1e-9) {
                order.push(*p);
            }
        }
    }
    let mut rest: Vec<(f64, f64)> = published.iter().filter(|p| !order.iter().any(|q| (q.0 - p.0).abs() < 1e-9)).copied().collect();
    rest.sort_by(|a, b| a.0.total_cmp(&b.0));
    order.extend(rest);
    let mut kept: Vec<(f64, f64)> = Vec::new();
    for (s, a) in order {
        let fits = kept.iter().all(|&(ks, ka)| if ks < s { ka <= a + 1e-9 } else { ka + 1e-9 >= a });
        if fits {
            kept.push((s, a));
        }
    }
    kept.sort_by(|a, b| a.0.total_cmp(&b.0));
    kept
}

/// [`consistent`] for an ingredient line's published amounts as stored, `(servings, amount, unit)`: amounts in
/// the unit of a trusted yield are checked against each other; ones that would shrink as servings grow are
/// dropped, to be derived instead.
pub fn keep_consistent(list: &mut Vec<(f64, f64, Option<String>)>, trusted: &[f64]) {
    let anchor = list.iter().find(|p| trusted.iter().any(|t| (p.0 - t).abs() < 1e-9)).or(list.first());
    let Some(unit) = anchor.map(|p| p.2.clone()) else { return };
    let same: Vec<(f64, f64)> = list.iter().filter(|p| p.2 == unit).map(|p| (p.0, p.1)).collect();
    let kept = consistent(&same, trusted);
    list.retain(|p| p.2 != unit || kept.iter().any(|k| (k.0 - p.0).abs() < 1e-9));
}

/// The published yield a derived amount starts from: the nearest one, ties to the larger.
pub fn nearest(published: &[f64], target: f64) -> Option<f64> {
    published.iter().copied().min_by(|a, b| (a - target).abs().total_cmp(&(b - target).abs()).then(b.total_cmp(a)))
}

/// An ingredient line's amount at `target`: as published when it is (in its clearest words), else derived from
/// the published yields in the unit of the nearest one and rounded to kitchen amounts, or counted in pinches
/// when that is how the nearest one reads. A bare count takes the food's `count` wording.
pub fn line_text(published: &[(f64, f64, Option<String>)], target: f64, count: Option<&CountUnit>) -> (Option<String>, bool) {
    let yields: Vec<f64> = published.iter().map(|p| p.0).collect();
    let near = nearest(&yields, target);
    let in_pinches = near
        .and_then(|n| published.iter().find(|p| p.0 == n))
        .and_then(|p| Some(Quantity::new(p.1, Unit::parse(p.2.as_deref()?)?).tidied().unit == Unit::Pinch))
        .unwrap_or(false);
    let show = |a: f64, unit: &Option<String>, round: bool| match unit.as_deref().and_then(Unit::parse) {
        Some(u) => {
            let counted = count.filter(|_| u == Unit::Count(String::new()));
            let q = Quantity::new(a, counted.map_or(u, |c| Unit::Count(c.noun.clone())));
            let shown = if round && !in_pinches { q.rounded() } else { q.tidied() };
            match counted.and_then(|c| c.size.as_ref()) {
                Some(size) => format!("{shown} ({})", size.scaled(shown.amount).tidied()),
                None => shown.to_string(),
            }
        }
        None => format!("{} {}", format_amount(a), unit.as_deref().unwrap_or("")).trim().to_string(),
    };
    if let Some((_, a, u)) = published.iter().find(|(s, _, _)| (s - target).abs() < 1e-9) {
        return (Some(show(*a, u, false)), false);
    }
    let Some(near) = near else { return (None, false) };
    let unit = published.iter().find(|p| p.0 == near).and_then(|p| p.2.clone());
    let same_unit: Vec<(f64, f64)> = published.iter().filter(|p| p.2 == unit).map(|p| (p.0, p.1)).collect();
    let Some((a, _)) = amount_at(&same_unit, target) else { return (None, false) };
    // Rounded, it must still sit between the published amounts on either side; when it would not, it reads as
    // that neighbour (13.76 oz published for 2 and 4 is 13¾ oz at 3, not a rounded-up 14 oz).
    if let Some(u) = unit.as_deref().and_then(Unit::parse) {
        let r = match Quantity::new(a, u.clone()).in_unit(&Unit::Tsp).filter(|tsp| in_pinches && *tsp > 0.0) {
            // Whole pinches, back in the line's unit.
            Some(tsp) => a * (tsp / PINCH_TSP).round().max(1.0) * PINCH_TSP / tsp,
            None => Quantity::new(a, u).rounded().in_unit(&Unit::parse(unit.as_deref().unwrap_or("")).unwrap()).unwrap_or(a),
        };
        let below = same_unit.iter().filter(|p| p.0 < target).max_by(|x, y| x.0.total_cmp(&y.0));
        let above = same_unit.iter().filter(|p| p.0 > target).min_by(|x, y| x.0.total_cmp(&y.0));
        if let Some(&(_, b)) = below.filter(|p| r < p.1 - 1e-9) {
            return (Some(show(b, &unit, false)), true);
        }
        if let Some(&(_, b)) = above.filter(|p| r > p.1 + 1e-9) {
            return (Some(show(b, &unit, false)), true);
        }
    }
    (Some(show(a, &unit, true)), true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_counts_read_in_the_foods_own_words() {
        let paste = CountUnit::from_props(Some("packet"), Some("1.5 oz"));
        let list = vec![(2.0, 1.0, Some("unit".into())), (4.0, 2.0, Some("unit".into()))];
        assert_eq!(line_text(&list, 2.0, paste.as_ref()), (Some("1 packet (1½ oz)".into()), false));
        assert_eq!(line_text(&list, 3.0, paste.as_ref()), (Some("1½ packets (2¼ oz)".into()), true));
        assert_eq!(line_text(&list, 4.0, paste.as_ref()), (Some("2 packets (3 oz)".into()), false));

        let picks = vec![(2.0, 2.0, Some("pick".into()))];
        let scallion = CountUnit::from_props(Some("stalk"), None);
        assert_eq!(line_text(&picks, 2.0, scallion.as_ref()), (Some("2 stalks".into()), false));

        // A real unit stays as published, and a food with no wording reads as a bare number.
        let ounces = vec![(2.0, 4.0, Some("ounce".into()))];
        assert_eq!(line_text(&ounces, 2.0, paste.as_ref()), (Some("4 oz".into()), false));
        assert_eq!(line_text(&list, 2.0, None), (Some("1".into()), false));
    }

    #[test]
    fn published_amounts_are_exact_and_others_rounded() {
        let a = vec![(2.0, 0.5, Some("cup".into())), (4.0, 1.0, Some("cup".into()))];
        assert_eq!(line_text(&a, 2.0, None), (Some("½ cup".into()), false));
        assert_eq!(line_text(&a, 3.0, None), (Some("¾ cup".into()), true));
        assert_eq!(line_text(&a, 6.0, None), (Some("1½ cups".into()), true));
        let counts = vec![(2.0, 1.0, Some("unit".into())), (4.0, 2.0, Some("unit".into()))];
        assert_eq!(line_text(&counts, 5.0, None), (Some("2½".into()), true));
        // Rounding never passes a published neighbour.
        let can = vec![(2.0, 13.76, Some("ounce".into())), (4.0, 13.76, Some("ounce".into()))];
        assert_eq!(line_text(&can, 3.0, None), (Some("13¾ oz".into()), true));
        // Published amounts read in their clearest words, and pinches stay pinches between them.
        let oil = vec![(2.0, 3.0, Some("teaspoon (tsp)".into())), (4.0, 3.0, Some("teaspoon (tsp)".into())), (6.0, 6.0, Some("teaspoon (tsp)".into()))];
        assert_eq!(line_text(&oil, 2.0, None), (Some("1 TBSP".into()), false));
        assert_eq!(line_text(&oil, 3.0, None), (Some("1 TBSP".into()), true));
        assert_eq!(line_text(&oil, 6.0, None), (Some("2 TBSP".into()), false));
        let sugar = vec![(2.0, 0.06, Some("teaspoon (tsp)".into())), (4.0, 0.12, Some("teaspoon (tsp)".into())), (6.0, 0.18, Some("teaspoon (tsp)".into()))];
        let at = |t: f64| line_text(&sugar, t, None).0.unwrap();
        assert_eq!([at(1.0), at(2.0), at(3.0), at(4.0), at(5.0), at(6.0)], ["1 pinch", "1 pinch", "1 pinch", "2 pinches", "2 pinches", "3 pinches"]);
        let odd = vec![(2.0, 4.0, Some("nach Geschmack".into()))];
        assert_eq!(line_text(&odd, 2.0, None), (Some("4 nach Geschmack".into()), false));
    }

    #[test]
    fn published_yields_are_exact() {
        assert_eq!(amount_at(&[(2.0, 10.0), (4.0, 20.0)], 4.0), Some((20.0, true)));
    }

    #[test]
    fn interpolates_between_and_scales_outside() {
        assert_eq!(amount_at(&[(2.0, 10.0), (4.0, 20.0)], 3.0), Some((15.0, false)));
        assert_eq!(amount_at(&[(2.0, 0.25), (4.0, 0.25)], 3.0), Some((0.25, false)));
        assert_eq!(amount_at(&[(2.0, 10.0), (4.0, 20.0)], 6.0), Some((30.0, false)));
        assert_eq!(amount_at(&[(2.0, 10.0), (4.0, 20.0)], 1.0), Some((5.0, false)));
        assert_eq!(amount_at(&[], 3.0), None);
    }

    #[test]
    fn amounts_never_shrink_as_servings_grow() {
        // HelloFresh: 1 lemon for 2 and for 4, but ¾ for 3 (¾ of the 4-serving amount).
        let lemon = [(2.0, 1.0), (3.0, 0.75), (4.0, 1.0), (6.0, 2.0)];
        let kept = consistent(&lemon, &[2.0, 4.0]);
        assert_eq!(kept, vec![(2.0, 1.0), (4.0, 1.0), (6.0, 2.0)]);
        assert_eq!(amount_at(&kept, 3.0), Some((1.0, false)));
        // A normal line keeps everything.
        let carrots = [(2.0, 12.0), (3.0, 18.0), (4.0, 24.0), (6.0, 36.0)];
        assert_eq!(consistent(&carrots, &[2.0, 4.0]), carrots.to_vec());
        // A 6-serving amount below the 4-serving one goes.
        assert_eq!(consistent(&[(2.0, 1.0), (4.0, 2.0), (6.0, 1.5)], &[2.0, 4.0]), vec![(2.0, 1.0), (4.0, 2.0)]);
    }

    #[test]
    fn nearest_prefers_larger_on_ties() {
        assert_eq!(nearest(&[2.0, 4.0], 3.0), Some(4.0));
        assert_eq!(nearest(&[2.0, 4.0], 1.0), Some(2.0));
    }
}
