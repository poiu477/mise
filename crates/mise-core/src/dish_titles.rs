//! Dish titles: strip sources' internal working notes, and find the part of a title that stays the same when
//! only the protein changes ("Hot Honey Chicken" and "Hot Honey Pork Filet" share "hot honey").

use std::sync::LazyLock;

use regex::Regex;

use crate::text::normalize_name;

static BRACKETED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\[[^\]]*\]").unwrap());
static COMPLIANCE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)(?:\b[A-Z]{2}/[A-Z]{2}\s+only\b(?:\s+compliant\b)?|\bcompliant\b)").unwrap());
static NOTE_PARENS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\s*\([^)]*\b(?:sku|swap|only|compliant|test)\b[^)]*\)").unwrap());
static WEEK_CODE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*W\d+\s+R\d+[A-Z0-9]*\s+").unwrap());
static NOTE_SUFFIX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+--\s.*$").unwrap());
static CODE_PARENS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\s*\((?:[^)]*\d[^)]*|[A-Z0-9]{2,5})\)\s*$").unwrap());
static SERVINGS_SUFFIX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\s*\|\s*\d+(?:\s*[–-]\s*\d+)?\s+servings?\s*$").unwrap());
static SPACES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+").unwrap());

/// A title as a person should see it: internal notes, codes and compliance tags removed.
pub fn clean_title(title: &str) -> String {
    let mut t = BRACKETED.replace_all(title, " ").into_owned();
    t = COMPLIANCE.replace_all(&t, " ").into_owned();
    t = NOTE_PARENS.replace_all(&t, " ").into_owned();
    t = WEEK_CODE.replace(&t, "").into_owned();
    t = NOTE_SUFFIX.replace(&t, "").into_owned();
    t = SERVINGS_SUFFIX.replace(&t, "").into_owned();
    // Parenthetical codes can stack: "(Pork Chop 10oz) (D2V)".
    loop {
        let next = CODE_PARENS.replace(&t, "").into_owned();
        if next == t {
            break;
        }
        t = next;
    }
    SPACES.replace_all(t.trim(), " ").into_owned()
}

/// Proteins and format words that vary between versions of one dish ("2x", "Prep & Bake", "One-Pan").
const SWAPPABLE: &[&str] = &[
    "chicken", "pork", "beef", "steak", "shrimp", "salmon", "tilapia", "cod", "barramundi", "trout", "steelhead",
    "scallops", "turkey", "tofu", "sausage", "bacon", "lamb", "duck", "fish", "seafood", "plant", "based",
    "protein", "impossible", "beyond", "filet", "filets", "fillet", "fillets", "cutlet", "cutlets", "thigh",
    "thighs", "breast", "breasts", "tenderloin", "sirloin", "ribeye", "rib", "eye", "strip", "bavette", "ranch",
    "chop", "chops", "ground", "meatballs", "meatball", "patty", "patties", "burger", "burgers", "organic",
    "2x", "double", "triple", "prep", "bake", "one", "pan", "pot", "sheet", "and", "with", "over", "the", "a",
];

/// The title with proteins and format words removed: equal cores mean "same dish, different protein".
pub fn protein_core(title: &str) -> String {
    normalize_name(&clean_title(title))
        .split(' ')
        .filter(|w| !SWAPPABLE.contains(w))
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_working_notes() {
        assert_eq!(
            clean_title("[SWAP PORK CHOPS TO CHICKEN CUTLETS] MA/CA only Compliant Al Pastor Pork Chops with Mango Salsa (Pork Chop 10oz)"),
            "Al Pastor Pork Chops with Mango Salsa"
        );
        assert_eq!(clean_title("W46 R146 Peachy Keen BBQ-Rubbed Pork Chops"), "Peachy Keen BBQ-Rubbed Pork Chops");
        assert_eq!(clean_title("Beef tacos with pineapple salsa -- RED ONION SWAP"), "Beef tacos with pineapple salsa");
        assert_eq!(clean_title("Sweet and Smoky Chicken Cutlet with Cherry Sauce (D2V)"), "Sweet and Smoky Chicken Cutlet with Cherry Sauce");
        assert_eq!(clean_title("with Cream Cheese, Zucchini & Parm | 2 Servings"), "with Cream Cheese, Zucchini & Parm");
        assert_eq!(clean_title("with Crushed Tortilla Chips | 2–4 servings"), "with Crushed Tortilla Chips");
        assert_eq!(clean_title("Chicken (Not) Pot Pie"), "Chicken (Not) Pot Pie");
        assert_eq!(
            clean_title("MA/CA only Steakhouse Pork Chop with Creamy Shallot Pan Sauce (New Sour Cream SKU)"),
            "Steakhouse Pork Chop with Creamy Shallot Pan Sauce"
        );
    }

    #[test]
    fn protein_swaps_share_a_core() {
        assert_eq!(protein_core("Hot Honey Chicken"), protein_core("Hot Honey Pork Filet"));
        assert_eq!(protein_core("Bulgogi Beef Tenderloin"), protein_core("Bulgogi Chicken"));
        assert_eq!(protein_core("One-Pan Pineapple Salsa & 2x Shrimp Tacos"), protein_core("One-Pan Pineapple Salsa & 2x Beef Tacos"));
        assert_ne!(protein_core("Chicken Schnitzel"), protein_core("Sweet Thai Chili Chicken"));
    }
}
