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
    fn nearest_prefers_larger_on_ties() {
        assert_eq!(nearest(&[2.0, 4.0], 3.0), Some(4.0));
        assert_eq!(nearest(&[2.0, 4.0], 1.0), Some(2.0));
    }
}
