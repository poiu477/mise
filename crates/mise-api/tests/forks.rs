//! A household's own version of a dish (a fork): made from a published version or one of its own, it is a recipe
//! the household can change, on the same dish, naming the version it came from, which stays as it was. Each test
//! runs on the demo recipes with two households, the owner's ("home") and next door's.

#[macro_use]
mod common;

use std::ops::Deref;

use axum::http::StatusCode;
use common::{As, TestApp};
use serde_json::{Value, json};
use uuid::Uuid;

/// The demo install with two households, each with someone signed in.
struct Street {
    app: TestApp,
    home: String,
    next_door: String,
}

impl Deref for Street {
    type Target = TestApp;

    fn deref(&self) -> &TestApp {
        &self.app
    }
}

impl Street {
    /// None when there is no database (outside CI).
    async fn new() -> Option<Street> {
        let app = TestApp::with_demo().await?;
        let home = app.setup_owner().await;
        let household = app
            .json(
                "POST",
                "/api/v1/households",
                &home,
                Some(json!({"name": "Next door"})),
            )
            .await;
        app.json(
            "POST",
            "/api/v1/users",
            &home,
            Some(json!({"username": "neighbour", "password": "another long password", "household_id": household["id"]})),
        )
        .await;
        let next_door = app.sign_in("neighbour", "another long password").await;
        Some(Street {
            app,
            home,
            next_door,
        })
    }

    /// The dish page as this browser sees it.
    async fn dish(&self, cookie: &str, slug: &str) -> Value {
        self.json("GET", &format!("/api/v1/dishes/{slug}"), cookie, None)
            .await
    }

    /// Fork version `id` as this browser and expect the copy stored.
    async fn fork(&self, cookie: &str, id: &Value) -> Value {
        self.json(
            "POST",
            &format!("/api/v1/variants/{}/fork", id.as_str().unwrap()),
            cookie,
            None,
        )
        .await
    }

    /// Version `id` at `servings`, as this browser sees it.
    async fn version(&self, cookie: &str, id: &Value, servings: u32) -> Value {
        self.json(
            "GET",
            &format!(
                "/api/v1/variants/{}?servings={servings}",
                id.as_str().unwrap()
            ),
            cookie,
            None,
        )
        .await
    }

    /// A text column of the version `id`.
    async fn column(&self, sql: &str, id: &Value) -> Option<String> {
        sqlx::query_scalar(sqlx::AssertSqlSafe(sql.to_string()))
            .bind(id.as_str().unwrap().parse::<Uuid>().unwrap())
            .fetch_one(&self.pool)
            .await
            .unwrap()
    }
}

/// The entry for version `id` among a dish page's versions.
fn entry<'a>(dish: &'a Value, id: &Value) -> &'a Value {
    dish["variants"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["id"] == *id)
        .unwrap_or_else(|| panic!("version {id} isn't on the dish page: {dish}"))
}

/// The ingredient named `name` on a rendered version.
fn ingredient<'a>(version: &'a Value, name: &str) -> &'a Value {
    version["ingredients"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["name"] == name)
        .unwrap_or_else(|| panic!("no {name} in {version}"))
}

/// A lentil soup, written for 4.
fn dal() -> Value {
    json!({
        "title": "Red Lentil Dal",
        "servings": 4,
        "ingredients": [
            {"text": "1 cup red lentils, rinsed"},
            {"text": "2 cloves garlic, minced"},
            {"text": "1 (14 oz) can coconut milk"},
            {"text": "Salt, to taste"},
        ],
        "steps": ["Simmer the lentils and garlic in the coconut milk with 3 cups water until soft."],
    })
}

