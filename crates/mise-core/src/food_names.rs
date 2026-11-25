//! Rules for relating ingredient names: plural folding, brand stripping, and deciding whether one name is a
//! more specific kind of another ("chopped chicken breast" is a chicken breast) or only related to it
//! ("smoked paprika" is not a kind of paprika).

use crate::text::normalize_name;

/// The brands matching strips from ingredient names. The words are data (the `brands` table, which curation edits);
/// this module keeps only the rules. Longest first, so a brand that holds another matches whole: "tillamook country
/// smoker" before "tillamook".
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Brands(Vec<String>);

impl Brands {
    /// The brands named, each as [`normalize_name`] writes it ("Frank's RedHot®" is "frank's redhot"). Blanks and
    /// repeats are dropped.
    pub fn new<S: AsRef<str>>(names: impl IntoIterator<Item = S>) -> Brands {
        let mut list: Vec<String> = names
            .into_iter()
            .map(|n| normalize_name(n.as_ref()))
            .filter(|n| !n.is_empty())
            .collect();
        let words = |b: &String| b.split(' ').count();
        list.sort_by(|a, b| {
            words(b)
                .cmp(&words(a))
                .then(b.len().cmp(&a.len()))
                .then(a.cmp(b))
        });
        list.dedup();
        Brands(list)
    }

    /// Whether `word` is one of a brand's words: "frank's" is, through "frank's redhot".
    fn has_word(&self, word: &str) -> bool {
        self.0.iter().any(|b| b.split(' ').any(|w| w == word))
    }
}

/// Extra words that only narrow a food down: cut, form, size, state, colour, variety, grade.
/// "Chopped chicken breast", "yukon gold potatoes", "organic baby spinach" stay kinds of their base.
const NARROWING: &[&str] = &[
    "chopped",
    "diced",
    "minced",
    "sliced",
    "shredded",
    "grated",
    "crumbled",
    "cubed",
    "halved",
    "quartered",
    "julienned",
    "spiralized",
    "riced",
    "trimmed",
    "peeled",
    "pitted",
    "boneless",
    "skinless",
    "bone",
    "in",
    "fresh",
    "frozen",
    "dried",
    "dry",
    "organic",
    "baby",
    "jumbo",
    "large",
    "small",
    "mini",
    "medium",
    "whole",
    "thin",
    "thick",
    "cut",
    "low",
    "fat",
    "reduced",
    "sodium",
    "unsalted",
    "salted",
    "plain",
    "natural",
    "pure",
    "lean",
    "extra",
    "light",
    "fully",
    "cooked",
    "precooked",
    "uncooked",
    "raw",
    "loose",
    "packed",
    "block",
    "red",
    "yellow",
    "white",
    "green",
    "orange",
    "purple",
    "gold",
    "golden",
    "brown",
    "black",
    "rainbow",
    "yukon",
    "russet",
    "fingerling",
    "roma",
    "grape",
    "cherry",
    "heirloom",
    "vidalia",
    "persian",
    "english",
    "italian",
    "flat",
    "leaf",
    "curly",
    "lacinato",
    "seedless",
    "navel",
    "granny",
    "smith",
    "honeycrisp",
    "strips",
    "florets",
    "cutlets",
    "tenders",
    "fillets",
    "filets",
    "slices",
    "pieces",
    "chunks",
    "tips",
    "steaks",
    "breasts",
    "thighs",
    "wedges",
    "spears",
    "leaves",
    "sprigs",
    "stalks",
    "stalk",
    "head",
    "heads",
    "crowns",
    "grass",
    "fed",
    "free",
    "range",
    "wild",
    "caught",
    "farm",
    "raised",
    "nitrite",
    "nitrate",
    "uncured",
    "all",
    "natural",
    "premium",
    "classic",
    "original",
    "regular",
    "style",
    "ground",
    "chops",
    "chop",
    "loin",
    "tenderloin",
    "shoulder",
    "belly",
    "ribs",
    "roast",
    "medallions",
    "jasmine",
    "basmati",
    "arborio",
    "long",
    "short",
    "grain",
    "button",
    "cremini",
    "portobello",
];

