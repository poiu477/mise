//! Render the gate report as one self-contained HTML page: inline CSS and JS, no external requests.
//! Every piece of data text goes through `esc` (titles and ingredient names are scraped).

use std::fmt::Write;

use chrono::{DateTime, Utc};
use serde_json::json;

use super::{Card, Parent, Related, RelatedDish, Report, STRATA, Summary};

const CSS: &str = include_str!("report.css");
const JS: &str = include_str!("report.js");

pub fn render(r: &Report) -> String {
    let mut h = String::with_capacity(4 << 20);
    let title = format!("Dish grouping gate check — {} dishes", r.cards.len());
    let _ = write!(
        h,
        "<!doctype html>\n<html lang=\"en\"><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
         <title>{}</title><style>{CSS}</style></head><body>",
        esc(&title)
    );
    toolbar(&mut h, r);
    h.push_str("<main class=\"wrap\">");
    summary(&mut h, r);
    let _ = write!(h, "<p class=\"count-line\">Showing <b id=\"shown\">{}</b> of {} dishes.</p>", r.cards.len(), r.cards.len());
    for (i, c) in r.cards.iter().enumerate() {
        card(&mut h, i + 1, c);
    }
    let _ = write!(
        h,
        "<footer>Generated {} by <code>mise gate-report --sample {} --seed {}</code>. Decisions are kept in this browser's \
         localStorage until exported.</footer></main>",
        esc(&r.generated_at.format("%Y-%m-%d %H:%M UTC").to_string()),
        r.requested,
        r.seed
    );

    let data = json!({
        "report": { "generated_at": r.generated_at.to_rfc3339(), "seed": r.seed, "sample": r.requested },
        "dishes": r.cards.iter().map(|c| json!({
            "id": c.dish.id, "slug": c.dish.slug, "title": c.dish.title, "content_type": c.dish.content_type,
            "stratum": STRATA[c.stratum].key,
        })).collect::<Vec<_>>(),
    });
    let _ = writeln!(
        h,
        "<script type=\"application/json\" id=\"gate-data\">{}</script><script>{JS}</script></body></html>",
        script_json(&data)
    );
    h
}

fn toolbar(h: &mut String, r: &Report) {
    let total = r.cards.len();
    let _ = write!(
        h,
        "<div class=\"bar\"><div class=\"wrap\">\
         <div class=\"title\"><h1>Dish grouping gate check</h1>\
         <span class=\"muted\">seed {seed} · generated {gen}</span></div>\
         <div class=\"progress\" title=\"Dishes with a decision\"><span class=\"num\"><b id=\"p-done\">0</b> of {total} reviewed</span>\
         <div class=\"track\"><div class=\"fill\" id=\"p-fill\"></div></div></div>\
         <span class=\"muted num\">right <b id=\"p-ok\">0</b> · flagged <b id=\"p-flag\">0</b> · unsure <b id=\"p-unsure\">0</b></span>\
         <span class=\"storage-warn\" id=\"storage-warn\">Browser storage unavailable: export before closing!</span>\
         <div class=\"filters\">\
         <select id=\"f-state\" aria-label=\"Decision filter\"><option value=\"all\">All</option><option value=\"undecided\">Undecided</option>\
         <option value=\"decided\">Decided</option><option value=\"flagged\">Flagged</option><option value=\"ok\">Looks right</option>\
         <option value=\"unsure\">Not sure</option></select>\
         <select id=\"f-stratum\" aria-label=\"Stratum filter\"><option value=\"all\">All strata</option>",
        seed = r.seed,
        gen = esc(&r.generated_at.format("%Y-%m-%d %H:%M").to_string()),
    );
    for s in STRATA {
        let _ = write!(h, "<option value=\"{}\">{}</option>", s.key, esc(s.label));
    }
    h.push_str(
        "</select><label class=\"muted\" title=\"Match any dish carrying the stratum's tag, not only dishes drawn for it\">\
         <input type=\"checkbox\" id=\"f-any\"> any tag</label>\
         <select id=\"f-type\" aria-label=\"Content type filter\"><option value=\"all\">All types</option>",
    );
    for t in r.summary.by_type.keys() {
        let _ = write!(h, "<option value=\"{0}\">{0}</option>", esc(t));
    }
    h.push_str(
        "</select><input type=\"search\" id=\"f-q\" placeholder=\"Search titles\" aria-label=\"Search titles\">\
         <button type=\"button\" class=\"primary\" id=\"export\">Export decisions (JSON)</button>\
         <button type=\"button\" id=\"import\">Import…</button><input type=\"file\" id=\"import-file\" accept=\"application/json,.json\" hidden>\
         <button type=\"button\" id=\"clear\">Clear</button></div></div></div>",
    );
}

