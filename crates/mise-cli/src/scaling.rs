//! `mise scaling-report`: the phase 2 gate check for scaled amounts (read-only).
//!
//! Two automatic tests run over every recipe, using answers the seed already holds:
//! - Step amounts: each "1 TBSP butter (2 TBSP for 4 servings)" is derived for 4 servings from the 2-serving
//!   text and the ingredient's official amounts alone, then compared with the published 4-serving amount.
//! - Ingredient amounts: for lines published at 2, 3, 4 and 6 servings, 3 is derived from 2 and 4, and 6 from 4,
//!   then compared with the official amounts.
//!
//! A stratified sample of recipes is written as one HTML page to check by hand: every step rendered for 3 and
//! 6 servings (no published text exists for those), with each derived amount highlighted.

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::hash::{Hash, Hasher};
use std::path::Path;

use anyhow::{Context as _, Result};
use mise_core::quantity::{Quantity, Unit};
use mise_core::servings;
use mise_core::step_amounts::{Context, Method, Part, Template, amount_at, text_base};
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

/// Line amounts per (variant, line key): (servings, amount, unit).
type LineAmounts = HashMap<(Uuid, String), Vec<(f64, f64, Option<String>)>>;

struct Step {
    position: i32,
    text: String,
    template: Template,
}

#[derive(Default)]
struct Tally {
    total: usize,
    right: usize,
    by_method: HashMap<&'static str, (usize, usize)>,
}

impl Tally {
    fn add(&mut self, method: &'static str, ok: bool) {
        self.total += 1;
        self.right += ok as usize;
        let e = self.by_method.entry(method).or_default();
        e.0 += 1;
        e.1 += ok as usize;
    }
    fn pct(&self) -> f64 {
        if self.total == 0 { 0.0 } else { 100.0 * self.right as f64 / self.total as f64 }
    }
}

fn method_name(m: Method) -> &'static str {
    match m {
        Method::Published => "published",
        Method::FromLine => "from the ingredient's official amounts",
        Method::Interpolated => "interpolated",
        Method::Proportional => "in proportion to servings",
        Method::Unchanged => "unchanged",
    }
}

fn stable_hash(seed: u64, id: Uuid) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    seed.hash(&mut h);
    id.hash(&mut h);
    h.finish()
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn fmt_line(amount: f64, unit: &Option<String>) -> String {
    match unit.as_deref().and_then(Unit::parse) {
        Some(u) => Quantity::new(amount, u).to_string(),
        None => format!("{} {}", mise_core::quantity::format_amount(amount), unit.as_deref().unwrap_or("")).trim().to_string(),
    }
}

pub struct Summary {
    pub step_overall: (usize, f64),
    pub step_sample: (usize, f64),
    pub ingredient_3: (usize, f64),
    pub ingredient_6: (usize, f64),
    pub whole_line_3: (usize, f64),
    pub whole_line_6: (usize, f64),
}