/// Words that may follow a base name and still narrow it ("chicken breast strips", "broccoli florets").
/// When the base comes first, only these count: English puts the head noun last, so "flour tortillas" is
/// not a kind of flour.
const TRAILING_NARROWING: &[&str] = &[
    "strips",
    "florets",
    "cutlets",
    "tenders",
    "fillets",
    "filets",
    "slices",
    "pieces",
    "chunks",
    "tips",
    "steaks",
    "breasts",
    "thighs",
    "wedges",
    "spears",
    "leaves",
    "sprigs",
    "stalks",
    "crowns",
    "chops",
    "loin",
    "medallions",
    "ribs",
    "mix",
];

/// Extra words that make a different food, so the pair is related rather than parent and child.
/// "Smoked paprika", "sweet potato", "peanut butter", "cream cheese", "coconut milk".
const TRANSFORMING: &[&str] = &[
    "smoked",
    "sweet",
    "hot",
    "spicy",
    "coconut",
    "cream",
    "sour",
    "pickled",
    "roasted",
    "toasted",
    "fried",
    "crispy",
    "candied",
    "caramelized",
    "peanut",
    "almond",
    "oat",
    "soy",
    "garlic",
    "onion",
    "honey",
    "maple",
    "chili",
    "chile",
    "lemon",
    "lime",
    "herb",
    "truffle",
    "buttermilk",
    "condensed",
    "evaporated",
    "powder",
    "powdered",
    "paste",
    "sauce",
    "seasoning",
    "spice",
    "blend",
    "mix",
    "dressing",
    "oil",
    "vinegar",
    "juice",
    "zest",
    "extract",
    "syrup",
    "jam",
    "jelly",
    "stock",
    "broth",
    "concentrate",
    "base",
    "glaze",
];

/// Words whose trailing "s" is not a plural.
const NOT_PLURAL: &[&str] = &[
    "molasses",
    "hummus",
    "couscous",
    "asparagus",
    "swiss",
    "brussels",
    "chives",
    "grits",
    "citrus",
    "octopus",
    "pancreas",
    "series",
    "species",
    "bass",
    "glass",
    "cress",
    "watercress",
    "harissa",
];

/// Whether `word` is in `list`, ignoring a plural "s": "breasts" matches "breast" and the other way about.
fn in_list(list: &[&str], word: &str) -> bool {
    list.contains(&word) || list.contains(&format!("{word}s").as_str())
}

/// Words that only say how a food was cut up or handled before it went in the bag, and the joiners that hold two
/// foods in one name. Dropping these is what makes "Red Onion, Diced" and "Red Onions" one food.
///
/// Deliberately much shorter than [`NARROWING`], which also holds colours, varieties and cuts ("green", "baby",
/// "steaks"). Those say *which* food it is: cutting them would offer "Bell Pepper" and "Green Bell Pepper" as one
/// food, when a colour is a kind of the base and [`relate`] already links it as one.
const PREPARED: &[&str] = &[
    "chopped",
    "diced",
    "minced",
    "sliced",
    "shredded",
    "grated",
    "crumbled",
    "cubed",
    "halved",
    "quartered",
    "julienned",
    "spiralized",
    "trimmed",
    "peeled",
    "pitted",
    "precooked",
    "cooked",
    "and",
    "or",
    "with",
    "plus",
];

