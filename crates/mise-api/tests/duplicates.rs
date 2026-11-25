//! Near-duplicate foods: matching gives every name it doesn't know a food of its own, so two spellings of one
//! food become two foods and nothing used to ask about it. Each pair is raised as a `food_duplicate` item, and
//! the answer is kept the way the `mise` curation commands keep theirs, so a rebuilt database keeps it.
//!
//! The lines go in through a bundle, which stores an ingredient's name as the source wrote it: that is how
//! "Red Onion, Diced" reaches matching with the note still in the name, and becomes a food of its own.

#[macro_use]
mod common;

use common::TestApp;
use mise_ingest::foods;
use serde_json::{Value, json};
use sqlx::PgPool;

const QUEUE: &str = "/api/v1/review-items";

/// Imports two recipes, each with its own ingredient names, as one bundle.
async fn import(pool: &PgPool, first: &[&str], second: &[&str]) {
    let dir = tempfile::tempdir().unwrap();
    let manifest = json!({
        "format": "mise-seed/1",
        "source": {"key": "seed:duplicate-test", "name": "Recipes the duplicate tests pair up"},
        "created": "2026-11-25T00:00:00Z",
        "media_root": ".",
    });
    std::fs::write(dir.path().join("manifest.json"), manifest.to_string()).unwrap();
    let recipe = |id: &str, title: &str, names: &[&str]| {
        json!({
            "external_id": id,
            "title": title,
            "content_type": "recipe",
            "published_at": "2026-11-01T12:00:00Z",
            "yields": [2],
            "ingredients": names.iter().map(|n| json!({"key": n, "name": n, "shipped": true, "allergens": []})).collect::<Vec<_>>(),
            "amounts": names.iter().map(|n| json!({"key": n, "servings": 2, "amount": 1, "unit": "unit"})).collect::<Vec<_>>(),
            "steps": [{"index": 1, "text": "Cook everything."}],
        })
    };
    let a = recipe("hash", "Red Onion Hash", first);
    let b = recipe("salad", "Chopped Salad", second);
    std::fs::write(dir.path().join("variants.ndjson"), format!("{a}\n{b}\n")).unwrap();
    mise_ingest::bundle::import_bundle(pool, dir.path())
        .await
        .unwrap();
}

/// The two spellings of one food the tests use, in different versions.
async fn with_near_duplicates() -> Option<(TestApp, String)> {
    let app = TestApp::new().await?;
    import(
        &app.pool,
        &["Red Onions", "Garlic"],
        &["Red Onion, Diced", "Parsley"],
    )
    .await;
    foods::match_foods(&app.pool).await.unwrap();
    let owner = app.setup_owner().await;
    Some((app, owner))
}

/// The open `food_duplicate` items.
async fn items(app: &TestApp, owner: &str) -> Vec<Value> {
    let queue = app.json("GET", QUEUE, owner, None).await;
    queue["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|i| i["kind"] == "food_duplicate")
        .cloned()
        .collect()
}

/// Answers one item.
async fn decide(app: &TestApp, owner: &str, item: &Value, answer: Value) -> Value {
    let path = format!("{QUEUE}/{}/decision", item["id"].as_str().unwrap());
    app.json("POST", &path, owner, Some(answer)).await
}

/// Undoes matching and runs it again, as `mise match-foods --reset` does.
async fn rematch(pool: &PgPool) {
    foods::reset_matching(pool).await.unwrap();
    foods::match_foods(pool).await.unwrap();
}

/// The foods the two red onion lines point at, by name, and how many lines each covers.
async fn onion_foods(pool: &PgPool) -> Vec<(String, i64)> {
    sqlx::query_as(
        "SELECT f.name, count(*) FROM ingredient_lines il JOIN foods f ON f.id = il.food_id
         WHERE il.raw_name IN ('Red Onions', 'Red Onion, Diced') GROUP BY f.name ORDER BY f.name",
    )
    .fetch_all(pool)
    .await
    .unwrap()
}

/// How many open `food_duplicate` items there are.
async fn open(pool: &PgPool) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM review_items WHERE kind = 'food_duplicate' AND status = 'open'",
    )
    .fetch_one(pool)
    .await
    .unwrap()
}

