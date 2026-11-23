//! Ingredient lines as people write them ("2 cups flour, sifted"), read into an amount, a unit, the food's name
//! and a note. Recipes typed into the app and lines imported from web pages come as one text; the rest of mise
//! works on the parts.

use crate::quantity::{Unit, parse_number};

/// One ingredient line, read. "1 (14 oz) can diced tomatoes, drained" is 1 can of "diced tomatoes", with the note
/// "14 oz; drained".
#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    /// How much, when the line says: 2 for "2 cups flour", none for "salt, to taste".
    pub amount: Option<f64>,
    /// What the amount counts or measures; `Unit::Count("")` for a bare number ("2 eggs"). None without an amount.
    pub unit: Option<Unit>,
    /// What the line asks for, as written: "flour", "large eggs". This is what gets matched to a food.
    pub name: String,
    /// Everything else: preparation, package sizes, "to taste", "optional".
    pub note: Option<String>,
}

impl Line {
    /// The unit as the database and seed bundles spell it: "tablespoon", "ounce", "clove", and "unit" for a bare
    /// number. `Unit::parse` reads each back.
    pub fn unit_word(&self) -> Option<String> {
        self.unit.as_ref().map(|u| {
            match u {
                Unit::Tsp => "teaspoon",
                Unit::Tbsp => "tablespoon",
                Unit::Cup => "cup",
                Unit::FlOz => "fluid ounce",
                Unit::Ml => "milliliter",
                Unit::L => "liter",
                Unit::Oz => "ounce",
                Unit::Lb => "pound",
                Unit::G => "gram",
                Unit::Kg => "kilogram",
                Unit::Pinch => "pinch",
                Unit::Count(noun) if noun.is_empty() => "unit",
                Unit::Count(noun) => noun,
            }
            .to_string()
        })
    }
}

/// Number words a line can open with. "a" and "an" count only before a unit ("a pinch of salt"), since "a few
/// sprigs of thyme" is not one of anything.
const NUMBER_WORDS: &[(&str, f64)] = &[
    ("half", 0.5),
    ("one", 1.0),
    ("two", 2.0),
    ("three", 3.0),
    ("four", 4.0),
    ("five", 5.0),
    ("six", 6.0),
    ("seven", 7.0),
    ("eight", 8.0),
    ("nine", 9.0),
    ("ten", 10.0),
    ("eleven", 11.0),
    ("twelve", 12.0),
];

/// Sizes written between the number and the unit: "2 large cloves garlic". They move to the note.
const SIZES: &[&str] = &["small", "medium", "large", "heaping", "scant", "level"];

/// Ways a line ends that say how the ingredient is used, not what it is, written without a comma: "lime wedges
/// for serving".
const TRAILING_NOTES: &[&str] = &[
    "to taste",
    "for serving",
    "for garnish",
    "for garnishing",
    "for frying",
    "as needed",
    "if needed",
    "optional",
];

