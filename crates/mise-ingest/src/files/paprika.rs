//! Paprika: an export (`.paprikarecipes`) is a zip of recipes (`<name>.paprikarecipe`), each gzipped JSON. A recipe
//! keeps its ingredients and directions as text, one to a line; its serving count, times and source as written
//! ("4 servings", "35 mins"); its categories by name; and its photo inline, as base64 in `photo_data`.

use base64::Engine;
use serde_json::{Map, Value, json};

use super::{InlinePhoto, names, text, via_schema_org};
use crate::schema_org::{ReadError, Record};

/// Whether a JSON object is a Paprika recipe: its `uid`, and its ingredients or directions as text.
pub(super) fn is(map: &Map<String, Value>) -> bool {
    map.contains_key("uid")
        && ["ingredients", "directions"]
            .iter()
            .any(|k| map.get(*k).is_some_and(Value::is_string))
}

/// Fields that hold a photo, which the record's copy of the recipe leaves out.
const PHOTO_FIELDS: &[&str] = &["photo_data", "photos"];

/// The recipe, and its photo when it has one inline (or why that couldn't be read).
pub(super) fn read(map: &Map<String, Value>) -> Result<(Record, InlinePhoto), ReadError> {
    let schema = json!({
        "@type": "Recipe",
        "name": text(map, &["name"]),
        "description": text(map, &["description"]),
        "recipeIngredient": text(map, &["ingredients"]),
        "recipeInstructions": text(map, &["directions"]),
        "recipeYield": text(map, &["servings"]),
        "prepTime": text(map, &["prep_time"]),
        "cookTime": text(map, &["cook_time"]),
        "totalTime": text(map, &["total_time"]),
        "recipeCategory": names(map.get("categories")),
        "url": text(map, &["source_url"]),
        "image": text(map, &["image_url"]),
    });
    let photo = text(map, &["photo_data"]).map(|data| {
        let data: String = data.chars().filter(|c| !c.is_whitespace()).collect();
        base64::engine::general_purpose::STANDARD
            .decode(data)
            .map_err(|_| "it isn't readable base64".to_owned())
    });
    let mut raw = map.clone();
    for field in PHOTO_FIELDS {
        raw.remove(*field);
    }
    let id = text(map, &["uid"]).map(str::to_owned);
    Ok((via_schema_org(schema, Value::Object(raw), id)?, photo))
}