fn summary(h: &mut String, r: &Report) {
    let s: &Summary = &r.summary;
    h.push_str("<section class=\"summary\"><div class=\"tiles\">");
    let pct = |n: usize, of: usize| if of == 0 { 0.0 } else { 100.0 * n as f64 / of as f64 };
    for (v, k) in [
        (fmt_n(s.dishes), "dishes".to_string()),
        (fmt_n(s.variants), "versions".to_string()),
        (format!("{:.1}", s.mean_versions), format!("versions per dish (max {})", s.max_versions)),
        (fmt_n(s.multi_version_dishes), format!("dishes with 2+ versions ({:.0}%)", pct(s.multi_version_dishes, s.dishes))),
        (fmt_n(s.with_any_option), format!("dishes with options ({:.0}%)", pct(s.with_any_option, s.dishes))),
        (fmt_n(s.in_merge_review), "dishes in an open merge review".to_string()),
        (fmt_n(s.lineage_linked), "dishes sharing lineage with another".to_string()),
        (fmt_n(s.title_twinned), "dishes sharing their cleaned title with another".to_string()),
        (fmt_n(s.protein_twinned), "dishes with a protein twin (likely missed merges)".to_string()),
    ] {
        let _ = write!(h, "<div class=\"tile\"><div class=\"v\">{}</div><div class=\"k\">{}</div></div>", esc(&v), esc(&k));
    }
    h.push_str("</div><div class=\"panels\">");

    // Sampling breakdown first: it says what the 500 are.
    let _ = write!(
        h,
        "<div class=\"panel wide\"><h3>Sample: {} dishes, seed {}</h3><table><thead><tr><th>Stratum</th><th>What it catches</th>\
         <th class=\"n\">Dishes in data</th><th class=\"n\">Sampled</th></tr></thead><tbody>",
        r.cards.len(),
        r.seed
    );
    for st in &r.strata {
        let _ = write!(
            h,
            "<tr><td><span class=\"tag primary\">{}</span></td><td>{}</td><td class=\"n\">{}</td><td class=\"n\">{}</td></tr>",
            esc(st.label),
            esc(st.about),
            fmt_n(st.population),
            st.sampled
        );
    }
    let mut by_type: std::collections::BTreeMap<&str, usize> = Default::default();
    for c in &r.cards {
        *by_type.entry(c.dish.content_type.as_str()).or_default() += 1;
    }
    let types: Vec<String> = by_type.iter().map(|(t, n)| format!("{t} {n}")).collect();
    let _ = write!(
        h,
        "</tbody></table><p class=\"help\">Strata are drawn in the order shown and a dish counts once, for the first stratum it \
         qualifies for (its card also shows the other tags). A stratum with too few dishes hands its share to the others. \
         Sample by content type: {}.</p>\
         <p class=\"help\">Keys: <kbd>j</kbd>/<kbd>k</kbd> next/previous card, <kbd>1</kbd> looks right, <kbd>2</kbd> should split, \
         <kbd>3</kbd> should merge, <kbd>4</kbd> not sure, <kbd>b</kbd> bad title, <kbd>n</kbd> notes, <kbd>Esc</kbd> leave a field. \
         Decisions save as you go; use <b>Export decisions</b> to hand them back. ★ marks the default version.</p></div>",
        esc(&types.join(" · "))
    );

    hist(h, "Versions per dish", &s.versions_hist, s.dishes);
    hist(h, "Distinct titles per dish", &s.titles_hist, s.dishes);

    h.push_str("<div class=\"panel\"><h3>Content types</h3><table><thead><tr><th>Type</th><th class=\"n\">Dishes</th><th class=\"n\">Versions</th></tr></thead><tbody>");
    for (t, (d, v)) in &s.by_type {
        let _ = write!(h, "<tr><td>{}</td><td class=\"n\">{}</td><td class=\"n\">{}</td></tr>", esc(t), fmt_n(*d), fmt_n(*v));
    }
    h.push_str("</tbody></table><h3 style=\"margin-top:10px\">Option coverage</h3><table><thead><tr><th>Option</th><th class=\"n\">Dishes</th>\
                <th class=\"n\">of all</th><th class=\"n\">of 2+ versions</th></tr></thead><tbody>");
    for (label, n) in &s.options {
        let _ = write!(
            h,
            "<tr><td>{}</td><td class=\"n\">{}</td><td class=\"n\">{:.1}%</td><td class=\"n\">{:.1}%</td></tr>",
            esc(label),
            fmt_n(*n),
            pct(*n, s.dishes),
            pct(*n, s.multi_version_dishes)
        );
    }
    h.push_str("</tbody></table></div>");

    h.push_str("<div class=\"panel\"><h3>Review queue</h3><table><thead><tr><th>Kind</th><th>Status</th><th class=\"n\">Items</th></tr></thead><tbody>");
    for (kind, status, n) in &s.review_counts {
        let _ = write!(h, "<tr><td>{}</td><td>{}</td><td class=\"n\">{}</td></tr>", esc(kind), esc(status), fmt_n(*n as usize));
    }
    h.push_str("</tbody></table></div></div></section>");
}

