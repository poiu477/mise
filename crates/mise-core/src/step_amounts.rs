//! Amounts inside step text. Published steps are written for the base yield with the larger yield in
//! parentheses: "1 TBSP butter (2 TBSP for 4 servings)", "half the lime zest (all for 4)", "(For 4 servings,
//! spread squash across the entire sheet.)". A step is parsed once into parts, then rendered for any yield:
//! a published yield reads exactly as published; any other is derived from the ingredient's official amounts
//! or interpolated between the published ones, rounded to kitchen amounts, and flagged.

use std::sync::LazyLock;

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::quantity::{Quantity, Unit, parse_number};
use crate::text::normalize_name;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Part {
    Text { text: String },
    /// An amount: as written for the base yield, as written for other published yields, and the ingredient
    /// line it measures out, when the text names one.
    Amount {
        base: Quantity,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        alts: Vec<(f64, Quantity)>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        line: Option<String>,
    },
    /// Words that change with the yield: "half the lime zest" → "all the lime zest".
    Words { base: String, alts: Vec<(f64, String)> },
    /// A remark for larger yields, kept verbatim (with its leading space): shown above the base yield.
    Note { from: f64, text: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Template {
    pub parts: Vec<Part>,
    /// A yield-dependent parenthetical the parser could not place; the published text is kept for it.
    #[serde(default)]
    pub unparsed: usize,
}

impl Template {
    pub fn amounts(&self) -> impl Iterator<Item = &Part> {
        self.parts.iter().filter(|p| matches!(p, Part::Amount { .. }))
    }

    /// Any part varies by yield in the published text.
    pub fn has_alts(&self) -> bool {
        self.parts.iter().any(|p| match p {
            Part::Amount { alts, .. } => !alts.is_empty(),
            Part::Words { .. } | Part::Note { .. } => true,
            Part::Text { .. } => false,
        })
    }
}

const NUM: &str = r"(?:\d+(?:\.\d+)?\s+\d+\s*[⁄/∕]\s*\d+|\d+\s?[½⅓⅔¼¾⅛⅜⅝⅞]|\d+\s*[⁄/∕]\s*\d+|\d+(?:\.\d+)?|[½⅓⅔¼¾⅛⅜⅝⅞])";
const UNIT: &str = r"(?i:tbsps?|tablespoons?|tsps?|teaspoons?|cups?|fl\.? oz|oz|ounces?|lbs?|pounds?|grams?|g|ml|milliliters?|cloves?|pinch(?:es)?)";

static QTY: LazyLock<Regex> = LazyLock::new(|| Regex::new(&format!(r"({NUM})\s*({UNIT})\b")).unwrap());
/// "(… for 4 servings)", "(… for 4)".
static ALT: LazyLock<Regex> = LazyLock::new(|| {
    // A remark may follow the yield: "(5 TBSP for 4 servings; we sent more)".
    Regex::new(r"(?i)\s*\(\s*([^()]*?)[\s,;]+for\s+(\d+)(?:\s+(?:servings?|people|portions))?(?:\s*[;,]\s*([^()]*?))?\s*\)").unwrap()
});
/// "(For 4 servings, spread squash across the entire sheet.)".
static NOTE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\s*\(\s*for\s+(\d+)\s+servings?\s*[,:]\s*[^()]*\)").unwrap());
/// A share or count word that a word-alternative replaces ("half the lime (whole lime for 4)").
static SHARE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(half|one-half|a third|one-third|two-thirds|a quarter|one-quarter|a fourth|three-fourths|three-quarters|one|two|three|four|all|whole)\b")
        .unwrap()
});
/// The largest yield a parenthetical may name.
const MAX_YIELD: f64 = 100.0;

/// "we used ½ tsp; 1 tsp", "we\nused ¾ tsp, 1½ tsp", "we used 1 tsp kosher salt; use 2 tsp".
static WE_USED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?is)^we\s+used\s+(.+?)\s*[;,]\s*(?:use\s+)?(.+)$").unwrap());

struct Found {
    start: usize,
    end: usize,
    q: Quantity,
}

