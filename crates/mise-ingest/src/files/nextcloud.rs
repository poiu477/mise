//! Nextcloud Cookbook: a folder of recipes, each its own folder holding a `recipe.json` (schema.org Recipe JSON-LD,
//! with Nextcloud's own `id`, `dateCreated` and image addresses beside the schema.org fields) and its photo as
//! `full.jpg`, with `thumb.jpg` and `thumb16.jpg` made from it. Downloading the `Recipes` folder from Nextcloud
//! gives the same as a zip.

use serde_json::Value;

use crate::schema_org::Record;

/// Whether a schema.org document is a Nextcloud Cookbook recipe: Nextcloud names every one `recipe.json`.
pub(super) fn is(path: &str) -> bool {
    path.rsplit('/')
        .next()
        .is_some_and(|f| f.eq_ignore_ascii_case("recipe.json"))
}

/// The record with Nextcloud's id for the recipe as its id, when it has one: Nextcloud keeps it while the recipe
/// is changed, renamed or given another link.
pub(super) fn with_id(mut record: Record) -> Record {
    let id = match record.raw.get("id") {
        Some(Value::String(s)) if !s.trim().is_empty() => Some(s.trim().to_owned()),
        Some(Value::Number(n)) => Some(n.to_string()),
        _ => None,
    };
    if let Some(id) = id {
        record.external_id = id;
    }
    record
}