/// Reads one ingredient line. Never fails: a line it can't take apart comes back whole as the name.
///
/// ```
/// use mise_core::ingredient_line::parse;
/// use mise_core::quantity::Unit;
///
/// let line = parse("1½ cups flour, sifted");
/// assert_eq!(line.amount, Some(1.5));
/// assert_eq!(line.unit, Some(Unit::Cup));
/// assert_eq!(line.name, "flour");
/// assert_eq!(line.note.as_deref(), Some("sifted"));
///
/// let line = parse("2 eggs");
/// assert_eq!((line.amount, line.unit_word().as_deref(), line.name.as_str()), (Some(2.0), Some("unit"), "eggs"));
///
/// let line = parse("Salt and pepper, to taste");
/// assert_eq!((line.amount, line.name.as_str(), line.note.as_deref()), (None, "Salt and pepper", Some("to taste")));
/// ```
pub fn parse(text: &str) -> Line {
    let text = tidy(text);
    let mut notes: Vec<String> = Vec::new();
    let mut rest = text.as_str();

    let mut amount = leading_amount(&mut rest);
    // "1-inch piece ginger", "14-ounce can tomatoes": the number sizes the item, and there is one of it.
    if amount.is_some() && rest.starts_with('-') && rest[1..].starts_with(char::is_alphabetic) {
        let end = rest.find(' ').unwrap_or(rest.len());
        let size_end = text.len() - rest.len() + end;
        notes.push(text[..size_end].to_string());
        rest = rest[end..].trim_start();
        amount = Some(1.0);
    }
    if amount.is_some() {
        // Package sizes after the count: "1 (14 oz) can", "2 14-ounce cans".
        loop {
            if let Some(inner) = leading_parenthetical(&mut rest) {
                notes.push(inner);
            } else if let Some(size) = leading_size(&mut rest) {
                notes.push(size);
            } else {
                break;
            }
        }
    }
    let unit = amount.and_then(|_| leading_unit(&mut rest, &mut notes));
    let unit = match (amount, unit) {
        (Some(_), None) => Some(Unit::Count(String::new())),
        (_, unit) => unit,
    };
    if unit.is_some() {
        // "1 cup (240 ml) milk": the same amount another way.
        while let Some(inner) = leading_parenthetical(&mut rest) {
            notes.push(inner);
        }
        rest = strip_word(rest, "of").unwrap_or(rest);
    }

    // The name runs to the first comma or parenthesis; what follows is preparation.
    let (name, after) = match rest.find([',', '(']) {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, ""),
    };
    let mut name = name.trim().to_string();
    for trailing in TRAILING_NOTES {
        if let Some(head) = strip_suffix_word(&name, trailing) {
            notes.push((*trailing).to_string());
            name = head;
            break;
        }
    }
    notes.extend(note_parts(after));

    // "2 cloves", "1 can": nothing is left to name, so the unit word was the food.
    if name.is_empty() {
        return Line {
            amount,
            unit: amount.map(|_| Unit::Count(String::new())),
            name: match amount {
                Some(_) => after_amount(&text).to_string(),
                None => text.clone(),
            },
            note: None,
        };
    }
    Line {
        amount,
        unit,
        name,
        note: (!notes.is_empty()).then(|| notes.join("; ")),
    }
}

/// One line, spaces tidied, without a list marker ("- ", "• ", "* ") in front.
fn tidy(text: &str) -> String {
    let joined = text.split_whitespace().collect::<Vec<_>>().join(" ");
    joined
        .trim_start_matches(['-', '•', '*', '–', '·'])
        .trim()
        .to_string()
}

/// The amount a line opens with, taken off `rest`: "2", "1½", "1 1/2", "2-3" and "2 to 3" (the lower end), "two",
/// "a" before a unit. A zero is no amount.
fn leading_amount(rest: &mut &str) -> Option<f64> {
    if let Some((value, used)) = parse_number(rest) {
        let mut after = &rest[used..];
        // A number runs into its unit ("200g") but not into a word: "7up" is a name.
        let glued = after.starts_with(char::is_alphabetic);
        if glued && Unit::parse(first_word(after)).is_none_or(|u| u == Unit::Count(String::new())) {
            return None;
        }
        // A range gives the least that will do.
        let trimmed = after.trim_start();
        for sep in ["-", "–", "—", "to ", "or "] {
            if let Some(tail) = trimmed.strip_prefix(sep)
                && let Some((_, used)) = parse_number(tail.trim_start())
            {
                let tail = tail.trim_start();
                after = &tail[used..];
                break;
            }
        }
        *rest = after.trim_start();
        return (value > 0.0 && value.is_finite()).then_some(value);
    }
    let word = first_word(rest);
    let lower = word.to_lowercase();
    let after = rest[word.len()..].trim_start();
    if let Some((_, value)) = NUMBER_WORDS.iter().find(|(w, _)| *w == lower) {
        // "half a lemon", "half an onion".
        let after = match lower.as_str() {
            "half" => strip_word(after, "a")
                .or_else(|| strip_word(after, "an"))
                .unwrap_or(after),
            _ => after,
        };
        if after.is_empty() {
            return None;
        }
        *rest = after;
        return Some(*value);
    }
    if matches!(lower.as_str(), "a" | "an") && is_unit(first_word(after)) {
        *rest = after;
        return Some(1.0);
    }
    None
}

/// What follows the amount in a line that has one: "cloves" for "2 cloves".
fn after_amount(text: &str) -> &str {
    let mut rest = text;
    leading_amount(&mut rest);
    rest
}

