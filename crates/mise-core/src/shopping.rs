//! The shopping list: what several recipes need of one food, added up and worded for buying.

use crate::quantity::{Family, PINCH_TSP, Quantity, Unit};
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

/// The total of `needs` in words: volumes and weights added up, counts rounded up to whole items, and counted items
/// folded into a measured amount where their size is known. Amounts that don't convert are joined with "+".
pub fn total(needs: &[Quantity], count: Option<&CountUnit>) -> Option<String> {
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
        parts.push(weight(grams).to_string());
    }
    if tsp > 0.0 {
        parts.push(volume(tsp).to_string());
    }
    for (noun, n) in counted {
        // Nobody buys half a zucchini; the small slip guards against 2.0000001 reading as 3.
        let whole = Quantity::new((n - 1e-6).ceil().max(1.0), Unit::Count(noun.clone()));
        parts.push(match count.filter(|c| c.noun == noun) {
            Some(c) => c.show(&whole),
            None => whole.to_string(),
        });
    }
    (!parts.is_empty()).then(|| parts.join(" + "))
}

fn weight(grams: f64) -> Quantity {
    let oz = grams / Quantity::new(1.0, Unit::Oz).in_unit(&Unit::G).unwrap_or(1.0);
    if oz >= 16.0 { Quantity::new(oz / 16.0, Unit::Lb).rounded() } else { Quantity::new(oz, Unit::Oz).rounded() }
}

fn volume(tsp: f64) -> Quantity {
    if tsp >= 12.0 {
        Quantity::new(tsp / 48.0, Unit::Cup).rounded()
    } else if tsp >= 3.0 {
        Quantity::new(tsp / 3.0, Unit::Tbsp).rounded()
    } else {
        Quantity::new(tsp, Unit::Tsp).rounded()
    }
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

    #[test]
    fn adds_up_within_a_kind_of_measure() {
        assert_eq!(total(&[q(1.0, Unit::Tsp), q(2.0, Unit::Tsp)], None).unwrap(), "1 TBSP");
        assert_eq!(total(&[q(2.0, Unit::Tbsp), q(2.0, Unit::Tbsp)], None).unwrap(), "¼ cup");
        assert_eq!(total(&[q(8.0, Unit::Oz), q(4.0, Unit::Oz)], None).unwrap(), "12 oz");
        assert_eq!(total(&[q(10.0, Unit::Oz), q(10.0, Unit::Oz)], None).unwrap(), "1¼ lb");
        assert_eq!(total(&[q(1.0, Unit::Pinch), q(1.0, Unit::Pinch)], None).unwrap(), "⅛ tsp");
    }

    #[test]
    fn counts_are_bought_whole() {
        let lemon = CountUnit::from_props(Some("whole"), None);
        assert_eq!(total(&[bare(0.5), bare(1.0)], lemon.as_ref()).unwrap(), "2 whole");
        assert_eq!(total(&[bare(0.25)], lemon.as_ref()).unwrap(), "1 whole");
        assert_eq!(total(&[bare(1.0), bare(1.0)], lemon.as_ref()).unwrap(), "2 whole");
        assert_eq!(total(&[bare(3.0)], None).unwrap(), "3");
    }

    #[test]
    fn counts_carry_their_size() {
        let paste = CountUnit::from_props(Some("packet"), Some("1.5 oz"));
        assert_eq!(total(&[bare(1.0), bare(2.0)], paste.as_ref()).unwrap(), "3 packets (4½ oz)");
    }

    #[test]
    fn counted_items_fold_into_a_measure_of_the_same_kind() {
        let paste = CountUnit::from_props(Some("packet"), Some("1.5 oz"));
        assert_eq!(total(&[bare(1.0), q(1.5, Unit::Oz)], paste.as_ref()).unwrap(), "3 oz");
        let ketchup = CountUnit::from_props(Some("packet"), Some("2 TBSP"));
        assert_eq!(total(&[bare(1.0), q(1.0, Unit::Tbsp)], ketchup.as_ref()).unwrap(), "3 TBSP");
    }

    #[test]
    fn what_will_not_convert_is_joined() {
        let garlic = q(2.0, Unit::Count("clove".into()));
        assert_eq!(total(&[garlic, q(1.0, Unit::Tbsp)], None).unwrap(), "1 TBSP + 2 cloves");
        let lettuce = CountUnit::from_props(Some("head"), None);
        assert_eq!(total(&[bare(1.0), q(4.0, Unit::Oz)], lettuce.as_ref()).unwrap(), "4 oz + 1 head");
    }

    #[test]
    fn nothing_to_add_up_has_no_amount() {
        assert_eq!(total(&[], None), None);
    }

    #[test]
    fn sections_sort_as_the_shop_is_walked() {
        assert!(section_order("Produce") < section_order("Baking"));
        assert!(section_order("Baking") < section_order(OTHER));
        assert!(section_order(OTHER) < section_order(CHECK));
        assert_eq!(section_order("Somewhere new"), section_order(OTHER));
    }
}
