//! Review queues: an admin answers what the import pipeline left to decide, and the answer is stored as the `mise`
//! commands store it, so re-running the stage keeps it and the exports bring it back on a rebuilt database.
//!
//! The demo recipes leave nothing to review, so each test adds two recipes written here (a bundle of its own) that
//! the stages ask about: "Pepper" (black or bell?), "Garden Tomato" (a kind of tomato?), and a skillet copied from
//! a salad that shares nothing with it (the same dish?).

#[macro_use]
mod common;

use axum::http::StatusCode;
use common::{As, TestApp};
use mise_ingest::{curate, dish_decisions};
use serde_json::{Value, json};
use sqlx::PgPool;

const QUEUE: &str = "/api/v1/review-items";

/// One recipe line of the bundle: its name, shipped, at 2 servings.
fn recipe(id: &str, title: &str, parent: Option<&str>, lines: &[&str]) -> Value {
    json!({
        "external_id": id,
        "parent_external_id": parent,
        "title": title,
        "content_type": "recipe",
        "published_at": "2026-11-01T12:00:00Z",
        "yields": [2],
        "ingredients": lines.iter().map(|n| json!({"key": n, "name": n, "shipped": true, "allergens": []})).collect::<Vec<_>>(),
        "amounts": lines.iter().map(|n| json!({"key": n, "servings": 2, "amount": 1, "unit": "unit"})).collect::<Vec<_>>(),
        "steps": [{"index": 1, "text": "Cook everything."}],
    })
}

/// Imports the two recipes and runs matching and grouping over them, as `mise` would after `import-bundle`.
async fn add_doubtful_recipes(pool: &PgPool) {
    let dir = tempfile::tempdir().unwrap();
    let manifest = json!({
        "format": "mise-seed/1",
        "source": {"key": "seed:review-test", "name": "Recipes the review tests ask about"},
        "created": "2026-11-17T00:00:00Z",
        "media_root": ".",
    });
    std::fs::write(dir.path().join("manifest.json"), manifest.to_string()).unwrap();
    let salad = recipe(
        "salad",
        "Summer Garden Salad",
        None,
        &["Tomato", "Garden Tomato", "Cucumber"],
    );
    let skillet = recipe(
        "skillet",
        "Pepper Steak Skillet",
        Some("salad"),
        &["Pepper", "Flank Steak", "Shallot"],
    );
    std::fs::write(
        dir.path().join("variants.ndjson"),
        format!("{salad}\n{skillet}\n"),
    )
    .unwrap();
    mise_ingest::bundle::import_bundle(pool, dir.path())
        .await
        .unwrap();
    mise_ingest::foods::match_foods(pool).await.unwrap();
    mise_ingest::dishes::group_dishes(pool).await.unwrap();
}

/// The demo recipes plus the two doubtful ones, with the owner signed in.
async fn with_queue() -> Option<(TestApp, String)> {
    let app = TestApp::with_demo().await?;
    add_doubtful_recipes(&app.pool).await;
    let owner = app.setup_owner().await;
    Some((app, owner))
}

/// The open item of `kind`, from the queue.
async fn item(app: &TestApp, owner: &str, kind: &str) -> Value {
    let queue = app.json("GET", QUEUE, owner, None).await;
    queue["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["kind"] == kind)
        .unwrap_or_else(|| panic!("no open {kind} item in {queue}"))
        .clone()
}

/// Answers `item` and expects it to be taken.
async fn decide(app: &TestApp, owner: &str, item: &Value, answer: Value) -> Value {
    let path = format!("{QUEUE}/{}/decision", item["id"].as_str().unwrap());
    app.json("POST", &path, owner, Some(answer)).await
}

/// How many items of `kind` are open.
async fn open(pool: &PgPool, kind: &str) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM review_items WHERE kind = $1 AND status = 'open'")
        .bind(kind)
        .fetch_one(pool)
        .await
        .unwrap()
}

/// The three questions the test recipes raise, as (kind, what it asks about).
const ASKED: [(&str, &str); 3] = [
    ("food_match", "Pepper"),
    ("food_parent", "Garden Tomato"),
    ("dish_merge", "Pepper Steak Skillet"),
];

/// How many open items ask `kind` about `subject` (a food's name or a version's title). Matching from scratch may
/// ask new questions about other foods (is Black Pepper a kind of Pepper?); the decided ones must not come back.
async fn asked(pool: &PgPool, kind: &str, subject: &str) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM review_items WHERE kind = $1 AND status = 'open'
           AND coalesce(subject->>'name', subject->>'title') = $2",
    )
    .bind(kind)
    .bind(subject)
    .fetch_one(pool)
    .await
    .unwrap()
}

