//! Search, the dish page, its options and a version at any serving count, through the router on the demo recipes
//! (`fixtures/demo-bundle`, loaded as the demo install loads them; see `common::demo`).

#[macro_use]
mod common;

use std::{collections::BTreeMap, ops::Deref};

use axum::http::StatusCode;
use common::{As, TestApp};
use serde_json::Value;

/// The demo install with its owner signed in.
struct Demo {
    app: TestApp,
    cookie: String,
}

impl Deref for Demo {
    type Target = TestApp;

    fn deref(&self) -> &TestApp {
        &self.app
    }
}

impl Demo {
    /// The demo recipes with the owner set up; None when there is no database (outside CI).
    async fn new() -> Option<Demo> {
        let app = TestApp::with_demo().await?;
        let cookie = app.setup_owner().await;
        Some(Demo { app, cookie })
    }

    /// GET as the owner and expect 200 OK.
    async fn get(&self, path: &str) -> Value {
        self.json("GET", path, &self.cookie, None).await
    }

    /// GET as the owner, whatever the status.
    async fn get_reply(&self, path: &str) -> common::Reply {
        self.send("GET", path, As::Cookie(&self.cookie), None).await
    }

    /// The titles a search returns, in order.
    async fn search(&self, query: &str) -> Vec<String> {
        let page = self.get(&format!("/api/v1/dishes?{query}")).await;
        titles(&page["items"])
    }

    /// A version as the dish page sends it there, at a serving count.
    async fn version(&self, slug: &str, servings: &str) -> Value {
        let dish = self.get(&format!("/api/v1/dishes/{slug}")).await;
        let id = dish["default_variant"].as_str().expect("a default version");
        self.get(&format!("/api/v1/variants/{id}?servings={servings}"))
            .await
    }
}

/// The `title` of each item.
fn titles(items: &Value) -> Vec<String> {
    items
        .as_array()
        .expect("a list")
        .iter()
        .map(|i| i["title"].as_str().expect("a title").to_string())
        .collect()
}

/// Each ingredient as a person reads it: "12 oz Spaghetti", or the name alone when it has no amount ("Salt").
fn ingredients(version: &Value) -> Vec<String> {
    version["ingredients"]
        .as_array()
        .expect("ingredients")
        .iter()
        .map(|i| {
            let name = i["name"].as_str().expect("a name");
            match i["amount"].as_str() {
                Some(amount) => format!("{amount} {name}"),
                None => name.to_string(),
            }
        })
        .collect()
}

/// The names of the ingredients marked as scaled for this serving count.
fn scaled(version: &Value) -> Vec<String> {
    version["ingredients"]
        .as_array()
        .expect("ingredients")
        .iter()
        .filter(|i| i["scaled"] == true)
        .map(|i| i["name"].as_str().expect("a name").to_string())
        .collect()
}

/// The step at this position in the version.
fn step(version: &Value, position: usize) -> &Value {
    &version["steps"][position]
}

#[tokio::test]
async fn search_lists_every_dish_with_the_most_versions_first() {
    let demo = test_app!(Demo::new());
    let page = demo.get("/api/v1/dishes").await;
    assert_eq!(page["total"], 4);
    assert_eq!(
        titles(&page["items"]),
        [
            "Black Bean Tacos",
            "Garlic Rice Bowl",
            "Lemon Herb Chicken",
            "Weeknight Tomato Pasta",
        ]
    );
    let tacos = &page["items"][0];
    assert_eq!(tacos["slug"], "black-bean-tacos");
    assert_eq!(tacos["subtitle"], "with Lime Crema");
    assert_eq!(tacos["versions"], 2);
    assert_eq!(tacos["content_type"], "recipe");
}

#[tokio::test]
async fn search_matches_titles_subtitles_and_small_typos() {
    let demo = test_app!(Demo::new());
    assert_eq!(demo.search("q=pasta").await, ["Weeknight Tomato Pasta"]);
    assert_eq!(demo.search("q=CHICKEN").await, ["Lemon Herb Chicken"]);
    // "Scallions" is only in the subtitle.
    assert_eq!(demo.search("q=scallions").await, ["Garlic Rice Bowl"]);
    assert_eq!(
        demo.search("q=Weeknight%20Tomatoe%20Pasta").await,
        ["Weeknight Tomato Pasta"]
    );
    assert!(demo.search("q=risotto").await.is_empty());
    // A blank query is no query.
    assert_eq!(demo.search("q=%20%20").await.len(), 4);
}

