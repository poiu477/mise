//! The shopping list: what several recipes need of one food, added up and worded for buying.

use crate::quantity::{Family, PINCH_TSP, Quantity, Unit, decimal, format_exact};
use crate::servings::CountUnit;

/// Store sections in the order a shop is walked. Foods with none go under `OTHER`.
pub const SECTIONS: &[&str] = &[
    "Produce",
    "Meat & Seafood",
    "Dairy & Eggs",
    "Bakery",
    "Pasta & Grains",
    "Canned & Jarred",
    "Sauces & Condiments",
    "Spices & Seasonings",
    "Baking",
    "Snacks & Nuts",
];
pub const OTHER: &str = "Other";
/// Pantry lines the recipes expect you to have: last, to check rather than buy.
pub const CHECK: &str = "Check you have";

/// Where a section sorts in the shop.
pub fn section_order(name: &str) -> usize {
    match name {
        OTHER => SECTIONS.len(),
        CHECK => SECTIONS.len() + 1,
        _ => SECTIONS.iter().position(|s| *s == name).unwrap_or(SECTIONS.len()),
    }
}

/// How a household reads weights on its shopping list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WeightUnit {
    /// "1 lb 14 oz", and ounces under a pound.
    #[default]
    LbOz,
    Oz,
    Lb,
    /// Grams, and kilograms from one.
    Metric,
}

impl WeightUnit {
    pub const ALL: [WeightUnit; 4] = [WeightUnit::LbOz, WeightUnit::Oz, WeightUnit::Lb, WeightUnit::Metric];

    pub fn as_str(self) -> &'static str {
        match self {
            WeightUnit::LbOz => "lb_oz",
            WeightUnit::Oz => "oz",
            WeightUnit::Lb => "lb",
            WeightUnit::Metric => "metric",
        }
    }

    pub fn parse(s: &str) -> Option<WeightUnit> {
        WeightUnit::ALL.into_iter().find(|w| w.as_str() == s)
    }

    /// For a package's size: pounds read as pounds and ounces, so a 1 oz packet isn't "0.06 lb".
    fn for_size(self) -> WeightUnit {
        if self == WeightUnit::Lb { WeightUnit::LbOz } else { self }
    }

    /// The weight in these units, exactly.
    pub fn show(self, grams: f64) -> String {
        let oz = grams / Quantity::new(1.0, Unit::Oz).in_unit(&Unit::G).unwrap_or(1.0);
        match self {
            WeightUnit::Oz => Quantity::new(oz, Unit::Oz).exact(),
            WeightUnit::Lb => Quantity::new(oz / 16.0, Unit::Lb).exact(),
            WeightUnit::LbOz => {
                let lb = ((oz + 0.005) / 16.0).floor();
                let rest = oz - lb * 16.0;
                match (lb > 0.0, rest >= 0.005) {
                    (false, _) => Quantity::new(oz, Unit::Oz).exact(),
                    (true, false) => format!("{lb} lb"),
                    (true, true) => format!("{lb} lb {}", Quantity::new(rest, Unit::Oz).exact()),
                }
            }
            WeightUnit::Metric if grams >= 1000.0 => format!("{} kg", decimal(grams / 1000.0)),
            WeightUnit::Metric => format!("{} g", grams.round().max(1.0)),
        }
    }
}