fn quantities(text: &str, offset: usize) -> Vec<Found> {
    QTY.captures_iter(text)
        .filter_map(|c| {
            let m = c.get(0).unwrap();
            // Not part of a range or a size ("2-3 TBSP" stays as written; "1-inch" never matches a unit).
            let before = text[..m.start()].chars().next_back();
            if before.is_some_and(|ch| ch == '-' || ch == '–' || ch.is_alphanumeric() || ch == '⁄' || ch == '/' || ch == '∕') {
                return None;
            }
            let (amount, _) = parse_number(c.get(1).unwrap().as_str().trim())?;
            let unit = Unit::parse(c.get(2).unwrap().as_str())?;
            Some(Found { start: offset + m.start(), end: offset + m.end(), q: Quantity::new(amount, unit) })
        })
        .collect()
}

/// Ingredient lines as (line key, name) for linking amounts to what they measure.
pub type Lines<'a> = &'a [(String, String)];

/// The line an amount measures: the longest line name that the words just after the amount begin with
/// ("1 TBSP butter" → Butter, "1 TBSP bulgogi sauce" → Bulgogi Sauce).
fn link(after: &str, lines: Lines) -> Option<String> {
    let words = normalize_name(&after.chars().take(48).collect::<String>());
    let words = words.strip_prefix("of ").unwrap_or(&words);
    lines
        .iter()
        .filter_map(|(key, name)| {
            let n = normalize_name(name);
            let singular = n.strip_suffix('s').unwrap_or(&n).to_string();
            (!n.is_empty() && (words.starts_with(&n) || words.starts_with(&singular))).then(|| (n.len(), key.clone()))
        })
        .max()
        .map(|(_, k)| k)
}

