//! The weekly plan and its shopping list end to end through the router, each test on its own throwaway database (see
//! `common`).

#[macro_use]
mod common;

use std::ops::Deref;

use axum::http::StatusCode;
use common::{As, TestApp};
use serde_json::{Value, json};
use uuid::Uuid;

const WEEK: &str = "2026-11-02";

/// The app with its owner signed in, and the dishes and foods each test makes up.
struct Kitchen {
    app: TestApp,
    cookie: String,
}

impl Deref for Kitchen {
    type Target = TestApp;

    fn deref(&self) -> &TestApp {
        &self.app
    }
}

impl Kitchen {
    /// A fresh app with its owner set up; None when there is no database (outside CI).
    async fn new() -> Option<Kitchen> {
        let app = TestApp::new().await?;
        let cookie = app.setup_owner().await;
        Some(Kitchen { app, cookie })
    }

    /// Send as the owner and expect 200 OK.
    async fn call(&self, method: &str, path: &str, body: Option<Value>) -> Value {
        self.json(method, path, &self.cookie, body).await
    }

    /// The owner's shopping list for `WEEK`.
    async fn list(&self) -> Value {
        self.call("GET", &format!("/api/shopping?week={WEEK}"), None)
            .await
    }

    /// A made-up food with these properties (count unit, store section and so on).
    async fn food(&self, name: &str, props: Value) -> Uuid {
        let id = Uuid::now_v7();
        sqlx::query("INSERT INTO foods (id, name, slug, props) VALUES ($1, $2, $3, $4)")
            .bind(id)
            .bind(name)
            .bind(name.to_lowercase().replace(' ', "-"))
            .bind(props)
            .execute(&self.pool)
            .await
            .unwrap();
        id
    }

    /// A dish with one version published at 2 and 4 servings; lines are (food, 2-serving amount, unit, shipped).
    async fn recipe(&self, title: &str, lines: &[(Uuid, f64, &str, bool)]) -> Uuid {
        let (dish, variant) = (Uuid::now_v7(), Uuid::now_v7());
        sqlx::query("INSERT INTO dishes (id, title, slug, content_type, origin) VALUES ($1, $2, $3, 'recipe', 'grouped')").bind(dish).bind(title).bind(title.to_lowercase().replace(' ', "-")).execute(&self.pool).await.unwrap();
        sqlx::query("INSERT INTO variants (id, dish_id, origin, title, has_steps, content_hash) VALUES ($1, $2, 'user', $3, true, 'x')").bind(variant).bind(dish).bind(title).execute(&self.pool).await.unwrap();
        sqlx::query("UPDATE dishes SET default_variant_id = $2 WHERE id = $1")
            .bind(dish)
            .bind(variant)
            .execute(&self.pool)
            .await
            .unwrap();
        for y in [2.0_f64, 4.0] {
            sqlx::query("INSERT INTO variant_yields (variant_id, servings) VALUES ($1, $2::float8::numeric)").bind(variant).bind(y).execute(&self.pool).await.unwrap();
        }
        for (i, (food, amount, unit, shipped)) in lines.iter().enumerate() {
            let line = Uuid::now_v7();
            sqlx::query("INSERT INTO ingredient_lines (id, variant_id, line_key, position, raw_name, food_id, shipped) VALUES ($1, $2, $3, $4, 'raw', $5, $6)")
                .bind(line).bind(variant).bind(format!("l{i}")).bind(i as i32).bind(food).bind(shipped).execute(&self.pool).await.unwrap();
            for (s, m) in [(2.0_f64, 1.0_f64), (4.0, 2.0)] {
                sqlx::query("INSERT INTO line_amounts (line_id, servings, amount, unit) VALUES ($1, $2::float8::numeric, $3::float8::numeric, $4)")
                    .bind(line).bind(s).bind(amount * m).bind(unit).execute(&self.pool).await.unwrap();
            }
        }
        variant
    }

    /// Someone in a household of their own, next door, signed in. Returns their session cookie.
    async fn neighbour(&self) -> String {
        let household = self
            .call(
                "POST",
                "/api/households",
                Some(json!({"name": "Next door"})),
            )
            .await;
        self.call("POST", "/api/users", Some(json!({"username": "neighbour", "password": "another long password", "household_id": household["id"]}))).await;
        self.sign_in("neighbour", "another long password").await
    }
}

fn item<'a>(list: &'a Value, name: &str) -> Option<&'a Value> {
    list["sections"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|s| s["items"].as_array().unwrap())
        .find(|i| i["name"] == name)
}

fn section_names(list: &Value) -> Vec<String> {
    list["sections"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["name"].as_str().unwrap().to_string())
        .collect()
}