/// A parenthetical at the start of `rest`, taken off it: "14 oz" from "(14 oz) can tomatoes".
fn leading_parenthetical(rest: &mut &str) -> Option<String> {
    let inner = rest.strip_prefix('(')?;
    let end = inner.find(')')?;
    let found = inner[..end].trim().to_string();
    *rest = inner[end + 1..].trim_start();
    (!found.is_empty()).then_some(found)
}

/// A package size at the start of `rest`, taken off it: "14-ounce" from "14-ounce cans tomatoes".
fn leading_size(rest: &mut &str) -> Option<String> {
    let word = first_word(rest);
    let (number, unit) = word.split_once('-')?;
    let (_, used) = parse_number(number)?;
    if used != number.len() || !unit.starts_with(char::is_alphabetic) {
        return None;
    }
    let found = word.to_string();
    *rest = rest[word.len()..].trim_start();
    Some(found)
}

/// The unit at the start of `rest`, taken off it. A size before it moves to the notes: "large cloves garlic" is
/// cloves, noted "large".
fn leading_unit(rest: &mut &str, notes: &mut Vec<String>) -> Option<Unit> {
    let word = first_word(rest);
    if SIZES.contains(&word.to_lowercase().as_str()) {
        let mut after = rest[word.len()..].trim_start();
        let unit = unit_at(&mut after)?;
        notes.push(word.to_lowercase());
        *rest = after;
        return Some(unit);
    }
    unit_at(rest)
}

/// The unit word or words at the start of `rest`, taken off it: "fl oz", "tablespoons", "cloves", "c." for cups.
fn unit_at(rest: &mut &str) -> Option<Unit> {
    for two in ["fl oz", "fl. oz", "fluid ounces", "fluid ounce"] {
        if let (Some(head), Some(after)) = (rest.get(..two.len()), rest.get(two.len()..))
            && head.eq_ignore_ascii_case(two)
            && !after.starts_with(char::is_alphanumeric)
        {
            *rest = after.trim_start_matches('.').trim_start();
            return Some(Unit::FlOz);
        }
    }
    let word = first_word(rest);
    let unit = match word.trim_end_matches('.') {
        "c" | "C" => Unit::Cup,
        "T" => Unit::Tbsp,
        "t" => Unit::Tsp,
        w if is_unit(w) => Unit::parse(w)?,
        _ => return None,
    };
    *rest = rest[word.len()..].trim_start();
    Some(unit)
}

/// `word` names a measure or a countable item ("cups", "oz", "cloves"), not merely nothing.
fn is_unit(word: &str) -> bool {
    let word = word.trim_end_matches('.');
    !word.is_empty()
        && !matches!(
            word.to_lowercase().as_str(),
            "unit" | "units" | "pick" | "picks"
        )
        && Unit::parse(word).is_some()
}

/// Up to the first space.
fn first_word(text: &str) -> &str {
    text.split(' ').next().unwrap_or("")
}

/// `text` without `word` in front, when it starts with that whole word and more follows.
fn strip_word<'a>(text: &'a str, word: &str) -> Option<&'a str> {
    let head = text.get(..word.len())?;
    let after = text[word.len()..].strip_prefix(' ')?;
    (head.eq_ignore_ascii_case(word) && !after.is_empty()).then_some(after.trim_start())
}

/// `name` without `phrase` at its end, when it ends with those whole words and something comes before them.
fn strip_suffix_word(name: &str, phrase: &str) -> Option<String> {
    let cut = name.len().checked_sub(phrase.len())?;
    let (head, tail) = (name.get(..cut)?, name.get(cut..)?);
    let head = head.strip_suffix(' ')?;
    (tail.eq_ignore_ascii_case(phrase) && !head.trim().is_empty()).then(|| head.trim().to_string())
}