#[tokio::test]
async fn two_spellings_of_one_food_are_raised_once() {
    let (app, owner) = test_app!(with_near_duplicates());
    // "Red Onions" and "Red Onion, Diced" are two foods, and neither is a kind of the other, so nothing but this
    // item would say so.
    let raised = items(&app, &owner).await;
    assert_eq!(raised.len(), 1, "{raised:#?}");
    let item = &raised[0];
    assert_eq!(item["subject"], "Red Onions", "the more used spelling");
    assert!(
        item["proposal"]
            .as_str()
            .unwrap()
            .contains("Red Onion, Diced"),
        "{item:#?}"
    );
    // The foods that differ only by a colour or a variety are kinds of their base, not duplicates.
    assert_eq!(open(&app.pool).await, 1);
}

#[tokio::test]
async fn merging_leaves_one_food_and_survives_a_reset() {
    let (app, owner) = test_app!(with_near_duplicates());
    let item = items(&app, &owner).await.remove(0);
    decide(&app, &owner, &item, json!({"decision": "accept"})).await;

    // One food now covers both lines.
    assert_eq!(onion_foods(&app.pool).await, [("Red Onions".into(), 2)]);
    assert_eq!(open(&app.pool).await, 0);

    // The merge is a curation edit, so matching from scratch keeps it and doesn't ask again.
    rematch(&app.pool).await;
    assert_eq!(onion_foods(&app.pool).await, [("Red Onions".into(), 2)]);
    assert_eq!(open(&app.pool).await, 0);
}

#[tokio::test]
async fn keeping_two_foods_stops_the_pair_coming_back() {
    let (app, owner) = test_app!(with_near_duplicates());
    let item = items(&app, &owner).await.remove(0);
    decide(&app, &owner, &item, json!({"decision": "reject"})).await;

    // Still two foods, and nothing open.
    assert_eq!(onion_foods(&app.pool).await.len(), 2);
    assert_eq!(open(&app.pool).await, 0);

    // The pair is kept by name, so it isn't raised again even though matching made both foods afresh.
    rematch(&app.pool).await;
    assert_eq!(onion_foods(&app.pool).await.len(), 2);
    assert_eq!(open(&app.pool).await, 0, "the pair was told apart");
}

#[tokio::test]
async fn one_can_be_filed_as_a_kind_of_the_other() {
    let (app, owner) = test_app!(with_near_duplicates());
    let item = items(&app, &owner).await.remove(0);
    decide(&app, &owner, &item, json!({"decision": "edit"})).await;

    // Two foods still, with the narrower name under the plain one the item asked about.
    let parent: Option<String> = sqlx::query_scalar(
        "SELECT p.name FROM foods c JOIN foods p ON p.id = c.parent_id WHERE c.name = 'Red Onion, Diced'",
    )
    .fetch_optional(&app.pool)
    .await
    .unwrap();
    assert_eq!(parent.as_deref(), Some("Red Onions"));
    assert_eq!(onion_foods(&app.pool).await.len(), 2);
    assert_eq!(open(&app.pool).await, 0);
}

#[tokio::test]
async fn the_export_brings_a_kept_pair_back() {
    let (app, owner) = test_app!(with_near_duplicates());
    let item = items(&app, &owner).await.remove(0);
    decide(&app, &owner, &item, json!({"decision": "reject"})).await;

    let ops: Vec<Value> = mise_ingest::curate::export(&app.pool)
        .await
        .unwrap()
        .iter()
        .map(|op| json!(op))
        .filter(|op| op["op"] == "distinct")
        .collect();
    assert_eq!(ops.len(), 1, "{ops:#?}");
    // Both names as the pairing keys them, so the answer holds however the pair is spelled next time.
    assert_eq!(ops[0]["op"], "distinct");
}
