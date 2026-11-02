//! Kitchen quantities: reading amounts as recipes write them ("1 1⁄2 TBSP", "¾ cup", "2 cloves"), converting
//! between units, scaling, and rounding to amounts a cook can measure.

use std::fmt;

use serde::{Deserialize, Serialize};

/// A unit as cooks use it. Volume and weight units convert within their family; counts never convert.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Unit {
    Tsp,
    Tbsp,
    Cup,
    FlOz,
    Ml,
    L,
    Oz,
    Lb,
    G,
    Kg,
    Pinch,
    /// A countable item, holding its noun as written ("clove", "wedge"); empty for a bare number.
    Count(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    Volume,
    Weight,
    Count,
    Pinch,
}

impl Unit {
    /// Recognise a unit word as recipes and the seed data spell it.
    pub fn parse(word: &str) -> Option<Unit> {
        let w = word.trim().trim_end_matches('.').to_lowercase();
        let w = w.split(" (").next().unwrap_or(&w); // "teaspoon (tsp)", "ounce(oz)" → the first word
        let w = w.split('(').next().unwrap_or(w).trim();
        Some(match w {
            "tsp" | "tsps" | "teaspoon" | "teaspoons" => Unit::Tsp,
            "tbsp" | "tbsps" | "tablespoon" | "tablespoons" | "tbs" => Unit::Tbsp,
            "cup" | "cups" => Unit::Cup,
            "fl oz" | "fluid ounce" | "fluid ounces" => Unit::FlOz,
            "ml" | "milliliter" | "milliliters" | "millilitre" | "millilitres" => Unit::Ml,
            "l" | "liter" | "liters" | "litre" | "litres" => Unit::L,
            "oz" | "ounce" | "ounces" => Unit::Oz,
            "lb" | "lbs" | "pound" | "pounds" => Unit::Lb,
            "g" | "gram" | "grams" => Unit::G,
            "kg" | "kilogram" | "kilograms" => Unit::Kg,
            "pinch" | "pinches" => Unit::Pinch,
            "unit" | "units" | "pick" | "picks" | "" => Unit::Count(String::new()),
            "clove" | "cloves" | "thumb" | "thumbs" | "slice" | "slices" | "piece" | "pieces" | "sprig" | "sprigs"
            | "can" | "cans" | "jar" | "jars" | "box" | "boxes" | "bunch" | "bunches" | "head" | "heads" | "stalk"
            | "stalks" | "pack" | "packs" | "bun" | "buns" | "tube" | "tubes" | "wedge" | "wedges" => {
                Unit::Count(singular(w).to_string())
            }
            _ => return None,
        })
    }

    pub fn family(&self) -> Family {
        match self {
            Unit::Tsp | Unit::Tbsp | Unit::Cup | Unit::FlOz | Unit::Ml | Unit::L => Family::Volume,
            Unit::Oz | Unit::Lb | Unit::G | Unit::Kg => Family::Weight,
            Unit::Pinch => Family::Pinch,
            Unit::Count(_) => Family::Count,
        }
    }

    /// Size in the family's base unit: teaspoons for volume, grams for weight, 1 otherwise.
    fn size(&self) -> f64 {
        match self {
            Unit::Tsp => 1.0,
            Unit::Tbsp => 3.0,
            Unit::Cup => 48.0,
            Unit::FlOz => 6.0,
            Unit::Ml => 1.0 / 4.929,
            Unit::L => 1000.0 / 4.929,
            Unit::Oz => 28.3495,
            Unit::Lb => 453.592,
            Unit::G => 1.0,
            Unit::Kg => 1000.0,
            Unit::Pinch | Unit::Count(_) => 1.0,
        }
    }

    /// Two units measure the same kind of thing (count nouns must match).
    pub fn compatible(&self, other: &Unit) -> bool {
        match (self, other) {
            (Unit::Count(a), Unit::Count(b)) => a == b || a.is_empty() || b.is_empty(),
            _ => self.family() == other.family(),
        }
    }
}

