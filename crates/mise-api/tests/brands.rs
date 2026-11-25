//! Brands are data: curation adds and removes them, and food matching strips the ones the database holds from
//! ingredient names. The ingest stages run here as `mise` runs them, on the API tests' throwaway database.

#[macro_use]
mod common;

use common::TestApp;
use mise_ingest::{curate, foods};
use serde_json::{Value, json};
use sqlx::PgPool;

/// Imports one recipe whose lines are `names`, as a bundle of its own.
async fn import(pool: &PgPool, names: &[&str]) {
    let dir = tempfile::tempdir().unwrap();
    let manifest = json!({
        "format": "mise-seed/1",
        "source": {"key": "seed:brands-test", "name": "A recipe the brand tests match"},
        "created": "2026-11-24T00:00:00Z",
        "media_root": ".",
    });
    std::fs::write(dir.path().join("manifest.json"), manifest.to_string()).unwrap();
    let recipe = json!({
        "external_id": "goulash",
        "title": "Paprika Goulash",
        "content_type": "recipe",
        "published_at": "2026-11-01T12:00:00Z",
        "yields": [2],
        "ingredients": names.iter().map(|n| json!({"key": n, "name": n, "shipped": true, "allergens": []})).collect::<Vec<_>>(),
        "amounts": names.iter().map(|n| json!({"key": n, "servings": 2, "amount": 1, "unit": "tsp"})).collect::<Vec<_>>(),
        "steps": [{"index": 1, "text": "Cook everything."}],
    });
    std::fs::write(dir.path().join("variants.ndjson"), format!("{recipe}\n")).unwrap();
    mise_ingest::bundle::import_bundle(pool, dir.path())
        .await
        .unwrap();
}

/// Applies `ops` as a curation file, as `mise food apply` does.
async fn apply(pool: &PgPool, ops: Value) {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("curation.json");
    std::fs::write(&file, ops.to_string()).unwrap();
    curate::apply_file(pool, &file).await.unwrap();
}

/// Undoes matching and runs it again, as `mise match-foods --reset` does.
async fn rematch(pool: &PgPool) {
    foods::reset_matching(pool).await.unwrap();
    foods::match_foods(pool).await.unwrap();
}

/// The food the line `raw` names: its kind and what it is a kind of.
async fn matched(pool: &PgPool, raw: &str) -> (String, Option<String>) {
    sqlx::query_as(
        "SELECT f.kind::text, p.name FROM ingredient_lines il JOIN foods f ON f.id = il.food_id
         LEFT JOIN foods p ON p.id = f.parent_id WHERE il.raw_name = $1",
    )
    .bind(raw)
    .fetch_one(pool)
    .await
    .unwrap()
}

/// Whether review asks if the food named `name` is a kind of something.
async fn asked_about(pool: &PgPool, name: &str) -> bool {
    sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM review_items WHERE kind = 'food_parent' AND status = 'open' AND subject->>'name' = $1)",
    )
    .bind(name)
    .fetch_one(pool)
    .await
    .unwrap()
}

#[tokio::test]
async fn a_brand_curation_adds_matches_under_the_unbranded_food_until_it_is_removed() {
    let app = test_app!(TestApp::new());
    let pool = &app.pool;
    import(pool, &["Paprika", "Smoky Hollow Paprika"]).await;
    foods::match_foods(pool).await.unwrap();
    // Unmarked and unknown, "Smoky Hollow" is two words nobody has said anything about: review asks.
    let unbranded = ("food".to_string(), None);
    assert_eq!(matched(pool, "Smoky Hollow Paprika").await, unbranded);
    assert!(asked_about(pool, "Smoky Hollow Paprika").await);

    apply(pool, json!([{"op": "brand", "name": "Smoky Hollow"}])).await;
    rematch(pool).await;
    assert_eq!(
        matched(pool, "Smoky Hollow Paprika").await,
        ("brand".to_string(), Some("Paprika".to_string()))
    );
    assert!(!asked_about(pool, "Smoky Hollow Paprika").await);

    apply(pool, json!([{"op": "unbrand", "name": "smoky hollow"}])).await;
    rematch(pool).await;
    assert_eq!(matched(pool, "Smoky Hollow Paprika").await, unbranded);
    assert!(asked_about(pool, "Smoky Hollow Paprika").await);
}

#[tokio::test]
async fn a_shipped_brand_can_be_removed_and_the_export_brings_the_brands_back() {
    let app = test_app!(TestApp::new());
    let pool = &app.pool;
    import(pool, &["Seasoning", "Old Bay Seasoning"]).await;
    foods::match_foods(pool).await.unwrap();
    let shipped = ("brand".to_string(), Some("Seasoning".to_string()));
    assert_eq!(matched(pool, "Old Bay Seasoning").await, shipped);

    // Marks, case and spacing don't matter, and saying it twice is harmless.
    let edits = json!([
        {"op": "unbrand", "name": "OLD BAY®"},
        {"op": "unbrand", "name": "old bay"},
        {"op": "brand", "name": "Smoky  Hollow"},
        {"op": "brand", "name": "smoky hollow"},
    ]);
    apply(pool, edits).await;
    rematch(pool).await;
    assert_eq!(
        matched(pool, "Old Bay Seasoning").await,
        ("food".to_string(), None)
    );

    let brands: Vec<Value> = curate::export(pool)
        .await
        .unwrap()
        .iter()
        .map(|op| json!(op))
        .filter(|op| op["op"] == "brand" || op["op"] == "unbrand")
        .collect();
    assert_eq!(
        brands,
        [
            json!({"op": "unbrand", "name": "old bay"}),
            json!({"op": "brand", "name": "smoky hollow"}),
        ]
    );

    // Adding it back restores the shipped brand, and the export has nothing left to say about it.
    apply(pool, json!([{"op": "brand", "name": "Old Bay"}])).await;
    rematch(pool).await;
    assert_eq!(matched(pool, "Old Bay Seasoning").await, shipped);
    let ops = curate::export(pool).await.unwrap();
    assert!(
        !ops.iter().any(|op| json!(op)["name"] == "old bay"),
        "{ops:#?}"
    );
}

#[tokio::test]
async fn a_brand_with_no_words_is_refused() {
    let app = test_app!(TestApp::new());
    let op = curate::Op::Brand { name: "®".into() };
    let err = curate::apply(&app.pool, &op).await.unwrap_err();
    assert!(err.downcast_ref::<curate::Refused>().is_some(), "{err:#}");
}
