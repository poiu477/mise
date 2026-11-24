//! Tandoor: an export in Tandoor's own format is a zip holding a zip for each recipe (`<id>.zip`), which holds
//! `recipe.json` and the photo as `image.<ext>`. A recipe is a list of steps, each with its instruction (Markdown)
//! and the ingredients it uses as amount, unit, food and note; a header among them is a line with `is_header` set.
//! The export keeps no ingredient as it was written, no id and no photo link. Times are whole minutes, working and
//! waiting.

use serde_json::{Map, Value, json};

use super::{amount_text, names, number, text, via_schema_org};
use crate::schema_org::{ReadError, Record};

/// Whether a JSON object is a Tandoor recipe: its steps, with the fields only Tandoor's export has.
pub(super) fn is(map: &Map<String, Value>) -> bool {
    map.get("steps").is_some_and(Value::is_array)
        && ["working_time", "waiting_time", "servings_text"]
            .iter()
            .any(|k| map.contains_key(*k))
}

/// Items of a list, in Tandoor's `order`, keeping the list's order among equals.
fn in_order(value: Option<&Value>) -> Vec<&Map<String, Value>> {
    let mut items: Vec<&Map<String, Value>> = match value {
        Some(Value::Array(items)) => items.iter().filter_map(Value::as_object).collect(),
        _ => Vec::new(),
    };
    items.sort_by_key(|m| m.get("order").and_then(Value::as_i64).unwrap_or(0));
    items
}

/// One ingredient as a cook writes it, or nothing for a header or an empty line.
fn line(item: &Map<String, Value>) -> Option<String> {
    if item.get("is_header").and_then(Value::as_bool) == Some(true) {
        return None;
    }
    let amount = if item.get("no_amount").and_then(Value::as_bool) == Some(true) {
        None
    } else {
        number(item, &["amount"])
    };
    let many = amount.is_some_and(|a| a > 1.0);
    let named = |key: &str| {
        let thing = item.get(key)?.as_object()?;
        let plural = many.then(|| text(thing, &["plural_name"])).flatten();
        plural.or_else(|| text(thing, &["name"])).map(str::to_owned)
    };
    let mut parts: Vec<String> = Vec::new();
    if let Some(amount) = amount {
        parts.push(amount_text(amount));
    }
    parts.extend(named("unit"));
    parts.extend(named("food"));
    let note = text(item, &["note"]);
    let line = match (parts.is_empty(), note) {
        (true, None) => return None,
        (true, Some(note)) => note.to_owned(),
        (false, None) => parts.join(" "),
        (false, Some(note)) => format!("{}, {note}", parts.join(" ")),
    };
    Some(line)
}

/// Whole minutes as an ISO 8601 duration.
fn minutes(n: Option<f64>) -> Option<String> {
    n.map(|n| format!("PT{}M", n.round()))
}

pub(super) fn read(map: &Map<String, Value>) -> Result<Record, ReadError> {
    let steps = in_order(map.get("steps"));
    let lines: Vec<String> = steps
        .iter()
        .flat_map(|step| in_order(step.get("ingredients")))
        .filter_map(line)
        .collect();
    // A named step is a section of its own; its instruction's lines are its steps.
    let method: Vec<Value> = steps
        .iter()
        .filter_map(|step| {
            let instruction = text(step, &["instruction"])?;
            Some(match text(step, &["name"]) {
                Some(name) => {
                    json!({"@type": "HowToSection", "name": name, "itemListElement": instruction})
                }
                None => json!(instruction),
            })
        })
        .collect();
    let servings = number(map, &["servings"]).map(amount_text);
    let recipe_yield = match (servings, text(map, &["servings_text"])) {
        (Some(n), Some(what)) => Some(format!("{n} {what}")),
        (n, _) => n,
    };
    let working = number(map, &["working_time"]);
    let waiting = number(map, &["waiting_time"]);
    let total = match (working, waiting) {
        (None, None) => None,
        (a, b) => Some(a.unwrap_or(0.0) + b.unwrap_or(0.0)),
    };
    let schema = json!({
        "@type": "Recipe",
        "name": text(map, &["name"]),
        "description": text(map, &["description"]),
        "recipeIngredient": lines,
        "recipeInstructions": method,
        "recipeYield": recipe_yield,
        "prepTime": minutes(working),
        "totalTime": minutes(total),
        "keywords": names(map.get("keywords")),
        "url": text(map, &["source_url"]),
    });
    via_schema_org(schema, Value::Object(map.clone()), None)
}