pub fn parse(text: &str, lines: Lines) -> Template {
    // Replacements as (start, end, part); gaps between them become text.
    let mut reps: Vec<(usize, usize, Part)> = Vec::new();
    let mut unparsed = 0;
    let mut used_spans: Vec<(usize, usize)> = Vec::new();

    let mut parens: Vec<(usize, usize, Option<regex::Captures>)> = Vec::new();
    for m in NOTE.find_iter(text) {
        parens.push((m.start(), m.end(), None));
    }
    for c in ALT.captures_iter(text) {
        let m = c.get(0).unwrap();
        if !parens.iter().any(|(s, e, _)| m.start() < *e && *s < m.end()) {
            parens.push((m.start(), m.end(), Some(c)));
        }
    }
    parens.sort_by_key(|p| p.0);

    let all_qty = quantities(text, 0);
    let in_paren = |pos: usize| parens.iter().any(|(s, e, _)| *s <= pos && pos < *e);
    let mut taken = vec![false; all_qty.len()];
    let mut prev_end = 0;

    for (start, end, caps) in &parens {
        let (start, end) = (*start, *end);
        // Only yields a kitchen cooks for; "(… for 0)" or "(for 99999)" is ordinary text.
        let named: Option<f64> = match caps {
            Some(c) => c[2].parse().ok(),
            None => NOTE.captures(&text[start..end]).and_then(|c| c[1].parse().ok()),
        };
        if !named.is_some_and(|n| (1.0..=MAX_YIELD).contains(&n)) {
            continue;
        }
        let Some(caps) = caps else {
            let from: f64 = NOTE.captures(&text[start..end]).and_then(|c| c[1].parse().ok()).unwrap_or(4.0);
            reps.push((start, end, Part::Note { from, text: text[start..end].to_string() }));
            prev_end = end;
            continue;
        };
        let inner = caps.get(1).unwrap();
        let servings: f64 = caps[2].parse().unwrap_or(4.0);

        // "salt (we used ½ tsp; 1 tsp for 4 servings)": both amounts inside the parentheses.
        // The words around the base amount stay ("(we used 1 tsp kosher salt)"); the alternative goes.
        if let Some(w) = WE_USED.captures(inner.as_str()) {
            let g1 = w.get(1).unwrap();
            let base = quantities(g1.as_str(), inner.start() + g1.start());
            let alt = quantities(w.get(2).unwrap().as_str(), 0);
            if let (Some(b), Some(a)) = (base.first(), alt.first()) {
                let g1_end = inner.start() + g1.end();
                reps.push((start, b.start, Part::Text { text: text[start..b.start].to_string() }));
                reps.push((b.start, b.end, Part::Amount { base: b.q.clone(), alts: vec![(servings, a.q.clone())], line: None }));
                reps.push((b.end, end, Part::Text { text: text[b.end..g1_end].to_string() + ")" }));
                prev_end = end;
                continue;
            }
        }

        let alts = quantities(inner.as_str(), inner.start());
        if !alts.is_empty() {
            // Pair each alternative, right to left, with the nearest unused base amount of the same kind before
            // the parentheses.
            let mut pairs = Vec::new();
            let mut ok = true;
            let mut cursor = all_qty.len();
            for a in alts.iter().rev() {
                let found = (0..cursor).rev().find(|&i| {
                    let b = &all_qty[i];
                    !taken[i] && b.start >= prev_end && b.end <= start && !in_paren(b.start) && b.q.unit.compatible(&a.q.unit)
                });
                match found {
                    Some(i) => {
                        pairs.push((i, a.q.clone()));
                        cursor = i;
                    }
                    None => {
                        ok = false;
                        break;
                    }
                }
            }
            if ok {
                for (i, alt) in pairs {
                    taken[i] = true;
                    let b = &all_qty[i];
                    reps.push((
                        b.start,
                        b.end,
                        Part::Amount { base: b.q.clone(), alts: vec![(servings, alt)], line: link(&text[b.end..], lines) },
                    ));
                    used_spans.push((b.start, b.end));
                }
                // The remark after the yield, if any, holds at every yield.
                let remark = caps.get(3).map(|m| m.as_str().trim()).filter(|r| !r.is_empty());
                reps.push((start, end, Part::Text { text: remark.map(|r| format!(" ({r})")).unwrap_or_default() }));
            } else {
                unparsed += 1;
            }
            prev_end = end;
            continue;
        }

        // A word alternative: the nearest share word before the parentheses.
        let window_start = prev_end.max(start.saturating_sub(60));
        let window_start = (window_start..=start).find(|&i| text.is_char_boundary(i)).unwrap_or(start);
        let share = SHARE.find_iter(&text[window_start..start]).last();
        match share {
            Some(s) => {
                let s_start = window_start + s.start();
                let base = text[s_start..start].trim_end().to_string();
                let alt_words = inner.as_str().trim();
                let alt = if SHARE.find(alt_words).is_some_and(|m| m.start() == 0 && m.end() == alt_words.len()) {
                    // "all" replaces only the share word: "half the lime zest" → "all the lime zest".
                    format!("{alt_words}{}", &text[window_start + s.end()..start].trim_end())
                } else if alt_words.to_lowercase().starts_with("whole") && base.to_lowercase().contains(" the ") {
                    format!("the {alt_words}")
                } else {
                    alt_words.to_string()
                };
                reps.push((s_start, s_start + base.len(), Part::Words { base, alts: vec![(servings, alt)] }));
                reps.push((start, end, Part::Text { text: String::new() }));
            }
            None => reps.push((start, end, Part::Note { from: servings, text: text[start..end].to_string() })),
        }
        prev_end = end;
    }

    // Amounts with no published alternative: kept as written, or scaled when the recipe never varies its text.
    for (i, f) in all_qty.iter().enumerate() {
        if !taken[i] && !in_paren(f.start) && !reps.iter().any(|(s, e, _)| f.start < *e && *s < f.end) {
            reps.push((f.start, f.end, Part::Amount { base: f.q.clone(), alts: vec![], line: link(&text[f.end..], lines) }));
        }
    }

    reps.sort_by_key(|r| (r.0, r.1));
    let mut parts = Vec::new();
    let mut at = 0;
    for (s, e, part) in reps {
        if s > at {
            parts.push(Part::Text { text: text[at..s].to_string() });
        }
        parts.push(part);
        at = at.max(e);
    }
    if at < text.len() {
        parts.push(Part::Text { text: text[at..].to_string() });
    }
    // Merge adjacent text.
    let mut merged: Vec<Part> = Vec::new();
    for p in parts {
        match (merged.last_mut(), &p) {
            (Some(Part::Text { text: a }), Part::Text { text: b }) => a.push_str(b),
            _ => merged.push(p),
        }
    }
    Template { parts: merged, unparsed }
}