pub async fn write_report(pool: &PgPool, sample: usize, seed: u64, out: &Path) -> Result<Summary> {
    // Load everything the renderer needs.
    let mut yields: HashMap<Uuid, Vec<f64>> = HashMap::new();
    for (v, s) in sqlx::query_as::<_, (Uuid, f64)>("SELECT variant_id, servings::float8 FROM variant_yields").fetch_all(pool).await? {
        yields.entry(v).or_default().push(s);
    }
    let mut amounts: LineAmounts = HashMap::new();
    let mut names: HashMap<Uuid, Vec<(String, String)>> = HashMap::new();
    for (v, key, name) in sqlx::query_as::<_, (Uuid, String, String)>(
        "SELECT variant_id, line_key, raw_name FROM ingredient_lines ORDER BY variant_id, position",
    )
    .fetch_all(pool)
    .await?
    {
        names.entry(v).or_default().push((key, name));
    }
    for (v, key, s, a, u) in sqlx::query_as::<_, (Uuid, String, f64, f64, Option<String>)>(
        "SELECT il.variant_id, il.line_key, la.servings::float8, la.amount::float8, la.unit
         FROM line_amounts la JOIN ingredient_lines il ON il.id = la.line_id WHERE la.amount > 0",
    )
    .fetch_all(pool)
    .await
    .context("loading line amounts")?
    {
        amounts.entry((v, key)).or_default().push((s, a, u));
    }
    let mut steps: HashMap<Uuid, Vec<Step>> = HashMap::new();
    for (v, position, text, t) in sqlx::query_as::<_, (Uuid, i32, String, serde_json::Value)>(
        "SELECT variant_id, position, text, template FROM steps WHERE template IS NOT NULL ORDER BY variant_id, position",
    )
    .fetch_all(pool)
    .await
    .context("loading step templates (run mise parse-steps first)")?
    {
        steps.entry(v).or_default().push(Step { position, text, template: serde_json::from_value(t)? });
    }

    // As the app does: published amounts that would shrink as servings grow are derived instead.
    for ((v, _), list) in amounts.iter_mut() {
        let base = text_base(yields.get(v).map(Vec::as_slice).unwrap_or(&[]), steps.get(v).into_iter().flatten().map(|s| &s.template));
        servings::keep_consistent(list, &[base, base * 2.0]);
    }

    // Sample: the default version of recipe dishes published at 2, 3, 4 and 6 servings whose steps vary by yield.
    let candidates: Vec<(Uuid, Uuid, String)> = sqlx::query_as(
        "SELECT d.id, d.default_variant_id, v.title FROM dishes d JOIN variants v ON v.id = d.default_variant_id
         WHERE d.content_type = 'recipe'",
    )
    .fetch_all(pool)
    .await?;
    let full = |v: &Uuid| {
        let ys = yields.get(v).map(Vec::as_slice).unwrap_or(&[]);
        [2.0, 3.0, 4.0, 6.0].iter().all(|t| ys.iter().any(|y| (y - t).abs() < 1e-9))
    };
    let mut picks: Vec<&(Uuid, Uuid, String)> = candidates
        .iter()
        .filter(|(_, v, _)| full(v) && steps.get(v).is_some_and(|ss| ss.iter().any(|s| s.template.has_alts())))
        .collect();
    picks.sort_by_key(|(d, _, _)| stable_hash(seed, *d));
    picks.truncate(sample);
    let in_sample: HashSet<Uuid> = picks.iter().map(|(_, v, _)| *v).collect();

    // Test 1: step amounts at the alternative yield, derived without the published alternative.
    let (mut overall, mut sampled) = (Tally::default(), Tally::default());
    let mut misses: HashMap<String, usize> = HashMap::new();
    for (v, ss) in &steps {
        let ys = yields.get(v).cloned().unwrap_or_default();
        let base = text_base(&ys, ss.iter().map(|s| &s.template));
        let line_amount = |key: &str, s: f64| line_value(&amounts, *v, key, s);
        let any_alts = ss.iter().any(|s| s.template.has_alts());
        let ctx = Context { base, scale_plain: !any_alts, line_amount: &line_amount };
        for s in ss {
            for p in &s.template.parts {
                let Part::Amount { alts, .. } = p else { continue };
                for (at, published) in alts {
                    let Some((pred, m)) = amount_at(p, *at, &ctx, false) else { continue };
                    let ok = pred.same_as(published);
                    overall.add(method_name(m), ok);
                    if in_sample.contains(v) {
                        sampled.add(method_name(m), ok);
                    }
                    if !ok {
                        *misses.entry(format!("{}: {} → derived {pred}, published {published}", method_name(m), base_text(p))).or_default() += 1;
                    }
                }
            }
        }
    }

    // Test 2: ingredient amounts at 3 (from 2 and 4) and 6 (from 4).
    let (mut ing3, mut ing6) = (Tally::default(), Tally::default());
    for ((_, _), list) in &amounts {
        let get = |t: f64| list.iter().find(|(s, _, _)| (s - t).abs() < 1e-9);
        let (Some(a2), Some(a3), Some(a4), Some(a6)) = (get(2.0), get(3.0), get(4.0), get(6.0)) else { continue };
        let Some(unit) = a2.2.as_deref().and_then(Unit::parse) else { continue };
        if [a3, a4, a6].iter().any(|a| a.2 != a2.2) {
            continue;
        }
        let known = [(2.0, a2.1), (4.0, a4.1)];
        for (target, official, tally) in [(3.0, a3.1, &mut ing3), (6.0, a6.1, &mut ing6)] {
            let Some((v, _)) = servings::amount_at(&known, target) else { continue };
            let pred = Quantity::new(v, unit.clone()).rounded();
            tally.add(if target == 3.0 { "interpolated" } else { "in proportion to servings" }, pred.same_as(&Quantity::new(official, unit.clone())));
        }
    }

    // Test 3: step amounts at the yields with no published text, interpolated from the text alone, where the
    // step uses a whole ingredient line: the line's official amount at that yield is then the right answer.
    let (mut whole3, mut whole6) = (Tally::default(), Tally::default());
    let no_lines = |_: &str, _: f64| -> Option<f64> { None };
    for (v, ss) in &steps {
        let ys = yields.get(v).cloned().unwrap_or_default();
        let base = text_base(&ys, ss.iter().map(|s| &s.template));
        let ctx = Context { base, scale_plain: false, line_amount: &no_lines };
        for s in ss {
            for p in &s.template.parts {
                let Part::Amount { base: b, alts, line: Some(line) } = p else { continue };
                let Some(list) = amounts.get(&(*v, line.clone())) else { continue };
                let official = |t: f64| {
                    list.iter()
                        .find(|(x, _, _)| (x - t).abs() < 1e-9)
                        .and_then(|(_, a, u)| Some(Quantity::new(*a, Unit::parse(u.as_deref()?)?)))
                };
                let whole = official(base).is_some_and(|q| q.same_as(b))
                    && !alts.is_empty()
                    && alts.iter().all(|(at, q)| official(*at).is_some_and(|l| l.same_as(q)));
                if !whole {
                    continue;
                }
                for (target, tally) in [(3.0, &mut whole3), (6.0, &mut whole6)] {
                    let (Some(truth), Some((pred, m))) = (official(target), amount_at(p, target, &ctx, true)) else { continue };
                    tally.add(method_name(m), pred.same_as(&truth));
                }
            }
        }
    }

    // Cards.
    let mut cards = String::new();
    for (no, (dish, v, title)) in picks.iter().enumerate() {
        let ys = yields.get(v).cloned().unwrap_or_default();
        let base = text_base(&ys, steps.get(v).into_iter().flatten().map(|s| &s.template));
        let line_amount = |key: &str, s: f64| line_value(&amounts, *v, key, s);
        let any_alts = steps.get(v).is_some_and(|ss| ss.iter().any(|s| s.template.has_alts()));
        let ctx = Context { base, scale_plain: !any_alts, line_amount: &line_amount };
        write_card(&mut cards, no + 1, *dish, title, *v, &names, &amounts, steps.get(v).map(Vec::as_slice).unwrap_or(&[]), &ctx);
    }

    let mut top: Vec<(String, usize)> = misses.into_iter().collect();
    top.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    top.truncate(40);
    let html = page(&overall, &sampled, &ing3, &ing6, &whole3, &whole6, &top, picks.len(), seed, &cards);
    if let Some(dir) = out.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(out, html).with_context(|| format!("writing {}", out.display()))?;
    Ok(Summary {
        step_overall: (overall.total, overall.pct()),
        step_sample: (sampled.total, sampled.pct()),
        ingredient_3: (ing3.total, ing3.pct()),
        ingredient_6: (ing6.total, ing6.pct()),
        whole_line_3: (whole3.total, whole3.pct()),
        whole_line_6: (whole6.total, whole6.pct()),
    })
}

