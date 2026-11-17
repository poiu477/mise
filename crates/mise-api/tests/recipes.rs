//! Recipes written in the app: created, read back as written, changed, given a photo, and found like any other
//! version in search, on a dish page and by id, by the household that wrote them and nobody else. Each test runs on
//! the demo recipes with two households, the owner's ("home") and next door's.

#[macro_use]
mod common;

use std::ops::Deref;

use axum::{
    body::Body,
    http::{StatusCode, header},
};
use common::{As, Reply, TestApp};
use http_body_util::BodyExt;
use image::{ImageFormat, Rgb, RgbImage, Rgba, RgbaImage};
use serde_json::{Value, json};

/// A made-up id, for what isn't there.
const SOME_ID: &str = "0190f4a2-0000-7000-8000-000000000000";

/// Words hidden in an uploaded photo's file, as a camera hides where a photo was taken.
const HIDDEN: &[u8] = b"taken at 12 Orchard Lane";

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

    /// Write `recipe` as this browser and expect it stored.
    async fn write(&self, cookie: &str, recipe: Value) -> Value {
        self.json("POST", "/api/v1/recipes", cookie, Some(recipe))
            .await
    }

    /// Send `bytes` as the recipe's photo, with this Content-Type.
    async fn send_photo(&self, id: &Value, from: As<'_>, kind: &str, bytes: Vec<u8>) -> Reply {
        let path = format!("/api/v1/recipes/{}/photo", id.as_str().unwrap());
        let req = TestApp::request("PUT", &path, from)
            .header(header::CONTENT_TYPE, kind)
            .body(Body::from(bytes))
            .unwrap();
        self.read(req).await
    }

    /// A photo's status, type and bytes, fetched as this browser.
    async fn fetch(&self, path: &str, cookie: &str) -> (StatusCode, String, Vec<u8>) {
        let req = TestApp::request("GET", path, As::Cookie(cookie))
            .body(Body::empty())
            .unwrap();
        let (parts, body) = self.oneshot(req).await.into_parts();
        let kind = parts
            .headers
            .get(header::CONTENT_TYPE)
            .and_then(|h| h.to_str().ok())
            .unwrap_or_default()
            .to_string();
        let bytes = body.collect().await.unwrap().to_bytes().to_vec();
        (parts.status, kind, bytes)
    }
}

/// A lentil soup, written for 4.
fn dal() -> Value {
    json!({
        "title": "Red Lentil Dal",
        "subtitle": "with Lime",
        "description": "A weeknight dal.",
        "servings": 4,
        "ingredients": [
            {"text": "1 cup red lentils, rinsed"},
            {"text": "2 cloves garlic, minced"},
            {"text": "1 yellow onion, diced"},
            {"text": "2 TBSP olive oil"},
            {"text": "1 (14 oz) can coconut milk"},
            {"text": "Salt, to taste"},
            {"text": "   "},
        ],
        "steps": [
            "Warm 2 TBSP olive oil in a pot. Cook the onion and garlic until soft.",
            "",
            "Add the lentils and coconut milk with 3 cups water. Simmer until the lentils fall apart.",
        ],
    })
}

/// The stored line whose text starts with `start`.
fn line<'a>(recipe: &'a Value, start: &str) -> &'a Value {
    recipe["ingredients"]
        .as_array()
        .unwrap()
        .iter()
        .find(|l| l["text"].as_str().unwrap().starts_with(start))
        .unwrap_or_else(|| panic!("no line starting {start:?} in {recipe}"))
}

/// A photo of flat colour, as a JPEG with `HIDDEN` in a comment inside the file.
fn jpeg(width: u32, height: u32) -> Vec<u8> {
    let mut plain = std::io::Cursor::new(Vec::new());
    RgbImage::from_pixel(width, height, Rgb([200, 90, 40]))
        .write_to(&mut plain, ImageFormat::Jpeg)
        .unwrap();
    let plain = plain.into_inner();
    // A comment segment (FF FE, then its length including the two length bytes) right after the start marker.
    let mut out = plain[..2].to_vec();
    out.extend([0xFF, 0xFE]);
    out.extend(((HIDDEN.len() + 2) as u16).to_be_bytes());
    out.extend(HIDDEN);
    out.extend(&plain[2..]);
    out
}