/// The key two names share when they look like the same food written differently: every word folded to its
/// singular, the words that only say how it was cut up or handled dropped ("diced", "shredded", "peeled"), then
/// sorted. So "Red Onions", "Red Onion, Diced" and "Diced Red Onion" share one key, while "Honey" and "Hot
/// Honey" do not, nor "Bell Pepper" and "Green Bell Pepper": those extra words say which food it is.
///
/// Unlike [`singular_key`], which keys a name to one food, this is deliberately looser, and it folds every word
/// rather than only the last. It is used only to put a pair in front of a person (a `food_duplicate` review
/// item), never to match a line on its own.
pub fn duplicate_key(name: &str) -> String {
    let words: Vec<String> = normalize_name(name)
        .split(' ')
        .filter(|w| !w.is_empty())
        .map(singular)
        .collect();
    let mut kept: Vec<&str> = words
        .iter()
        .map(String::as_str)
        .filter(|w| !in_list(PREPARED, w))
        .collect();
    // A name of nothing but those words ("Chopped and Diced") keeps them, so such names group by what they say
    // rather than all landing on one empty key.
    if kept.is_empty() {
        kept = words.iter().map(String::as_str).collect();
    }
    kept.sort_unstable();
    kept.dedup();
    kept.join(" ")
}

/// Fold the last word to its singular so "Onions" and "Onion" share one key.
pub fn singular_key(name: &str) -> String {
    let norm = normalize_name(name);
    let mut words: Vec<String> = norm.split(' ').map(str::to_string).collect();
    if let Some(last) = words.last_mut() {
        *last = singular(last);
    }
    words.join(" ")
}

fn singular(w: &str) -> String {
    if w.len() <= 3
        || NOT_PLURAL.contains(&w)
        || w.ends_with("ss")
        || w.ends_with("us")
        || w.ends_with("is")
    {
        return w.to_string();
    }
    if let Some(stem) = w.strip_suffix("ies") {
        return format!("{stem}y");
    }
    if let Some(stem) = w.strip_suffix("oes") {
        return format!("{stem}o");
    }
    if let Some(stem) = w
        .strip_suffix("ches")
        .or_else(|| w.strip_suffix("shes"))
        .or_else(|| w.strip_suffix("xes"))
    {
        return format!("{stem}{}", &w[stem.len()..w.len() - 2]);
    }
    w.strip_suffix('s')
        .map(str::to_string)
        .unwrap_or_else(|| w.to_string())
}

/// The brand in a name, and the name without it: "PHILADELPHIA® Cream Cheese" → ("philadelphia", "cream cheese").
/// A word marked ® or ™ counts as a brand even if it isn't one of `brands`.
pub fn strip_brand(raw: &str, brands: &Brands) -> Option<(String, String)> {
    let norm = normalize_name(raw);
    for brand in &brands.0 {
        if let Some(rest) = strip_words(&norm, brand)
            && !rest.is_empty()
        {
            return Some((brand.to_string(), rest));
        }
    }
    // An unknown mark: take the words before the first ® or ™ as the brand.
    let mark = raw.find(['®', '™'])?;
    let brand = normalize_name(&raw[..mark]);
    let rest = normalize_name(&raw[mark..]);
    (!brand.is_empty() && !rest.is_empty()).then_some((brand, rest))
}

fn strip_words(name: &str, phrase: &str) -> Option<String> {
    let padded = format!(" {name} ");
    let needle = format!(" {phrase} ");
    let at = padded.find(&needle)?;
    let rest = format!("{}{}", &padded[..at], &padded[at + needle.len() - 1..]);
    Some(rest.split_whitespace().collect::<Vec<_>>().join(" "))
}

/// How a more specific name relates to a shorter one it contains.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Relation {
    /// Only narrowing words were added: a kind of the base (auto-applied).
    KindOf,
    /// A transforming word was added: a different food, linked as related.
    Related,
    /// Unknown extra words: a person decides.
    Unsure,
}

