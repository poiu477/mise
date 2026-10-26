//! Which protein an ingredient is, for the "Protein" option of a dish (Chicken / Pork / Shrimp ...).

use crate::text::normalize_name;

/// Ingredients that mention a protein but are not one: stocks, sauces, seasonings.
const NOT_A_PROTEIN: &[&str] = &[
    "stock", "broth", "concentrate", "demi", "glace", "base", "bouillon", "seasoning", "sauce", "gravy", "powder",
    "flavor", "flavour", "spice", "blend", "rub", "fat", "dressing", "soup", "ramen",
];

/// Checked in order: the first match wins, so "pork tenderloin" is Pork before "tenderloin" can mean Beef, and
/// "chicken sausage" is Chicken.
const PROTEINS: &[(&[&str], &str)] = &[
    (&["impossible", "beyond", "plant based", "plant-based", "vegan"], "Plant-based"),
    (&["chicken"], "Chicken"),
    (&["turkey"], "Turkey"),
    (&["pork", "bacon", "chorizo", "prosciutto", "pancetta", "ham", "bratwurst"], "Pork"),
    (&["beef", "steak", "sirloin", "ribeye", "rib eye", "bavette", "tenderloin", "brisket", "meatball"], "Beef"),
    (&["lamb"], "Lamb"),
    (&["duck"], "Duck"),
    (&["shrimp", "prawn"], "Shrimp"),
    (&["salmon"], "Salmon"),
    (&["scallop"], "Scallops"),
    (&["tilapia", "cod", "barramundi", "trout", "steelhead", "sea bass", "mahi", "haddock", "fish", "pollock", "swordfish"], "Fish"),
    (&["tofu"], "Tofu"),
    (&["sausage"], "Pork"),
];

pub fn protein_of(food_name: &str) -> Option<&'static str> {
    let name = format!(" {} ", normalize_name(food_name));
    if NOT_A_PROTEIN.iter().any(|w| name.contains(&format!(" {w} ")) || name.contains(&format!(" {w}s "))) {
        return None;
    }
    PROTEINS.iter().find_map(|(words, label)| {
        words.iter().any(|w| name.contains(&format!(" {w} ")) || name.contains(&format!(" {w}s "))).then_some(*label)
    })
}

/// An amount converted to ounces, when the unit allows it.
pub fn ounces(amount: f64, unit: &str) -> Option<f64> {
    match normalize_name(unit).as_str() {
        "ounce" | "ounces" | "oz" => Some(amount),
        "pound" | "pounds" | "lb" | "lbs" => Some(amount * 16.0),
        "gram" | "grams" | "g" => Some(amount / 28.35),
        "kilogram" | "kilograms" | "kg" => Some(amount * 35.27),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proteins() {
        assert_eq!(protein_of("Chopped Chicken Breast"), Some("Chicken"));
        assert_eq!(protein_of("Pork Tenderloin"), Some("Pork"));
        assert_eq!(protein_of("Beef Tenderloin Filets"), Some("Beef"));
        assert_eq!(protein_of("Italian Chicken Sausage Mix"), Some("Chicken"));
        assert_eq!(protein_of("Italian Pork Sausage"), Some("Pork"));
        assert_eq!(protein_of("Shrimp"), Some("Shrimp"));
        assert_eq!(protein_of("Barramundi"), Some("Fish"));
        assert_eq!(protein_of("Impossible Burger Patties"), Some("Plant-based"));
        assert_eq!(protein_of("Bavette Steak"), Some("Beef"));
    }

    #[test]
    fn not_proteins() {
        assert_eq!(protein_of("Chicken Stock Concentrate"), None);
        assert_eq!(protein_of("Beef Demi-Glace"), None);
        assert_eq!(protein_of("Pork Ramen Stock Concentrate"), None);
        assert_eq!(protein_of("Zucchini"), None);
        assert_eq!(protein_of("Hamburger Buns"), None);
    }

    #[test]
    fn ounce_conversion() {
        assert_eq!(ounces(10.0, "ounce"), Some(10.0));
        assert_eq!(ounces(1.0, "pound"), Some(16.0));
        assert_eq!(ounces(2.0, "unit"), None);
    }
}