/// A picture with a see-through corner, as a PNG.
fn cutout() -> Vec<u8> {
    let mut image = RgbaImage::from_pixel(300, 200, Rgba([20, 120, 60, 255]));
    image.put_pixel(0, 0, Rgba([0, 0, 0, 0]));
    let mut out = std::io::Cursor::new(Vec::new());
    image.write_to(&mut out, ImageFormat::Png).unwrap();
    out.into_inner()
}

/// The width and height of the image in `bytes`.
fn dimensions(bytes: &[u8]) -> (u32, u32) {
    let image = image::load_from_memory(bytes).expect("an image");
    (image.width(), image.height())
}

#[tokio::test]
async fn a_written_recipe_reads_back_as_written_with_each_line_read() {
    let street = test_app!(Street::new());
    let stored = street.write(&street.home, dal()).await;

    assert_eq!(stored["title"], "Red Lentil Dal");
    assert_eq!(stored["subtitle"], "with Lime");
    assert_eq!(stored["servings"], 4.0);
    assert_eq!(stored["dish_slug"], "red-lentil-dal");
    assert_eq!(stored["image"], Value::Null);
    // Blank lines and steps are dropped.
    assert_eq!(stored["ingredients"].as_array().unwrap().len(), 6);
    assert_eq!(stored["steps"].as_array().unwrap().len(), 2);

    let lentils = line(&stored, "1 cup");
    assert_eq!(lentils["text"], "1 cup red lentils, rinsed");
    assert_eq!(
        (
            &lentils["amount"],
            &lentils["unit"],
            &lentils["name"],
            &lentils["note"]
        ),
        (
            &json!(1.0),
            &json!("cup"),
            &json!("red lentils"),
            &json!("rinsed")
        )
    );
    let can = line(&stored, "1 (14 oz)");
    assert_eq!(
        (&can["amount"], &can["unit"], &can["name"], &can["note"]),
        (
            &json!(1.0),
            &json!("can"),
            &json!("coconut milk"),
            &json!("14 oz")
        )
    );
    let salt = line(&stored, "Salt");
    assert_eq!(
        (&salt["amount"], &salt["unit"], &salt["name"], &salt["note"]),
        (
            &Value::Null,
            &Value::Null,
            &json!("Salt"),
            &json!("to taste")
        )
    );

    // Names the demo recipes use are matched to their foods; a new name becomes a food.
    assert_eq!(line(&stored, "2 cloves")["food"]["name"], "Garlic");
    assert_eq!(line(&stored, "1 yellow")["food"]["name"], "Yellow Onion");
    assert_eq!(line(&stored, "2 TBSP")["food"]["name"], "Olive Oil");
    assert_eq!(salt["food"]["name"], "Salt");
    assert_eq!(lentils["food"]["name"], "red lentils");

    let path = format!("/api/v1/recipes/{}", stored["id"].as_str().unwrap());
    let read = street.json("GET", &path, &street.home, None).await;
    assert_eq!(read, stored);
}

