//! Times in step text: "simmer 8-10 minutes", "rest for 1 hour 15 minutes", "cook a minute more". Kitchen mode
//! makes each one a button that starts a timer. A range's timer runs to its lower end, which is when to start
//! checking; the upper end is kept so the alarm can say how much longer the step allows.

use std::sync::LazyLock;

use regex::{Captures, Regex};

use crate::quantity::parse_number;

/// A time a step asks for, and where the step says it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepTime {
    /// Where the time starts in the step's text, in bytes: the "8" of "8-10 minutes".
    pub start: usize,
    /// Where it ends, in bytes: just past "minutes".
    pub end: usize,
    /// How long to set a timer for, in seconds: the time itself, or a range's lower end (480 for "8-10 minutes").
    pub seconds: u32,
    /// A range's upper end, in seconds (600 for "8-10 minutes"); `None` for a single time.
    pub until: Option<u32>,
}

/// A number written with digits: "2", "1.5", "½", "1½", "1 1/2", "3/4". The word boundary keeps it from
/// starting inside a word or a longer number; a lone glyph has no boundary to ask for.
const DIGITS: &str =
    r"\b\d+(?:\.\d+)?(?:\s?[½⅓⅔¼¾⅛⅜⅝⅞]|\s\d+\s*[⁄/∕]\s*\d+)?|\b\d+\s*[⁄/∕]\s*\d+|[½⅓⅔¼¾⅛⅜⅝⅞]";

/// The numbers recipes spell out before a unit, longest first so "forty-five" isn't read as "forty".
const WORDS: &[(&str, f64)] = &[
    ("half an", 0.5),
    ("half a", 0.5),
    ("twenty-five", 25.0),
    ("twenty five", 25.0),
    ("forty-five", 45.0),
    ("forty five", 45.0),
    ("fifteen", 15.0),
    ("twelve", 12.0),
    ("twenty", 20.0),
    ("thirty", 30.0),
    ("ninety", 90.0),
    ("forty", 40.0),
    ("fifty", 50.0),
    ("sixty", 60.0),
    ("three", 3.0),
    ("seven", 7.0),
    ("eight", 8.0),
    ("four", 4.0),
    ("five", 5.0),
    ("nine", 9.0),
    ("one", 1.0),
    ("two", 2.0),
    ("six", 6.0),
    ("ten", 10.0),
    ("an", 1.0),
    ("a", 1.0),
];

/// Seconds, minutes and hours as steps write them. Days are left alone: nobody sets a kitchen timer for two.
const UNIT: &str = r"seconds?|secs?|minutes?|mins?|hours?|hrs?";

/// What may stand between a number and its unit: a space, the hyphen of "a 10-minute rest", or "more" and its
/// kin ("2 more minutes").
const GAP: &str = r"(?:\s+(?:more|additional|extra)\s+|\s+|-)?";

/// A time, with the lower end of a range before it when there is one: "8-10 minutes", "10 to 12 minutes",
/// "45 minutes to 1 hour", "5 minutes".
static TIME: LazyLock<Regex> = LazyLock::new(|| {
    let words = WORDS
        .iter()
        .map(|(w, _)| w.replace(' ', r"\s+"))
        .collect::<Vec<_>>()
        .join("|");
    let num = format!(r"{DIGITS}|\b(?:{words})\b");
    Regex::new(&format!(
        r"(?i)(?:(?P<a>{num})(?:{GAP}(?P<au>{UNIT})\b)?(?P<join>\s*[-–—]\s*|\s+(?:to|or)\s+))?(?P<b>{num}){GAP}(?P<bu>{UNIT})\b"
    ))
    .unwrap()
});

/// The smaller part of a time in two units, read just after the larger: " 15 minutes" in "1 hour 15 minutes",
/// " and 30 seconds" in "2 minutes and 30 seconds".
static SMALLER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"(?i)^\s+(?:and\s+)?(?P<n>{DIGITS}){GAP}(?P<u>{UNIT})\b"
    ))
    .unwrap()
});

/// The longest time a timer is set for: four days, more than any step asks and well inside a `u32` of seconds.
const LONGEST: f64 = 4.0 * 24.0 * 3600.0;

/// How many seconds one of a unit is: 60 for "minutes" or "min".
fn unit_seconds(unit: &str) -> f64 {
    match unit.as_bytes()[0].to_ascii_lowercase() {
        b's' => 1.0,
        b'm' => 60.0,
        _ => 3600.0,
    }
}

