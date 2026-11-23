//! Importing a recipe from a link: fixture pages served by a site the test runs on this machine (the import is let
//! through to that site's exact address and nothing else that isn't public), read into a version the importing
//! household alone sees, grouped into a dish; a repeat import changes nothing; private addresses are refused.

#[macro_use]
mod common;

use std::path::Path;

use axum::http::StatusCode;
use common::{As, TestApp, site::Site};
use serde_json::{Value, json};

/// A fixture page from `fixtures/web-pages`, written for mise.
fn fixture(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/web-pages")
        .join(name);
    std::fs::read_to_string(path).unwrap()
}

/// A page written for this test, its photo at `photo`.
fn soup_page(photo: &str, extra_step: &str) -> String {
    let recipe = json!({
        "@context": "https://schema.org", "@type": "Recipe",
        "name": "Smoky Leek and Potato Soup",
        "description": "A thick soup for a cold evening.",
        "image": photo,
        "recipeYield": "4 servings",
        "recipeIngredient": ["3 leeks, sliced", "1 lb potatoes, diced", "2 TBSP butter", "4 cups stock", "Salt, to taste"],
        "recipeInstructions": [
            {"@type": "HowToStep", "text": "Melt 2 TBSP butter and soften the leeks."},
            {"@type": "HowToStep", "text": format!("Add the potatoes and stock and simmer until soft.{extra_step}")},
        ],
    });
    format!(
        "<!doctype html><html><head><title>Leek soup</title>\
         <script type=\"application/ld+json\">{recipe}</script></head><body><h1>Leek soup</h1></body></html>"
    )
}

/// A small photo, as a PNG.
fn png() -> Vec<u8> {
    let mut out = std::io::Cursor::new(Vec::new());
    image::RgbImage::from_pixel(12, 8, image::Rgb([120, 160, 60]))
        .write_to(&mut out, image::ImageFormat::Png)
        .unwrap();
    out.into_inner()
}

/// The demo install with a site to import from, the owner signed in, and a second household signed in too.
struct Kitchen {
    app: TestApp,
    site: Site,
    home: String,
    next_door: String,
}

impl Kitchen {
    async fn new() -> Option<Kitchen> {
        let site = Site::start().await;
        let app = TestApp::with_demo_fetching(vec![site.addr]).await?;
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
        Some(Kitchen {
            app,
            site,
            home,
            next_door,
        })
    }

    /// Imports `url` as this browser; the status and body.
    async fn import(&self, cookie: &str, url: &str) -> (StatusCode, Value) {
        let reply = self
            .app
            .send(
                "POST",
                "/api/v1/imports",
                As::Cookie(cookie),
                Some(json!({"url": url})),
            )
            .await;
        (reply.status, reply.body)
    }

    /// How many rows `table` has.
    async fn count(&self, table: &str) -> i64 {
        sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT count(*) FROM {table}")))
            .fetch_one(&self.app.pool)
            .await
            .unwrap()
    }
}