#[tokio::test]
async fn a_written_recipe_opens_at_any_serving_count() {
    let street = test_app!(Street::new());
    let stored = street.write(&street.home, dal()).await;
    let id = stored["id"].as_str().unwrap();

    let amounts = |version: &Value| -> Vec<(String, Value)> {
        version["ingredients"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| (i["name"].as_str().unwrap().to_string(), i["amount"].clone()))
            .collect()
    };
    let as_written = street
        .json("GET", &format!("/api/v1/variants/{id}"), &street.home, None)
        .await;
    assert_eq!(as_written["servings"], 4.0);
    assert_eq!(as_written["official"], true);
    assert_eq!(
        amounts(&as_written),
        [
            ("red lentils".to_string(), json!("1 cup")),
            ("garlic".to_string(), json!("2 cloves")),
            ("yellow onion".to_string(), json!("1 onion")),
            ("olive oil".to_string(), json!("2 TBSP")),
            ("coconut milk".to_string(), json!("1 can")),
            ("Salt".to_string(), Value::Null),
        ]
    );
    assert_eq!(as_written["ingredients"][0]["note"], "rinsed");
    assert_eq!(as_written["steps"].as_array().unwrap().len(), 2);

    let doubled = street
        .json(
            "GET",
            &format!("/api/v1/variants/{id}?servings=8"),
            &street.home,
            None,
        )
        .await;
    assert_eq!(doubled["official"], false);
    assert_eq!(amounts(&doubled)[0].1, "2 cups");
    assert_eq!(amounts(&doubled)[1].1, "4 cloves");
    // The oil in the first step is tied to its ingredient line, so it doubles with it.
    let first = doubled["steps"][0]["text"].as_str().unwrap();
    assert!(first.starts_with("Warm ¼ cup olive oil"), "{first}");
}

#[tokio::test]
async fn a_written_recipe_is_found_in_search_and_on_its_dish_page_by_its_household_only() {
    let street = test_app!(Street::new());
    let stored = street.write(&street.home, dal()).await;
    let id = stored["id"].as_str().unwrap();

    let found = street
        .json("GET", "/api/v1/dishes?q=lentil", &street.home, None)
        .await;
    assert_eq!(found["total"], 1, "{found}");
    assert_eq!(found["items"][0]["slug"], "red-lentil-dal");
    assert_eq!(found["items"][0]["title"], "Red Lentil Dal");
    assert_eq!(found["items"][0]["default_variant"], *id);
    let by_food = street
        .json(
            "GET",
            "/api/v1/dishes?ingredient=red%20lentils",
            &street.home,
            None,
        )
        .await;
    assert_eq!(by_food["total"], 1, "{by_food}");

    let dish = street
        .json("GET", "/api/v1/dishes/red-lentil-dal", &street.home, None)
        .await;
    assert_eq!(dish["title"], "Red Lentil Dal");
    assert_eq!(dish["subtitle"], "with Lime");
    assert_eq!(dish["default_variant"], *id);
    assert_eq!(dish["variants"].as_array().unwrap().len(), 1);
    assert_eq!(dish["variants"][0]["yields"], json!([4.0]));

    // It can be planned like any other version.
    let planned = street
        .send(
            "POST",
            "/api/v1/plan/entries",
            As::Cookie(&street.home),
            Some(json!({"week": "2027-01-04", "variant_id": id, "servings": 2})),
        )
        .await;
    assert_eq!(planned.status, StatusCode::OK, "{}", planned.body);

    // Next door, it doesn't exist.
    let found = street
        .json("GET", "/api/v1/dishes?q=lentil", &street.next_door, None)
        .await;
    assert_eq!(found["total"], 0, "{found}");
    for path in [
        "/api/v1/dishes/red-lentil-dal".to_string(),
        format!("/api/v1/variants/{id}"),
        format!("/api/v1/recipes/{id}"),
    ] {
        let reply = street
            .send("GET", &path, As::Cookie(&street.next_door), None)
            .await;
        assert_eq!(
            reply.status,
            StatusCode::NOT_FOUND,
            "{path}: {}",
            reply.body
        );
    }
}