/// A food's parent's name and its origin, by the food's name.
async fn food(pool: &PgPool, name: &str) -> (Option<String>, String) {
    sqlx::query_as(
        "SELECT p.name, f.origin FROM foods f LEFT JOIN foods p ON p.id = f.parent_id WHERE f.name = $1",
    )
    .bind(name)
    .fetch_one(pool)
    .await
    .unwrap()
}

/// Whether the two test recipes are versions of one dish.
async fn one_dish(pool: &PgPool) -> bool {
    sqlx::query_scalar(
        "SELECT count(DISTINCT v.dish_id) = 1 FROM variants v JOIN source_records sr ON sr.id = v.source_record_id
         JOIN sources s ON s.id = sr.source_id WHERE s.key = 'seed:review-test'",
    )
    .fetch_one(pool)
    .await
    .unwrap()
}

#[tokio::test]
async fn the_queue_shows_what_each_stage_asks() {
    let (app, owner) = test_app!(with_queue());
    let queue = app.json("GET", QUEUE, &owner, None).await;
    let items = queue["items"].as_array().unwrap();
    let kinds: Vec<&str> = items.iter().map(|i| i["kind"].as_str().unwrap()).collect();
    assert_eq!(
        kinds,
        ["food_match", "food_parent", "dish_merge"],
        "{queue}"
    );

    assert_eq!(items[0]["subject"], "Pepper");
    assert_eq!(items[0]["proposal"], "Pepper is a food of its own");
    assert!(
        items[0]["note"]
            .as_str()
            .unwrap()
            .contains("black pepper or bell pepper")
    );
    assert_eq!(items[0]["verdicts"], json!(["accept", "edit"]));
    assert_eq!(items[1]["proposal"], "Garden Tomato is a kind of Tomato");
    assert_eq!(items[1]["recipe_uses"], 1);
    assert_eq!(
        items[2]["proposal"],
        "Pepper Steak Skillet is a version of Summer Garden Salad"
    );
    assert_eq!(
        items[2]["dishes"]["version"]["dish_slug"],
        "pepper-steak-skillet"
    );
    assert_eq!(
        items[2]["dishes"]["copied_from"]["dish_slug"],
        "summer-garden-salad"
    );
    assert_eq!(items[2]["verdicts"], json!(["accept", "reject"]));

    let only = app
        .json("GET", &format!("{QUEUE}?kind=dish_merge"), &owner, None)
        .await;
    assert_eq!(only["items"].as_array().unwrap().len(), 1, "{only}");
}

#[tokio::test]
async fn accepted_items_stay_decided_when_their_stage_runs_again() {
    let (app, owner) = test_app!(with_queue());
    let pool = &app.pool;

    let pepper = item(&app, &owner, "food_match").await;
    let left = decide(&app, &owner, &pepper, json!({"decision": "accept"})).await;
    assert_eq!(left["items"].as_array().unwrap().len(), 2, "{left}");
    let tomato = item(&app, &owner, "food_parent").await;
    decide(&app, &owner, &tomato, json!({"decision": "accept"})).await;
    let skillet = item(&app, &owner, "dish_merge").await;
    let left = decide(
        &app,
        &owner,
        &skillet,
        json!({"decision": "accept", "note": "same pan"}),
    )
    .await;
    assert_eq!(left["items"], json!([]));

    // Stored as the commands store them: the foods are the household's own, the pair is a dish decision.
    assert_eq!(food(pool, "Pepper").await, (None, "user".into()));
    assert_eq!(
        food(pool, "Garden Tomato").await,
        (Some("Tomato".into()), "user".into())
    );
    let decisions = dish_decisions::export(pool).await.unwrap();
    assert_eq!(decisions.len(), 1);
    assert_eq!(
        (decisions[0].decision.as_str(), decisions[0].note.as_deref()),
        ("merge", Some("same pan"))
    );
    let (status, by): (String, String) =
        sqlx::query_as("SELECT status, decision->>'by' FROM review_items WHERE id = $1::uuid")
            .bind(skillet["id"].as_str())
            .fetch_one(pool)
            .await
            .unwrap();
    assert_eq!((status.as_str(), by.as_str()), ("accepted", common::OWNER));

    // Matching from scratch and grouping again don't ask again, and keep each answer.
    mise_ingest::foods::reset_matching(pool).await.unwrap();
    mise_ingest::foods::match_foods(pool).await.unwrap();
    mise_ingest::dishes::group_dishes(pool).await.unwrap();
    for (kind, subject) in ASKED {
        assert_eq!(asked(pool, kind, subject).await, 0, "{kind} {subject}");
    }
    assert_eq!(food(pool, "Pepper").await, (None, "user".into()));
    assert_eq!(
        food(pool, "Garden Tomato").await,
        (Some("Tomato".into()), "user".into())
    );
    assert!(one_dish(pool).await, "the skillet joined the salad's dish");

    // An item already answered can't be answered again.
    let path = format!("{QUEUE}/{}/decision", pepper["id"].as_str().unwrap());
    let again = app
        .send(
            "POST",
            &path,
            As::Cookie(&owner),
            Some(json!({"decision": "accept"})),
        )
        .await;
    assert_eq!(again.status, StatusCode::CONFLICT, "{}", again.body);
}