fn line_value(amounts: &LineAmounts, v: Uuid, key: &str, s: f64) -> Option<f64> {
    let list = amounts.get(&(v, key.to_string()))?;
    list.iter().find(|(x, _, _)| (x - s).abs() < 1e-9).map(|(_, a, _)| *a)
}

fn base_text(p: &Part) -> String {
    match p {
        Part::Amount { base, alts, .. } => {
            format!("{base} ({})", alts.iter().map(|(s, q)| format!("{q} for {s}")).collect::<Vec<_>>().join(", "))
        }
        _ => String::new(),
    }
}

/// A step rendered for `target` as HTML, with derived amounts and approximate wording highlighted.
fn render_html(t: &Template, target: f64, ctx: &Context) -> String {
    let mut h = String::new();
    for p in &t.parts {
        match p {
            Part::Text { text } => h.push_str(&esc(text)),
            Part::Amount { .. } => {
                let (q, m) = amount_at(p, target, ctx, true).expect("an amount part");
                match m {
                    Method::Published | Method::Unchanged => h.push_str(&esc(&q.to_string())),
                    m => {
                        let _ = write!(h, "<mark title=\"{}\">{}</mark>", esc(method_name(m)), esc(&q.to_string()));
                    }
                }
            }
            Part::Words { base, alts } => {
                if (target - ctx.base).abs() < 1e-9 {
                    h.push_str(&esc(base));
                } else if let Some((_, w)) = alts.iter().find(|(s, _)| (s - target).abs() < 1e-9) {
                    h.push_str(&esc(w));
                } else {
                    let mut opts: Vec<(f64, &str)> = alts.iter().map(|(s, w)| (*s, w.as_str())).collect();
                    opts.push((ctx.base, base));
                    opts.sort_by(|a, b| (a.0 - target).abs().total_cmp(&(b.0 - target).abs()).then(b.0.total_cmp(&a.0)));
                    let _ = write!(h, "<mark class=\"check\" title=\"wording from the nearest published yield\">{}</mark>", esc(opts[0].1));
                }
            }
            Part::Note { text, from } => {
                if target > ctx.base {
                    let class = if (target - from).abs() < 1e-9 { "note" } else { "note check" };
                    let _ = write!(h, "<span class=\"{class}\">{}</span>", esc(text));
                }
            }
        }
    }
    h
}