/// A number as the step wrote it, and whether it was the article of "a minute" or "an hour".
fn number(written: &str) -> Option<(f64, bool)> {
    let lower = written.to_lowercase();
    let spaced = lower.split_whitespace().collect::<Vec<_>>().join(" ");
    if let Some((_, value)) = WORDS.iter().find(|(w, _)| *w == spaced) {
        return Some((
            *value,
            matches!(spaced.as_str(), "a" | "an" | "half a" | "half an"),
        ));
    }
    let (value, used) = parse_number(&spaced)?;
    (used == spaced.len()).then_some((value, false))
}

/// A number of a unit in whole seconds, when it is a time worth a timer: at least a second, at most
/// [`LONGEST`]. An article counts only before a whole "minute" or "hour": "a minute more" is a time, "a second
/// bowl" is a bowl.
fn seconds(written: &str, unit: &str) -> Option<u32> {
    let (value, article) = number(written)?;
    if article && !matches!(unit.to_lowercase().as_str(), "minute" | "hour") {
        return None;
    }
    let total = (value * unit_seconds(unit)).round();
    (1.0..=LONGEST).contains(&total).then_some(total as u32)
}

/// Whether a match's first number is the tail of something else: the "5" of ".5" or of "1/5".
fn mid_number(text: &str, start: usize) -> bool {
    let mut before = text[..start].chars().rev();
    match before.next() {
        Some('⁄' | '/' | '∕') => true,
        Some('.') => before.next().is_some_and(|c| c.is_ascii_digit()),
        _ => false,
    }
}

/// The time one match of [`TIME`] names, read as a range when it gives a lower end that makes sense, and as the
/// single time before the unit otherwise.
fn read(text: &str, c: &Captures) -> Option<StepTime> {
    let b = c.name("b")?;
    let unit = c.name("bu")?;
    let upper = seconds(b.as_str(), unit.as_str())?;
    let mut time = StepTime {
        start: b.start(),
        end: unit.end(),
        seconds: upper,
        until: None,
    };
    if let (Some(a), Some(join)) = (c.name("a"), c.name("join")) {
        let own_unit = c.name("au");
        // "2-1/2 hours" is two and a half, not a range that runs backwards.
        let fraction = number(b.as_str()).is_some_and(|(v, _)| v < 1.0);
        let whole = number(a.as_str()).filter(|(v, article)| !article && v.fract() == 0.0);
        // "forty-five minutes" is one number with a hyphen in it.
        let one_number = own_unit
            .is_none()
            .then(|| seconds(&text[a.start()..b.end()], unit.as_str()))
            .flatten();
        if let Some(total) = one_number {
            time.start = a.start();
            time.seconds = total;
        } else if let (true, Some((whole, _)), None) =
            (fraction && join.as_str().trim() == "-", whole, own_unit)
        {
            let total = (whole * unit_seconds(unit.as_str())).round() + f64::from(upper);
            if total <= LONGEST {
                time.start = a.start();
                time.seconds = total as u32;
            }
        } else if let Some(lower) =
            seconds(a.as_str(), own_unit.map_or(unit.as_str(), |u| u.as_str()))
            && lower < upper
            && number(a.as_str()).is_some_and(|(_, article)| !article)
        {
            time.start = a.start();
            time.seconds = lower;
            time.until = Some(upper);
        }
    }
    if mid_number(text, time.start) {
        return None;
    }
    // "1 hour 15 minutes": a single time may go on in a smaller unit.
    if time.until.is_none()
        && let Some(more) = SMALLER.captures(&text[time.end..])
        && let (Some(n), Some(u)) = (more.name("n"), more.name("u"))
        && unit_seconds(u.as_str()) < unit_seconds(unit.as_str())
        && let Some(extra) = seconds(n.as_str(), u.as_str())
        && f64::from(time.seconds) + f64::from(extra) <= LONGEST
    {
        time.seconds += extra;
        time.end += u.end();
    }
    Some(time)
}

