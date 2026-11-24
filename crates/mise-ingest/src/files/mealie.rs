//! Mealie: an export is a zip with each recipe at `recipes/<slug>/<slug>.json` and its photo at
//! `recipes/<slug>/images/original.webp`. The zip's JSON names fields in snake_case (`recipe_ingredient`); the API's,
//! which a recipe saved from Mealie's web app has, in camelCase (`recipeIngredient`); both are read.
//!
//! An ingredient is kept as Mealie read it (quantity, unit, food, note) and as it was written (`original_text`),
//! which mise prefers, reading amounts itself. A step's `title` starts a section that runs until the next title.
//! Times are text as written ("25 minutes"), with the seconds beside them in newer versions, which come first.

use serde_json::{Map, Value, json};

use super::{amount_text, names, number, text, via_schema_org};
use crate::schema_org::{ReadError, Record};

/// Whether a JSON object is a Mealie recipe: its `slug`, with ingredients or instructions as Mealie keeps them.
pub(super) fn is(map: &Map<String, Value>) -> bool {
    map.contains_key("slug")
        && [
            "recipe_ingredient",
            "recipeIngredient",
            "recipe_instructions",
            "recipeInstructions",
        ]
        .iter()
        .any(|k| map.get(*k).is_some_and(Value::is_array))
}

/// A field by its snake_case name, or its camelCase one.
fn field<'a>(map: &'a Map<String, Value>, snake: &str, camel: &str) -> Option<&'a Value> {
    map.get(snake).or_else(|| map.get(camel))
}

/// One ingredient line as a cook writes it.
fn line(item: &Value) -> Option<String> {
    let item = match item {
        Value::String(s) => return Some(s.trim().to_owned()).filter(|s| !s.is_empty()),
        Value::Object(m) => m,
        _ => return None,
    };
    if let Some(written) = text(item, &["original_text", "originalText"]) {
        return Some(written.to_owned());
    }
    let named = |key: &str, plural: bool| {
        let thing = item.get(key)?;
        if let Some(s) = thing.as_str() {
            return Some(s.trim().to_owned());
        }
        let thing = thing.as_object()?;
        let plural = plural
            .then(|| text(thing, &["plural_name", "pluralName"]))
            .flatten();
        plural.or_else(|| text(thing, &["name"])).map(str::to_owned)
    };
    let quantity = number(item, &["quantity"]);
    let many = quantity.is_some_and(|q| q > 1.0);
    let Some(food) = named("food", many).filter(|f| !f.is_empty()) else {
        // A line Mealie didn't read into parts keeps all of it in the note.
        return text(item, &["note"])
            .or_else(|| text(item, &["display"]))
            .map(str::to_owned);
    };
    let mut parts: Vec<String> = Vec::new();
    if let Some(q) = quantity {
        parts.push(amount_text(q));
    }
    if let Some(unit) = named("unit", many).filter(|u| !u.is_empty()) {
        parts.push(unit);
    }
    parts.push(food);
    let mut line = parts.join(" ");
    if let Some(note) = text(item, &["note"]) {
        line = format!("{line}, {note}");
    }
    Some(line)
}

/// The method as schema.org steps, each run of steps under a title as a section.
fn instructions(steps: Option<&Value>) -> Vec<Value> {
    let Some(Value::Array(steps)) = steps else {
        return Vec::new();
    };
    let mut out: Vec<Value> = Vec::new();
    for step in steps {
        let (title, text) = match step {
            Value::String(s) => (None, Some(s.as_str())),
            Value::Object(m) => (
                text(m, &["title"]),
                text(m, &["text"]).or_else(|| text(m, &["summary"])),
            ),
            _ => continue,
        };
        if let Some(title) = title {
            out.push(json!({"@type": "HowToSection", "name": title, "itemListElement": []}));
        }
        let Some(text) = text else { continue };
        let how = json!({"@type": "HowToStep", "text": text});
        match out.last_mut() {
            Some(section) if section["@type"] == "HowToSection" => section["itemListElement"]
                .as_array_mut()
                .expect("a section made here lists its steps")
                .push(how),
            _ => out.push(how),
        }
    }
    out
}

/// A time: its seconds when Mealie keeps them, else as written.
fn time(map: &Map<String, Value>, snake: &str, camel: &str) -> Option<String> {
    let seconds = number(
        map,
        &[&format!("{snake}_seconds"), &format!("{camel}Seconds")],
    );
    seconds
        .map(|s| format!("PT{}S", s.round()))
        .or_else(|| text(map, &[snake, camel]).map(str::to_owned))
}

pub(super) fn read(map: &Map<String, Value>) -> Result<Record, ReadError> {
    let lines: Vec<String> = match field(map, "recipe_ingredient", "recipeIngredient") {
        Some(Value::Array(items)) => items.iter().filter_map(line).collect(),
        _ => Vec::new(),
    };
    // Servings first; then what the yield counts, when it says ("1 loaf", or "24" and "cookies").
    let mut yields: Vec<Value> = Vec::new();
    if let Some(servings) = number(map, &["recipe_servings", "recipeServings"]) {
        yields.push(json!(amount_text(servings)));
    }
    let yield_text = text(map, &["recipe_yield", "recipeYield"]);
    match (
        number(map, &["recipe_yield_quantity", "recipeYieldQuantity"]),
        yield_text,
    ) {
        (Some(q), Some(what)) => yields.push(json!(format!("{} {what}", amount_text(q)))),
        (None, Some(what)) => yields.push(json!(what)),
        _ => {}
    }
    let schema = json!({
        "@type": "Recipe",
        "name": text(map, &["name"]),
        "description": text(map, &["description"]),
        "recipeIngredient": lines,
        "recipeInstructions": instructions(field(map, "recipe_instructions", "recipeInstructions")),
        "recipeYield": yields,
        "prepTime": time(map, "prep_time", "prepTime"),
        "cookTime": time(map, "perform_time", "performTime")
            .or_else(|| time(map, "cook_time", "cookTime")),
        "totalTime": time(map, "total_time", "totalTime"),
        "recipeCategory": names(field(map, "recipe_category", "recipeCategory")),
        "keywords": names(map.get("tags")),
        "url": text(map, &["org_url", "orgURL"]),
    });
    let id = text(map, &["id"])
        .or_else(|| text(map, &["slug"]))
        .map(str::to_owned);
    via_schema_org(schema, Value::Object(map.clone()), id)
}