#[tokio::test]
async fn editing_replaces_the_recipe_and_its_dish_follows_the_title() {
    let street = test_app!(Street::new());
    let stored = street.write(&street.home, dal()).await;
    let path = format!("/api/v1/recipes/{}", stored["id"].as_str().unwrap());
    let parmesan = street
        .json("GET", "/api/v1/foods?q=parmesan", &street.home, None)
        .await["items"][0]
        .clone();
    assert_eq!(parmesan["name"], "Parmesan");

    let changed = street
        .json(
            "PUT",
            &path,
            &street.home,
            Some(json!({
                "title": "Golden Dal",
                "servings": 6,
                "ingredients": [
                    {"text": "1½ cups red lentils"},
                    // The cook says which food the line means, whatever its name matches.
                    {"text": "a handful of hard cheese, grated", "food_id": parmesan["id"]},
                ],
                "steps": ["Simmer everything together."],
            })),
        )
        .await;
    assert_eq!(changed["id"], stored["id"]);
    assert_eq!(changed["title"], "Golden Dal");
    assert_eq!(changed["subtitle"], Value::Null);
    assert_eq!(changed["description"], Value::Null);
    assert_eq!(changed["servings"], 6.0);
    assert_eq!(changed["dish_slug"], "golden-dal");
    assert_eq!(changed["ingredients"].as_array().unwrap().len(), 2);
    assert_eq!(changed["ingredients"][0]["amount"], 1.5);
    assert_eq!(changed["ingredients"][1]["food"], parmesan);
    assert_eq!(changed["steps"], json!(["Simmer everything together."]));
    assert_eq!(street.json("GET", &path, &street.home, None).await, changed);

    // The dish took the new title and address; the old address still finds it.
    for slug in ["golden-dal", "red-lentil-dal"] {
        let dish = street
            .json("GET", &format!("/api/v1/dishes/{slug}"), &street.home, None)
            .await;
        assert_eq!(
            (&dish["slug"], &dish["title"]),
            (&json!("golden-dal"), &json!("Golden Dal"))
        );
    }
    // Renamed back, it takes its first address again rather than a numbered one.
    let back = street.json("PUT", &path, &street.home, Some(dal())).await;
    assert_eq!(back["dish_slug"], "red-lentil-dal");
    let old = street
        .json("GET", "/api/v1/dishes/golden-dal", &street.home, None)
        .await;
    assert_eq!(old["slug"], "red-lentil-dal");
}

#[tokio::test]
async fn two_recipes_with_one_title_get_addresses_of_their_own() {
    let street = test_app!(Street::new());
    let first = street.write(&street.home, dal()).await;
    let second = street.write(&street.next_door, dal()).await;
    let third = street.write(&street.home, dal()).await;
    assert_eq!(first["dish_slug"], "red-lentil-dal");
    assert_eq!(second["dish_slug"], "red-lentil-dal-2");
    assert_eq!(third["dish_slug"], "red-lentil-dal-3");
    // A title with nothing to make an address from still gets one.
    let symbols = street
        .write(
            &street.home,
            json!({"title": "!!!", "servings": 1, "ingredients": [], "steps": []}),
        )
        .await;
    assert_eq!(symbols["dish_slug"], "recipe");
    // The demo dishes keep theirs.
    let tacos = street
        .write(
            &street.home,
            json!({"title": "Black Bean Tacos", "servings": 2, "ingredients": [], "steps": []}),
        )
        .await;
    assert_eq!(tacos["dish_slug"], "black-bean-tacos-2");
}

#[tokio::test]
async fn a_recipe_that_can_t_be_stored_is_refused_with_the_reason() {
    let street = test_app!(Street::new());
    let with = |change: fn(&mut Value)| {
        let mut recipe = dal();
        change(&mut recipe);
        recipe
    };
    let cases: [(Value, &str); 8] = [
        (
            with(|r| r["title"] = json!("  ")),
            "give the recipe a title",
        ),
        (
            with(|r| r["title"] = json!("a".repeat(201))),
            "the title can have at most 200 characters",
        ),
        (
            with(|r| r["servings"] = json!(0)),
            "servings must be more than 0 and at most 1000",
        ),
        (
            with(|r| r["servings"] = json!(1001)),
            "servings must be more than 0 and at most 1000",
        ),
        (
            with(|r| r["ingredients"][1]["text"] = json!("b".repeat(301))),
            "ingredient 2 can have at most 300 characters",
        ),
        (
            with(|r| r["ingredients"] = json!(vec![json!({"text": "1 egg"}); 201])),
            "a recipe can have at most 200 ingredients",
        ),
        (
            with(|r| r["steps"][0] = json!("step\u{0}")),
            "step 1 can have at most 4000 characters",
        ),
        (
            with(|r| r["ingredients"][2]["food_id"] = json!(SOME_ID)),
            "ingredient 3: no such food",
        ),
    ];
    for (recipe, reason) in cases {
        let reply = street
            .send(
                "POST",
                "/api/v1/recipes",
                As::Cookie(&street.home),
                Some(recipe),
            )
            .await;
        assert_eq!(reply.status, StatusCode::BAD_REQUEST, "{reason}");
        assert_eq!(reply.body["error"], reason);
    }
    let stored: i64 = sqlx::query_scalar("SELECT count(*) FROM variants WHERE origin = 'user'")
        .fetch_one(&street.pool)
        .await
        .unwrap();
    assert_eq!(stored, 0, "a refused recipe leaves nothing behind");
}