fn hist(h: &mut String, title: &str, rows: &[(&str, usize)], total: usize) {
    let max = rows.iter().map(|r| r.1).max().unwrap_or(1).max(1);
    let _ = write!(h, "<div class=\"panel\"><h3>{}</h3><table class=\"hist\"><tbody>", esc(title));
    for (label, n) in rows {
        let share = if total == 0 { 0.0 } else { 100.0 * *n as f64 / total as f64 };
        let _ = write!(
            h,
            "<tr title=\"{label}: {n} dishes ({share:.1}%)\"><td>{label}</td><td class=\"bar-cell\"><div class=\"hbar\" style=\"width:{w:.1}%\"></div></td>\
             <td class=\"n\">{n}</td><td class=\"n muted\">{share:.1}%</td></tr>",
            label = esc(label),
            n = fmt_n(*n),
            w = 100.0 * *n as f64 / max as f64,
        );
    }
    h.push_str("</tbody></table></div>");
}

fn card(h: &mut String, no: usize, c: &Card) {
    let d = &c.dish;
    let tags: Vec<&str> = c.tags.iter().map(|&t| STRATA[t].key).collect();
    let search = format!("{} {} {}", d.title, d.subtitle.as_deref().unwrap_or(""), d.slug).to_lowercase();
    let _ = write!(
        h,
        "<article class=\"card\" id=\"dish-{no}\" data-id=\"{id}\" data-stratum=\"{st}\" data-tags=\"{tags}\" data-type=\"{ct}\" data-search=\"{search}\">\
         <div class=\"card-h\"><span class=\"no\">#{no}</span><div class=\"ttl\"><h2>{title}</h2>{sub}</div>\
         <div class=\"facts\"><span class=\"ct\">{ct}</span> <b>{n}</b> version{s} · {range} · {titles} title{ts} · <span class=\"slug\">{slug}</span></div>\
         <div class=\"tags\"><span class=\"tag primary\" title=\"Drawn for this stratum\">{label}</span>",
        id = d.id,
        st = STRATA[c.stratum].key,
        tags = tags.join(" "),
        ct = esc(&d.content_type),
        search = esc(&search),
        title = esc(&d.title),
        sub = d.subtitle.as_deref().map(|s| format!("<p class=\"sub\">{}</p>", esc(s))).unwrap_or_default(),
        n = c.versions,
        s = if c.versions == 1 { "" } else { "s" },
        range = esc(&date_range(c.first, c.last)),
        titles = c.title_groups.len(),
        ts = if c.title_groups.len() == 1 { "" } else { "s" },
        slug = esc(&d.slug),
        label = esc(STRATA[c.stratum].label),
    );
    for &t in c.tags.iter().filter(|&&t| t != c.stratum) {
        let _ = write!(h, "<span class=\"tag\">{}</span>", esc(STRATA[t].label));
    }
    h.push_str("</div></div><div class=\"body\"><div class=\"main\">");

    // Options.
    if c.options.is_empty() {
        h.push_str("<div class=\"opts muted\">No options (every version has the same protein, portion and ingredients).</div>");
    } else {
        h.push_str("<div class=\"opts\">");
        for o in &c.options {
            let _ = write!(h, "<span><span class=\"lbl\">{}</span>", esc(&o.label));
            let parts: Vec<String> = o
                .choices
                .iter()
                .map(|(label, n, years)| {
                    let def = o.default_label.as_deref() == Some(label.as_str());
                    format!(
                        "<span class=\"choice{}\" title=\"{}{}\">{} <span class=\"cnt\">×{}</span></span>",
                        if def { " def" } else { "" },
                        esc(years),
                        if def { " · default" } else { "" },
                        esc(label),
                        n
                    )
                })
                .collect();
            h.push_str(&parts.join(" · "));
            h.push_str("</span>");
        }
        h.push_str("</div>");
    }

    // Versions grouped by cleaned title.
    h.push_str(
        "<div class=\"tablewrap\"><table class=\"tg\"><thead><tr><th>Title (cleaned)</th><th class=\"n\">#</th><th>Years</th>\
         <th>Protein</th><th>Portion</th><th>Version</th></tr></thead><tbody>",
    );
    for g in &c.title_groups {
        let _ = write!(
            h,
            "<tr{}><td class=\"t\">{}{}{}</td><td class=\"n\">{}</td><td class=\"num\">{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
            if g.has_default { " class=\"def\"" } else { "" },
            esc(&g.title),
            g.subtitle.as_deref().map(|s| format!("<br><span class=\"muted\">{}</span>", esc(s))).unwrap_or_default(),
            if g.copied_in > 0 {
                format!(" <span class=\"copied\" title=\"Versions copied from a version in another dish\">↪ {} from other dish</span>", g.copied_in)
            } else {
                String::new()
            },
            g.count,
            esc(&g.years),
            counted(&g.proteins),
            counted(&g.portions),
            counted(&g.versions),
        );
    }
    h.push_str("</tbody></table></div>");

    // Every version.
    let _ = write!(
        h,
        "<details><summary>All {} version{} with raw titles</summary><div class=\"tablewrap\"><table class=\"all\"><thead><tr>\
         <th>Published</th><th>Raw title / subtitle</th><th>Protein</th><th>Portion</th><th>Version</th><th>Steps</th><th>Copied from</th>\
         </tr></thead><tbody>",
        c.versions,
        if c.versions == 1 { "" } else { "s" }
    );
    for v in &c.rows {
        let parent = match &v.parent {
            Parent::None => "<span class=\"muted\">—</span>".to_string(),
            Parent::SameDish => "this dish".to_string(),
            Parent::Other(title, slug) => format!("<b>other dish:</b> {} <span class=\"slug\">{}</span>", esc(title), esc(slug)),
        };
        let _ = write!(
            h,
            "<tr{}><td class=\"num\">{}</td><td class=\"raw\">{}{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
            if v.is_default { " class=\"def\"" } else { "" },
            esc(&date(v.published)),
            esc(&v.title),
            v.subtitle.as_deref().map(|s| format!("<br><span class=\"muted\">{}</span>", esc(s))).unwrap_or_default(),
            esc(&v.protein),
            esc(&v.portion),
            esc(&v.version),
            if v.has_steps { "yes" } else { "<b>no</b>" },
            parent,
        );
    }
    h.push_str("</tbody></table></div></details></div><div class=\"side\">");

    // Default version's ingredients.
    h.push_str("<div><h3>Default version</h3>");
    if let Some(dv) = &c.default_version {
        let _ = write!(
            h,
            "<p class=\"dv\">{} · {}{}</p>",
            esc(&mise_core::dish_titles::clean_title(&dv.title)),
            esc(&date(dv.published)),
            if dv.has_steps { "" } else { " · no steps" }
        );
    }
    if c.shipped.is_empty() {
        h.push_str("<p class=\"muted\">No shipped ingredients.</p>");
    } else {
        h.push_str("<ul class=\"foods\">");
        for f in &c.shipped {
            food(h, &f.name, f.matched);
        }
        h.push_str("</ul>");
    }
    if !c.pantry.is_empty() {
        let _ = write!(h, "<details><summary>Pantry ({})</summary><ul class=\"foods small\">", c.pantry.len());
        for f in &c.pantry {
            food(h, &f.name, f.matched);
        }
        h.push_str("</ul></details>");
    }
    h.push_str("</div>");

    // Open merge reviews.
    if !c.reviews.is_empty() {
        let _ = write!(h, "<div><h3>Open merge review{}</h3>", if c.reviews.len() == 1 { "" } else { "s" });
        for rv in &c.reviews {
            let lead = if rv.this_is_copy {
                format!("This dish's “{}” was copied from “{}” in", esc(&rv.copy_title), esc(&rv.source_title))
            } else {
                format!("“{}” in", esc(&rv.copy_title))
            };
            let tail = if rv.this_is_copy { String::new() } else { format!(" was copied from this dish's “{}”", esc(&rv.source_title)) };
            let _ = write!(
                h,
                "<div class=\"rev\">{lead} {other}{tail}.<br><span class=\"muted\">title similarity</span> <b>{sim}</b> · \
                 <span class=\"muted\">shipped overlap</span> <b>{ov}</b> · <span class=\"muted\">confidence</span> {conf}</div>",
                other = dish_ref(&rv.other),
                sim = rv.title_similarity.map(|x| format!("{x:.2}")).unwrap_or_else(|| "?".into()),
                ov = rv.shipped_overlap.map(|x| format!("{x:.2}")).unwrap_or_else(|| "?".into()),
                conf = rv.confidence.map(|x| format!("{x:.2}")).unwrap_or_else(|| "?".into()),
            );
        }
        h.push_str("</div>");
    }

    // Look-alike dishes.
    if !c.related.is_empty() {
        h.push_str("<div><h3>Look-alike dishes</h3>");
        for rel in &c.related {
            related(h, rel);
        }
        h.push_str("</div>");
    }
    h.push_str("</div></div>");

    // Decision.
    let name = format!("d{no}");
    let _ = write!(
        h,
        "<form class=\"decide\" autocomplete=\"off\">\
         <label class=\"opt right\"><input type=\"radio\" name=\"{name}\" value=\"right\"> Looks right</label>\
         <label class=\"opt split\"><input type=\"radio\" name=\"{name}\" value=\"split\"> Wrong merge — should split</label>\
         <label class=\"opt merge\"><input type=\"radio\" name=\"{name}\" value=\"merge\"> Should be merged with…</label>\
         <label class=\"opt unsure\"><input type=\"radio\" name=\"{name}\" value=\"unsure\"> Not sure</label>\
         <label class=\"opt title\"><input type=\"checkbox\" class=\"bad-title\"> Bad title</label>\
         <input type=\"text\" class=\"merge-with\" placeholder=\"merge with: dish slug or title\" aria-label=\"Merge with\">\
         <span class=\"saved\"></span>\
         <textarea placeholder=\"Notes (which versions to split off, a better title…)\" aria-label=\"Notes\"></textarea></form></article>"
    );
}

