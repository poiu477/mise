//! Recipes that belong to a household: a version one household owns is invisible to every other, in search, on the
//! dish page, fetched by id and planned by id, while the demo recipes, which no household owns, stay everyone's. Own
//! versions are added as an import or the editor would add them, then grouped and given options by `mise-ingest`.

#[macro_use]
mod common;

use std::ops::Deref;

use axum::http::StatusCode;
use common::{As, TestApp};
use serde_json::{Value, json};
use uuid::Uuid;

const WEEK: &str = "2027-01-04";

/// The demo install with two households: the owner's ("home") and next door's, each with someone signed in.
struct Street {
    app: TestApp,
    home: String,
    home_id: Uuid,
    next_door: String,
    next_door_id: Uuid,
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
        let home_id = Uuid::parse_str(
            app.json("GET", "/api/v1/me", &home, None).await["household"]["id"]
                .as_str()
                .expect("a household"),
        )
        .unwrap();
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
        let next_door_id = Uuid::parse_str(household["id"].as_str().unwrap()).unwrap();
        Some(Street {
            app,
            home,
            home_id,
            next_door,
            next_door_id,
        })
    }

    /// A version `household` owns, as an import would add it: published at 2 servings with two lines, not yet in a
    /// dish. Run `regroup` to place it.
    async fn own_recipe(&self, household: Uuid, title: &str) -> Uuid {
        let id = Uuid::now_v7();
        sqlx::query(
            "INSERT INTO variants (id, origin, title, has_steps, content_hash, published_at, household_id)
             VALUES ($1, 'user', $2, false, $1::text, '2026-11-10T12:00:00Z', $3)",
        )
        .bind(id)
        .bind(title)
        .bind(household)
        .execute(&self.pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO variant_yields (variant_id, servings) VALUES ($1, 2)")
            .bind(id)
            .execute(&self.pool)
            .await
            .unwrap();
        for (i, name) in ["Red Lentils", "Coconut Milk"].iter().enumerate() {
            let line = Uuid::now_v7();
            sqlx::query("INSERT INTO ingredient_lines (id, variant_id, line_key, position, raw_name, shipped) VALUES ($1, $2, $3, $4, $5, true)")
                .bind(line).bind(id).bind(format!("l{i}")).bind(i as i32).bind(name).execute(&self.pool).await.unwrap();
            sqlx::query("INSERT INTO line_amounts (line_id, servings, amount, unit) VALUES ($1, 2, 1, 'cup')")
                .bind(line)
                .execute(&self.pool)
                .await
                .unwrap();
        }
        id
    }

    /// A copy `household` owns of version `parent`, as forking it would make: the same title, lines and steps, less
    /// the line keyed `without`, and published after it, so it is the dish's newest complete version. Run `regroup` to
    /// place it.
    async fn fork(&self, household: Uuid, parent: Uuid, without: &str) -> Uuid {
        let id = Uuid::now_v7();
        sqlx::query(
            "INSERT INTO variants (id, origin, parent_variant_id, title, subtitle, description, has_steps, meta, content_hash,
                                   published_at, household_id)
             SELECT $1, 'user', p.id, p.title, p.subtitle, p.description, p.has_steps, p.meta - 'media', $1::text,
                    '2026-11-10T12:00:00Z', $3
             FROM variants p WHERE p.id = $2",
        )
        .bind(id)
        .bind(parent)
        .bind(household)
        .execute(&self.pool)
        .await
        .unwrap();
        for sql in [
            "INSERT INTO variant_yields (variant_id, servings) SELECT $2, servings FROM variant_yields WHERE variant_id = $1",
            "INSERT INTO ingredient_lines (id, variant_id, line_key, position, raw_name, food_id, note, shipped, allergens, source_food, flexibility)
             SELECT gen_random_uuid(), $2, line_key, position, raw_name, food_id, note, shipped, allergens, source_food, flexibility
             FROM ingredient_lines WHERE variant_id = $1 AND line_key <> $3",
            "INSERT INTO line_amounts (line_id, servings, amount, unit)
             SELECT n.id, a.servings, a.amount, a.unit FROM line_amounts a JOIN ingredient_lines o ON o.id = a.line_id
             JOIN ingredient_lines n ON n.variant_id = $2 AND n.line_key = o.line_key WHERE o.variant_id = $1",
            "INSERT INTO steps (id, variant_id, position, text, template)
             SELECT gen_random_uuid(), $2, position, text, template FROM steps WHERE variant_id = $1",
        ] {
            sqlx::query(sql)
                .bind(parent)
                .bind(id)
                .bind(without)
                .execute(&self.pool)
                .await
                .unwrap();
        }
        id
    }

    /// Match foods, group dishes and derive options again, as the pipeline does after an import.
    async fn regroup(&self) {
        mise_ingest::foods::match_foods(&self.pool)
            .await
            .expect("match foods");
        mise_ingest::dishes::group_dishes(&self.pool)
            .await
            .expect("group dishes");
        mise_ingest::options::derive_options(&self.pool)
            .await
            .expect("derive options");
    }

    /// The slug of the dish holding `variant`.
    async fn slug_of(&self, variant: Uuid) -> String {
        sqlx::query_scalar(
            "SELECT d.slug FROM dishes d JOIN variants v ON v.dish_id = d.id WHERE v.id = $1",
        )
        .bind(variant)
        .fetch_one(&self.pool)
        .await
        .unwrap()
    }

    async fn get(&self, path: &str, cookie: &str) -> common::Reply {
        self.send("GET", path, As::Cookie(cookie), None).await
    }

    /// The titles and version counts a search returns to this household.
    async fn search(&self, query: &str, cookie: &str) -> Vec<(String, i64)> {
        let page = self
            .json("GET", &format!("/api/v1/dishes?{query}"), cookie, None)
            .await;
        page["items"]
            .as_array()
            .expect("items")
            .iter()
            .map(|d| {
                (
                    d["title"].as_str().unwrap().to_string(),
                    d["versions"].as_i64().unwrap(),
                )
            })
            .collect()
    }

    /// How many dishes this household can browse.
    async fn total(&self, cookie: &str) -> i64 {
        self.json("GET", "/api/v1/dishes", cookie, None).await["total"]
            .as_i64()
            .unwrap()
    }

    /// Plan `variant` for `WEEK` as this household, whatever the answer.
    async fn plan(&self, variant: Uuid, cookie: &str) -> common::Reply {
        self.send(
            "POST",
            "/api/v1/plan/entries",
            As::Cookie(cookie),
            Some(json!({"week": WEEK, "variant_id": variant})),
        )
        .await
    }

    /// The ids of the versions on this household's plan for `WEEK`.
    async fn planned(&self, cookie: &str) -> Vec<String> {
        let plan = self
            .json("GET", &format!("/api/v1/plan?week={WEEK}"), cookie, None)
            .await;
        plan["entries"]
            .as_array()
            .expect("entries")
            .iter()
            .map(|e| e["variant_id"].as_str().unwrap().to_string())
            .collect()
    }
}