/// If `base` appears in `specific` as whole words (at the start or the end), classify the extra words.
/// Names are compared by [`singular_key`], so "Yukon Gold Potatoes" contains "Potato". A brand's words only narrow.
pub fn relate(specific: &str, base: &str, brands: &Brands) -> Option<Relation> {
    let s: Vec<String> = singular_key(specific)
        .split(' ')
        .map(str::to_string)
        .collect();
    let b: Vec<String> = singular_key(base).split(' ').map(str::to_string).collect();
    if b.len() >= s.len() || b.is_empty() {
        return None;
    }
    // Plural-insensitive membership: "breasts" and "breast" are both narrowing.
    let is_in = in_list;
    let extra: Vec<&str> = if s.ends_with(&b) {
        s[..s.len() - b.len()].iter().map(String::as_str).collect()
    } else if s.starts_with(&b) {
        let trailing: Vec<&str> = s[b.len()..].iter().map(String::as_str).collect();
        if trailing.iter().all(|w| is_in(TRAILING_NARROWING, w)) {
            return Some(Relation::KindOf);
        }
        if trailing.iter().any(|w| is_in(TRANSFORMING, w)) {
            return Some(Relation::Related);
        }
        return None; // a different head noun: "flour tortillas" is not about flour
    } else {
        return None;
    };
    if extra.iter().any(|w| is_in(TRANSFORMING, w)) {
        Some(Relation::Related)
    } else if extra
        .iter()
        .all(|w| is_in(NARROWING, w) || brands.has_word(w))
    {
        Some(Relation::KindOf)
    } else {
        Some(Relation::Unsure)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn none() -> Brands {
        Brands::default()
    }

    fn brands() -> Brands {
        Brands::new([
            "Cholula",
            "Philadelphia",
            "Tillamook",
            "Tillamook Country Smoker",
        ])
    }

    #[test]
    fn plurals_fold() {
        assert_eq!(singular_key("Onions"), "onion");
        assert_eq!(singular_key("Grape Tomatoes"), "grape tomato");
        assert_eq!(singular_key("Cherries"), "cherry");
        assert_eq!(singular_key("Molasses"), "molasses");
        assert_eq!(singular_key("Couscous"), "couscous");
        assert_eq!(singular_key("Peaches"), "peach");
    }

    /// Whether the pair would be put to a person as possibly one food.
    fn same_food(a: &str, b: &str) -> bool {
        duplicate_key(a) == duplicate_key(b)
    }

    #[test]
    fn names_of_one_food_share_a_duplicate_key() {
        // Word order, plurals and a preparation note left in the name don't make another food.
        assert!(same_food("Red Onions", "Red Onion, Diced"));
        assert!(same_food("Red Onion, Diced", "Diced Red Onion"));
        assert!(same_food("Cheddar Cheese", "Shredded Cheddar Cheese"));
        assert!(same_food("Kale", "Chopped Kale"));
        assert!(same_food("Diced Steak", "Steak"));
        assert!(same_food(
            "Bok Choy and Napa Cabbage",
            "Bok Choy & Napa Cabbage"
        ));
    }

    #[test]
    fn a_word_that_changes_the_food_keeps_it_apart() {
        // Each of these scores above 0.65 by trigram, which is why the pairing goes by words, not that score.
        assert!(!same_food("Honey", "Hot Honey"));
        assert!(!same_food("Bell Pepper", "Green Bell Pepper"));
        assert!(!same_food("Butter", "Peanut Butter"));
        assert!(!same_food("Sourdough Bread", "Sourdough Bread Bowl"));
        assert!(!same_food("Chili Powder", "Ancho Chili Powder"));
        assert!(!same_food("Potatoes", "Sweet Potatoes"));
    }

    #[test]
    fn a_name_of_nothing_but_prepared_words_keeps_them() {
        // Cutting every word would land each on one empty key and offer them all as the same food.
        assert_eq!(duplicate_key("Chopped Diced"), "chopped diced");
        assert!(!same_food("Chopped Diced", "Peeled"));
    }

    #[test]
    fn a_colour_or_variety_is_not_a_duplicate_but_a_kind() {
        // These are what `relate` files as a kind of the base, so the queue shouldn't ask about them as duplicates.
        assert!(!same_food("Bell Pepper", "Green Bell Pepper"));
        assert!(!same_food("Potato", "Yukon Gold Potato"));
        assert_eq!(
            relate("Green Bell Pepper", "Bell Pepper", &none()),
            Some(Relation::KindOf)
        );
    }

    #[test]
    fn brands_strip() {
        assert_eq!(
            strip_brand("PHILADELPHIA® Cream Cheese", &brands()),
            Some(("philadelphia".into(), "cream cheese".into()))
        );
        assert_eq!(
            strip_brand("Cholula® Hot Sauce", &brands()),
            Some(("cholula".into(), "hot sauce".into()))
        );
        assert_eq!(strip_brand("Cream Cheese", &brands()), None);
        assert_eq!(
            strip_brand("Zesty Brand™ Salsa", &brands()),
            Some(("zesty brand".into(), "salsa".into()))
        );
    }

    #[test]
    fn a_listed_brand_strips_unmarked_and_the_longest_wins() {
        assert_eq!(
            strip_brand("Cholula Hot Sauce", &brands()),
            Some(("cholula".into(), "hot sauce".into()))
        );
        assert_eq!(
            strip_brand("Tillamook Country Smoker Jerky", &brands()),
            Some(("tillamook country smoker".into(), "jerky".into()))
        );
        assert_eq!(strip_brand("Cholula Hot Sauce", &none()), None);
        assert_eq!(
            Brands::new(["Old Bay", " old  BAY ", ""]),
            Brands::new(["old bay"])
        );
    }

    #[test]
    fn a_brands_words_only_narrow() {
        assert_eq!(
            relate("Tillamook Cheddar", "Cheddar", &brands()),
            Some(Relation::KindOf)
        );
        assert_eq!(
            relate("Tillamook Cheddar", "Cheddar", &none()),
            Some(Relation::Unsure)
        );
    }

    #[test]
    fn narrowing_words_make_a_kind_of() {
        assert_eq!(
            relate("Chopped Chicken Breast", "Chicken Breast", &none()),
            Some(Relation::KindOf)
        );
        assert_eq!(
            relate("Yukon Gold Potatoes", "Potato", &none()),
            Some(Relation::KindOf)
        );
        assert_eq!(
            relate("Chicken Breast Strips", "Chicken Breast", &none()),
            Some(Relation::KindOf)
        );
        assert_eq!(
            relate("Broccoli Florets", "Broccoli", &none()),
            Some(Relation::KindOf)
        );
        assert_eq!(
            relate("Red Onion", "Onions", &none()),
            Some(Relation::KindOf)
        );
        assert_eq!(
            relate("Orange Bell Pepper", "Bell Pepper", &none()),
            Some(Relation::KindOf)
        );
    }

    #[test]
    fn transforming_words_are_only_related() {
        assert_eq!(
            relate("Smoked Paprika", "Paprika", &none()),
            Some(Relation::Related)
        );
        assert_eq!(
            relate("Sweet Potatoes", "Potatoes", &none()),
            Some(Relation::Related)
        );
        assert_eq!(
            relate("Toasted Sesame Oil", "Sesame Oil", &none()),
            Some(Relation::Related)
        );
        assert_eq!(
            relate("Peanut Butter", "Butter", &none()),
            Some(Relation::Related)
        );
        assert_eq!(
            relate("Garlic Powder", "Garlic", &none()),
            Some(Relation::Related)
        );
    }

    #[test]
    fn unknown_words_go_to_review_and_unrelated_names_dont_match() {
        assert_eq!(
            relate("Bavette Steak", "Steak", &none()),
            Some(Relation::Unsure)
        );
        assert_eq!(relate("Cream Cheese", "Sour Cream", &none()), None);
        assert_eq!(relate("Rice", "Rice", &none()), None);
        assert_eq!(relate("Flour Tortillas", "Flour", &none()), None);
        assert_eq!(relate("Honey Butter Cornbread", "Honey", &none()), None);
        assert_eq!(
            relate("Pork Chops", "Pork", &none()),
            Some(Relation::KindOf)
        );
        assert_eq!(
            relate("Jasmine Rice", "Rice", &none()),
            Some(Relation::KindOf)
        );
        assert_eq!(
            relate("Ground Pork", "Pork", &none()),
            Some(Relation::KindOf)
        );
    }
}