/// Every time a step's text asks for, in the order it reads.
///
/// ```
/// use mise_core::step_times::find;
///
/// let step = "Stir in the tomatoes. Simmer until thick, 8-10 minutes.";
/// let times = find(step);
/// assert_eq!(times.len(), 1);
/// assert_eq!(&step[times[0].start..times[0].end], "8-10 minutes");
/// // A range's timer runs to its lower end, when it's time to check; the upper end says how long it may go on.
/// assert_eq!(times[0].seconds, 8 * 60);
/// assert_eq!(times[0].until, Some(10 * 60));
///
/// // A single time has no upper end, and may be written in two units.
/// let roast = find("Roast for 1 hour 15 minutes, then rest a minute.");
/// assert_eq!(roast[0].seconds, 75 * 60);
/// assert_eq!(roast[0].until, None);
/// assert_eq!(roast[1].seconds, 60);
///
/// // Amounts, temperatures and sizes are not times.
/// assert_eq!(find("Heat the oven to 425 degrees. Cut 2 onions into 1-inch wedges."), []);
/// ```
pub fn find(text: &str) -> Vec<StepTime> {
    let mut times = Vec::new();
    let mut at = 0;
    while let Some(c) = TIME.captures_at(text, at) {
        let whole = c.get(0).expect("a match has a group 0");
        match read(text, &c) {
            Some(time) => {
                at = time.end;
                times.push(time);
            }
            // Not a time after all: look again from the next character, so a time just inside this match is found.
            None => {
                let first = text[whole.start()..]
                    .chars()
                    .next()
                    .map_or(1, char::len_utf8);
                at = whole.start() + first;
            }
        }
    }
    times
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each time found, as the step wrote it, with its timer and a range's upper end in seconds.
    fn found(text: &str) -> Vec<(&str, u32, Option<u32>)> {
        find(text)
            .into_iter()
            .map(|t| (&text[t.start..t.end], t.seconds, t.until))
            .collect()
    }

    #[test]
    fn a_range_starts_at_its_lower_end_and_keeps_its_upper() {
        assert_eq!(
            found("Add the onion and cook until soft, 5-6 minutes."),
            [("5-6 minutes", 300, Some(360))]
        );
        assert_eq!(
            found("Bake 10 to 12 minutes, until golden."),
            [("10 to 12 minutes", 600, Some(720))]
        );
        assert_eq!(
            found("Sear 2–3 min per side."),
            [("2–3 min", 120, Some(180))]
        );
        assert_eq!(
            found("Toast the crumbs 1 or 2 minutes."),
            [("1 or 2 minutes", 60, Some(120))]
        );
    }

    #[test]
    fn a_range_may_change_units() {
        assert_eq!(
            found("Braise 45 minutes to 1 hour, until the meat pulls apart."),
            [("45 minutes to 1 hour", 2700, Some(3600))]
        );
        assert_eq!(
            found("Chill 1 to 1½ hours."),
            [("1 to 1½ hours", 3600, Some(5400))]
        );
    }

    #[test]
    fn single_times_in_seconds_minutes_and_hours() {
        assert_eq!(
            found("Add the garlic and cook 30 seconds. Cover and cook 1 minute more."),
            [("30 seconds", 30, None), ("1 minute", 60, None)]
        );
        assert_eq!(found("Simmer for 2 hrs."), [("2 hrs", 7200, None)]);
        assert_eq!(found("Rest 10 mins."), [("10 mins", 600, None)]);
        assert_eq!(found("Blend for 45 secs."), [("45 secs", 45, None)]);
    }

    #[test]
    fn fractions_and_decimals_of_a_unit() {
        assert_eq!(found("Roast 1½ hours."), [("1½ hours", 5400, None)]);
        assert_eq!(found("Roast 1 1/2 hours."), [("1 1/2 hours", 5400, None)]);
        assert_eq!(found("Roast 1.5 hours."), [("1.5 hours", 5400, None)]);
        assert_eq!(found("Steep ½ hour."), [("½ hour", 1800, None)]);
        assert_eq!(found("Boil 2½ minutes."), [("2½ minutes", 150, None)]);
        // A hyphen before a fraction joins a mixed number; it isn't a range that runs backwards.
        assert_eq!(found("Roast 2-1/2 hours."), [("2-1/2 hours", 9000, None)]);
    }

    #[test]
    fn two_units_make_one_time() {
        assert_eq!(
            found("Bake 1 hour 15 minutes."),
            [("1 hour 15 minutes", 4500, None)]
        );
        assert_eq!(
            found("Boil the eggs 6 minutes and 30 seconds."),
            [("6 minutes and 30 seconds", 390, None)]
        );
        assert_eq!(
            found("Proof for an hour and 20 minutes."),
            [("an hour and 20 minutes", 4800, None)]
        );
        // A larger unit after a smaller one starts a new time.
        assert_eq!(
            found("Stir 5 minutes 1 hour before serving."),
            [("5 minutes", 300, None), ("1 hour", 3600, None)]
        );
    }

    #[test]
    fn numbers_written_as_words() {
        assert_eq!(
            found("Let the dough rest for twenty minutes."),
            [("twenty minutes", 1200, None)]
        );
        assert_eq!(
            found("Fry five to six minutes, turning once."),
            [("five to six minutes", 300, Some(360))]
        );
        assert_eq!(
            found("Cook a minute more, then rest half an hour."),
            [("a minute", 60, None), ("half an hour", 1800, None)]
        );
        assert_eq!(
            found("Simmer Forty-Five Minutes."),
            [("Forty-Five Minutes", 2700, None)]
        );
    }

    #[test]
    fn a_time_used_as_a_describing_word() {
        assert_eq!(
            found("Give the steak a 10-minute rest."),
            [("10-minute", 600, None)]
        );
        assert_eq!(
            found("Microwave in 30-second bursts."),
            [("30-second", 30, None)]
        );
    }

    #[test]
    fn more_minutes_are_minutes() {
        assert_eq!(
            found("Flip and cook 2 more minutes."),
            [("2 more minutes", 120, None)]
        );
        assert_eq!(
            found("Cook 3 to 4 additional minutes."),
            [("3 to 4 additional minutes", 180, Some(240))]
        );
    }

    #[test]
    fn several_times_in_one_step_read_in_order() {
        let step = "Cook the chicken until browned, 5-7 minutes per side. Add the garlic and cook 30 seconds; \
                    simmer until reduced by half, 2-3 minutes.";
        assert_eq!(
            found(step),
            [
                ("5-7 minutes", 300, Some(420)),
                ("30 seconds", 30, None),
                ("2-3 minutes", 120, Some(180)),
            ]
        );
    }

    #[test]
    fn amounts_sizes_and_temperatures_are_not_times() {
        for text in [
            "Heat 1 TBSP olive oil in a large pan over medium heat.",
            "Heat the oven to 425 degrees.",
            "Cut the potatoes into 1- to 2-inch pieces.",
            "Divide between 2 to 4 bowls.",
            "Mix the sour cream in a second bowl.",
            "Add a second handful of spinach.",
            "Stir in the last-minute herbs.",
            "Marinate for 2 days.",
            "Cook a few minutes, until fragrant.",
            "Cook 0 minutes.",
        ] {
            assert_eq!(found(text), [], "{text}");
        }
    }

    #[test]
    fn a_number_before_the_range_is_left_out_of_it() {
        // "2 to" belongs to the bowls, not to the minutes.
        assert_eq!(
            found("Divide between 2 bowls and rest 3 to 4 minutes."),
            [("3 to 4 minutes", 180, Some(240))]
        );
        // A range that would run backwards is only its last time.
        assert_eq!(
            found("Turn at 10 or 5 minutes before the end."),
            [("5 minutes", 300, None)]
        );
    }

    #[test]
    fn a_time_never_starts_inside_a_word_or_a_number() {
        assert_eq!(found("Use the v2 minutes setting."), []);
        // "ten" inside "written" is not a number, and the time after it is still found.
        assert_eq!(
            found("As written or 5 minutes longer."),
            [("5 minutes", 300, None)]
        );
        assert_eq!(found("Soften 4 minutes."), [("4 minutes", 240, None)]);
    }

    #[test]
    fn a_time_too_long_for_a_timer_is_left_alone() {
        assert_eq!(found("Age for 5000 hours."), []);
        assert_eq!(
            found("Cure 96 hours, then 97 hours more."),
            [("96 hours", 345_600, None)]
        );
    }

    #[test]
    fn positions_are_byte_offsets_on_character_boundaries() {
        let step = "Reserve ½ cup cooking water, then simmer 8–10 minutes.";
        let times = find(step);
        assert_eq!(times.len(), 1);
        assert_eq!(&step[times[0].start..times[0].end], "8–10 minutes");
    }

    #[test]
    fn any_text_is_read_without_panicking() {
        // Pieces a step might hold, in every order of three: each time found lies on character boundaries,
        // in reading order, and is at least a second long.
        const PIECES: &[&str] = &[
            "1",
            "10",
            "0",
            "1.5",
            ".5",
            "½",
            "1½",
            "1/2",
            "⅞",
            "-",
            "–",
            " to ",
            " or ",
            " and ",
            " ",
            "a",
            "an",
            "half an",
            "ten",
            "minutes",
            "minute",
            "min",
            "hour",
            "hrs",
            "seconds",
            "sec",
            "more ",
            "é",
            "🍋",
            ".",
            "/",
            "(",
            ")",
            "99999999999999999999",
        ];
        let n = PIECES.len();
        for i in 0..n.pow(3) {
            let text: String = [i, i / n, i / n / n]
                .iter()
                .map(|k| PIECES[k % n])
                .collect();
            let mut last = 0;
            for t in find(&text) {
                assert!(t.start >= last && t.start < t.end, "{text:?}: {t:?}");
                assert!(text.is_char_boundary(t.start) && text.is_char_boundary(t.end));
                assert!(t.seconds >= 1, "{text:?}: {t:?}");
                assert!(t.until.is_none_or(|u| u > t.seconds), "{text:?}: {t:?}");
                last = t.end;
            }
        }
    }
}