/// The yield a variant's step text is written for: half the yield its parentheticals name when that is
/// published ("(4 TBSP for 8)" means text for 4), else the largest published yield below it, else 2 when
/// published, else the smallest.
pub fn text_base<'a>(published: &[f64], templates: impl IntoIterator<Item = &'a Template>) -> f64 {
    let named = templates
        .into_iter()
        .flat_map(|t| t.parts.iter())
        .filter_map(|p| match p {
            Part::Amount { alts, .. } if !alts.is_empty() => alts.iter().map(|a| a.0).reduce(f64::min),
            Part::Words { alts, .. } => alts.iter().map(|a| a.0).reduce(f64::min),
            _ => None,
        })
        .reduce(f64::min);
    if let Some(n) = named {
        // Text names double its own yield ("for 4" over text for 2, "for 8" over text for 4).
        if published.iter().any(|y| (y - n / 2.0).abs() < 1e-9) {
            return n / 2.0;
        }
        if let Some(b) = published.iter().copied().filter(|y| *y < n).reduce(f64::max) {
            return b;
        }
        return n / 2.0;
    }
    if published.iter().any(|y| (y - 2.0).abs() < 1e-9) { 2.0 } else { published.iter().copied().reduce(f64::min).unwrap_or(2.0) }
}

/// What a render needs to know about the variant.
pub struct Context<'a> {
    /// The yield the step text is written for.
    pub base: f64,
    /// Amounts without a published alternative scale with the yield (the recipe's text never varies by yield,
    /// as with a web import); otherwise they are the same at every yield, as in the seed's convention.
    pub scale_plain: bool,
    /// Official amount of an ingredient line at a yield.
    pub line_amount: &'a dyn Fn(&str, f64) -> Option<f64>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Rendered {
    pub text: String,
    /// Some amount was derived rather than published.
    pub scaled: bool,
    /// Some wording was chosen from the nearest published yield and may not fit exactly.
    pub check: bool,
}

/// How a derived amount was reached, for the test report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Published,
    FromLine,
    Interpolated,
    Proportional,
    Unchanged,
}

/// One amount at a yield: the published value when there is one, otherwise derived. `use_alts` false
/// ignores published alternatives (used to test derivation against them).
pub fn amount_at(part: &Part, target: f64, ctx: &Context, use_alts: bool) -> Option<(Quantity, Method)> {
    let (q, m) = derive(part, target, ctx, use_alts)?;
    if matches!(m, Method::Published | Method::Unchanged) {
        return Some((q, m));
    }
    // A derived amount stays between the published amounts on either side of it: rounding, or deriving one
    // yield from the ingredient and the next from the text, must never give a larger yield less.
    let Part::Amount { base, alts, .. } = part else { return Some((q, m)) };
    let mut points: Vec<(f64, &Quantity)> = vec![(ctx.base, base)];
    if use_alts {
        points.extend(alts.iter().map(|(s, a)| (*s, a)));
    }
    let Some(v) = q.in_unit(&base.unit) else { return Some((q, m)) };
    let side = |below: bool| {
        points
            .iter()
            .filter(|p| if below { p.0 < target } else { p.0 > target })
            .min_by(|a, b| (a.0 - target).abs().total_cmp(&(b.0 - target).abs()))
            .and_then(|p| Some((p.1, p.1.in_unit(&base.unit)?)))
    };
    if let Some((lq, lv)) = side(true) {
        if v < lv - 1e-9 {
            return Some((lq.clone(), m));
        }
    }
    if let Some((uq, uv)) = side(false) {
        if v > uv + 1e-9 {
            return Some((uq.clone(), m));
        }
    }
    Some((q, m))
}