fn plural(noun: &str) -> String {
    match noun {
        "whole" => noun.to_string(),
        "loaf" => "loaves".to_string(),
        _ if noun.ends_with("ch") || noun.ends_with('x') => format!("{noun}es"),
        _ => format!("{noun}s"),
    }
}

fn singular(w: &str) -> &str {
    match w {
        "boxes" => "box",
        "bunches" => "bunch",
        _ => w.strip_suffix('s').unwrap_or(w),
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Quantity {
    pub amount: f64,
    pub unit: Unit,
}

impl Quantity {
    pub fn new(amount: f64, unit: Unit) -> Self {
        Quantity { amount, unit }
    }

    pub fn scaled(&self, factor: f64) -> Quantity {
        Quantity { amount: self.amount * factor, unit: self.unit.clone() }
    }

    /// The amount expressed in `unit`, when the two convert.
    pub fn in_unit(&self, unit: &Unit) -> Option<f64> {
        self.unit.compatible(unit).then(|| self.amount * self.unit.size() / unit.size())
    }

    /// Same amount within 2% once both are in one unit.
    pub fn same_as(&self, other: &Quantity) -> bool {
        other.in_unit(&self.unit).is_some_and(|b| (self.amount - b).abs() <= 0.02 * self.amount.abs().max(b.abs()) + 1e-9)
    }

    /// The same amount in the clearest words, for a published amount: 3 tsp reads as 1 TBSP and 6 tsp as 2 TBSP,
    /// and a source's pinches stored as decimals (0.06 tsp, a "large pinch" as 0.12 tsp) read as pinches again.
    /// Measurable amounts are never moved: ⅛ tsp stays ⅛ tsp.
    pub fn tidied(&self) -> Quantity {
        let a = self.amount;
        match self.unit {
            Unit::Tsp if a >= 3.0 && is_multiple(a / 3.0, 0.5) => Quantity::new(a / 3.0, Unit::Tbsp),
            Unit::Tsp if a > 0.0 && a < 0.25 && (a - round_to(a, 0.125)).abs() > 0.004 => {
                Quantity::new((a / PINCH_TSP).round().max(1.0), Unit::Pinch)
            }
            _ => self.clone(),
        }
    }

    /// Round to something a cook can measure: spoons to ⅛ tsp (½ for tablespoons), cups to quarters and
    /// thirds, weights to the nearest half ounce, counts to halves. Moves between tsp, TBSP and cup when the
    /// amount reads better in the next unit.
    pub fn rounded(&self) -> Quantity {
        let a = self.amount;
        if a <= 0.0 {
            return self.clone();
        }
        match &self.unit {
            Unit::Tsp => {
                if a >= 3.0 && is_multiple(a / 3.0, 0.5) {
                    return Quantity::new(a / 3.0, Unit::Tbsp).rounded();
                }
                // Less than a sixteenth of a teaspoon is a pinch.
                if a < 0.0625 {
                    return Quantity::new(1.0, Unit::Pinch);
                }
                let step = if a < 1.0 { 0.125 } else { 0.25 };
                Quantity::new(round_to(a, step).max(0.125), Unit::Tsp)
            }
            Unit::Tbsp => {
                if a < 0.5 {
                    return Quantity::new(a * 3.0, Unit::Tsp).rounded();
                }
                if a >= 8.0 {
                    return Quantity::new(a / 16.0, Unit::Cup).rounded();
                }
                Quantity::new(round_to(a, 0.5), Unit::Tbsp)
            }
            Unit::Cup => {
                if a < 0.25 {
                    return Quantity::new(a * 16.0, Unit::Tbsp).rounded();
                }
                Quantity::new(round_cup(a), Unit::Cup)
            }
            // Nothing positive rounds to zero: small amounts move to the next smaller unit or keep the smallest
            // measurable step.
            // Below a quarter ounce (a few sprigs of herbs) the amount stays as small as it is.
            Unit::Oz | Unit::FlOz if a < 0.25 => Quantity::new(round_to(a, 0.01).max(0.01), self.unit.clone()),
            Unit::Oz | Unit::FlOz => Quantity::new(round_to(a, if a < 1.0 { 0.25 } else { 0.5 }), self.unit.clone()),
            Unit::Lb if a < 0.25 => Quantity::new(a * 16.0, Unit::Oz).rounded(),
            Unit::Lb => Quantity::new(round_to(a, 0.25), Unit::Lb),
            Unit::G | Unit::Ml => Quantity::new(round_to(a, if a >= 20.0 { 5.0 } else { 1.0 }).max(1.0), self.unit.clone()),
            Unit::Kg if a < 0.05 => Quantity::new(a * 1000.0, Unit::G).rounded(),
            Unit::L if a < 0.05 => Quantity::new(a * 1000.0, Unit::Ml).rounded(),
            Unit::Kg | Unit::L => Quantity::new(round_to(a, 0.05), self.unit.clone()),
            // Whole and half items; quarters below one; slivers (0.07 of a thumb of ginger) as they are.
            Unit::Count(_) if a < 0.25 => Quantity::new(round_to(a, 0.01).max(0.01), self.unit.clone()),
            Unit::Count(_) if a < 1.0 => Quantity::new(round_to(a, 0.25), self.unit.clone()),
            Unit::Count(_) => Quantity::new(round_to(a, 0.5), self.unit.clone()),
            Unit::Pinch => self.clone(),
        }
    }

    /// The amount as a cook measures it, never less: spoons and cups as one standard measure where that is at
    /// most 5% over, else the fewest that come closest ("1 tsp + 1 pinch"); weights up to the next half ounce
    /// (quarter below one), counts up to the next half (quarter below one).
    pub fn measured(&self) -> Measure {
        let a = self.amount;
        if a <= 0.0 {
            return Measure(vec![self.clone()]);
        }
        let up = |step: f64| vec![Quantity::new(ceil_to(a, step), self.unit.clone())];
        Measure(match &self.unit {
            Unit::Tsp | Unit::Tbsp | Unit::Cup => spoons(self.in_unit(&Unit::Tsp).unwrap_or(a)),
            Unit::Pinch => up(1.0),
            Unit::FlOz => up(0.5),
            Unit::Ml | Unit::G => up(if a >= 20.0 { 5.0 } else { 1.0 }),
            Unit::L | Unit::Kg if a < 0.05 => return Quantity::new(a * 1000.0, if self.unit == Unit::L { Unit::Ml } else { Unit::G }).measured(),
            Unit::L | Unit::Kg => up(0.05),
            Unit::Lb if a < 0.25 => return Quantity::new(a * 16.0, Unit::Oz).measured(),
            Unit::Lb => up(0.25),
            Unit::Oz | Unit::Count(_) if a < 0.25 => up(0.01),
            Unit::Oz | Unit::Count(_) if a < 1.0 => up(0.25),
            Unit::Oz | Unit::Count(_) => up(0.5),
        })
    }
}

/// A pinch, as sources convert it: a sixteenth of a teaspoon.
pub const PINCH_TSP: f64 = 1.0 / 16.0;

/// Sources store some measures a hair over (⅔ cup as 0.67), so an amount this close over one counts as it.
const SLACK: f64 = 0.006;
/// How far over one standard measure may go before a smaller one is added to it instead.
const SIMPLER: f64 = 0.05;

/// An amount as a cook measures it: one standard measure, or a few added up.
#[derive(Debug, Clone, PartialEq)]
pub struct Measure(pub Vec<Quantity>);

impl Measure {
    /// The whole amount in `unit`, when every part converts (a pinch counts as a sixteenth of a teaspoon).
    pub fn in_unit(&self, unit: &Unit) -> Option<f64> {
        self.0
            .iter()
            .map(|q| match q.unit {
                Unit::Pinch => Quantity::new(q.amount * PINCH_TSP, Unit::Tsp).in_unit(unit),
                _ => q.in_unit(unit),
            })
            .sum()
    }
}

impl fmt::Display for Measure {
    /// "1 tsp + 1 pinch", "¼ cup + 1 TBSP".
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let parts: Vec<String> = self.0.iter().map(Quantity::to_string).collect();
        f.write_str(&parts.join(" + "))
    }
}

/// `a` up to the next multiple of `step`, at least one step.
fn ceil_to(a: f64, step: f64) -> f64 {
    (a * (1.0 - SLACK) / step).ceil().max(1.0) * step
}

/// Cups, tablespoons, teaspoons and a pinch adding up to at least `tsp`: the fewest measures within 5% over,
/// else the closest.
fn spoons(tsp: f64) -> Vec<Quantity> {
    // Largest first, so that of two equally good answers the one in bigger measures is kept.
    const CUP_PARTS: [f64; 6] = [0.75, 2.0 / 3.0, 0.5, 1.0 / 3.0, 0.25, 0.0];
    const TBSP: [f64; 7] = [3.5, 3.0, 2.5, 2.0, 1.5, 1.0, 0.0];
    const TSP: [f64; 13] = [2.75, 2.5, 2.25, 2.0, 1.75, 1.5, 1.25, 1.0, 0.75, 0.5, 0.25, 0.125, 0.0];
    if !tsp.is_finite() || tsp > 1e6 {
        return vec![Quantity::new(tsp / 48.0, Unit::Cup)];
    }
    let need = tsp * (1.0 - SLACK);
    let within = tsp * (1.0 + SIMPLER) + 1e-9;
    // (parts, total, measures): the best within 5% by fewest parts, and the closest overall.
    let mut simple: Option<(usize, f64, [f64; 4])> = None;
    let mut closest: Option<(usize, f64, [f64; 4])> = None;
    // Fewer whole cups than this can't reach the amount with the smaller measures.
    let fewest = ((need - 48.0) / 48.0).floor().max(0.0) as u32;
    for whole in (fewest..=(tsp / 48.0).ceil() as u32).rev() {
        for frac in CUP_PARTS {
            let cups = whole as f64 + frac;
            for t in TBSP {
                for s in TSP {
                    for p in [0.0, 1.0] {
                        let total = cups * 48.0 + t * 3.0 + s + p * PINCH_TSP;
                        if total < need - 1e-9 || total <= 0.0 {
                            continue;
                        }
                        let m = [cups, t, s, p];
                        let parts = m.iter().filter(|x| **x > 0.0).count();
                        if total <= within && simple.is_none_or(|b| (parts, total) < (b.0, b.1 - 1e-9)) {
                            simple = Some((parts, total, m));
                        }
                        if closest.is_none_or(|b| total < b.1 - 1e-9 || (total < b.1 + 1e-9 && parts < b.0)) {
                            closest = Some((parts, total, m));
                        }
                    }
                }
            }
        }
    }
    let Some((_, _, [cups, t, s, p])) = simple.or(closest) else { return vec![Quantity::new(tsp, Unit::Tsp)] };
    [(cups, Unit::Cup), (t, Unit::Tbsp), (s, Unit::Tsp), (p, Unit::Pinch)]
        .into_iter()
        .filter(|(a, _)| *a > 0.0)
        .map(|(a, u)| Quantity::new(a, u))
        .collect()
}

fn round_to(a: f64, step: f64) -> f64 {
    (a / step).round() * step
}

fn is_multiple(a: f64, step: f64) -> bool {
    ((a / step) - (a / step).round()).abs() < 1e-6
}

/// Cups round to the nearest quarter or third.
fn round_cup(a: f64) -> f64 {
    let whole = a.floor();
    let frac = a - whole;
    let best = [0.0, 0.25, 1.0 / 3.0, 0.5, 2.0 / 3.0, 0.75, 1.0]
        .into_iter()
        .min_by(|x, y| (x - frac).abs().total_cmp(&(y - frac).abs()))
        .unwrap();
    whole + best
}

const GLYPHS: &[(f64, &str)] = &[
    (0.125, "⅛"),
    (0.25, "¼"),
    (1.0 / 3.0, "⅓"),
    (0.375, "⅜"),
    (0.5, "½"),
    (0.625, "⅝"),
    (2.0 / 3.0, "⅔"),
    (0.75, "¾"),
    (0.875, "⅞"),
];

/// "1½", "¾", "2"; falls back to a trimmed decimal when no kitchen fraction fits. A positive amount never
/// reads as "0" (some sources record traces such as 0.004 oz).
pub fn format_amount(a: f64) -> String {
    if a > 0.0 && a < 0.01 {
        return "<0.01".to_string();
    }
    let whole = a.floor();
    let frac = a - whole;
    // Within 2% of a kitchen fraction reads as that fraction (a published 0.99 TBSP is 1 TBSP); a positive
    // amount never reads as 0.
    if frac < 0.02 && whole >= 1.0 {
        return format!("{whole}");
    }
    if frac > 0.98 {
        return format!("{}", whole + 1.0);
    }
    match GLYPHS.iter().find(|(v, _)| (v - frac).abs() < 0.021) {
        Some((_, g)) if whole == 0.0 => (*g).to_string(),
        Some((_, g)) => format!("{whole}{g}"),
        None => {
            let s = format!("{a:.2}");
            s.trim_end_matches('0').trim_end_matches('.').to_string()
        }
    }
}

/// The amount as it is: a kitchen fraction only where it is one (within a source's two-decimal rounding, so
/// 0.33 is ⅓), else up to two decimals.
pub fn format_exact(a: f64) -> String {
    let whole = a.floor();
    let frac = a - whole;
    if frac < 0.005 || frac > 0.995 {
        return format!("{}", a.round());
    }
    match GLYPHS.iter().find(|(v, _)| (v - frac).abs() < 0.005) {
        Some((_, g)) if whole == 0.0 => (*g).to_string(),
        Some((_, g)) => format!("{whole}{g}"),
        None => decimal(a),
    }
}

/// Up to two decimals, trailing zeros dropped; never "0" for a positive amount.
pub fn decimal(a: f64) -> String {
    if a > 0.0 && a < 0.005 {
        return "0.01".to_string();
    }
    let s = format!("{a:.2}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

impl Quantity {
    /// Like the display, but the amount exactly as it is: "1.3 cups", "2¼ oz".
    pub fn exact(&self) -> String {
        self.worded(format_exact(self.amount))
    }

    fn worded(&self, n: String) -> String {
        let plural = self.amount > 1.0 + 1e-9;
        let unit = match &self.unit {
            Unit::Tsp => "tsp".to_string(),
            Unit::Tbsp => "TBSP".to_string(),
            Unit::Cup => if plural { "cups" } else { "cup" }.to_string(),
            Unit::FlOz => "fl oz".to_string(),
            Unit::Ml => "ml".to_string(),
            Unit::L => "L".to_string(),
            Unit::Oz => "oz".to_string(),
            Unit::Lb => "lb".to_string(),
            Unit::G => "g".to_string(),
            Unit::Kg => "kg".to_string(),
            Unit::Pinch => if plural { "pinches" } else { "pinch" }.to_string(),
            Unit::Count(noun) if noun.is_empty() => return n,
            Unit::Count(noun) => {
                if plural {
                    self::plural(noun)
                } else {
                    noun.clone()
                }
            }
        };
        format!("{n} {unit}")
    }
}

impl fmt::Display for Quantity {
    /// In the house style of the seed's step text: "1½ TBSP", "¾ cup", "2 cups", "1 tsp", "10 oz".
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.worded(format_amount(self.amount)))
    }
}

/// Read a number at the start of `s` as recipes write it: "2", "1.5", "½", "1½", "1 1⁄2", "3/4", "1 ½".
/// Returns the value and how many bytes it used.
pub fn parse_number(s: &str) -> Option<(f64, usize)> {
    let glyph = |c: char| GLYPHS.iter().find(|(_, g)| g.starts_with(c)).map(|(v, _)| *v);
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    let digits = &s[..i];
    if i == 0 {
        // A lone glyph or a fraction like "3/4" can't start here without digits; a glyph can.
        let c = s.chars().next()?;
        return glyph(c).map(|v| (v, c.len_utf8()));
    }
    let mut value: f64 = digits.parse().ok()?;
    // Decimal.
    if s[i..].starts_with('.') && s[i + 1..].starts_with(|c: char| c.is_ascii_digit()) {
        let mut j = i + 1;
        while j < bytes.len() && bytes[j].is_ascii_digit() {
            j += 1;
        }
        return Some((s[..j].parse().ok()?, j));
    }
    // "3/4" or "3⁄4" directly. Text extracted from printed cards sometimes loses the space in a mixed number:
    // "11⁄2" is 1½ and "31⁄4" is 3¼ (a recipe never writes 5½ as eleven halves).
    if let Some((den, used)) = fraction_tail(&s[i..]) {
        let n = value as u64;
        if n >= 10 && den <= 16.0 && n % 10 > 0 && ((n % 10) as f64) < den {
            return Some(((n / 10) as f64 + (n % 10) as f64 / den, i + used));
        }
        return Some((value / den, i + used));
    }
    // Mixed number: "1½", "1 ½", "1 1⁄2".
    let rest = &s[i..];
    let trimmed = rest.trim_start_matches(' ');
    let skipped = rest.len() - trimmed.len();
    if skipped <= 1 {
        if let Some(c) = trimmed.chars().next() {
            if let Some(v) = glyph(c) {
                return Some((value + v, i + skipped + c.len_utf8()));
            }
        }
        if skipped == 1 {
            let mut j = 0;
            let tb = trimmed.as_bytes();
            while j < tb.len() && tb[j].is_ascii_digit() {
                j += 1;
            }
            if j > 0 {
                if let Some((den, used)) = fraction_tail(&trimmed[j..]) {
                    let num: f64 = trimmed[..j].parse().ok()?;
                    value += num / den;
                    return Some((value, i + skipped + j + used));
                }
            }
        }
    }
    Some((value, i))
}

/// "/4", "⁄4" (fraction slash) or "∕4" (division slash, in some extracted text) → (4, bytes used).
fn fraction_tail(s: &str) -> Option<(f64, usize)> {
    let sep = match s.chars().next()? {
        '/' => 1,
        c @ ('⁄' | '∕') => c.len_utf8(),
        _ => return None,
    };
    let rest = &s[sep..];
    let n = rest.bytes().take_while(u8::is_ascii_digit).count();
    if n == 0 {
        return None;
    }
    let den: f64 = rest[..n].parse().ok()?;
    (den > 0.0).then_some((den, sep + n))
}

/// Number words used in step text ("juice from one lemon wedge (two wedges for 4)").
pub fn number_word(w: &str) -> Option<f64> {
    Some(match w.to_lowercase().as_str() {
        "a" | "an" | "one" | "whole" | "a whole" => 1.0,
        "two" => 2.0,
        "three" => 3.0,
        "four" => 4.0,
        "five" => 5.0,
        "six" => 6.0,
        "eight" => 8.0,
        "half" | "half a" | "a half" | "one-half" => 0.5,
        "a third" | "one-third" | "one third" => 1.0 / 3.0,
        "a quarter" | "one-quarter" | "a fourth" | "one-fourth" => 0.25,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(a: f64, u: Unit) -> Quantity {
        Quantity::new(a, u)
    }

    #[test]
    fn parses_numbers_as_written() {
        assert_eq!(parse_number("2 TBSP"), Some((2.0, 1)));
        assert_eq!(parse_number("½ cup"), Some((0.5, '½'.len_utf8())));
        assert_eq!(parse_number("1½ cups").map(|x| x.0), Some(1.5));
        assert_eq!(parse_number("1 1⁄2 tsp").map(|x| x.0), Some(1.5));
        assert_eq!(parse_number("1⁄2 TBSP").map(|x| x.0), Some(0.5));
        assert_eq!(parse_number("3/4 tsp").map(|x| x.0), Some(0.75));
        assert_eq!(parse_number("1.5 oz").map(|x| x.0), Some(1.5));
        assert_eq!(parse_number("1 ½ cups").map(|x| x.0), Some(1.5));
        assert_eq!(parse_number("10 oz").map(|x| x.0), Some(10.0));
        assert_eq!(parse_number("cup"), None);
        assert_eq!(parse_number("11⁄2 tsp").map(|x| x.0), Some(1.5));
        assert_eq!(parse_number("31⁄4 cups").map(|x| x.0), Some(3.25));
        assert_eq!(parse_number("12⁄3 cups").map(|x| x.0), Some(1.0 + 2.0 / 3.0));
        assert_eq!(parse_number("1⁄2 tsp").map(|x| x.0), Some(0.5));
        assert_eq!(parse_number("1∕3 cup").map(|x| x.0), Some(1.0 / 3.0));
    }

    #[test]
    fn recognises_units() {
        assert_eq!(Unit::parse("TBSP"), Some(Unit::Tbsp));
        assert_eq!(Unit::parse("teaspoon (tsp)"), Some(Unit::Tsp));
        assert_eq!(Unit::parse("tablespoon (tbsp)"), Some(Unit::Tbsp));
        assert_eq!(Unit::parse("ounce(oz)"), Some(Unit::Oz));
        assert_eq!(Unit::parse("cloves"), Some(Unit::Count("clove".into())));
        assert_eq!(Unit::parse("pick"), Some(Unit::Count(String::new())));
        assert_eq!(Unit::parse("minutes"), None);
    }

    #[test]
    fn rounds_to_measurable_amounts() {
        assert_eq!(q(0.3, Unit::Tsp).rounded(), q(0.25, Unit::Tsp));
        assert_eq!(q(1.4, Unit::Tsp).rounded(), q(1.5, Unit::Tsp));
        assert_eq!(q(4.5, Unit::Tsp).rounded(), q(1.5, Unit::Tbsp));
        assert_eq!(q(1.2, Unit::Tbsp).rounded(), q(1.0, Unit::Tbsp));
        assert_eq!(q(1.25, Unit::Tbsp).rounded(), q(1.5, Unit::Tbsp));
        assert_eq!(q(0.25, Unit::Tbsp).rounded(), q(0.75, Unit::Tsp));
        assert_eq!(q(0.7, Unit::Cup).rounded(), q(2.0 / 3.0, Unit::Cup));
        assert_eq!(q(0.125, Unit::Cup).rounded(), q(2.0, Unit::Tbsp));
        assert_eq!(q(7.3, Unit::Oz).rounded(), q(7.5, Unit::Oz));
        assert_eq!(q(0.03, Unit::Oz).rounded(), q(0.03, Unit::Oz));
        assert_eq!(q(0.03, Unit::Tsp).rounded(), q(1.0, Unit::Pinch));
        assert_eq!(format_amount(0.99), "1");
        assert_eq!(format_amount(0.016), "0.02");
        assert_eq!(format_amount(2.52), "2½");
        assert_eq!(q(1.3, Unit::Count("clove".into())).rounded(), q(1.5, Unit::Count("clove".into())));
        assert_eq!(q(0.7, Unit::Count("lime".into())).rounded(), q(0.75, Unit::Count("lime".into())));
        assert_eq!(q(0.035, Unit::Count("thumb".into())).rounded(), q(0.04, Unit::Count("thumb".into())));
    }

    #[test]
    fn measures_up_in_standard_spoons_and_cups() {
        let m = |a: f64, u: Unit| q(a, u).measured().to_string();
        assert_eq!(m(1.05, Unit::Tsp), "1 tsp + 1 pinch");
        assert_eq!(m(1.0, Unit::Tsp), "1 tsp");
        assert_eq!(m(0.9, Unit::Tsp), "1 tsp");
        assert_eq!(m(0.3, Unit::Tsp), "¼ tsp + 1 pinch");
        assert_eq!(m(4.5, Unit::Tsp), "1½ TBSP");
        assert_eq!(m(2.6, Unit::Tbsp), "2½ TBSP + ½ tsp");
        assert_eq!(m(1.1, Unit::Cup), "1 cup + 1½ TBSP");
        assert_eq!(m(3.7, Unit::Cup), "3¾ cups");
        assert_eq!(m(0.03, Unit::Tsp), "1 pinch");
        // Of two that add up the same, the bigger measures.
        assert_eq!(m(13.0, Unit::Tsp), "¼ cup + 1 tsp");
        // A source's ⅓ and ⅔ cup stored as 0.33 and 0.67 are those, and 0.495 cup (between them) is ½ cup.
        assert_eq!(m(0.33, Unit::Cup), "⅓ cup");
        assert_eq!(m(0.67, Unit::Cup), "⅔ cup");
        assert_eq!(m(0.495, Unit::Cup), "½ cup");
        assert_eq!(q(1.05, Unit::Tsp).measured().in_unit(&Unit::Tsp), Some(1.0625));
    }

    #[test]
    fn measures_weights_and_counts_up() {
        let m = |a: f64, u: Unit| q(a, u).measured().to_string();
        assert_eq!(m(7.3, Unit::Oz), "7½ oz");
        assert_eq!(m(7.0, Unit::Oz), "7 oz");
        assert_eq!(m(0.6, Unit::Oz), "¾ oz");
        assert_eq!(m(1.1, Unit::Lb), "1¼ lb");
        assert_eq!(m(1.2, Unit::Count("clove".into())), "1½ cloves");
        assert_eq!(m(0.3, Unit::Count("lime".into())), "½ lime");
        assert_eq!(m(2.2, Unit::Pinch), "3 pinches");
        assert_eq!(m(212.0, Unit::G), "215 g");
    }

    #[test]
    fn tidies_published_amounts_without_changing_them() {
        assert_eq!(q(3.0, Unit::Tsp).tidied().to_string(), "1 TBSP");
        assert_eq!(q(6.0, Unit::Tsp).tidied().to_string(), "2 TBSP");
        assert_eq!(q(2.25, Unit::Tsp).tidied().to_string(), "2¼ tsp");
        assert_eq!(q(0.06, Unit::Tsp).tidied().to_string(), "1 pinch");
        assert_eq!(q(0.068, Unit::Tsp).tidied().to_string(), "1 pinch");
        assert_eq!(q(0.12, Unit::Tsp).tidied().to_string(), "2 pinches");
        assert_eq!(q(0.18, Unit::Tsp).tidied().to_string(), "3 pinches");
        assert_eq!(q(0.125, Unit::Tsp).tidied().to_string(), "⅛ tsp");
        assert_eq!(q(0.5, Unit::Tsp).tidied().to_string(), "½ tsp");
    }

    #[test]
    fn formats_in_house_style() {
        assert_eq!(q(1.5, Unit::Tbsp).to_string(), "1½ TBSP");
        assert_eq!(q(0.75, Unit::Cup).to_string(), "¾ cup");
        assert_eq!(q(2.0, Unit::Cup).to_string(), "2 cups");
        assert_eq!(q(0.125, Unit::Tsp).to_string(), "⅛ tsp");
        assert_eq!(q(2.0, Unit::Count("clove".into())).to_string(), "2 cloves");
        assert_eq!(q(2.0, Unit::Count("bunch".into())).to_string(), "2 bunches");
        assert_eq!(q(2.0, Unit::Count("box".into())).to_string(), "2 boxes");
        assert_eq!(q(1.5, Unit::Count("loaf".into())).to_string(), "1½ loaves");
        assert_eq!(q(2.0, Unit::Count("whole".into())).to_string(), "2 whole");
        assert_eq!(q(0.3, Unit::Oz).to_string(), "0.3 oz");
        assert_eq!(q(0.004, Unit::Oz).to_string(), "<0.01 oz");
    }

    #[test]
    fn compares_across_units() {
        assert!(q(3.0, Unit::Tsp).same_as(&q(1.0, Unit::Tbsp)));
        assert!(q(0.25, Unit::Cup).same_as(&q(4.0, Unit::Tbsp)));
        assert!(!q(1.0, Unit::Tbsp).same_as(&q(1.0, Unit::Oz)));
    }
}