#[tokio::test]
async fn rejected_and_edited_items_stay_decided_when_their_stage_runs_again() {
    let (app, owner) = test_app!(with_queue());
    let pool = &app.pool;

    // Pepper is the black pepper the demo recipes already use: the two become one food.
    let pepper = item(&app, &owner, "food_match").await;
    decide(
        &app,
        &owner,
        &pepper,
        json!({"decision": "edit", "same_as": "Black Pepper"}),
    )
    .await;
    let tomato = item(&app, &owner, "food_parent").await;
    decide(&app, &owner, &tomato, json!({"decision": "reject"})).await;
    let skillet = item(&app, &owner, "dish_merge").await;
    decide(&app, &owner, &skillet, json!({"decision": "reject"})).await;

    let pepper_line: (String,) = sqlx::query_as(
        "SELECT f.name FROM ingredient_lines il JOIN foods f ON f.id = il.food_id WHERE il.raw_name = 'Pepper'",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(pepper_line.0, "Black Pepper");
    assert_eq!(food(pool, "Garden Tomato").await, (None, "user".into()));

    mise_ingest::foods::reset_matching(pool).await.unwrap();
    mise_ingest::foods::match_foods(pool).await.unwrap();
    mise_ingest::dishes::group_dishes(pool).await.unwrap();
    for (kind, subject) in ASKED {
        assert_eq!(asked(pool, kind, subject).await, 0, "{kind} {subject}");
    }
    assert_eq!(food(pool, "Garden Tomato").await, (None, "user".into()));
    assert!(!one_dish(pool).await, "the skillet keeps a dish of its own");
}

#[tokio::test]
async fn the_exports_bring_the_decisions_back_on_a_rebuilt_database() {
    let (app, owner) = test_app!(with_queue());
    let pepper = item(&app, &owner, "food_match").await;
    let edit = json!({"decision": "edit", "same_as": "Black Pepper"});
    decide(&app, &owner, &pepper, edit).await;
    let tomato = item(&app, &owner, "food_parent").await;
    decide(&app, &owner, &tomato, json!({"decision": "accept"})).await;
    let skillet = item(&app, &owner, "dish_merge").await;
    decide(&app, &owner, &skillet, json!({"decision": "accept"})).await;

    let foods = curate::export(&app.pool).await.unwrap();
    let dishes = dish_decisions::export(&app.pool).await.unwrap();
    let ops: Vec<Value> = foods.iter().map(|o| json!(o)).collect();
    for op in [
        json!({"op": "merge", "from": "Pepper", "into": "Black Pepper"}),
        json!({"op": "set_parent", "food": "Garden Tomato", "parent": "Tomato"}),
        json!({"op": "item_size", "food": "Garlic", "item": "clove", "size": "1 tsp"}),
        json!({"op": "density", "food": "Parmesan", "volume": "1 cup", "weight": "3 oz"}),
    ] {
        assert!(ops.contains(&op), "{op} isn't in the export: {ops:#?}");
    }

    // Written out as the commands write them, and applied to a database rebuilt from the same recipes, without the
    // demo's own curation file: the export holds it.
    let dir = tempfile::tempdir().unwrap();
    let (food_file, dish_file) = (
        dir.path().join("foods.json"),
        dir.path().join("dishes.json"),
    );
    std::fs::write(&food_file, serde_json::to_string(&foods).unwrap()).unwrap();
    std::fs::write(&dish_file, serde_json::to_string(&dishes).unwrap()).unwrap();
    let rebuilt = test_app!(TestApp::new());
    let pool = &rebuilt.pool;
    let bundle = common::demo::bundle_dir();
    mise_ingest::bundle::import_bundle(pool, &bundle)
        .await
        .unwrap();
    mise_ingest::foods::match_foods(pool).await.unwrap();
    add_doubtful_recipes(pool).await;
    curate::apply_file(pool, &food_file).await.unwrap();
    dish_decisions::apply_file(pool, &dish_file).await.unwrap();
    mise_ingest::dishes::group_dishes(pool).await.unwrap();

    assert_eq!(curate::export(pool).await.unwrap(), foods);
    assert_eq!(dish_decisions::export(pool).await.unwrap(), dishes);
    assert_eq!(
        food(pool, "Garden Tomato").await,
        (Some("Tomato".into()), "user".into())
    );
    assert!(one_dish(pool).await);
    // The stages raised the questions again on the rebuilt database, and the decisions answered them.
    for (kind, subject) in ASKED {
        assert_eq!(asked(pool, kind, subject).await, 0, "{kind} {subject}");
    }

    // Applying the export to the database it came from changes nothing.
    curate::apply_file(&app.pool, &food_file).await.unwrap();
    assert_eq!(curate::export(&app.pool).await.unwrap(), foods);
}

#[tokio::test]
async fn answers_an_item_doesnt_take_are_refused() {
    let (app, owner) = test_app!(with_queue());
    let me = As::Cookie(&owner);
    let path = |i: &Value| format!("{QUEUE}/{}/decision", i["id"].as_str().unwrap());
    let refused = |answer: Value, item: Value| {
        let app = &app;
        async move {
            let reply = app.send("POST", &path(&item), me, Some(answer)).await;
            assert_eq!(reply.status, StatusCode::BAD_REQUEST, "{}", reply.body);
            reply.body["error"].as_str().unwrap().to_string()
        }
    };
    let pepper = item(&app, &owner, "food_match").await;
    let tomato = item(&app, &owner, "food_parent").await;
    let skillet = item(&app, &owner, "dish_merge").await;

    let why = refused(json!({"decision": "reject"}), pepper.clone()).await;
    assert!(why.contains("name the food it's the same as"), "{why}");
    let why = refused(json!({"decision": "edit"}), tomato.clone()).await;
    assert!(why.contains("name either"), "{why}");
    let unknown = json!({"decision": "edit", "same_as": "Dragon Fruit Jam"});
    let why = refused(unknown, pepper.clone()).await;
    assert!(why.starts_with("no food named 'Dragon Fruit Jam'"), "{why}");
    let loop_ = json!({"decision": "edit", "parent": "Garden Tomato"});
    let tomato_itself = refused(loop_, tomato.clone()).await;
    assert!(tomato_itself.contains("loop"), "{tomato_itself}");
    let why = refused(json!({"decision": "edit"}), skillet.clone()).await;
    assert!(why.contains("accepted or rejected"), "{why}");

    // Nothing was decided, and nothing changed.
    assert_eq!(open(&app.pool, "food_match").await, 1);
    assert_eq!(open(&app.pool, "food_parent").await, 1);
    assert_eq!(food(&app.pool, "Garden Tomato").await.1, "matched");
}

#[tokio::test]
async fn only_admins_see_or_answer_the_queue() {
    let (app, owner) = test_app!(with_queue());
    app.json(
        "POST",
        "/api/v1/users",
        &owner,
        Some(json!({"username": "cook", "password": "a cook's password"})),
    )
    .await;
    let cook = app.sign_in("cook", "a cook's password").await;
    let pepper = item(&app, &owner, "food_match").await;
    let path = format!("{QUEUE}/{}/decision", pepper["id"].as_str().unwrap());
    let accept = json!({"decision": "accept"});

    let made = app
        .json(
            "POST",
            "/api/v1/kitchen-tokens",
            &owner,
            Some(json!({"name": "Kitchen"})),
        )
        .await;
    let link = made["link"].as_str().unwrap();
    let device = app.send("GET", link, As::Nobody, None).await;
    let device = device.cookie("mise_kitchen").unwrap();
    let device = device.split(';').next().unwrap();

    for from in [As::Cookie(&cook), As::Kitchen(device)] {
        let listed = app.send("GET", QUEUE, from, None).await;
        assert_eq!(listed.status, StatusCode::FORBIDDEN, "{}", listed.body);
        let decided = app.send("POST", &path, from, Some(accept.clone())).await;
        assert_eq!(decided.status, StatusCode::FORBIDDEN, "{}", decided.body);
    }
    assert_eq!(open(&app.pool, "food_match").await, 1);
}