#[allow(clippy::too_many_arguments)]
fn write_card(
    h: &mut String,
    no: usize,
    dish: Uuid,
    title: &str,
    v: Uuid,
    names: &HashMap<Uuid, Vec<(String, String)>>,
    amounts: &LineAmounts,
    steps: &[Step],
    ctx: &Context,
) {
    let _ = write!(h, "<article class=\"card\" id=\"r-{no}\" data-id=\"{dish}\"><header><span class=\"no\">#{no}</span><h2>{}</h2></header>", esc(title));
    // Ingredients: official amounts, and whether 3 and 6 derive correctly from 2 and 4.
    h.push_str("<table class=\"ing\"><thead><tr><th>Ingredient</th><th>2</th><th>3</th><th>4</th><th>6</th></tr></thead><tbody>");
    for (key, name) in names.get(&v).map(Vec::as_slice).unwrap_or(&[]) {
        let list = amounts.get(&(v, key.clone()));
        let get = |t: f64| list.and_then(|l| l.iter().find(|(s, _, _)| (s - t).abs() < 1e-9));
        let _ = write!(h, "<tr><td>{}</td>", esc(name));
        let known: Vec<(f64, f64)> = [2.0, 4.0].iter().filter_map(|&t| get(t).map(|x| (t, x.1))).collect();
        for t in [2.0, 3.0, 4.0, 6.0] {
            match get(t) {
                Some((_, a, u)) => {
                    let mut cell = esc(&fmt_line(*a, u));
                    if t == 3.0 || t == 6.0 {
                        if let (Some(unit), Some((d, _))) = (u.as_deref().and_then(Unit::parse), servings::amount_at(&known, t)) {
                            let pred = Quantity::new(d, unit.clone()).rounded();
                            if !pred.same_as(&Quantity::new(*a, unit)) {
                                let _ = write!(cell, " <span class=\"miss\" title=\"derived from 2 and 4\">≠ {}</span>", esc(&pred.to_string()));
                            }
                        }
                    }
                    let _ = write!(h, "<td>{cell}</td>");
                }
                None => {
                    // Not published, or left out for shrinking: what the app shows instead.
                    let all: Vec<(f64, f64)> = list.map(|l| l.iter().map(|x| (x.0, x.1)).collect()).unwrap_or_default();
                    let unit = list.and_then(|l| l.first()).and_then(|x| x.2.clone());
                    match servings::amount_at(&all, t) {
                        Some((d, _)) => {
                            let shown = match unit.as_deref().and_then(Unit::parse) {
                                Some(u) => Quantity::new(d, u).rounded().to_string(),
                                None => fmt_line(d, &unit),
                            };
                            let _ = write!(h, "<td><mark title=\"derived: the published amount shrank, or none was published\">≈ {}</mark></td>", esc(&shown));
                        }
                        None => h.push_str("<td class=\"muted\">–</td>"),
                    }
                }
            }
        }
        h.push_str("</tr>");
    }
    h.push_str("</tbody></table><ol class=\"steps\">");
    for s in steps {
        let _ = write!(h, "<li><p class=\"pub\" title=\"as published\">{}</p>", esc(&s.text));
        for t in [3.0, 6.0] {
            let _ = write!(h, "<p class=\"at\"><span class=\"y\">{t}</span>{}</p>", render_html(&s.template, t, ctx));
        }
        // Where deriving 4 from 2 would have missed the published amount.
        for p in &s.template.parts {
            let Part::Amount { alts, .. } = p else { continue };
            for (at, published) in alts {
                if let Some((pred, _)) = amount_at(p, *at, ctx, false) {
                    if !pred.same_as(published) {
                        let _ = write!(h, "<p class=\"miss\">At {at}: would derive {} — published {}</p>", esc(&pred.to_string()), esc(&published.to_string()));
                    }
                }
            }
        }
        let _ = s.position;
        h.push_str("</li>");
    }
    let _ = write!(
        h,
        "</ol><form class=\"decide\" data-id=\"{dish}\"><label><input type=\"radio\" name=\"d{no}\" value=\"right\"> Amounts look right</label>\
         <label><input type=\"radio\" name=\"d{no}\" value=\"wrong\"> Something is wrong</label>\
         <input type=\"text\" class=\"notes\" placeholder=\"What is wrong (optional)\"></form></article>"
    );
}