#[tokio::test]
async fn search_by_ingredient_looks_at_the_version_a_dish_opens_on() {
    let demo = test_app!(Demo::new());
    assert_eq!(
        demo.search("ingredient=garlic").await,
        [
            "Garlic Rice Bowl",
            "Lemon Herb Chicken",
            "Weeknight Tomato Pasta",
        ]
    );
    // The tacos open on their newest version, with Monterey Jack; only the older one has Cheddar.
    assert_eq!(demo.search("ingredient=jack").await, ["Black Bean Tacos"]);
    assert!(demo.search("ingredient=cheddar").await.is_empty());
    assert_eq!(
        demo.search("q=tacos&ingredient=beans").await,
        ["Black Bean Tacos"]
    );
    assert!(demo.search("q=pasta&ingredient=beans").await.is_empty());
}

#[tokio::test]
async fn search_filters_by_type_and_pages() {
    let demo = test_app!(Demo::new());
    assert_eq!(demo.search("type=recipe").await.len(), 4);
    let none = demo.get("/api/v1/dishes?type=ready_meal").await;
    assert_eq!(none["total"], 0);
    assert_eq!(none["items"], Value::Array(vec![]));

    let page = demo.get("/api/v1/dishes?limit=2&offset=1").await;
    assert_eq!(
        page["total"], 4,
        "the total counts every match, not the page"
    );
    assert_eq!(
        titles(&page["items"]),
        ["Garlic Rice Bowl", "Lemon Herb Chicken"]
    );
    // Out-of-range paging is clamped rather than refused.
    assert_eq!(demo.search("limit=0").await, ["Black Bean Tacos"]);
    assert_eq!(demo.search("limit=1000&offset=-5").await.len(), 4);

    let bad = demo.get_reply("/api/v1/dishes?limit=lots").await;
    assert_eq!(bad.status, StatusCode::BAD_REQUEST, "{}", bad.body);
}

#[tokio::test]
async fn a_dish_page_lists_its_versions_newest_first() {
    let demo = test_app!(Demo::new());
    let dish = demo.get("/api/v1/dishes/black-bean-tacos").await;
    assert_eq!(dish["title"], "Black Bean Tacos");
    assert_eq!(dish["subtitle"], "with Lime Crema");
    let versions = dish["variants"].as_array().expect("versions");
    assert_eq!(versions.len(), 2);
    assert_eq!(versions[0]["published_at"], "2026-11-02T12:00:00Z");
    assert_eq!(versions[1]["published_at"], "2026-11-01T12:00:00Z");
    assert_eq!(
        dish["default_variant"], versions[0]["id"],
        "a dish opens on its newest version"
    );
    for v in versions {
        assert_eq!(v["yields"], serde_json::json!([2.0, 4.0]));
        assert_eq!(v["has_steps"], true);
        assert_eq!(v["image"], Value::Null);
    }

    let rice = demo.get("/api/v1/dishes/garlic-rice-bowl").await;
    assert_eq!(rice["variants"].as_array().expect("versions").len(), 1);
    assert_eq!(rice["variants"][0]["yields"], serde_json::json!([2.0]));
}

#[tokio::test]
async fn an_unknown_dish_or_version_is_not_found() {
    let demo = test_app!(Demo::new());
    let dish = demo.get_reply("/api/v1/dishes/no-such-dish").await;
    assert_eq!(dish.status, StatusCode::NOT_FOUND);
    assert!(dish.body["error"].is_string(), "{}", dish.body);

    let version = demo
        .get_reply(&format!("/api/v1/variants/{}", uuid::Uuid::now_v7()))
        .await;
    assert_eq!(version.status, StatusCode::NOT_FOUND);

    let bad = demo.get_reply("/api/v1/variants/not-a-uuid").await;
    assert_eq!(bad.status, StatusCode::BAD_REQUEST, "{}", bad.body);
}

#[tokio::test]
async fn options_offer_each_version_and_each_choice_picks_one() {
    let demo = test_app!(Demo::new());
    let dish = demo.get("/api/v1/dishes/black-bean-tacos").await;
    let options = dish["options"].as_array().expect("options");
    assert_eq!(options.len(), 1);
    let version = &options[0];
    assert_eq!(version["key"], "version");
    assert_eq!(version["label"], "Version");
    assert_eq!(version["default"], "standard");
    // Choices offered by as many versions as each other come in no set order (derive-options ranks them by count
    // only), so they are gathered by label.
    let choices = version["choices"].as_array().expect("choices");
    let mut cheeses = BTreeMap::new();
    for choice in choices {
        assert_eq!(choice["variants"], 1);
        assert_eq!(choice["years"], "2026");
        // The web app toggles an option by picking the version whose attributes match the choice.
        let matching: Vec<&Value> = dish["variants"]
            .as_array()
            .expect("versions")
            .iter()
            .filter(|v| v["attributes"]["version"] == choice["value"])
            .collect();
        assert_eq!(matching.len(), 1, "one version per choice: {choice}");
        let id = matching[0]["id"].as_str().expect("an id");
        if choice["value"] == version["default"] {
            assert_eq!(
                dish["default_variant"], id,
                "the default choice is the version the dish opens on"
            );
        }
        let rendered = demo.get(&format!("/api/v1/variants/{id}")).await;
        let cheese = ingredients(&rendered)
            .into_iter()
            .find(|i| i.contains("Cheese"))
            .expect("a cheese");
        let label = choice["label"].as_str().expect("a label");
        cheeses.insert(label, cheese);
    }
    assert_eq!(
        cheeses,
        BTreeMap::from([
            ("Standard", "½ cup Monterey Jack Cheese".to_string()),
            (
                "with Cheddar Cheese instead of Monterey Jack Cheese",
                "½ cup Cheddar Cheese".to_string()
            ),
        ])
    );
}