fn related(h: &mut String, rel: &Related) {
    let _ = write!(
        h,
        "<div class=\"rel\">{} <button type=\"button\" class=\"mw\" data-merge=\"{}\" title=\"Mark this dish as 'should be merged with' that one\">merge with</button>\
         <br><span class=\"why\">{}</span>{}",
        dish_ref(&rel.dish),
        esc(&rel.dish.slug),
        esc(&rel.why.join(", ")),
        rel.overlap
            .map(|o| format!(" · <span class=\"muted\">shipped overlap</span> <span class=\"ov\">{:.0}%</span>", o * 100.0))
            .unwrap_or_default()
    );
    if !rel.shipped.is_empty() {
        h.push_str("<details><summary>its shipped ingredients</summary><ul class=\"foods small\">");
        for f in &rel.shipped {
            food(h, f, true);
        }
        h.push_str("</ul></details>");
    }
    h.push_str("</div>");
}

/// A dish named in another dish's card, linked when it has its own card.
fn dish_ref(d: &RelatedDish) -> String {
    let name = match d.in_sample {
        Some(n) => format!("<a href=\"#dish-{n}\"><b>{}</b></a> <span class=\"muted\">(#{n})</span>", esc(&d.title)),
        None => format!("<b>{}</b>", esc(&d.title)),
    };
    let sub = d.subtitle.as_deref().map(|s| format!(" <span class=\"muted\">{}</span>", esc(s))).unwrap_or_default();
    format!(
        "{name}{sub} <span class=\"muted\">· {} · {} version{}{}{}</span> <span class=\"slug\">{}</span>",
        esc(&d.content_type),
        d.versions,
        if d.versions == 1 { "" } else { "s" },
        if d.years.is_empty() { "" } else { " · " },
        esc(&d.years),
        esc(&d.slug)
    )
}