#[tokio::test]
async fn only_the_household_that_wrote_a_recipe_can_change_it() {
    let street = test_app!(Street::new());
    let stored = street.write(&street.home, dal()).await;
    let id = stored["id"].clone();
    let path = format!("/api/v1/recipes/{}", id.as_str().unwrap());
    let photo = format!("{path}/photo");

    let next_door = As::Cookie(&street.next_door);
    let edit = street.send("PUT", &path, next_door, Some(dal())).await;
    assert_eq!(edit.status, StatusCode::NOT_FOUND, "{}", edit.body);
    let sent = street
        .send_photo(&id, next_door, "image/jpeg", jpeg(40, 30))
        .await;
    assert_eq!(sent.status, StatusCode::NOT_FOUND, "{}", sent.body);
    let removed = street.send("DELETE", &photo, next_door, None).await;
    assert_eq!(removed.status, StatusCode::NOT_FOUND, "{}", removed.body);

    // A version that wasn't written in the app isn't a recipe to edit, even for a household that can see it.
    let tacos = street
        .json("GET", "/api/v1/dishes/black-bean-tacos", &street.home, None)
        .await["default_variant"]
        .clone();
    let seed = format!("/api/v1/recipes/{}", tacos.as_str().unwrap());
    for (method, body) in [("GET", None), ("PUT", Some(dal()))] {
        let reply = street
            .send(method, &seed, As::Cookie(&street.home), body)
            .await;
        assert_eq!(
            reply.status,
            StatusCode::NOT_FOUND,
            "{method}: {}",
            reply.body
        );
    }

    // A read-only token reads the recipe but can't write one.
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
    let reader = As::Bearer(&token);
    let read = street.send("GET", &path, reader, None).await;
    assert_eq!(read.status, StatusCode::OK, "{}", read.body);
    for (method, target, body) in [
        ("POST", "/api/v1/recipes", Some(dal())),
        ("PUT", path.as_str(), Some(dal())),
        ("DELETE", photo.as_str(), None),
    ] {
        let reply = street.send(method, target, reader, body).await;
        assert_eq!(reply.status, StatusCode::FORBIDDEN, "{method} {target}");
    }

    assert_eq!(
        street.json("GET", &path, &street.home, None).await,
        stored,
        "nothing changed"
    );
}