#[tokio::test]
async fn a_week_planned_becomes_a_merged_list_by_section() {
    let app = test_app!(Kitchen::new());
    let paste = app.food("Tomato Paste", json!({"count_unit": "packet", "count_size": "1.5 oz", "store_section": "Canned & Jarred"})).await;
    let onion = app
        .food(
            "Onion",
            json!({"count_unit": "whole", "store_section": "Produce"}),
        )
        .await;
    let salt = app.food("Salt", json!({})).await;
    let chili = app
        .recipe(
            "Chili",
            &[
                (paste, 1.0, "unit", true),
                (onion, 0.5, "unit", true),
                (salt, 0.25, "teaspoon", false),
            ],
        )
        .await;
    let ragu = app
        .recipe(
            "Ragu",
            &[(paste, 1.0, "unit", true), (onion, 1.0, "pick", true)],
        )
        .await;

    let plan = app
        .call(
            "POST",
            "/api/plan/entries",
            Some(json!({"week": WEEK, "variant_id": chili})),
        )
        .await;
    assert_eq!(
        plan["entries"][0]["servings"], 2.0,
        "defaults to 2 where published"
    );
    assert_eq!(plan["week"], WEEK);
    // Any day of the week lands on its Monday.
    app.call(
        "POST",
        "/api/plan/entries",
        Some(json!({"week": "2026-11-05", "variant_id": ragu, "servings": 4})),
    )
    .await;

    let list = app.list().await;
    assert_eq!(list["meals"], 2);
    assert_eq!(
        section_names(&list),
        ["Produce", "Canned & Jarred", "Check you have"]
    );
    assert_eq!(
        item(&list, "Tomato Paste").unwrap()["amount"],
        "3 packets (4½ oz)"
    );
    assert_eq!(
        item(&list, "Tomato Paste").unwrap()["recipes"],
        json!(["Chili", "Ragu"])
    );
    assert_eq!(
        item(&list, "Onion").unwrap()["amount"],
        "2½ whole",
        "half + 2 picks, exactly"
    );
    assert_eq!(item(&list, "Salt").unwrap()["amount"], "¼ tsp");

    // The list follows the plan.
    let entry = plan["entries"][0]["id"].as_str().unwrap();
    app.call(
        "PATCH",
        &format!("/api/plan/entries/{entry}"),
        Some(json!({"servings": 4})),
    )
    .await;
    assert_eq!(
        item(&app.list().await, "Tomato Paste").unwrap()["amount"],
        "4 packets (6 oz)"
    );
    let plan = app
        .call("DELETE", &format!("/api/plan/entries/{entry}"), None)
        .await;
    assert_eq!(plan["entries"].as_array().unwrap().len(), 1);
    assert_eq!(
        item(&app.list().await, "Tomato Paste").unwrap()["amount"],
        "2 packets (3 oz)"
    );
}

#[tokio::test]
async fn ticks_and_changes_survive_the_plan_changing() {
    let app = test_app!(Kitchen::new());
    let onion = app
        .food(
            "Onion",
            json!({"count_unit": "whole", "store_section": "Produce"}),
        )
        .await;
    let salt = app.food("Salt", json!({})).await;
    let butter = app
        .food("Butter", json!({"store_section": "Dairy & Eggs"}))
        .await;
    let chili = app
        .recipe(
            "Chili",
            &[
                (onion, 1.0, "unit", true),
                (salt, 1.0, "teaspoon", false),
                (butter, 1.0, "tablespoon", true),
            ],
        )
        .await;
    app.call(
        "POST",
        "/api/plan/entries",
        Some(json!({"week": WEEK, "variant_id": chili})),
    )
    .await;
    let list = app.list().await;
    let onion_key = item(&list, "Onion").unwrap()["key"]
        .as_str()
        .unwrap()
        .to_string();
    let butter_key = item(&list, "Butter").unwrap()["key"]
        .as_str()
        .unwrap()
        .to_string();

    app.call(
        "PUT",
        "/api/shopping/marks",
        Some(json!({"week": WEEK, "key": onion_key, "checked": true})),
    )
    .await;
    let list = app
        .call(
            "PUT",
            "/api/shopping/marks",
            Some(json!({"week": WEEK, "key": butter_key, "hidden": true})),
        )
        .await;
    assert!(item(&list, "Butter").is_none());
    assert_eq!(list["removed"][0]["name"], "Butter");
    let list = app
        .call(
            "POST",
            "/api/pantry-staples",
            Some(json!({"week": WEEK, "food_id": salt})),
        )
        .await;
    assert!(item(&list, "Salt").is_none());
    assert_eq!(list["staples"][0]["key"], salt.to_string());
    let list = app
        .call(
            "POST",
            "/api/shopping/extras",
            Some(json!({"week": WEEK, "name": "  Paper towels "})),
        )
        .await;
    assert_eq!(list["extras"][0]["name"], "Paper towels");

    // More of the same food keeps the tick, and everything else stands.
    app.call(
        "POST",
        "/api/plan/entries",
        Some(json!({"week": WEEK, "variant_id": chili})),
    )
    .await;
    let list = app.list().await;
    assert_eq!(item(&list, "Onion").unwrap()["checked"], true);
    assert_eq!(item(&list, "Onion").unwrap()["amount"], "2 whole");
    assert!(item(&list, "Butter").is_none());

    // Everything can be put back.
    let list = app
        .call(
            "PUT",
            "/api/shopping/marks",
            Some(json!({"week": WEEK, "key": butter_key, "hidden": false})),
        )
        .await;
    assert!(item(&list, "Butter").is_some());
    assert_eq!(
        item(&list, "Onion").unwrap()["checked"],
        true,
        "putting one back leaves another's tick alone"
    );
    let list = app
        .call(
            "DELETE",
            &format!("/api/pantry-staples/{salt}?week={WEEK}"),
            None,
        )
        .await;
    assert!(item(&list, "Salt").is_some());
    let extra = list["extras"][0]["id"].as_str().unwrap().to_string();
    let list = app
        .call(
            "PATCH",
            &format!("/api/shopping/extras/{extra}"),
            Some(json!({"checked": true})),
        )
        .await;
    assert_eq!(list["extras"][0]["checked"], true);
    let list = app
        .call("DELETE", &format!("/api/shopping/extras/{extra}"), None)
        .await;
    assert_eq!(list["extras"], json!([]));
}