#[tokio::test]
async fn a_dish_with_one_version_offers_no_options() {
    let demo = test_app!(Demo::new());
    for slug in [
        "garlic-rice-bowl",
        "lemon-herb-chicken",
        "weeknight-tomato-pasta",
    ] {
        let dish = demo.get(&format!("/api/v1/dishes/{slug}")).await;
        assert_eq!(dish["options"], Value::Array(vec![]), "{slug}");
    }
}

#[tokio::test]
async fn a_version_opens_at_its_smallest_published_yield() {
    let demo = test_app!(Demo::new());
    let dish = demo.get("/api/v1/dishes/weeknight-tomato-pasta").await;
    let id = dish["default_variant"].as_str().expect("a version");
    let pasta = demo.get(&format!("/api/v1/variants/{id}")).await;
    assert_eq!(pasta["servings"], 2.0);
    assert_eq!(pasta["yields"], serde_json::json!([2.0, 4.0]));
    assert_eq!(pasta["dish_slug"], "weeknight-tomato-pasta");
    assert_eq!(pasta["title"], "Weeknight Tomato Pasta");
    assert_eq!(pasta["meta"]["prep_time"], "PT25M");
    assert_eq!(
        ingredients(&pasta),
        [
            "6 oz Spaghetti",
            "1 can Crushed Tomatoes",
            "2 cloves Garlic",
            "1 onion Yellow Onion",
            "¼ cup Parmesan",
            "1 pinch Red Pepper Flakes",
            "1 TBSP Olive Oil",
            "Salt",
        ]
    );
}

#[tokio::test]
async fn a_version_at_a_published_yield_reads_as_published() {
    let demo = test_app!(Demo::new());
    let pasta = demo.version("weeknight-tomato-pasta", "4").await;
    assert_eq!(pasta["servings"], 4.0);
    assert_eq!(pasta["published_yield"], true);
    assert_eq!(pasta["official"], true);
    assert_eq!(
        ingredients(&pasta),
        [
            "12 oz Spaghetti",
            "2 cans Crushed Tomatoes",
            "4 cloves Garlic",
            "2 onions Yellow Onion",
            "½ cup Parmesan",
            "2 pinches Red Pepper Flakes",
            "2 TBSP Olive Oil",
            "Salt",
        ]
    );
    assert!(scaled(&pasta).is_empty());
    // Pantry lines are marked so the shopping list can ask rather than add them.
    let oil = &pasta["ingredients"][6];
    assert_eq!(oil["shipped"], false);
    assert_eq!(pasta["ingredients"][0]["shipped"], true);

    // "(2 TBSP for 4 servings)" in the published text becomes the amount itself.
    assert_eq!(
        step(&pasta, 1)["text"],
        "Heat 2 TBSP olive oil in a large pan over medium heat. Add the onion and cook until soft, 5-6 \
         minutes. Add the garlic and red pepper flakes; cook 1 minute more."
    );
    assert_eq!(step(&pasta, 1)["scaled"], false);
    assert_eq!(
        step(&pasta, 3)["text"],
        "Meanwhile, cook the spaghetti until al dente, 9-11 minutes. Reserve 1 cup cooking water, then drain."
    );
}

#[tokio::test]
async fn a_version_between_published_yields_is_scaled_and_rounded_up() {
    let demo = test_app!(Demo::new());
    let pasta = demo.version("weeknight-tomato-pasta", "3").await;
    assert_eq!(pasta["servings"], 3.0);
    assert_eq!(pasta["published_yield"], false);
    assert_eq!(pasta["official"], false);
    assert_eq!(
        ingredients(&pasta),
        [
            "9 oz Spaghetti",
            "1½ cans Crushed Tomatoes",
            "3 cloves Garlic",
            "1½ onions Yellow Onion",
            "⅓ cup + 2 tsp Parmesan",
            "2 pinches Red Pepper Flakes",
            "1½ TBSP Olive Oil",
            "Salt",
        ]
    );
    // Salt has no amount to scale.
    assert_eq!(
        scaled(&pasta),
        [
            "Spaghetti",
            "Crushed Tomatoes",
            "Garlic",
            "Yellow Onion",
            "Parmesan",
            "Red Pepper Flakes",
            "Olive Oil",
        ]
    );
    assert_eq!(
        step(&pasta, 1)["text"],
        "Heat 1½ TBSP olive oil in a large pan over medium heat. Add the onion and cook until soft, 5-6 \
         minutes. Add the garlic and red pepper flakes; cook 1 minute more."
    );
    assert_eq!(step(&pasta, 1)["scaled"], true);
    assert_eq!(
        step(&pasta, 3)["text"],
        "Meanwhile, cook the spaghetti until al dente, 9-11 minutes. Reserve ¾ cup cooking water, then drain."
    );
    // A step with no amount in it reads the same at any yield.
    assert_eq!(step(&pasta, 2)["scaled"], false);
}