/// The ids of a dish page's versions.
fn version_ids(dish: &Value) -> Vec<String> {
    dish["variants"]
        .as_array()
        .expect("versions")
        .iter()
        .map(|v| v["id"].as_str().unwrap().to_string())
        .collect()
}

#[tokio::test]
async fn a_household_s_own_recipe_is_invisible_next_door() {
    let street = test_app!(Street::new());
    let stew = street
        .own_recipe(street.home_id, "Coconut Lentil Stew")
        .await;
    street.regroup().await;
    let slug = street.slug_of(stew).await;
    let (home, next_door) = (&street.home, &street.next_door);

    // Search: by its title, and in the full list.
    assert_eq!(
        street.search("q=coconut+lentil", home).await,
        [("Coconut Lentil Stew".to_string(), 1)]
    );
    assert_eq!(street.search("q=coconut+lentil", next_door).await, []);
    assert_eq!(
        street.search("ingredient=lentil", home).await,
        [("Coconut Lentil Stew".to_string(), 1)]
    );
    assert_eq!(street.search("ingredient=lentil", next_door).await, []);
    assert_eq!(street.total(home).await, street.total(next_door).await + 1);

    // The dish page, and the version fetched by id.
    let page = street.get(&format!("/api/v1/dishes/{slug}"), home).await;
    assert_eq!(page.status, StatusCode::OK);
    assert_eq!(page.body["default_variant"], stew.to_string());
    let hidden = street
        .get(&format!("/api/v1/dishes/{slug}"), next_door)
        .await;
    assert_eq!(hidden.status, StatusCode::NOT_FOUND, "{}", hidden.body);
    let version = format!("/api/v1/variants/{stew}");
    assert_eq!(street.get(&version, home).await.status, StatusCode::OK);
    let hidden = street.get(&version, next_door).await;
    assert_eq!(hidden.status, StatusCode::NOT_FOUND);
    assert_eq!(
        hidden.body,
        street
            .get(&format!("/api/v1/variants/{}", Uuid::now_v7()), next_door)
            .await
            .body,
        "another household's version reads exactly as one that doesn't exist"
    );

    // Planned by id.
    let refused = street.plan(stew, next_door).await;
    assert_eq!(refused.status, StatusCode::NOT_FOUND, "{}", refused.body);
    assert_eq!(street.planned(next_door).await, Vec::<String>::new());
    assert_eq!(street.plan(stew, home).await.status, StatusCode::OK);
    assert_eq!(street.planned(home).await, [stew.to_string()]);
}