#[tokio::test]
async fn a_fixture_page_becomes_a_version_of_the_household_s_own() {
    let k = test_app!(Kitchen::new());
    k.site.serve(
        "/hummus",
        "text/html; charset=utf-8",
        fixture("no-image.html"),
    );
    let (status, body) = k.import(&k.home, &k.site.url("/hummus")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let recipe = &body["recipes"][0];
    assert_eq!(recipe["title"], "Five-Minute Hummus");
    assert_eq!(recipe["unchanged"], false);
    assert_eq!(recipe["photo_problem"], Value::Null);

    // It opens on its dish's page, at its published 4 servings and any other.
    let slug = recipe["dish_slug"].as_str().unwrap();
    let dish = k
        .app
        .json("GET", &format!("/api/v1/dishes/{slug}"), &k.home, None)
        .await;
    assert_eq!(dish["title"], "Five-Minute Hummus");
    let id = recipe["variant_id"].as_str().unwrap();
    let version = k
        .app
        .json(
            "GET",
            &format!("/api/v1/variants/{id}?servings=8"),
            &k.home,
            None,
        )
        .await;
    let lines: Vec<(&str, &str)> = version["ingredients"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| {
            (
                i["name"].as_str().unwrap(),
                i["amount"].as_str().unwrap_or(""),
            )
        })
        .collect();
    // 6 TBSP, in the measures mise rounds into.
    assert!(lines.contains(&("tahini", "⅓ cup + 2 tsp")), "{lines:?}");
    assert!(lines.contains(&("chickpeas", "2 cans")), "{lines:?}");
    assert!(
        version["ingredients"]
            .as_array()
            .unwrap()
            .iter()
            .all(|i| i["food_id"].is_string())
    );
    assert_eq!(version["steps"].as_array().unwrap().len(), 1);
    let found = k
        .app
        .json("GET", "/api/v1/dishes?q=hummus", &k.home, None)
        .await;
    assert_eq!(found["total"], 1);

    // Nobody else sees it.
    let other = k
        .app
        .send(
            "GET",
            &format!("/api/v1/variants/{id}"),
            As::Cookie(&k.next_door),
            None,
        )
        .await;
    assert_eq!(other.status, StatusCode::NOT_FOUND);
    let found = k
        .app
        .json("GET", "/api/v1/dishes?q=hummus", &k.next_door, None)
        .await;
    assert_eq!(found["total"], 0);
    let owner: Option<uuid::Uuid> = sqlx::query_scalar(
        "SELECT sr.household_id FROM variants v JOIN source_records sr ON sr.id = v.source_record_id WHERE v.id = $1::uuid",
    )
    .bind(id)
    .fetch_one(&k.app.pool)
    .await
    .unwrap();
    assert!(owner.is_some(), "the record belongs to the household too");
}

#[tokio::test]
async fn a_page_s_photo_is_kept_as_its_picture_only() {
    let k = test_app!(Kitchen::new());
    k.site.serve("/soup.png", "image/png", png());
    k.site.serve(
        "/soup",
        "text/html",
        soup_page(&k.site.url("/soup.png"), ""),
    );
    let (status, body) = k.import(&k.home, &k.site.url("/soup")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let recipe = &body["recipes"][0];
    assert_eq!(recipe["photo_problem"], Value::Null);
    let id = recipe["variant_id"].as_str().unwrap();
    let version = k
        .app
        .json("GET", &format!("/api/v1/variants/{id}"), &k.home, None)
        .await;
    assert!(
        version["image"]["small"].is_string(),
        "{}",
        version["image"]
    );
    assert_eq!(k.site.asked("/soup.png"), 1);
}

#[tokio::test]
async fn importing_the_same_unchanged_page_again_changes_nothing() {
    let k = test_app!(Kitchen::new());
    k.site.serve("/soup.png", "image/png", png());
    k.site.serve(
        "/soup",
        "text/html",
        soup_page(&k.site.url("/soup.png"), ""),
    );
    let (_, first) = k.import(&k.home, &k.site.url("/soup")).await;
    let tables = [
        "source_records",
        "variants",
        "ingredient_lines",
        "steps",
        "dishes",
        "foods",
        "media",
        "variant_media",
    ];
    let mut before = Vec::new();
    for t in tables {
        before.push(k.count(t).await);
    }
    let dishes_before: Vec<(uuid::Uuid, String)> =
        sqlx::query_as("SELECT id, slug FROM dishes ORDER BY id")
            .fetch_all(&k.app.pool)
            .await
            .unwrap();

    let (status, again) = k.import(&k.home, &k.site.url("/soup")).await;
    assert_eq!(status, StatusCode::OK, "{again}");
    assert_eq!(again["recipes"][0]["unchanged"], true);
    assert_eq!(
        again["recipes"][0]["variant_id"],
        first["recipes"][0]["variant_id"]
    );
    assert_eq!(
        again["recipes"][0]["dish_slug"],
        first["recipes"][0]["dish_slug"]
    );
    let mut after = Vec::new();
    for t in tables {
        after.push(k.count(t).await);
    }
    assert_eq!(after, before, "rows in {tables:?}");
    let dishes_after: Vec<(uuid::Uuid, String)> =
        sqlx::query_as("SELECT id, slug FROM dishes ORDER BY id")
            .fetch_all(&k.app.pool)
            .await
            .unwrap();
    assert_eq!(dishes_after, dishes_before);
    assert_eq!(
        k.site.asked("/soup.png"),
        1,
        "the photo wasn't fetched again"
    );

    // Another household importing the same page gets a version of its own.
    let (_, theirs) = k.import(&k.next_door, &k.site.url("/soup")).await;
    assert_eq!(theirs["recipes"][0]["unchanged"], false);
    assert_ne!(
        theirs["recipes"][0]["variant_id"],
        first["recipes"][0]["variant_id"]
    );
}

#[tokio::test]
async fn a_changed_page_becomes_a_new_version_of_the_same_dish() {
    let k = test_app!(Kitchen::new());
    k.site.serve("/soup", "text/html", soup_page("", ""));
    let (_, first) = k.import(&k.home, &k.site.url("/soup")).await;
    k.site
        .serve("/soup", "text/html", soup_page("", " Blend half of it."));
    let (status, second) = k.import(&k.home, &k.site.url("/soup")).await;
    assert_eq!(status, StatusCode::OK, "{second}");
    let (old, new) = (&first["recipes"][0], &second["recipes"][0]);
    assert_eq!(new["unchanged"], false);
    assert_ne!(new["variant_id"], old["variant_id"]);
    assert_eq!(new["dish_slug"], old["dish_slug"]);
    let parent: Option<uuid::Uuid> =
        sqlx::query_scalar("SELECT parent_variant_id FROM variants WHERE id = $1::uuid")
            .bind(new["variant_id"].as_str().unwrap())
            .fetch_one(&k.app.pool)
            .await
            .unwrap();
    assert_eq!(
        parent.map(|p| p.to_string()).as_deref(),
        old["variant_id"].as_str()
    );
}

#[tokio::test]
async fn a_private_address_is_refused() {
    let k = test_app!(Kitchen::new());
    // The test's own site is let through at its exact address; the same machine on another port is not.
    let elsewhere = format!(
        "http://127.0.0.1:{}/",
        k.site.addr.port().wrapping_add(1).max(1024)
    );
    for url in [
        elsewhere.as_str(),
        "http://localhost:8080/",
        "http://10.0.0.1/",
        "http://192.168.1.1/admin",
        "http://169.254.169.254/latest/meta-data/",
        "http://[::1]/",
        "http://[::ffff:127.0.0.1]/",
    ] {
        let (status, body) = k.import(&k.home, url).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{url}: {body}");
        assert_eq!(body["code"], "bad_request");
        assert!(
            body["error"]
                .as_str()
                .unwrap()
                .ends_with("isn't an address on the public internet"),
            "{url}: {body}"
        );
    }
    let (status, body) = k.import(&k.home, "file:///etc/passwd").await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    let (status, body) = k.import(&k.home, "not a link").await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(
        k.count("source_records WHERE household_id IS NOT NULL")
            .await,
        0
    );
}

#[tokio::test]
async fn a_photo_at_a_private_address_is_left_out() {
    let k = test_app!(Kitchen::new());
    k.site.serve(
        "/soup",
        "text/html",
        soup_page("http://192.168.1.10/soup.jpg", ""),
    );
    let (status, body) = k.import(&k.home, &k.site.url("/soup")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["recipes"][0]["photo_problem"],
        "its photo couldn't be kept: 192.168.1.10 isn't an address on the public internet"
    );
    assert_eq!(k.count("variant_media").await, 0);
}

#[tokio::test]
async fn a_page_without_a_recipe_or_that_is_not_a_page_is_refused() {
    let k = test_app!(Kitchen::new());
    k.site
        .serve("/article", "text/html", fixture("no-recipe.html"));
    k.site.serve("/photo", "image/png", png());
    for (path, why) in [
        ("/article", "no recipe found"),
        ("/photo", "the link doesn't lead to a web page"),
    ] {
        let (status, body) = k.import(&k.home, &k.site.url(path)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{path}: {body}");
        assert!(
            body["error"].as_str().unwrap().contains(why),
            "{path}: {body}"
        );
    }
    let (status, body) = k.import(&k.home, &k.site.url("/gone")).await;
    assert_eq!(status, StatusCode::BAD_GATEWAY, "{body}");
    assert_eq!(body["code"], "site_failed");
}