fn derive(part: &Part, target: f64, ctx: &Context, use_alts: bool) -> Option<(Quantity, Method)> {
    let Part::Amount { base, alts, line } = part else { return None };
    if (target - ctx.base).abs() < 1e-9 {
        return Some((base.clone(), Method::Published));
    }
    if use_alts {
        if let Some((_, q)) = alts.iter().find(|(s, _)| (s - target).abs() < 1e-9) {
            return Some((q.clone(), Method::Published));
        }
    }
    // The ingredient's official amounts, when they move with the yield the way the text does. Pantry lines
    // often repeat one amount for every yield ("¼ tsp salt" at 2 and 4) and say nothing about the step. Only
    // for amounts the text varies (or text that never varies): "Reserve ½ cup potato cooking liquid" with no
    // alternative is the same at every yield, whatever the potatoes do.
    let varies = !alts.is_empty() || ctx.scale_plain || !use_alts;
    if let Some(line) = line.as_ref().filter(|_| varies) {
        if let (Some(lb), Some(lt)) = ((ctx.line_amount)(line, ctx.base), (ctx.line_amount)(line, target)) {
            let agrees = |s: f64, q: &Quantity| match ((ctx.line_amount)(line, s), q.in_unit(&base.unit)) {
                (Some(ls), Some(qa)) if base.amount > 0.0 => ((ls / lb) - (qa / base.amount)).abs() <= 0.15 * (qa / base.amount),
                _ => true,
            };
            let consistent = !use_alts || alts.iter().all(|(s, q)| agrees(*s, q));
            // The line must move across the yields it is published for (base and its double), not merely when
            // extended past them: a flat pantry line (1 tsp sugar for 2 and for 4) says nothing about the step.
            let moves = (ctx.line_amount)(line, ctx.base * 2.0).is_some_and(|l2| (l2 - lb).abs() > 1e-9);
            if lb > 0.0 && moves && consistent {
                return Some((base.scaled(lt / lb).rounded(), Method::FromLine));
            }
        }
    }
    let alts: Vec<&(f64, Quantity)> = if use_alts { alts.iter().collect() } else { vec![] };
    let alts: Vec<&(f64, Quantity)> = alts.into_iter().filter(|(s, _)| (s - ctx.base).abs() > 1e-9).collect();
    // Between published yields: interpolated. Outside them: in proportion to the nearest one, so an amount
    // never runs down to nothing below the smallest yield ("½ cup (1½ cups for 4)" at 1 serving is ¼ cup).
    if !alts.is_empty() && ctx.base > 0.0 {
        let mut points: Vec<(f64, f64)> = vec![(ctx.base, base.amount)];
        points.extend(alts.iter().filter_map(|(s, q)| Some((*s, q.in_unit(&base.unit)?))));
        if points.len() > 1 {
            if let Some((v, _)) = crate::servings::amount_at(&points, target) {
                let near = crate::servings::nearest(&points.iter().map(|p| p.0).collect::<Vec<_>>(), target).unwrap_or(ctx.base);
                let unit = alts.iter().find(|(s, _)| (s - near).abs() < 1e-9).map_or(base.unit.clone(), |(_, q)| q.unit.clone());
                let v = Quantity::new(v.max(0.0), base.unit.clone()).in_unit(&unit).unwrap_or(v);
                return Some((Quantity::new(v, unit).rounded(), Method::Interpolated));
            }
        }
    }
    if ctx.base <= 0.0 {
        return Some((base.clone(), Method::Unchanged));
    }
    if ctx.scale_plain || !alts.is_empty() || !use_alts {
        return Some((base.scaled(target / ctx.base).rounded(), Method::Proportional));
    }
    Some((base.clone(), Method::Unchanged))
}