#[tokio::test]
async fn a_published_version_is_copied_into_the_household_and_left_as_it_was() {
    let street = test_app!(Street::new());
    let dish = street.dish(&street.home, "weeknight-tomato-pasta").await;
    let original = dish["default_variant"].clone();
    let before = (
        street.version(&street.home, &original, 2).await,
        street.version(&street.home, &original, 4).await,
    );

    let copy = street.fork(&street.home, &original).await;

    // The copy is written for the smallest published yield, its lines written out as a cook would type them, each
    // still matched to the original's food.
    assert_eq!(copy["title"], before.0["title"]);
    assert_eq!(copy["dish_slug"], "weeknight-tomato-pasta");
    assert_eq!(copy["servings"], 2.0);
    let texts: Vec<&str> = copy["ingredients"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l["text"].as_str().unwrap())
        .collect();
    assert_eq!(
        texts[..4],
        [
            "6 oz Spaghetti",
            "1 can Crushed Tomatoes",
            "2 cloves Garlic",
            "1 Yellow Onion"
        ]
    );
    for line in copy["ingredients"].as_array().unwrap() {
        let name = line["name"].as_str().unwrap();
        assert_eq!(
            line["food"]["id"],
            ingredient(&before.0, name)["food_id"],
            "{name} keeps its food"
        );
    }
    let steps: Vec<String> =
        sqlx::query_scalar("SELECT text FROM steps WHERE variant_id = $1 ORDER BY position")
            .bind(original.as_str().unwrap().parse::<Uuid>().unwrap())
            .fetch_all(&street.pool)
            .await
            .unwrap();
    assert_eq!(copy["steps"], json!(steps), "the steps as published");
    assert_eq!(copy["image"], before.0["image"], "the original's photo");

    // At 2 the copy reads as the original; at 4 it is worked out from 2.
    let at_2 = street.version(&street.home, &copy["id"], 2).await;
    for (mine, theirs) in at_2["ingredients"]
        .as_array()
        .unwrap()
        .iter()
        .zip(before.0["ingredients"].as_array().unwrap())
    {
        assert_eq!(
            (&mine["name"], &mine["amount"]),
            (&theirs["name"], &theirs["amount"])
        );
    }
    assert_eq!(
        ingredient(&at_2, "Olive Oil")["shipped"],
        false,
        "pantry marks come along"
    );

    // The dish now has both, the copy the household's to change, naming the original and where it was published.
    let source: Option<String> = street
        .column(
            "SELECT s.name FROM variants v JOIN source_records r ON r.id = v.source_record_id
             JOIN sources s ON s.id = r.source_id WHERE v.id = $1",
            &original,
        )
        .await;
    let dish = street.dish(&street.home, "weeknight-tomato-pasta").await;
    assert_eq!(dish["variants"].as_array().unwrap().len(), 2);
    let mine = entry(&dish, &copy["id"]);
    assert_eq!(mine["editable"], true);
    assert_eq!(mine["original"]["id"], original);
    assert_eq!(mine["original"]["title"], before.0["title"]);
    assert_eq!(mine["original"]["by"], json!(source));
    assert_eq!(
        mine["attributes"],
        entry(&dish, &original)["attributes"],
        "the same options"
    );
    assert_eq!(entry(&dish, &original)["editable"], false);
    assert_eq!(entry(&dish, &original)["original"], Value::Null);

    // Changing the copy leaves the original as it was.
    let mut changed = dal();
    changed["title"] = before.0["title"].clone();
    street
        .json(
            "PUT",
            &format!("/api/v1/recipes/{}", copy["id"].as_str().unwrap()),
            &street.home,
            Some(changed),
        )
        .await;
    assert_eq!(street.version(&street.home, &original, 2).await, before.0);
    assert_eq!(street.version(&street.home, &original, 4).await, before.1);
    assert_eq!(
        street.dish(&street.home, "weeknight-tomato-pasta").await["default_variant"],
        original,
        "the dish still opens on the original"
    );
}