/// What comes after the name, as notes: ", drained (optional)" gives "drained" and "optional".
fn note_parts(after: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut depth = 0usize;
    let mut push = |part: &mut String| {
        let found = part.trim().trim_matches(',').trim().to_string();
        if !found.is_empty() {
            parts.push(found);
        }
        part.clear();
    };
    for c in after.chars() {
        match c {
            '(' if depth == 0 => {
                push(&mut current);
                depth = 1;
            }
            ')' if depth == 1 => {
                push(&mut current);
                depth = 0;
            }
            '(' => {
                depth += 1;
                current.push(c);
            }
            ')' => {
                depth = depth.saturating_sub(1);
                current.push(c);
            }
            c => current.push(c),
        }
    }
    push(&mut current);
    parts
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A line as (amount, unit word, name, note).
    fn read(text: &str) -> (Option<f64>, Option<String>, String, Option<String>) {
        let line = parse(text);
        (line.amount, line.unit_word(), line.name, line.note)
    }

    fn line(
        amount: f64,
        unit: &str,
        name: &str,
        note: Option<&str>,
    ) -> (Option<f64>, Option<String>, String, Option<String>) {
        (
            Some(amount),
            Some(unit.to_string()),
            name.to_string(),
            note.map(str::to_string),
        )
    }

    fn unmeasured(
        name: &str,
        note: Option<&str>,
    ) -> (Option<f64>, Option<String>, String, Option<String>) {
        (None, None, name.to_string(), note.map(str::to_string))
    }

    #[test]
    fn measures_come_off_the_front() {
        assert_eq!(read("2 cups flour"), line(2.0, "cup", "flour", None));
        assert_eq!(
            read("1 tablespoon olive oil"),
            line(1.0, "tablespoon", "olive oil", None)
        );
        assert_eq!(
            read("3 TBSP soy sauce"),
            line(3.0, "tablespoon", "soy sauce", None)
        );
        assert_eq!(
            read("1 tsp. ground cumin"),
            line(1.0, "teaspoon", "ground cumin", None)
        );
        assert_eq!(
            read("8 oz spaghetti"),
            line(8.0, "ounce", "spaghetti", None)
        );
        assert_eq!(
            read("1 lb ground beef"),
            line(1.0, "pound", "ground beef", None)
        );
        assert_eq!(
            read("4 fl oz cream"),
            line(4.0, "fluid ounce", "cream", None)
        );
        assert_eq!(
            read("250 ml whole milk"),
            line(250.0, "milliliter", "whole milk", None)
        );
        assert_eq!(read("2 c. rice"), line(2.0, "cup", "rice", None));
        assert_eq!(read("2 T butter"), line(2.0, "tablespoon", "butter", None));
        assert_eq!(
            read("1 cup (240 ml) milk"),
            line(1.0, "cup", "milk", Some("240 ml"))
        );
        assert_eq!(read("1 t salt"), line(1.0, "teaspoon", "salt", None));
    }

    #[test]
    fn numbers_read_as_recipes_write_them() {
        assert_eq!(read("½ cup sugar"), line(0.5, "cup", "sugar", None));
        assert_eq!(read("1½ cups stock"), line(1.5, "cup", "stock", None));
        assert_eq!(read("1 1/2 cups stock"), line(1.5, "cup", "stock", None));
        assert_eq!(read("3/4 cup oats"), line(0.75, "cup", "oats", None));
        assert_eq!(
            read("1.5 kg potatoes"),
            line(1.5, "kilogram", "potatoes", None)
        );
        assert_eq!(read("200g feta"), line(200.0, "gram", "feta", None));
        assert_eq!(read("two eggs"), line(2.0, "unit", "eggs", None));
        assert_eq!(read("half a lemon"), line(0.5, "unit", "lemon", None));
        assert_eq!(read("a pinch of salt"), line(1.0, "pinch", "salt", None));
    }

    #[test]
    fn a_range_takes_its_lower_end() {
        assert_eq!(
            read("2-3 cloves garlic"),
            line(2.0, "clove", "garlic", None)
        );
        assert_eq!(read("2 to 3 carrots"), line(2.0, "unit", "carrots", None));
        assert_eq!(
            read("1 – 2 tsp chili flakes"),
            line(1.0, "teaspoon", "chili flakes", None)
        );
    }

    #[test]
    fn a_bare_number_counts_the_food() {
        assert_eq!(read("2 eggs"), line(2.0, "unit", "eggs", None));
        assert_eq!(read("2 large eggs"), line(2.0, "unit", "large eggs", None));
        assert_eq!(
            read("1 yellow onion, diced"),
            line(1.0, "unit", "yellow onion", Some("diced"))
        );
    }

    #[test]
    fn counted_items_keep_their_noun() {
        assert_eq!(read("2 cloves garlic"), line(2.0, "clove", "garlic", None));
        assert_eq!(
            read("2 cloves of garlic"),
            line(2.0, "clove", "garlic", None)
        );
        assert_eq!(
            read("1 can black beans"),
            line(1.0, "can", "black beans", None)
        );
        assert_eq!(read("3 sprigs thyme"), line(3.0, "sprig", "thyme", None));
        assert_eq!(
            read("2 large cloves garlic, minced"),
            line(2.0, "clove", "garlic", Some("large; minced"))
        );
    }

    #[test]
    fn package_sizes_become_notes() {
        assert_eq!(
            read("1 (14 oz) can diced tomatoes, drained"),
            line(1.0, "can", "diced tomatoes", Some("14 oz; drained"))
        );
        assert_eq!(
            read("2 14-ounce cans chickpeas"),
            line(2.0, "can", "chickpeas", Some("14-ounce"))
        );
        assert_eq!(
            read("1-inch piece ginger, grated"),
            line(1.0, "piece", "ginger", Some("1-inch; grated"))
        );
    }

    #[test]
    fn preparation_and_asides_become_notes() {
        assert_eq!(
            read("1 cup flour, sifted"),
            line(1.0, "cup", "flour", Some("sifted"))
        );
        assert_eq!(
            read("1 cup walnuts (optional)"),
            line(1.0, "cup", "walnuts", Some("optional"))
        );
        assert_eq!(
            read("1 onion, peeled, halved (keep the root on)"),
            line(
                1.0,
                "unit",
                "onion",
                Some("peeled, halved; keep the root on")
            )
        );
        assert_eq!(
            read("2 limes for serving"),
            line(2.0, "unit", "limes", Some("for serving"))
        );
    }

    #[test]
    fn a_line_without_an_amount_is_all_name() {
        assert_eq!(read("Salt"), unmeasured("Salt", None));
        assert_eq!(
            read("salt and pepper to taste"),
            unmeasured("salt and pepper", Some("to taste"))
        );
        assert_eq!(
            read("Olive oil, for frying"),
            unmeasured("Olive oil", Some("for frying"))
        );
        assert_eq!(
            read("a few sprigs of thyme"),
            unmeasured("a few sprigs of thyme", None)
        );
        assert_eq!(read("7up"), unmeasured("7up", None));
        assert_eq!(read("0 sugar"), unmeasured("sugar", None));
    }

    #[test]
    fn a_unit_with_nothing_after_it_is_the_food() {
        assert_eq!(read("2 cloves"), line(2.0, "unit", "cloves", None));
        assert_eq!(read("one"), unmeasured("one", None));
    }

    #[test]
    fn list_markers_and_stray_spaces_come_off() {
        assert_eq!(read("-  2  cups   flour "), line(2.0, "cup", "flour", None));
        assert_eq!(read("• 1 egg"), line(1.0, "unit", "egg", None));
        assert_eq!(read(""), unmeasured("", None));
    }

    #[test]
    fn every_unit_word_reads_back_as_its_unit() {
        for text in [
            "1 tsp x",
            "1 tbsp x",
            "1 cup x",
            "1 fl oz x",
            "1 ml x",
            "1 l x",
            "1 oz x",
            "1 lb x",
            "1 g x",
            "1 kg x",
            "1 pinch x",
            "1 clove x",
            "1 x",
        ] {
            let line = parse(text);
            assert_eq!(Unit::parse(&line.unit_word().unwrap()), line.unit, "{text}");
        }
    }

    #[test]
    fn no_text_makes_it_panic() {
        for text in [
            "(",
            ")",
            "((",
            "1 (",
            "1 ()",
            "1-",
            "1 -",
            "-1",
            "½",
            "1 ½ (",
            ",",
            "1,",
            "1 cup,",
            "a",
            "an",
            "half",
            "half a",
            "1 to",
            "1 to 2",
            "1-2",
            "fl oz",
            "1 fl oz",
            "1 fl",
            "１ cup",
            "1 cup ünïcode, ñ",
            "1\u{a0}cup flour",
            "🍅 2 tomatoes",
            "2 🍅",
            "to taste",
            "1 to taste",
        ] {
            parse(text);
        }
    }
}