#[tokio::test]
async fn a_version_beyond_its_published_yields_scales_from_the_nearest() {
    let demo = test_app!(Demo::new());
    let pasta = demo.version("weeknight-tomato-pasta", "6").await;
    assert_eq!(pasta["official"], false);
    assert_eq!(
        ingredients(&pasta),
        [
            "18 oz Spaghetti",
            "3 cans Crushed Tomatoes",
            "6 cloves Garlic",
            "3 onions Yellow Onion",
            "¾ cup Parmesan",
            "3 pinches Red Pepper Flakes",
            "3 TBSP Olive Oil",
            "Salt",
        ]
    );
    assert_eq!(
        step(&pasta, 3)["text"],
        "Meanwhile, cook the spaghetti until al dente, 9-11 minutes. Reserve 1½ cups cooking water, then \
         drain."
    );

    // The rice bowl was published for 2 only, so every other count is scaled from it.
    let rice = demo.version("garlic-rice-bowl", "1").await;
    assert_eq!(rice["published_yield"], false);
    assert_eq!(rice["official"], false);
    assert_eq!(
        ingredients(&rice),
        [
            "⅓ cup + 2 tsp Jasmine Rice",
            "1 clove Garlic",
            "1 stalk Scallions",
            "1½ tsp Soy Sauce",
            "½ tsp Sesame Oil",
            "Salt",
        ]
    );
}

#[tokio::test]
async fn a_serving_count_out_of_range_is_refused() {
    let demo = test_app!(Demo::new());
    let dish = demo.get("/api/v1/dishes/weeknight-tomato-pasta").await;
    let id = dish["default_variant"].as_str().expect("a version");
    for servings in ["0", "-2", "24.5", "100"] {
        let reply = demo
            .get_reply(&format!("/api/v1/variants/{id}?servings={servings}"))
            .await;
        assert_eq!(reply.status, StatusCode::BAD_REQUEST, "{servings}");
        assert_eq!(
            reply.body["error"], "servings must be more than 0 and at most 24",
            "{servings}"
        );
    }
    let bad = demo
        .get_reply(&format!("/api/v1/variants/{id}?servings=two"))
        .await;
    assert_eq!(bad.status, StatusCode::BAD_REQUEST, "{}", bad.body);
    // The largest derived yield is allowed.
    let most = demo
        .get(&format!("/api/v1/variants/{id}?servings=24"))
        .await;
    assert_eq!(most["servings"], 24.0);
}

#[tokio::test]
async fn signed_out_requests_get_401() {
    let demo = test_app!(Demo::new());
    let dish = demo.get("/api/v1/dishes/black-bean-tacos").await;
    let id = dish["default_variant"].as_str().expect("a version");
    let paths = [
        "/api/v1/dishes".to_string(),
        "/api/v1/dishes?q=tacos".to_string(),
        "/api/v1/dishes/black-bean-tacos".to_string(),
        format!("/api/v1/variants/{id}"),
        format!("/api/v1/variants/{id}?servings=3"),
        format!("/api/v1/media/{}/400", uuid::Uuid::now_v7()),
    ];
    for path in &paths {
        for from in [
            As::Nobody,
            As::Cookie("not-a-session"),
            As::Bearer("mise_not-a-token"),
        ] {
            let reply = demo.send("GET", path, from, None).await;
            assert_eq!(reply.status, StatusCode::UNAUTHORIZED, "{path}");
            assert!(reply.body["error"].is_string(), "{path}: {}", reply.body);
            assert!(
                !reply.body.to_string().contains("Black Bean"),
                "{path} leaks the recipe"
            );
        }
    }
}

#[tokio::test]
async fn a_signed_out_session_can_no_longer_read_recipes() {
    let demo = test_app!(Demo::new());
    demo.get("/api/v1/dishes").await;
    let out = demo
        .send(
            "POST",
            "/api/v1/auth/logout",
            As::Cookie(&demo.cookie),
            None,
        )
        .await;
    assert!(out.status.is_success(), "sign out: {}", out.body);
    let reply = demo.get_reply("/api/v1/dishes").await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
}