#[tokio::test]
async fn the_copy_is_the_forking_households_alone() {
    let street = test_app!(Street::new());
    let original =
        street.dish(&street.home, "weeknight-tomato-pasta").await["default_variant"].clone();
    let copy = street.fork(&street.home, &original).await;
    let id = copy["id"].as_str().unwrap();

    let owner: Option<String> = street
        .column(
            "SELECT h.name FROM variants v JOIN households h ON h.id = v.household_id WHERE v.id = $1",
            &copy["id"],
        )
        .await;
    assert_ne!(owner.as_deref(), Some("Next door"));
    assert!(owner.is_some(), "the copy belongs to a household");

    // Next door sees the dish as before and can't reach the copy by any address.
    let theirs = street
        .dish(&street.next_door, "weeknight-tomato-pasta")
        .await;
    assert_eq!(theirs["variants"].as_array().unwrap().len(), 1);
    for (method, path) in [
        ("GET", format!("/api/v1/variants/{id}")),
        ("GET", format!("/api/v1/recipes/{id}")),
        ("POST", format!("/api/v1/variants/{id}/fork")),
        ("DELETE", format!("/api/v1/recipes/{id}")),
    ] {
        let reply = street
            .send(method, &path, As::Cookie(&street.next_door), None)
            .await;
        assert_eq!(
            reply.status,
            StatusCode::NOT_FOUND,
            "{method} {path}: {}",
            reply.body
        );
    }

    // They can make their own from the same original; neither household sees the other's.
    let their_copy = street.fork(&street.next_door, &original).await;
    let home = street.dish(&street.home, "weeknight-tomato-pasta").await;
    let next_door = street
        .dish(&street.next_door, "weeknight-tomato-pasta")
        .await;
    assert!(
        home["variants"]
            .as_array()
            .unwrap()
            .iter()
            .all(|v| v["id"] != their_copy["id"])
    );
    assert!(
        next_door["variants"]
            .as_array()
            .unwrap()
            .iter()
            .all(|v| v["id"] != copy["id"])
    );

    // A made-up id, or one that isn't a UUID, is refused as such.
    let missing = street
        .send(
            "POST",
            "/api/v1/variants/0190f4a2-0000-7000-8000-000000000000/fork",
            As::Cookie(&street.home),
            None,
        )
        .await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);
    let garbled = street
        .send(
            "POST",
            "/api/v1/variants/pasta/fork",
            As::Cookie(&street.home),
            None,
        )
        .await;
    assert_eq!(garbled.status, StatusCode::BAD_REQUEST);

    // A read-only token can't make one.
    let token = street
        .json(
            "POST",
            "/api/v1/me/tokens",
            &street.home,
            Some(json!({"name": "reader", "scope": "read"})),
        )
        .await["token"]
        .as_str()
        .unwrap()
        .to_string();
    let path = format!("/api/v1/variants/{}/fork", original.as_str().unwrap());
    let reply = street.send("POST", &path, As::Bearer(&token), None).await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    assert_eq!(
        street.dish(&street.home, "weeknight-tomato-pasta").await["variants"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
}

#[tokio::test]
async fn a_households_own_recipe_forks_as_written_and_names_the_household() {
    let street = test_app!(Street::new());
    let written = street
        .json("POST", "/api/v1/recipes", &street.home, Some(dal()))
        .await;

    let copy = street.fork(&street.home, &written["id"]).await;
    assert_eq!(copy["dish_slug"], written["dish_slug"], "on the same dish");
    assert_eq!(copy["servings"], 4.0);
    // Lines typed into the app keep their words.
    for (mine, theirs) in copy["ingredients"]
        .as_array()
        .unwrap()
        .iter()
        .zip(written["ingredients"].as_array().unwrap())
    {
        assert_eq!(mine["text"], theirs["text"]);
        assert_eq!(mine["food"], theirs["food"]);
    }

    let household: Option<String> = street
        .column(
            "SELECT h.name FROM variants v JOIN households h ON h.id = v.household_id WHERE v.id = $1",
            &written["id"],
        )
        .await;
    let dish = street
        .dish(&street.home, written["dish_slug"].as_str().unwrap())
        .await;
    let mine = entry(&dish, &copy["id"]);
    assert_eq!(mine["original"]["id"], written["id"]);
    assert_eq!(mine["original"]["by"], json!(household));

    // A fork of the fork names the fork; deleting the original leaves the copy, no longer naming it.
    let again = street.fork(&street.home, &copy["id"]).await;
    let dish = street
        .dish(&street.home, written["dish_slug"].as_str().unwrap())
        .await;
    assert_eq!(entry(&dish, &again["id"])["original"]["id"], copy["id"]);
    let reply = street
        .send(
            "DELETE",
            &format!("/api/v1/recipes/{}", written["id"].as_str().unwrap()),
            As::Cookie(&street.home),
            None,
        )
        .await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT);
    let dish = street
        .dish(&street.home, copy["dish_slug"].as_str().unwrap())
        .await;
    assert_eq!(entry(&dish, &copy["id"])["original"], Value::Null);
    assert_eq!(dish["variants"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn allergens_and_pantry_marks_stay_with_their_ingredients_through_a_change() {
    let street = test_app!(Street::new());
    let original =
        street.dish(&street.home, "weeknight-tomato-pasta").await["default_variant"].clone();
    sqlx::query("UPDATE ingredient_lines SET allergens = '{milk}' WHERE variant_id = $1 AND raw_name = 'Parmesan'")
        .bind(original.as_str().unwrap().parse::<Uuid>().unwrap())
        .execute(&street.pool)
        .await
        .unwrap();

    let copy = street.fork(&street.home, &original).await;
    let at_2 = street.version(&street.home, &copy["id"], 2).await;
    assert_eq!(ingredient(&at_2, "Parmesan")["allergens"], json!(["milk"]));
    assert_eq!(ingredient(&at_2, "Salt")["shipped"], false);
    assert_eq!(ingredient(&at_2, "Spaghetti")["shipped"], true);

    // Double the cheese, drop the pepper flakes, add basil: the cheese is still milk, the basil knows nothing.
    let mut lines: Vec<Value> = copy["ingredients"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|l| l["name"] != "Red Pepper Flakes")
        .map(|l| match l["name"].as_str() {
            Some("Parmesan") => json!({"text": "1 cup Parmesan, grated"}),
            _ => json!({"text": l["text"]}),
        })
        .collect();
    lines.push(json!({"text": "fresh basil, to taste"}));
    let changed = street
        .json(
            "PUT",
            &format!("/api/v1/recipes/{}", copy["id"].as_str().unwrap()),
            &street.home,
            Some(json!({"title": copy["title"], "servings": 2, "ingredients": lines, "steps": copy["steps"]})),
        )
        .await;
    assert!(
        changed["ingredients"]
            .as_array()
            .unwrap()
            .iter()
            .all(|l| l["name"] != "Red Pepper Flakes")
    );
    let at_2 = street.version(&street.home, &copy["id"], 2).await;
    assert_eq!(ingredient(&at_2, "Parmesan")["allergens"], json!(["milk"]));
    assert_eq!(ingredient(&at_2, "Olive Oil")["shipped"], false);
    assert_eq!(ingredient(&at_2, "fresh basil")["allergens"], json!([]));
    assert_eq!(ingredient(&at_2, "fresh basil")["shipped"], Value::Null);
}