#[tokio::test]
async fn an_addition_sent_twice_with_one_key_is_added_once() {
    let app = test_app!(Kitchen::new());
    let add = |name: &str, key: Option<&str>| {
        let mut body = json!({"week": WEEK, "name": name});
        if let Some(k) = key {
            body["client_key"] = json!(k);
        }
        app.call("POST", "/api/shopping/extras", Some(body))
    };
    let names = |list: &Value| -> Vec<String> {
        list["extras"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x["name"].as_str().unwrap().to_string())
            .collect()
    };

    // The answer to the first was lost, so the device sends it again: one item, the same one.
    let first = add("Foil", Some("local-1")).await;
    let again = add("Foil", Some("local-1")).await;
    assert_eq!(names(&again), ["Foil"]);
    assert_eq!(again["extras"][0]["id"], first["extras"][0]["id"]);
    assert_eq!(again["extras"][0]["client_key"], "local-1");

    // A key is the addition, not the name: another key adds another, and a used key adds nothing.
    let list = add("Foil", Some("local-2")).await;
    assert_eq!(names(&list), ["Foil", "Foil"]);
    let list = add("Tape", Some("local-1")).await;
    assert_eq!(names(&list), ["Foil", "Foil"]);

    // Scripts can leave the key out, and each such call adds.
    add("Tape", None).await;
    let list = add("Tape", None).await;
    assert_eq!(names(&list), ["Foil", "Foil", "Tape", "Tape"]);
    assert_eq!(list["extras"][2]["client_key"], Value::Null);

    // Keys are the household's own: next door's device can use the same one.
    let other = app.neighbour().await;
    let theirs = app
        .json(
            "POST",
            "/api/shopping/extras",
            &other,
            Some(json!({"week": WEEK, "name": "Bin bags", "client_key": "local-1"})),
        )
        .await;
    assert_eq!(names(&theirs), ["Bin bags"]);
    assert_eq!(names(&app.list().await), ["Foil", "Foil", "Tape", "Tape"]);
}