fn tally_rows(t: &Tally) -> String {
    let mut rows: Vec<(&&str, &(usize, usize))> = t.by_method.iter().collect();
    rows.sort_by(|a, b| b.1.0.cmp(&a.1.0));
    rows.iter()
        .map(|(m, (n, ok))| format!("<tr><td>{m}</td><td>{n}</td><td>{:.1}%</td></tr>", 100.0 * *ok as f64 / (*n).max(1) as f64))
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn page(overall: &Tally, sampled: &Tally, ing3: &Tally, ing6: &Tally, whole3: &Tally, whole6: &Tally, misses: &[(String, usize)], n: usize, seed: u64, cards: &str) -> String {
    let meta = json!({ "kind": "mise-scaling-decisions", "seed": seed, "sample": n });
    let miss_rows: String = misses.iter().map(|(m, c)| format!("<tr><td>{}</td><td>{c}</td></tr>", esc(m))).collect();
    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\
<title>Scaling check</title><style>{CSS}</style></head><body><main>\
<h1>Scaling check: {n} recipes</h1>\
<p class=\"lede\">Phase 2 gate. Each card shows a recipe's official ingredient amounts and every step as published, then as mise renders it \
for 3 and 6 servings, which have no published step text. <mark>Highlighted</mark> amounts are derived; <mark class=\"check\">dotted</mark> \
wording and notes come from the nearest published yield. Red lines show where deriving 4 servings from the 2-serving text would \
have missed the published amount.</p>\
<section class=\"summary\"><h2>Automatic checks</h2>\
<table><thead><tr><th>Test</th><th>Cases</th><th>Right</th></tr></thead><tbody>\
<tr><td>Step amounts at 4 servings, derived from 2 (all recipes)</td><td>{}</td><td>{:.1}%</td></tr>\
<tr><td>Step amounts at 4 servings, derived from 2 (this sample)</td><td>{}</td><td>{:.1}%</td></tr>\
<tr><td>Step amounts at 3 servings, interpolated from the text, where the step uses a whole ingredient</td><td>{}</td><td>{:.1}%</td></tr>\
<tr><td>Step amounts at 6 servings, extrapolated from the text, where the step uses a whole ingredient</td><td>{}</td><td>{:.1}%</td></tr>\
<tr><td>Ingredient amounts at 3 servings, from 2 and 4</td><td>{}</td><td>{:.1}%</td></tr>\
<tr><td>Ingredient amounts at 6 servings, from 4</td><td>{}</td><td>{:.1}%</td></tr></tbody></table>\
<h3>Step amounts by how they were derived (all recipes)</h3><table><thead><tr><th>Method</th><th>Cases</th><th>Right</th></tr></thead><tbody>{}</tbody></table>\
<details><summary>Most common misses</summary><table><thead><tr><th>Amount</th><th>Times</th></tr></thead><tbody>{miss_rows}</tbody></table></details>\
<p class=\"tools\"><span id=\"progress\">0</span> of {n} reviewed · <button type=\"button\" id=\"export\">Export decisions (JSON)</button></p></section>\
{cards}</main><script id=\"meta\" type=\"application/json\">{}</script><script>{JS}</script></body></html>",
        overall.total, overall.pct(), sampled.total, sampled.pct(), whole3.total, whole3.pct(), whole6.total, whole6.pct(),
        ing3.total, ing3.pct(), ing6.total, ing6.pct(),
        tally_rows(overall),
        meta.to_string().replace("</", "<\\/"),
    )
}

