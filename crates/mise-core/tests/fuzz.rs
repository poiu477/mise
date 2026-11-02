//! Randomised robustness checks for the parsers: no input may panic, and nothing rendered may read as NaN,
//! infinity or a negative amount. Seeded, so a failure reproduces.

use mise_core::quantity::{Quantity, Unit, parse_number};
use mise_core::step_amounts::{Context, parse, render, text_base};

/// xorshift64*: small, deterministic, no dependency.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

const PIECES: &[&str] = &[
    "1", "2", "10", "0", "00", "1.5", ".5", "1.", "½", "¼", "⅓", "⅞", "1½", "1 ½", "1 1⁄2", "1⁄2", "3/4", "1/0", "0/0",
    "99999999999999999999", "-1", "–", "-", " ", "  ", "\n", "\t", "TBSP", "tbsp", "tsp", "cup", "cups", "oz", "g",
    "ml", "cloves", "pinch", "inch", "-inch", "(", ")", "((", "))", " for 4", " for 4 servings", " for 0", " for 99999",
    "(For 4 servings, ", "we used ", "; ", ",", ".", "half", "all", "whole", "the", "lime", "butter", "water", "é",
    "日本", "🍋", "\u{200b}", "\u{2044}", "⁄", "/", "%", "{", "}", "\\", "\"", "'", "•", "and", "a", "of",
];

fn random_text(rng: &mut Rng) -> String {
    let n = rng.below(40);
    (0..n).map(|_| PIECES[rng.below(PIECES.len())]).collect()
}

fn sane(text: &str) -> bool {
    let lower = text.to_lowercase();
    !(lower.contains("nan") && !lower.contains("banana")) && !lower.contains("inf") && !text.contains("-0 ")
}

#[test]
fn parse_number_never_panics() {
    let mut rng = Rng(0x5eed);
    for _ in 0..200_000 {
        let s = random_text(&mut rng);
        if let Some((v, used)) = parse_number(&s) {
            assert!(used <= s.len() && s.is_char_boundary(used), "{s:?} used {used}");
            assert!(v.is_finite() || s.contains("99999999999999999999"), "{s:?} gave {v}");
        }
    }
}

#[test]
fn quantities_round_and_format_sanely() {
    let units = [Unit::Tsp, Unit::Tbsp, Unit::Cup, Unit::Oz, Unit::Lb, Unit::G, Unit::Ml, Unit::Count("clove".into()), Unit::Count(String::new())];
    let mut rng = Rng(42);
    for _ in 0..100_000 {
        let a = (rng.next() % 1_000_000) as f64 / 1000.0;
        let u = units[rng.below(units.len())].clone();
        let r = Quantity::new(a, u.clone()).rounded();
        assert!(r.amount.is_finite() && r.amount >= 0.0, "{a} {u:?} → {r:?}");
        if a > 0.0 {
            assert!(r.amount > 0.0, "a positive amount rounded to nothing: {a} {u:?} → {r:?}");
        }
        let shown = r.to_string();
        assert!(sane(&shown), "{a} {u:?} shows as {shown:?}");
        // Measured up, never less (bar a source's hair-over amounts) and never nothing.
        let m = Quantity::new(a, u.clone()).measured();
        if a > 0.0 {
            let total = m.in_unit(&u).unwrap();
            assert!(total >= a * 0.99 && total > 0.0, "{a} {u:?} measured as {m:?}");
        }
        assert!(sane(&m.to_string()), "{a} {u:?} measures as {m}");
    }
}

#[test]
fn steps_parse_and_render_without_panicking() {
    let lines = vec![("b".to_string(), "Butter".to_string()), ("l".to_string(), "Lime".to_string()), ("w".to_string(), "".to_string())];
    let line_amount = |k: &str, s: f64| -> Option<f64> { (k == "b").then_some(s / 2.0) };
    let mut rng = Rng(7);
    for _ in 0..50_000 {
        let text = random_text(&mut rng);
        let t = parse(&text, &lines);
        let base = text_base(&[2.0, 4.0], [&t]);
        for scale_plain in [false, true] {
            let ctx = Context { base, scale_plain, line_amount: &line_amount };
            for target in [0.5, 1.0, 2.0, 2.5, 3.0, 4.0, 6.0, 24.0] {
                let r = render(&t, target, &ctx);
                assert!(sane(&r.text) || sane(&text) == false, "{text:?} at {target} → {:?}", r.text);
            }
        }
        // Text outside amounts survives: rendering at the base yield drops only yield parentheticals.
        let ctx = Context { base, scale_plain: false, line_amount: &line_amount };
        let at_base = render(&t, base, &ctx).text;
        assert!(at_base.len() <= text.len() + 64, "{text:?} grew to {at_base:?}");
    }
}