pub fn render(t: &Template, target: f64, ctx: &Context) -> Rendered {
    let mut out = Rendered::default();
    let at_base = (target - ctx.base).abs() < 1e-9;
    for p in &t.parts {
        match p {
            Part::Text { text } => out.text.push_str(text),
            Part::Amount { .. } => {
                let (q, m) = amount_at(p, target, ctx, true).expect("an amount part");
                out.scaled |= !matches!(m, Method::Published | Method::Unchanged);
                out.text.push_str(&q.to_string());
            }
            Part::Words { base, alts } => {
                if at_base {
                    out.text.push_str(base);
                    continue;
                }
                let exact = alts.iter().find(|(s, _)| (s - target).abs() < 1e-9);
                let choice = exact.map(|(_, w)| w.as_str()).unwrap_or_else(|| {
                    out.check = true;
                    // The nearest published wording; ties go to the larger yield.
                    let mut options: Vec<(f64, &str)> = alts.iter().map(|(s, w)| (*s, w.as_str())).collect();
                    options.push((ctx.base, base.as_str()));
                    options.sort_by(|a, b| (a.0 - target).abs().total_cmp(&(b.0 - target).abs()).then(b.0.total_cmp(&a.0)));
                    options[0].1
                });
                out.text.push_str(choice);
            }
            Part::Note { from, text } => {
                if target > ctx.base {
                    out.check |= (target - from).abs() > 1e-9;
                    out.text.push_str(text);
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines() -> Vec<(String, String)> {
        vec![("butter".into(), "Butter".into()), ("bulgogi".into(), "Bulgogi Sauce".into()), ("lime".into(), "Lime".into())]
    }

    fn ctx(base: f64, f: &dyn Fn(&str, f64) -> Option<f64>) -> Context<'_> {
        Context { base, scale_plain: false, line_amount: f }
    }

    fn none(_: &str, _: f64) -> Option<f64> {
        None
    }

    #[test]
    fn amount_with_alternative() {
        let t = parse("Melt 1 TBSP butter (2 TBSP for 4 servings) in a pan.", &lines());
        assert_eq!(t.unparsed, 0);
        let c = ctx(2.0, &none);
        assert_eq!(render(&t, 2.0, &c).text, "Melt 1 TBSP butter in a pan.");
        assert_eq!(render(&t, 4.0, &c).text, "Melt 2 TBSP butter in a pan.");
        let six = render(&t, 6.0, &c);
        assert_eq!(six.text, "Melt 3 TBSP butter in a pan.");
        assert!(six.scaled);
        assert!(matches!(&t.parts[1], Part::Amount { line: Some(l), .. } if l == "butter"));
    }

    #[test]
    fn short_form_and_cups() {
        let t = parse("Whisk in ½ cup water (1 cup for 4). Simmer.", &lines());
        let c = ctx(2.0, &none);
        assert_eq!(render(&t, 4.0, &c).text, "Whisk in 1 cup water. Simmer.");
        assert_eq!(render(&t, 3.0, &c).text, "Whisk in ¾ cup water. Simmer.");
    }

    #[test]
    fn several_amounts_in_one_parenthetical() {
        let text = "Add vinegar, 2 TBSP water, 1 tsp sugar, and a pinch of salt (4 TBSP water and 2 tsp sugar for 4 servings); stir.";
        let t = parse(text, &lines());
        assert_eq!(t.unparsed, 0);
        let c = ctx(2.0, &none);
        assert_eq!(render(&t, 4.0, &c).text, "Add vinegar, 4 TBSP water, 2 tsp sugar, and a pinch of salt; stir.");
    }

    #[test]
    fn we_used() {
        let t = parse("Season with salt (we used ½ tsp; 1 tsp for 4 servings) and pepper.", &lines());
        let c = ctx(2.0, &none);
        assert_eq!(render(&t, 2.0, &c).text, "Season with salt (we used ½ tsp) and pepper.");
        assert_eq!(render(&t, 4.0, &c).text, "Season with salt (we used 1 tsp) and pepper.");
    }

    #[test]
    fn share_words() {
        let t = parse("Stir in half the lime zest (all for 4 servings) and juice from half the lime (whole lime for 4 servings).", &lines());
        let c = ctx(2.0, &none);
        assert_eq!(render(&t, 2.0, &c).text, "Stir in half the lime zest and juice from half the lime.");
        assert_eq!(render(&t, 4.0, &c).text, "Stir in all the lime zest and juice from the whole lime.");
        let three = render(&t, 3.0, &c);
        assert!(three.check);
    }

    #[test]
    fn we_used_variants_keep_their_words() {
        let c = ctx(2.0, &none);
        let t = parse("Season with salt (we
used 1 tsp kosher salt; use 2 tsp for 4 servings) and pepper.", &lines());
        assert_eq!(render(&t, 4.0, &c).text, "Season with salt (we
used 2 tsp kosher salt) and pepper.");
        let t = parse("Add salt (we used ¾ tsp, 1½ tsp for 4 servings).", &lines());
        assert_eq!(render(&t, 2.0, &c).text, "Add salt (we used ¾ tsp).");
    }

    #[test]
    fn never_runs_to_nothing_below_the_smallest_yield() {
        let t = parse("Stir in ½ cup water (1½ cups for 4).", &lines());
        let c = ctx(2.0, &none);
        assert_eq!(render(&t, 1.0, &c).text, "Stir in ¼ cup water.");
        assert_eq!(render(&t, 3.0, &c).text, "Stir in 1 cup water.");
    }

    #[test]
    fn remarks_after_the_yield_stay() {
        let t = parse("Stir in 2½ TBSP sweet soy glaze (5 TBSP for 4 servings; we sent more), then serve.", &lines());
        assert_eq!(t.unparsed, 0);
        let c = ctx(2.0, &none);
        assert_eq!(render(&t, 2.0, &c).text, "Stir in 2½ TBSP sweet soy glaze (we sent more), then serve.");
        assert_eq!(render(&t, 4.0, &c).text, "Stir in 5 TBSP sweet soy glaze (we sent more), then serve.");
        assert_eq!(render(&t, 5.0, &c).text, "Stir in 6½ TBSP sweet soy glaze (we sent more), then serve.");
    }

    #[test]
    fn amounts_the_text_keeps_fixed_ignore_their_line() {
        let lines = vec![("p".to_string(), "Potatoes".to_string())];
        let t = parse("Reserve ½ cup potatoes cooking liquid, then drain. Add 1 TBSP butter (2 TBSP for 4).", &lines);
        let line = |k: &str, s: f64| -> Option<f64> { (k == "p").then_some(s * 6.0) };
        let c = ctx(2.0, &line);
        assert!(render(&t, 4.0, &c).text.starts_with("Reserve ½ cup"));
        assert!(render(&t, 6.0, &c).text.starts_with("Reserve ½ cup"));
    }

    #[test]
    fn text_that_never_varies_scales_steadily() {
        let t = parse("Add remaining Tuscan Heat Spice, 1 tsp sugar, and water.", &lines());
        let c = Context { base: 2.0, scale_plain: true, line_amount: &none };
        let at = |n: f64| render(&t, n, &c).text;
        assert_eq!(at(4.0), "Add remaining Tuscan Heat Spice, 2 tsp sugar, and water.");
        assert_eq!(at(5.0), "Add remaining Tuscan Heat Spice, 2½ tsp sugar, and water.");
        assert_eq!(at(6.0), "Add remaining Tuscan Heat Spice, 1 TBSP sugar, and water.");
    }

    #[test]
    fn division_slash_fractions() {
        let t = parse("Stir in stock concentrate and 1∕3 cup water (2⁄3 cup for 4 servings).", &lines());
        let c = ctx(2.0, &none);
        assert_eq!(render(&t, 2.0, &c).text, "Stir in stock concentrate and ⅓ cup water.");
        assert_eq!(render(&t, 4.0, &c).text, "Stir in stock concentrate and ⅔ cup water.");
    }

    #[test]
    fn a_flat_pantry_line_does_not_drive_the_step() {
        let lines = vec![("s".to_string(), "Sugar".to_string())];
        let t = parse("Add 1 tsp sugar and water.", &lines);
        // 1 tsp for 2 and for 4, extended past 4 in proportion (as the API's lookup does).
        let line = |k: &str, sv: f64| -> Option<f64> { (k == "s").then_some(if sv <= 4.0 { 1.0 } else { sv / 4.0 }) };
        let c = Context { base: 2.0, scale_plain: true, line_amount: &line };
        assert_eq!(render(&t, 4.0, &c).text, "Add 2 tsp sugar and water.");
        assert_eq!(render(&t, 5.0, &c).text, "Add 2½ tsp sugar and water.");
    }

    #[test]
    fn notes_for_larger_yields() {
        let t = parse("Roast on top rack. (For 4 servings, spread squash across entire sheet.) Then flip.", &lines());
        let c = ctx(2.0, &none);
        assert_eq!(render(&t, 2.0, &c).text, "Roast on top rack. Then flip.");
        assert_eq!(render(&t, 4.0, &c).text, "Roast on top rack. (For 4 servings, spread squash across entire sheet.) Then flip.");
    }

    #[test]
    fn derives_from_official_line_amounts() {
        let t = parse("Add 1 TBSP bulgogi sauce (2 TBSP for 4 servings).", &lines());
        let line = |k: &str, s: f64| -> Option<f64> {
            if k != "bulgogi" {
                return None;
            }
            Some(match s as i32 {
                2 => 4.0,
                3 => 6.0,
                4 => 8.0,
                _ => return None,
            })
        };
        let c = ctx(2.0, &line);
        let three = render(&t, 3.0, &c);
        assert_eq!(three.text, "Add 1½ TBSP bulgogi sauce.");
        assert!(three.scaled);
    }

    #[test]
    fn leaves_ranges_and_sizes_alone() {
        let t = parse("Form into 10-12 1 1⁄2-inch meatballs; cook 3-4 minutes at 425 degrees.", &lines());
        assert!(t.amounts().next().is_none());
    }

    #[test]
    fn plain_amounts_follow_the_recipe() {
        let t = parse("Add ½ cup water.", &lines());
        let fixed = ctx(2.0, &none);
        assert_eq!(render(&t, 4.0, &fixed).text, "Add ½ cup water.");
        let scaling = Context { base: 2.0, scale_plain: true, line_amount: &none };
        assert_eq!(render(&t, 4.0, &scaling).text, "Add 1 cup water.");
    }

    #[test]
    fn round_trips_through_json() {
        let t = parse("Melt 1 TBSP butter (2 TBSP for 4 servings).", &lines());
        let back: Template = serde_json::from_str(&serde_json::to_string(&t).unwrap()).unwrap();
        assert_eq!(back, t);
    }
}