/// The total of `needs` in words, exactly: volumes and weights added up (weights in `weights`), counts as they
/// come, and counted items folded into a measured amount where their size is known. Amounts that don't convert
/// are joined with "+".
pub fn total(needs: &[Quantity], count: Option<&CountUnit>, weights: WeightUnit) -> Option<String> {
    let (mut tsp, mut grams) = (0.0, 0.0);
    let mut counted: Vec<(String, f64)> = Vec::new();
    for q in needs {
        match q.unit.family() {
            Family::Volume => tsp += q.in_unit(&Unit::Tsp).unwrap_or(0.0),
            Family::Pinch => tsp += q.amount * PINCH_TSP,
            Family::Weight => grams += q.in_unit(&Unit::G).unwrap_or(0.0),
            Family::Count => {
                let Unit::Count(noun) = &q.unit else { continue };
                let noun = if noun.is_empty() { count.map_or("", |c| c.noun.as_str()) } else { noun };
                match counted.iter_mut().find(|(n, _)| n == noun) {
                    Some((_, a)) => *a += q.amount,
                    None => counted.push((noun.to_string(), q.amount)),
                }
            }
        }
    }
    if let (Some(c), Some(size)) = (count, count.and_then(|c| c.size.as_ref())) {
        let measured = match size.unit.family() {
            Family::Volume => tsp > 0.0,
            Family::Weight => grams > 0.0,
            _ => false,
        };
        if let (true, Some(i)) = (measured, counted.iter().position(|(n, _)| *n == c.noun)) {
            let (_, n) = counted.remove(i);
            match size.unit.family() {
                Family::Volume => tsp += size.scaled(n).in_unit(&Unit::Tsp).unwrap_or(0.0),
                _ => grams += size.scaled(n).in_unit(&Unit::G).unwrap_or(0.0),
            }
        }
    }

    let mut parts = Vec::new();
    if grams > 0.0 {
        parts.push(weights.show(grams));
    }
    if tsp > 0.0 {
        parts.push(volume(tsp).exact());
    }
    for (noun, n) in counted {
        let q = Quantity::new(n, Unit::Count(noun.clone()));
        parts.push(match count.filter(|c| c.noun == noun).and_then(|c| c.size.as_ref()) {
            Some(size) if size.unit.family() == Family::Weight => {
                format!("{} ({})", q.exact(), weights.for_size().show(size.scaled(n).in_unit(&Unit::G).unwrap_or(0.0)))
            }
            Some(size) => format!("{} ({})", q.exact(), size.scaled(n).exact()),
            None => q.exact(),
        });
    }
    (!parts.is_empty()).then(|| parts.join(" + "))
}

