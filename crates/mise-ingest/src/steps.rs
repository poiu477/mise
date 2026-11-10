//! Parse step text into amount templates (mise_core::step_amounts), linking each amount to the ingredient line
//! it measures. Re-runnable: every step is parsed again from its published text.

use std::collections::HashMap;

use anyhow::{Context, Result};
use mise_core::step_amounts::{self, Part};
use rayon::prelude::*;
use sqlx::PgPool;
use uuid::Uuid;

const BATCH: usize = 5000;

/// What one parse found across every step, for the command's summary.
#[derive(Debug, Default)]
pub struct ParseStats {
    /// Steps parsed.
    pub steps: usize,
    /// Steps whose published text varies by yield.
    pub with_alts: usize,
    /// Amounts found in step text: "1 TBSP butter".
    pub amounts: usize,
    /// Amounts that publish another yield's amount too: "1 TBSP butter (2 TBSP for 4 servings)".
    pub amounts_with_alts: usize,
    /// Amounts tied to the ingredient line they measure out ("butter"), whose published amounts they can follow.
    pub linked: usize,
    /// Words that change with the yield: "half the lime zest" → "all the lime zest".
    pub word_alternatives: usize,
    /// Remarks for larger yields, kept as written: "(For 4 servings, use two baking sheets.)".
    pub notes: usize,
    /// Yield-dependent parentheticals left as published text.
    pub unparsed: usize,
}

/// Parses every step's published text again into a template it can render at any serving count, linking
/// amounts to the ingredient lines they measure out, and stores the templates. Steps of soups and rice dishes
/// drop the boil-off rule, since their water is the dish.
pub async fn parse_steps(pool: &PgPool) -> Result<ParseStats> {
    let mut lines: HashMap<Uuid, Vec<(String, String)>> = HashMap::new();
    let rows: Vec<(Uuid, String, String)> = sqlx::query_as(
        "SELECT variant_id, line_key, raw_name FROM ingredient_lines ORDER BY variant_id, position",
    )
    .fetch_all(pool)
    .await
    .context("loading ingredient lines")?;
    for (v, key, name) in rows {
        lines.entry(v).or_default().push((key, name));
    }
    let steps: Vec<(Uuid, Uuid, String, String)> =
        sqlx::query_as("SELECT s.id, s.variant_id, s.text, v.title FROM steps s JOIN variants v ON v.id = s.variant_id")
            .fetch_all(pool)
            .await
            .context("loading steps")?;

    let empty = Vec::new();
    let parsed: Vec<(Uuid, step_amounts::Template)> = steps
        .par_iter()
        .map(|(id, v, text, title)| {
            let mut t = step_amounts::parse(text, lines.get(v).unwrap_or(&empty));
            // A soup's or a rice dish's water is the dish, whatever the step says.
            if step_amounts::not_a_pan_sauce(title) {
                t.drop_boil_off();
            }
            (*id, t)
        })
        .collect();

    let mut stats = ParseStats {
        steps: parsed.len(),
        ..Default::default()
    };
    for (_, t) in &parsed {
        stats.with_alts += t.has_alts() as usize;
        stats.unparsed += t.unparsed;
        for p in &t.parts {
            match p {
                Part::Amount { alts, line, .. } => {
                    stats.amounts += 1;
                    stats.amounts_with_alts += !alts.is_empty() as usize;
                    stats.linked += line.is_some() as usize;
                }
                Part::Words { .. } => stats.word_alternatives += 1,
                Part::Note { .. } => stats.notes += 1,
                Part::Text { .. } => {}
            }
        }
    }

    for chunk in parsed.chunks(BATCH) {
        let ids: Vec<Uuid> = chunk.iter().map(|(id, _)| *id).collect();
        let json: Vec<String> = chunk
            .iter()
            .map(|(_, t)| serde_json::to_string(t))
            .collect::<Result<_, _>>()?;
        sqlx::query("UPDATE steps s SET template = x.t::jsonb FROM UNNEST($1::uuid[], $2::text[]) AS x(id, t) WHERE s.id = x.id")
            .bind(&ids)
            .bind(&json)
            .execute(pool)
            .await
            .context("writing step templates")?;
    }
    Ok(stats)
}
