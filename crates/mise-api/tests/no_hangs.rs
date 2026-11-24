//! Nothing an admin or an import does can stop every later import: a merge can't make a loop in the food tree, a loop
//! put there anyway makes option derivation fail instead of running on, an import cancelled mid-way leaves no lock
//! behind, and a recipe with a very long title can't break grouping.

#[macro_use]
mod common;

use std::time::Duration;

use axum::body::Body;
use axum::http::{StatusCode, header};
use common::{As, TestApp};
use mise_ingest::curate::{self, Op};
use serde_json::json;
use uuid::Uuid;

/// A food's parent, by name.
async fn parent_of(app: &TestApp, name: &str) -> Option<String> {
    sqlx::query_scalar(
        "SELECT p.name FROM foods f LEFT JOIN foods p ON p.id = f.parent_id WHERE f.name = $1",
    )
    .bind(name)
    .fetch_one(&app.pool)
    .await
    .unwrap()
}

/// Whether any food is, through its parents, a kind of itself.
async fn any_loop(app: &TestApp) -> bool {
    sqlx::query_scalar(
        "WITH RECURSIVE up AS (
             SELECT id AS start, parent_id AS at FROM foods WHERE parent_id IS NOT NULL
             UNION SELECT up.start, f.parent_id FROM up JOIN foods f ON f.id = up.at WHERE f.parent_id IS NOT NULL
         )
         SELECT EXISTS (SELECT 1 FROM up WHERE at = start)",
    )
    .fetch_one(&app.pool)
    .await
    .unwrap()
}

fn add(name: &str, parent: Option<&str>) -> Op {
    Op::Add {
        name: name.into(),
        parent: parent.map(str::to_owned),
        kind: "food".into(),
        aliases: vec![],
    }
}

#[tokio::test]
async fn merging_a_food_into_its_grandchild_makes_no_loop() {
    let app = test_app!(TestApp::with_demo());
    for op in [
        add("Test Allium", None),
        add("Test Onion", Some("Test Allium")),
        add("Test Red Onion", Some("Test Onion")),
    ] {
        curate::apply(&app.pool, &op).await.unwrap();
    }
    // from → Test Onion → into: before, the onion became a kind of the red onion, which was a kind of the onion.
    let merge = Op::Merge {
        from: "Test Allium".into(),
        into: "Test Red Onion".into(),
    };
    curate::apply(&app.pool, &merge).await.unwrap();
    assert_eq!(
        parent_of(&app, "Test Red Onion").await,
        None,
        "it takes the merged food's place"
    );
    assert_eq!(
        parent_of(&app, "Test Onion").await.as_deref(),
        Some("Test Red Onion")
    );
    assert!(!any_loop(&app).await);
    mise_ingest::options::derive_options(&app.pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn a_loop_in_the_tree_stops_option_derivation_with_an_error() {
    let app = test_app!(TestApp::with_demo());
    // Two foods the demo recipes use, each made the other's parent by hand.
    let foods: Vec<Uuid> = sqlx::query_scalar(
        "SELECT DISTINCT f.id FROM ingredient_lines il JOIN foods f ON f.id = il.food_id
         WHERE f.kind = 'food' AND coalesce(f.props->>'default_flexibility', '') <> 'strict' LIMIT 2",
    )
    .fetch_all(&app.pool)
    .await
    .unwrap();
    sqlx::query(
        "UPDATE foods SET parent_id = CASE id WHEN $1 THEN $2 ELSE $1 END WHERE id IN ($1, $2)",
    )
    .bind(foods[0])
    .bind(foods[1])
    .execute(&app.pool)
    .await
    .unwrap();
    let derived = tokio::time::timeout(
        Duration::from_secs(30),
        mise_ingest::options::derive_options(&app.pool),
    )
    .await
    .expect("it ends");
    let error = derived.expect_err("a loop is an error").to_string();
    assert!(error.contains("loop"), "{error}");
}

/// Whether this database holds the import stages' lock ("mise imp" as a bigint, which `pg_locks` splits in two).
async fn stages_locked(app: &TestApp) -> bool {
    sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM pg_locks WHERE locktype = 'advisory' AND objsubid = 1
                        AND classid::bigint = $1 AND objid::bigint = $2
                        AND database = (SELECT oid FROM pg_database WHERE datname = current_database()))",
    )
    .bind(0x6d69_7365_i64)
    .bind(0x2069_6d70_i64)
    .fetch_one(&app.pool)
    .await
    .unwrap()
}

#[tokio::test]
async fn an_import_cancelled_mid_stages_leaves_no_lock_behind() {
    let app = test_app!(TestApp::with_demo());
    // Cancel the stages at a spread of moments, some of them with the lock held; after each, the lock goes.
    for ms in [0, 1, 2, 5, 10, 20, 50, 100, 200] {
        let _ = tokio::time::timeout(
            Duration::from_millis(ms),
            mise_ingest::import::shared_stages(&app.pool),
        )
        .await;
        let mut freed = false;
        for _ in 0..50 {
            if !stages_locked(&app).await {
                freed = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert!(freed, "cancelled after {ms} ms, the lock stayed held");
    }
    // And the next import's stages run to the end.
    tokio::time::timeout(
        Duration::from_secs(60),
        mise_ingest::import::shared_stages(&app.pool),
    )
    .await
    .expect("the stages run")
    .unwrap();
}

#[tokio::test]
async fn a_very_long_title_is_cut_and_grouping_still_runs() {
    let app = test_app!(TestApp::with_demo());
    let cookie = app.setup_owner().await;
    let import = |name: String| {
        let recipe = json!({
            "@context": "https://schema.org", "@type": "Recipe", "name": name,
            "recipeIngredient": ["2 cups rice"], "recipeInstructions": ["Cook the rice."],
        });
        TestApp::request(
            "POST",
            "/api/v1/imports/file?name=recipe.json",
            As::Cookie(&cookie),
        )
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .body(Body::from(recipe.to_string()))
        .unwrap()
    };
    let long = "Rice ".repeat(2_000);
    let reply = app.read(import(long)).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    let title = reply.body["recipes"][0]["title"]
        .as_str()
        .unwrap()
        .to_owned();
    let version = app
        .json(
            "GET",
            &format!(
                "/api/v1/variants/{}",
                reply.body["recipes"][0]["variant_id"].as_str().unwrap()
            ),
            &cookie,
            None,
        )
        .await;
    let stored = version["title"].as_str().unwrap();
    assert!(stored.chars().count() <= 200, "{} characters", stored.len());
    let slug = reply.body["recipes"][0]["dish_slug"].as_str().unwrap();
    assert!(slug.len() <= 80, "{slug}");
    assert!(!title.is_empty());

    // Grouping still runs for the next import.
    let reply = app.read(import("Plain Rice".into())).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert_eq!(reply.body["recipes"][0]["dish_slug"], "plain-rice");
}
