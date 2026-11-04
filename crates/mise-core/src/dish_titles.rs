//! Dish titles: strip sources' internal working notes, and find the part of a title that stays the same when
//! only the protein changes ("Garlic Ginger Chicken" and "Garlic Ginger Pork Filet" share "garlic ginger").

use std::sync::LazyLock;

use regex::Regex;

use crate::text::normalize_name;

static BRACKETED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\[[^\]]*\]").unwrap());
static COMPLIANCE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(?:\b[A-Z]{2}/[A-Z]{2}\s+only\b(?:\s+compliant\b)?|\bcompliant\b)").unwrap()
});
static NOTE_PARENS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\s*\([^)]*\b(?:sku|swap|only|compliant|test)\b[^)]*\)").unwrap()
});
static WEEK_CODE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*W\d+\s+R\d+[A-Z0-9]*\s+").unwrap());
static NOTE_SUFFIX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+--\s.*$").unwrap());
static CODE_PARENS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\s*\((?:[^)]*\d[^)]*|[A-Z0-9]{2,5})\)\s*$").unwrap());
static SERVINGS_SUFFIX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\s*\|\s*\d+(?:\s*[–-]\s*\d+)?\s+servings?\s*$").unwrap());
static SPACES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+").unwrap());

/// A title as a person should see it: internal notes, codes and compliance tags removed.
pub fn clean_title(title: &str) -> String {
    let mut t = BRACKETED.replace_all(title, " ").into_owned();
    t = COMPLIANCE.replace_all(&t, " ").into_owned();
    t = NOTE_PARENS.replace_all(&t, " ").into_owned();
    t = WEEK_CODE.replace(&t, "").into_owned();
    t = NOTE_SUFFIX.replace(&t, "").into_owned();
    t = SERVINGS_SUFFIX.replace(&t, "").into_owned();
    // Parenthetical codes can stack: "(Turkey 10oz) (QA)".
    loop {
        let next = CODE_PARENS.replace(&t, "").into_owned();
        if next == t {
            break;
        }
        t = next;
    }
    SPACES.replace_all(t.trim(), " ").into_owned()
}

/// A title for comparison: cleaned, normalized, and with "and" dropped so "&" and "and" read the same.
pub fn title_key(title: &str) -> String {
    normalize_name(&clean_title(title))
        .split(' ')
        .filter(|w| *w != "and")
        .collect::<Vec<_>>()
        .join(" ")
}

/// Share of words two protein-neutral cores have in common (0 to 1).
pub fn core_similarity(a: &str, b: &str) -> f64 {
    let (ca, cb) = (protein_core(a), protein_core(b));
    let wa: std::collections::HashSet<&str> = ca.split(' ').filter(|w| !w.is_empty()).collect();
    let wb: std::collections::HashSet<&str> = cb.split(' ').filter(|w| !w.is_empty()).collect();
    let union = wa.union(&wb).count();
    if union == 0 {
        0.0
    } else {
        wa.intersection(&wb).count() as f64 / union as f64
    }
}

/// Proteins and format words that vary between versions of one dish ("2x", "Prep & Bake", "One-Pan").
const SWAPPABLE: &[&str] = &[
    "chicken",
    "pork",
    "beef",
    "steak",
    "shrimp",
    "salmon",
    "tilapia",
    "cod",
    "barramundi",
    "trout",
    "steelhead",
    "scallops",
    "turkey",
    "tofu",
    "sausage",
    "bacon",
    "lamb",
    "duck",
    "fish",
    "seafood",
    "plant",
    "based",
    "protein",
    "impossible",
    "beyond",
    "filet",
    "filets",
    "fillet",
    "fillets",
    "cutlet",
    "cutlets",
    "thigh",
    "thighs",
    "breast",
    "breasts",
    "tenderloin",
    "sirloin",
    "ribeye",
    "rib",
    "eye",
    "strip",
    "bavette",
    "ranch",
    "chop",
    "chops",
    "ground",
    "meatballs",
    "meatball",
    "patty",
    "patties",
    "burger",
    "burgers",
    "organic",
    "2x",
    "double",
    "triple",
    "prep",
    "bake",
    "one",
    "pan",
    "pot",
    "sheet",
    "and",
    "with",
    "over",
    "the",
    "a",
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
            clean_title(
                "[SWAP BEEF TO TURKEY] NY/NJ only Compliant Smoky Turkey Chili with Corn Salsa (Turkey 10oz)"
            ),
            "Smoky Turkey Chili with Corn Salsa"
        );
        assert_eq!(
            clean_title("W12 R034 Maple Mustard Pork Chops"),
            "Maple Mustard Pork Chops"
        );
        assert_eq!(
            clean_title("Lentil tacos with charred corn -- LIME SWAP"),
            "Lentil tacos with charred corn"
        );
        assert_eq!(
            clean_title("Lemon Pepper Chicken Thighs with Green Beans (QA)"),
            "Lemon Pepper Chicken Thighs with Green Beans"
        );
        assert_eq!(
            clean_title("with Spinach, Feta & Lemon | 2 Servings"),
            "with Spinach, Feta & Lemon"
        );
        assert_eq!(
            clean_title("with Toasted Pita Chips | 2–4 servings"),
            "with Toasted Pita Chips"
        );
        assert_eq!(clean_title("Veggie (Not) Lasagna"), "Veggie (Not) Lasagna");
        assert_eq!(
            clean_title("NY/NJ only Herb Roasted Chicken with Shallot Gravy (New Butter SKU)"),
            "Herb Roasted Chicken with Shallot Gravy"
        );
    }

    #[test]
    fn protein_swaps_share_a_core() {
        assert_eq!(
            protein_core("Garlic Ginger Chicken"),
            protein_core("Garlic Ginger Pork Filet")
        );
        assert_eq!(
            protein_core("Sesame Beef Tenderloin"),
            protein_core("Sesame Chicken")
        );
        assert_eq!(
            protein_core("Sheet-Pan Mango Salsa & 2x Shrimp Tacos"),
            protein_core("Sheet-Pan Mango Salsa & 2x Beef Tacos")
        );
        assert_ne!(
            protein_core("Chicken Milanese"),
            protein_core("Coconut Lime Chicken")
        );
    }

    #[test]
    fn and_reads_like_ampersand() {
        assert_eq!(
            title_key("Lemon Pea and Shrimp Orzo"),
            title_key("Lemon Pea & Shrimp Orzo")
        );
    }

    #[test]
    fn core_similarity_ignores_proteins() {
        assert!(core_similarity("Chicken Cobb Salad", "Chicken, Lentil & Spinach Salad") < 0.34);
        assert!(core_similarity("Cajun-Spiced Chicken", "Cajun Pork Filet") >= 0.34);
        assert_eq!(
            core_similarity("Garlic Ginger Chicken", "Garlic Ginger Pork Filet"),
            1.0
        );
    }
}