#[tokio::test]
async fn a_dish_two_households_both_wrote_stays_two_dishes() {
    let street = test_app!(Street::new());
    let ours = street
        .own_recipe(street.home_id, "Coconut Lentil Stew")
        .await;
    let theirs = street
        .own_recipe(street.next_door_id, "Coconut Lentil Stew")
        .await;
    street.regroup().await;
    let (our_dish, their_dish) = (street.slug_of(ours).await, street.slug_of(theirs).await);
    assert_ne!(
        our_dish, their_dish,
        "the same title doesn't join one household's version to another's dish"
    );

    for (cookie, mine, other) in [
        (&street.home, &our_dish, &their_dish),
        (&street.next_door, &their_dish, &our_dish),
    ] {
        assert_eq!(
            street.search("q=coconut+lentil", cookie).await,
            [("Coconut Lentil Stew".to_string(), 1)]
        );
        let page = street.get(&format!("/api/v1/dishes/{mine}"), cookie).await;
        assert_eq!(page.status, StatusCode::OK);
        assert_eq!(version_ids(&page.body).len(), 1);
        let hidden = street.get(&format!("/api/v1/dishes/{other}"), cookie).await;
        assert_eq!(hidden.status, StatusCode::NOT_FOUND);
    }
}

#[tokio::test]
async fn a_fork_joins_the_dish_everyone_sees_without_showing_next_door() {
    let street = test_app!(Street::new());
    let (home, next_door) = (&street.home, &street.next_door);
    let before = street
        .json("GET", "/api/v1/dishes/black-bean-tacos", next_door, None)
        .await;
    let parent: Uuid = before["default_variant"].as_str().unwrap().parse().unwrap();
    // Without the lime crema: a version of its own, so the dish's version option gains a choice.
    let fork = street.fork(street.home_id, parent, "crema").await;
    street.regroup().await;
    assert_eq!(street.slug_of(fork).await, "black-bean-tacos");

    // Home sees the fork among the dish's versions, and as a third choice.
    let ours = street
        .json("GET", "/api/v1/dishes/black-bean-tacos", home, None)
        .await;
    assert!(version_ids(&ours).contains(&fork.to_string()));
    assert_eq!(version_ids(&ours).len(), 3);
    assert_eq!(ours["options"][0]["choices"].as_array().unwrap().len(), 3);
    assert_eq!(
        street.search("q=black+bean", home).await,
        [("Black Bean Tacos".to_string(), 3)]
    );

    // Next door sees the dish exactly as before: its name, the version it opens on, its versions and its options.
    let theirs = street
        .json("GET", "/api/v1/dishes/black-bean-tacos", next_door, None)
        .await;
    assert_eq!(theirs, before);
    assert_eq!(
        ours["default_variant"], before["default_variant"],
        "a shared dish opens on a version everyone sees"
    );
    assert_eq!(
        street.search("q=black+bean", next_door).await,
        [("Black Bean Tacos".to_string(), 2)]
    );
    let hidden = street
        .get(&format!("/api/v1/variants/{fork}"), next_door)
        .await;
    assert_eq!(hidden.status, StatusCode::NOT_FOUND);
    assert_eq!(
        street.plan(fork, next_door).await.status,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn the_plan_and_shopping_list_drop_a_version_the_household_can_no_longer_see() {
    let street = test_app!(Street::new());
    let stew = street
        .own_recipe(street.home_id, "Coconut Lentil Stew")
        .await;
    street.regroup().await;
    let home = &street.home;
    assert_eq!(street.plan(stew, home).await.status, StatusCode::OK);
    let list = street
        .json("GET", &format!("/api/v1/shopping?week={WEEK}"), home, None)
        .await;
    assert_eq!(list["meals"], 1);

    // However it came about (sharing withdrawn, say), a plan entry for a version the household can't see shows nothing
    // of it.
    sqlx::query("UPDATE variants SET household_id = $2 WHERE id = $1")
        .bind(stew)
        .bind(street.next_door_id)
        .execute(&street.pool)
        .await
        .unwrap();
    assert_eq!(street.planned(home).await, Vec::<String>::new());
    let list = street
        .json("GET", &format!("/api/v1/shopping?week={WEEK}"), home, None)
        .await;
    assert_eq!(list["meals"], 0, "{list}");
}

#[tokio::test]
async fn only_private_visibility_is_accepted_for_now() {
    let street = test_app!(Street::new());
    let stew = street
        .own_recipe(street.home_id, "Coconut Lentil Stew")
        .await;
    let visibility: String = sqlx::query_scalar("SELECT visibility FROM variants WHERE id = $1")
        .bind(stew)
        .fetch_one(&street.pool)
        .await
        .unwrap();
    assert_eq!(visibility, "private", "versions are private by default");
    for wider in ["partners", "install"] {
        let refused = sqlx::query("UPDATE variants SET visibility = $2 WHERE id = $1")
            .bind(stew)
            .bind(wider)
            .execute(&street.pool)
            .await
            .expect_err(wider);
        assert!(
            refused.to_string().contains("variants_visibility_check"),
            "{refused}"
        );
    }
}