/// A volume in the largest unit it is a kitchen amount of (5 TBSP, not 0.31 cup), else the largest it reaches.
fn volume(tsp: f64) -> Quantity {
    let units: Vec<Quantity> = [(12.0, Unit::Cup), (3.0, Unit::Tbsp), (0.0, Unit::Tsp)]
        .into_iter()
        .filter(|(from, _)| tsp >= *from)
        .map(|(_, u)| Quantity::new(Quantity::new(tsp, Unit::Tsp).in_unit(&u).unwrap_or(tsp), u))
        .collect();
    units.iter().find(|q| !format_exact(q.amount).contains('.')).unwrap_or(&units[0]).clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(a: f64, u: Unit) -> Quantity {
        Quantity::new(a, u)
    }
    fn bare(a: f64) -> Quantity {
        q(a, Unit::Count(String::new()))
    }

    const W: WeightUnit = WeightUnit::LbOz;

    #[test]
    fn adds_up_exactly_within_a_kind_of_measure() {
        assert_eq!(total(&[q(1.0, Unit::Tsp), q(2.0, Unit::Tsp)], None, W).unwrap(), "1 TBSP");
        assert_eq!(total(&[q(2.0, Unit::Tbsp), q(2.0, Unit::Tbsp)], None, W).unwrap(), "¼ cup");
        assert_eq!(total(&[q(1.0, Unit::Cup), q(0.3, Unit::Cup)], None, W).unwrap(), "1.3 cups");
        assert_eq!(total(&[q(3.0, Unit::Tbsp), q(2.0, Unit::Tbsp)], None, W).unwrap(), "5 TBSP");
        assert_eq!(total(&[q(1.0, Unit::Pinch), q(1.0, Unit::Pinch)], None, W).unwrap(), "⅛ tsp");
        assert_eq!(total(&[q(8.0, Unit::Oz), q(4.0, Unit::Oz)], None, W).unwrap(), "12 oz");
    }

    #[test]
    fn weights_read_in_the_households_units() {
        let beef = [q(10.0, Unit::Oz), q(10.0, Unit::Oz), q(10.0, Unit::Oz)];
        assert_eq!(total(&beef, None, WeightUnit::LbOz).unwrap(), "1 lb 14 oz");
        assert_eq!(total(&beef, None, WeightUnit::Oz).unwrap(), "30 oz");
        assert_eq!(total(&beef, None, WeightUnit::Lb).unwrap(), "1⅞ lb");
        assert_eq!(total(&beef, None, WeightUnit::Metric).unwrap(), "850 g");
        assert_eq!(total(&[q(16.0, Unit::Oz), q(16.0, Unit::Oz)], None, WeightUnit::LbOz).unwrap(), "2 lb");
        assert_eq!(total(&[q(1.0, Unit::Lb), q(0.3, Unit::Oz)], None, WeightUnit::LbOz).unwrap(), "1 lb 0.3 oz");
        assert_eq!(total(&[q(2.5, Unit::Lb)], None, WeightUnit::Metric).unwrap(), "1.13 kg");
        assert_eq!(WeightUnit::parse("lb_oz"), Some(WeightUnit::LbOz));
        assert_eq!(WeightUnit::parse("stone"), None);
    }

    #[test]
    fn counts_stay_as_they_are() {
        let lemon = CountUnit::from_props(Some("whole"), None);
        assert_eq!(total(&[bare(0.5), bare(1.0)], lemon.as_ref(), W).unwrap(), "1½ whole");
        assert_eq!(total(&[bare(0.3)], lemon.as_ref(), W).unwrap(), "0.3 whole");
        assert_eq!(total(&[bare(3.0)], None, W).unwrap(), "3");
    }

    #[test]
    fn counts_carry_their_size() {
        let paste = CountUnit::from_props(Some("packet"), Some("1.5 oz"));
        assert_eq!(total(&[bare(1.0), bare(2.0)], paste.as_ref(), W).unwrap(), "3 packets (4½ oz)");
        assert_eq!(total(&[bare(1.0), bare(2.0)], paste.as_ref(), WeightUnit::Metric).unwrap(), "3 packets (128 g)");
        assert_eq!(total(&[bare(1.0), bare(2.0)], paste.as_ref(), WeightUnit::Lb).unwrap(), "3 packets (4½ oz)");
    }

    #[test]
    fn counted_items_fold_into_a_measure_of_the_same_kind() {
        let paste = CountUnit::from_props(Some("packet"), Some("1.5 oz"));
        assert_eq!(total(&[bare(1.0), q(1.5, Unit::Oz)], paste.as_ref(), W).unwrap(), "3 oz");
        let ketchup = CountUnit::from_props(Some("packet"), Some("2 TBSP"));
        assert_eq!(total(&[bare(1.0), q(1.0, Unit::Tbsp)], ketchup.as_ref(), W).unwrap(), "3 TBSP");
    }

    #[test]
    fn what_will_not_convert_is_joined() {
        let garlic = q(2.0, Unit::Count("clove".into()));
        assert_eq!(total(&[garlic, q(1.0, Unit::Tbsp)], None, W).unwrap(), "1 TBSP + 2 cloves");
        let lettuce = CountUnit::from_props(Some("head"), None);
        assert_eq!(total(&[bare(1.0), q(4.0, Unit::Oz)], lettuce.as_ref(), W).unwrap(), "4 oz + 1 head");
    }

    #[test]
    fn nothing_to_add_up_has_no_amount() {
        assert_eq!(total(&[], None, W), None);
    }

    #[test]
    fn sections_sort_as_the_shop_is_walked() {
        assert!(section_order("Produce") < section_order("Baking"));
        assert!(section_order("Baking") < section_order(OTHER));
        assert!(section_order(OTHER) < section_order(CHECK));
        assert_eq!(section_order("Somewhere new"), section_order(OTHER));
    }
}