#[tokio::test]
async fn weights_read_as_the_household_chooses() {
    let app = test_app!(Kitchen::new());
    let beef = app
        .food("Ground Beef", json!({"store_section": "Meat & Seafood"}))
        .await;
    let cheese = app.food("Cheddar", json!({})).await;
    let tacos = app
        .recipe(
            "Tacos",
            &[(beef, 10.0, "ounce", true), (cheese, 0.5, "cup", true)],
        )
        .await;
    let burgers = app.recipe("Burgers", &[(beef, 10.0, "ounce", true)]).await;
    app.call(
        "POST",
        "/api/plan/entries",
        Some(json!({"week": WEEK, "variant_id": tacos, "servings": 4})),
    )
    .await;
    app.call(
        "POST",
        "/api/plan/entries",
        Some(json!({"week": WEEK, "variant_id": burgers})),
    )
    .await;

    let list = app.list().await;
    assert_eq!(list["weight_unit"], "lb_oz");
    assert_eq!(item(&list, "Ground Beef").unwrap()["amount"], "1 lb 14 oz");
    assert_eq!(item(&list, "Ground Beef").unwrap()["weighed"], true);
    assert_eq!(item(&list, "Cheddar").unwrap()["weighed"], false);

    let settings = app
        .call(
            "PUT",
            "/api/shopping/settings",
            Some(json!({"weight_unit": "metric"})),
        )
        .await;
    assert_eq!(settings["weight_unit"], "metric");
    assert_eq!(
        app.call("GET", "/api/shopping/settings", None).await["weight_unit"],
        "metric"
    );
    assert_eq!(
        item(&app.list().await, "Ground Beef").unwrap()["amount"],
        "850 g"
    );

    // One food can read its own way, and go back.
    let list = app
        .call(
            "PUT",
            "/api/shopping/food-weights",
            Some(json!({"week": WEEK, "food_id": beef, "weight_unit": "lb"})),
        )
        .await;
    assert_eq!(item(&list, "Ground Beef").unwrap()["amount"], "1⅞ lb");
    assert_eq!(item(&list, "Ground Beef").unwrap()["weight_unit"], "lb");
    let list = app
        .call(
            "PUT",
            "/api/shopping/food-weights",
            Some(json!({"week": WEEK, "food_id": beef, "weight_unit": null})),
        )
        .await;
    assert_eq!(item(&list, "Ground Beef").unwrap()["amount"], "850 g");
    assert_eq!(
        item(&list, "Ground Beef").unwrap()["weight_unit"],
        Value::Null
    );

    assert!(
        app.send(
            "PUT",
            "/api/shopping/settings",
            As::Cookie(&app.cookie),
            Some(json!({"weight_unit": "stone"}))
        )
        .await
        .status
        .is_client_error()
    );
    assert_eq!(
        app.send(
            "PUT",
            "/api/shopping/food-weights",
            As::Cookie(&app.cookie),
            Some(json!({"food_id": Uuid::now_v7(), "weight_unit": "oz"}))
        )
        .await
        .status,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn a_household_sees_only_its_own_plan() {
    let app = test_app!(Kitchen::new());
    let onion = app.food("Onion", json!({})).await;
    let chili = app.recipe("Chili", &[(onion, 1.0, "unit", true)]).await;
    let plan = app
        .call(
            "POST",
            "/api/plan/entries",
            Some(json!({"week": WEEK, "variant_id": chili})),
        )
        .await;
    let entry = plan["entries"][0]["id"].as_str().unwrap().to_string();
    app.call(
        "POST",
        "/api/shopping/extras",
        Some(json!({"week": WEEK, "name": "Mine"})),
    )
    .await;

    let other = app.neighbour().await;

    let plan = app
        .send(
            "GET",
            &format!("/api/plan?week={WEEK}"),
            As::Cookie(&other),
            None,
        )
        .await
        .body;
    assert_eq!(plan["entries"], json!([]));
    let list = app
        .send(
            "GET",
            &format!("/api/shopping?week={WEEK}"),
            As::Cookie(&other),
            None,
        )
        .await
        .body;
    assert_eq!(list["meals"], 0);
    assert_eq!(list["extras"], json!([]));
    assert_eq!(
        app.send(
            "DELETE",
            &format!("/api/plan/entries/{entry}"),
            As::Cookie(&other),
            None
        )
        .await
        .status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.send(
            "PATCH",
            &format!("/api/plan/entries/{entry}"),
            As::Cookie(&other),
            Some(json!({"servings": 4}))
        )
        .await
        .status,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn bad_input_is_refused() {
    let app = test_app!(Kitchen::new());
    let onion = app.food("Onion", json!({})).await;
    let chili = app.recipe("Chili", &[(onion, 1.0, "unit", true)]).await;
    let app = &app;
    let post = move |path: &'static str, body: Value| async move {
        app.send("POST", path, As::Cookie(&app.cookie), Some(body))
            .await
            .status
    };
    assert_eq!(
        post("/api/plan/entries", json!({"variant_id": Uuid::now_v7()})).await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        post(
            "/api/plan/entries",
            json!({"variant_id": chili, "servings": 0})
        )
        .await,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        post(
            "/api/plan/entries",
            json!({"variant_id": chili, "servings": 1000})
        )
        .await,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        post("/api/shopping/extras", json!({"name": "   "})).await,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        post("/api/shopping/extras", json!({"name": "a\u{0}b"})).await,
        StatusCode::BAD_REQUEST
    );
    for key in [String::new(), "k".repeat(101), "a\u{0}b".into()] {
        assert_eq!(
            post(
                "/api/shopping/extras",
                json!({"name": "Foil", "client_key": key})
            )
            .await,
            StatusCode::BAD_REQUEST,
            "client_key {key:?}"
        );
    }
    assert_eq!(
        post("/api/pantry-staples", json!({"food_id": Uuid::now_v7()})).await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.send(
            "PUT",
            "/api/shopping/marks",
            As::Cookie(&app.cookie),
            Some(json!({"key": "", "checked": true}))
        )
        .await
        .status,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        app.send("GET", "/api/plan", As::Nobody, None).await.status,
        StatusCode::UNAUTHORIZED
    );
}