const CSS: &str = r#"
:root{--bg:#fbfaf7;--fg:#1d1b18;--muted:#6b665e;--line:#e3dfd6;--card:#fff;--mark:#fff1b8;--check:#e7f0ff;--miss:#b3261e;--ok:#1b6e3a}
@media (prefers-color-scheme:dark){:root:not([data-theme="light"]){--bg:#171614;--fg:#ece8e1;--muted:#a39d93;--line:#34312c;--card:#201e1b;--mark:#5a4a12;--check:#1f3150;--miss:#f2b8b5;--ok:#8fd19e}}
:root[data-theme="dark"]{--bg:#171614;--fg:#ece8e1;--muted:#a39d93;--line:#34312c;--card:#201e1b;--mark:#5a4a12;--check:#1f3150;--miss:#f2b8b5;--ok:#8fd19e}
*{box-sizing:border-box}body{margin:0;background:var(--bg);color:var(--fg);font:15px/1.5 system-ui,sans-serif}
main{max-width:980px;margin:0 auto;padding:24px 16px 80px}h1{font-size:26px;margin:0 0 8px}.lede{color:var(--muted);max-width:70ch}
table{border-collapse:collapse;width:100%;margin:8px 0 16px;font-size:14px}th,td{border-bottom:1px solid var(--line);padding:4px 8px;text-align:left;vertical-align:top}
th{color:var(--muted);font-weight:600}.summary{background:var(--card);border:1px solid var(--line);border-radius:10px;padding:12px 16px;margin:16px 0}
.card{background:var(--card);border:1px solid var(--line);border-radius:10px;padding:12px 16px;margin:16px 0}.card.done{border-color:var(--ok)}.card.flag{border-color:var(--miss)}
.card header{display:flex;gap:10px;align-items:baseline}.card h2{font-size:18px;margin:0}.no{color:var(--muted);font-variant-numeric:tabular-nums}
.ing td:not(:first-child),.ing th:not(:first-child){text-align:right;white-space:nowrap}.muted{color:var(--muted)}
.steps{padding-left:20px}.steps li{margin:10px 0}.pub{color:var(--muted);margin:2px 0;white-space:pre-line}
.at{margin:2px 0}.y{display:inline-block;min-width:28px;font-weight:700;color:var(--muted)}.y::after{content:" →"}
mark{background:var(--mark);color:inherit;border-radius:3px;padding:0 2px}mark.check,.note.check{background:var(--check);border-bottom:1px dotted var(--muted)}
.note{font-style:italic}.miss{color:var(--miss);font-size:13px;margin:2px 0}
.decide{display:flex;flex-wrap:wrap;gap:12px;align-items:center;border-top:1px solid var(--line);margin-top:8px;padding-top:8px}.decide .notes{flex:1;min-width:200px;padding:4px 8px;border:1px solid var(--line);border-radius:6px;background:var(--bg);color:var(--fg)}
button{font:inherit;padding:4px 10px;border:1px solid var(--line);border-radius:6px;background:var(--bg);color:var(--fg);cursor:pointer}
@media (max-width:600px){.ing{font-size:12px}th,td{padding:3px 4px}}
"#;

const JS: &str = r#"
(function(){"use strict";var KEY="mise-scaling-decisions-v1",store={};
try{store=JSON.parse(localStorage.getItem(KEY)||"{}")||{}}catch(e){store={}}
function save(){try{localStorage.setItem(KEY,JSON.stringify(store))}catch(e){}}
var forms=[].slice.call(document.querySelectorAll("form.decide"));
function paint(f){var r=store[f.dataset.id],card=f.closest(".card");card.classList.toggle("done",!!r&&r.decision==="right");card.classList.toggle("flag",!!r&&r.decision==="wrong");
document.getElementById("progress").textContent=Object.keys(store).filter(function(k){return store[k].decision}).length}
forms.forEach(function(f){var r=store[f.dataset.id];if(r){var i=f.querySelector("input[value='"+r.decision+"']");if(i)i.checked=true;f.querySelector(".notes").value=r.notes||""}
f.addEventListener("change",function(){var c=f.querySelector("input[type=radio]:checked");store[f.dataset.id]={decision:c?c.value:null,notes:f.querySelector(".notes").value||null,title:f.closest(".card").querySelector("h2").textContent};save();paint(f)});paint(f)});
document.getElementById("export").addEventListener("click",function(){var meta=JSON.parse(document.getElementById("meta").textContent);
var out={kind:meta.kind,seed:meta.seed,sample:meta.sample,exported_at:new Date().toISOString(),decisions:Object.keys(store).map(function(k){var r=store[k];return{dish_id:k,title:r.title,decision:r.decision,notes:r.notes}})};
var a=document.createElement("a");a.href=URL.createObjectURL(new Blob([JSON.stringify(out,null,2)],{type:"application/json"}));a.download="scaling-decisions.json";a.click()})})();
"#;