fn food(h: &mut String, name: &str, matched: bool) {
    let _ = write!(
        h,
        "<li{}>{}</li>",
        if matched { "" } else { " class=\"unmatched\" title=\"Not matched to a food; raw name shown\"" },
        esc(name)
    );
}

/// "Chicken ×5, Pork ×2"; a single value is shown bare.
fn counted(values: &[(String, usize)]) -> String {
    match values {
        [] => String::new(),
        [(v, _)] => esc(v),
        _ => values.iter().map(|(v, n)| format!("{} <span class=\"cnt\">×{n}</span>", esc(v))).collect::<Vec<_>>().join(", "),
    }
}

fn date(d: Option<DateTime<Utc>>) -> String {
    d.map(|d| d.format("%Y-%m-%d").to_string()).unwrap_or_else(|| "undated".into())
}

fn date_range(a: Option<DateTime<Utc>>, b: Option<DateTime<Utc>>) -> String {
    match (a, b) {
        (Some(a), Some(b)) if a.date_naive() == b.date_naive() => a.format("%Y-%m-%d").to_string(),
        (Some(a), Some(b)) => format!("{} – {}", a.format("%Y-%m"), b.format("%Y-%m")),
        _ => "undated".into(),
    }
}

fn fmt_n(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, ch) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// Escape text for HTML element content and quoted attribute values.
pub fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

/// JSON safe to place inside a <script> element: no "</script>", no "<!--", no stray line separators.
fn script_json(v: &serde_json::Value) -> String {
    v.to_string()
        .replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('&', "\\u0026")
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_markup_in_text_and_attributes() {
        assert_eq!(esc(r#"<b>"Tom's" & Jerry</b>"#), "&lt;b&gt;&quot;Tom&#39;s&quot; &amp; Jerry&lt;/b&gt;");
    }

    #[test]
    fn script_json_cannot_close_the_script() {
        let s = script_json(&json!({ "title": "</script><script>alert(1)</script>" }));
        assert!(!s.contains("</script"));
        assert_eq!(serde_json::from_str::<serde_json::Value>(&s).unwrap()["title"], "</script><script>alert(1)</script>");
    }

    #[test]
    fn thousands() {
        assert_eq!(fmt_n(0), "0");
        assert_eq!(fmt_n(999), "999");
        assert_eq!(fmt_n(16776), "16,776");
        assert_eq!(fmt_n(1234567), "1,234,567");
    }
}