#[tokio::test]
async fn a_photo_is_kept_as_a_picture_only_and_served_to_its_household() {
    let street = test_app!(Street::new());
    let stored = street.write(&street.home, dal()).await;
    let id = stored["id"].clone();
    let me = As::Cookie(&street.home);

    let upload = jpeg(1600, 1200);
    assert!(upload.windows(HIDDEN.len()).any(|w| w == HIDDEN));
    let sent = street.send_photo(&id, me, "image/jpeg", upload).await;
    assert_eq!(sent.status, StatusCode::OK, "{}", sent.body);
    let image = sent.body["image"].clone();
    assert!(image["id"].is_string(), "{}", sent.body);

    // Search, the dish page and the version all show it.
    let found = street
        .json("GET", "/api/v1/dishes?q=lentil", &street.home, None)
        .await;
    assert_eq!(found["items"][0]["image"], image);
    let version = street
        .json(
            "GET",
            &format!("/api/v1/variants/{}", id.as_str().unwrap()),
            &street.home,
            None,
        )
        .await;
    assert_eq!(version["image"], image);

    // Each size is served, and none carries what was hidden in the file.
    for (size, expected) in [
        ("small", (400, 300)),
        ("large", (1200, 900)),
        ("original", (1600, 1200)),
    ] {
        let (status, kind, bytes) = street
            .fetch(image[size].as_str().unwrap(), &street.home)
            .await;
        assert_eq!(status, StatusCode::OK, "{size}");
        assert_eq!(kind, "image/jpeg", "{size}");
        assert_eq!(dimensions(&bytes), expected, "{size}");
        assert!(
            !bytes.windows(HIDDEN.len()).any(|w| w == HIDDEN),
            "{size} still holds what the file hid"
        );
    }
    // Next door can't fetch it, by any address.
    let (status, _, _) = street
        .fetch(image["small"].as_str().unwrap(), &street.next_door)
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // A second photo replaces the first; a see-through one stays a PNG.
    let sent = street.send_photo(&id, me, "image/png", cutout()).await;
    assert_eq!(sent.status, StatusCode::OK, "{}", sent.body);
    let second = sent.body["image"].clone();
    assert_ne!(second["id"], image["id"]);
    let (status, kind, bytes) = street
        .fetch(second["small"].as_str().unwrap(), &street.home)
        .await;
    assert_eq!((status, kind.as_str()), (StatusCode::OK, "image/png"));
    assert_eq!(dimensions(&bytes), (300, 200));
    let (status, _, _) = street
        .fetch(image["small"].as_str().unwrap(), &street.home)
        .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "no version uses the first photo now"
    );

    // Taken off, the recipe has no photo, and taking it off again changes nothing.
    let path = format!("/api/v1/recipes/{}/photo", id.as_str().unwrap());
    for _ in 0..2 {
        let removed = street.json("DELETE", &path, &street.home, None).await;
        assert_eq!(removed["image"], Value::Null);
    }
    let (status, _, _) = street
        .fetch(second["small"].as_str().unwrap(), &street.home)
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_file_that_isn_t_a_photo_or_is_too_big_is_refused() {
    let street = test_app!(Street::new());
    let stored = street.write(&street.home, dal()).await;
    let id = stored["id"].clone();
    let me = As::Cookie(&street.home);
    let not_a_photo = "that isn't a photo mise can read: send a JPEG, PNG, WebP or GIF";

    // Text that says it is a JPEG.
    let text = street
        .send_photo(&id, me, "image/jpeg", b"<html>not a photo</html>".to_vec())
        .await;
    assert_eq!(text.status, StatusCode::BAD_REQUEST);
    assert_eq!(text.body["error"], not_a_photo);
    // A PNG cut off after its first bytes.
    let mut cut = cutout();
    cut.truncate(40);
    let cut = street.send_photo(&id, me, "image/png", cut).await;
    assert_eq!(cut.status, StatusCode::BAD_REQUEST);
    assert_eq!(cut.body["error"], not_a_photo);
    // A real photo sent as something else: a form another site could post.
    for kind in [
        "application/octet-stream",
        "multipart/form-data; boundary=x",
        "text/plain",
    ] {
        let reply = street.send_photo(&id, me, kind, jpeg(40, 30)).await;
        assert_eq!(reply.status, StatusCode::BAD_REQUEST, "{kind}");
        assert_eq!(
            reply.body["error"],
            "send the photo as the body, with its image type as the Content-Type"
        );
    }
    // A few bytes that claim to be a picture 60,000 pixels square: refused from its header, never decoded.
    let vast: Vec<u8> = [
        b"GIF89a".as_slice(),
        &60_000u16.to_le_bytes(),
        &60_000u16.to_le_bytes(),
        &[0x80, 0, 0, 0xFF, 0xFF, 0xFF, 0, 0, 0],
        &[0x21, 0xF9, 4, 1, 0, 0, 0, 0],
        &[0x2C, 0, 0, 0, 0, 1, 0, 1, 0, 0],
        &[2, 2, 0x44, 1, 0, 0x3B],
    ]
    .concat();
    let vast = street.send_photo(&id, me, "image/gif", vast).await;
    assert_eq!(vast.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        vast.body["error"],
        "that photo is too large: at most 50 megapixels"
    );
    // More than the server reads.
    let huge = street
        .send_photo(&id, me, "image/jpeg", vec![0xFF; (20 << 20) + 1])
        .await;
    assert_eq!(huge.status, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(huge.body["code"], "payload_too_large");

    let path = format!("/api/v1/recipes/{}", id.as_str().unwrap());
    let after = street.json("GET", &path, &street.home, None).await;
    assert_eq!(after["image"], Value::Null, "nothing was attached");
    let media: i64 = sqlx::query_scalar("SELECT count(*) FROM media")
        .fetch_one(&street.pool)
        .await
        .unwrap();
    assert_eq!(media, 0, "nothing was stored");
}

#[tokio::test]
async fn lines_are_read_without_saving_anything() {
    let street = test_app!(Street::new());
    let foods_before: i64 = sqlx::query_scalar("SELECT count(*) FROM foods")
        .fetch_one(&street.pool)
        .await
        .unwrap();
    let read = street
        .json(
            "POST",
            "/api/v1/recipes/lines",
            &street.home,
            Some(json!({"lines": ["3 cloves garlic", "", "2 onions, sliced", "1 tsp smoked paprika"]})),
        )
        .await;
    let lines = read["lines"].as_array().unwrap();
    assert_eq!(lines.len(), 3, "{read}");
    assert_eq!(lines[0]["food"]["name"], "Garlic");
    assert_eq!(
        (
            &lines[1]["amount"],
            &lines[1]["unit"],
            &lines[1]["name"],
            &lines[1]["note"]
        ),
        (
            &json!(2.0),
            &json!("unit"),
            &json!("onions"),
            &json!("sliced")
        )
    );
    // A name no food has yet comes back without one; nothing is made for it until a recipe is saved.
    assert_eq!(lines[2]["name"], "smoked paprika");
    assert_eq!(lines[2]["food"], Value::Null);
    let foods_after: i64 = sqlx::query_scalar("SELECT count(*) FROM foods")
        .fetch_one(&street.pool)
        .await
        .unwrap();
    assert_eq!(foods_after, foods_before);

    let too_many = street
        .send(
            "POST",
            "/api/v1/recipes/lines",
            As::Cookie(&street.home),
            Some(json!({"lines": vec!["1 egg"; 201]})),
        )
        .await;
    assert_eq!(too_many.status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn food_search_never_offers_a_food_known_only_from_another_household_s_recipe() {
    let street = test_app!(Street::new());
    let names = |found: &Value| -> Vec<String> {
        found["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f["name"].as_str().unwrap().to_string())
            .collect()
    };
    // The demo recipes' foods are everyone's, and a typo still finds them.
    for cookie in [&street.home, &street.next_door] {
        let found = street
            .json("GET", "/api/v1/foods?q=parmesen", cookie, None)
            .await;
        assert_eq!(names(&found), ["Parmesan"]);
    }

    street
        .write(
            &street.next_door,
            json!({
                "title": "Aunt May's Stew",
                "servings": 4,
                "ingredients": [{"text": "2 TBSP Aunt May's secret spice"}],
                "steps": [],
            }),
        )
        .await;
    let theirs = street
        .json("GET", "/api/v1/foods?q=secret", &street.next_door, None)
        .await;
    assert_eq!(names(&theirs), ["Aunt May's secret spice"]);
    let ours = street
        .json("GET", "/api/v1/foods?q=secret", &street.home, None)
        .await;
    assert_eq!(names(&ours), Vec::<String>::new());

    for query in ["", "?q=", "?q=%20", &format!("?q={}", "a".repeat(101))] {
        let reply = street
            .send(
                "GET",
                &format!("/api/v1/foods{query}"),
                As::Cookie(&street.home),
                None,
            )
            .await;
        assert_eq!(reply.status, StatusCode::BAD_REQUEST, "{query:?}");
    }
}
