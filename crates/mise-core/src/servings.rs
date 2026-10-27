//! Ingredient amounts at any serving count. A published yield is used exactly; any other is interpolated
//! between the nearest published yields on either side, or scaled in proportion to the nearest one when the
//! target lies outside them. Published amounts are not always linear (0.25 oz dill for both 2 and 4 servings),
//! which interpolation keeps.

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

#[cfg(test)]
mod tests {
    use super::*;

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
