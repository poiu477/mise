//! Kitchen devices through the router on the demo recipes: an admin issues a kitchen token, its one-time link sets
//! the device's cookie once, the device reads the household's plan, dishes, versions and photos and is refused every
//! other route, `/me` tells it nothing personal, and revoking the token ends it at once.

#[macro_use]
mod common;

use axum::http::{StatusCode, header};
use common::{As, OWNER, Reply, TestApp};
use image::{ImageFormat, Rgb, RgbImage};
use mise_ingest::media::{MediaOptions, store_and_link};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

/// The dish the plan holds and the photo belongs to.
const DISH: &str = "black-bean-tacos";

/// A made-up id, for routes whose own ids the test doesn't hold.
const SOME_ID: &str = "0190f4a2-0000-7000-8000-000000000000";

/// What a kitchen session may call: method and documented path. Everything else documented must be refused.
const ALLOWED: &[(&str, &str)] = &[
    ("GET", "/api/v1/me"),
    ("GET", "/api/v1/plan"),
    ("GET", "/api/v1/dishes"),
    ("GET", "/api/v1/dishes/{slug}"),
    ("GET", "/api/v1/variants/{id}"),
    ("GET", "/api/v1/media/{id}/{size}"),
];

/// Documented routes anyone may call, signed in or not; a kitchen device gets from them what a stranger gets.
const PUBLIC: &[&str] = &[
    "/api/v1/health",
    "/api/v1/openapi.json",
    "/api/v1/auth/config",
    "/api/v1/auth/setup",
    "/api/v1/auth/login",
    "/api/v1/auth/kitchen",
    "/api/auth/oidc/start",
    "/api/auth/oidc/callback",
];

/// The demo install: the owner signed in (an admin, with an email and a named household), this week's plan holding
/// the tacos, the tacos' photo stored, and a kitchen device linked.
struct Kitchen {
    app: TestApp,
    owner: String,
    /// The device's cookie.
    device: String,
    /// The kitchen token's id.
    token_id: String,
}

impl Kitchen {
    /// None when there is no database (outside CI).
    async fn new() -> Option<Kitchen> {
        let app = TestApp::with_demo().await?;
        let setup = app
            .send(
                "POST",
                "/api/v1/auth/setup",
                As::Nobody,
                Some(json!({
                    "username": OWNER,
                    "password": common::OWNER_PASSWORD,
                    "display_name": "Paula Cook",
                    "household": "The Cook household",
                })),
            )
            .await;
        assert_eq!(setup.status, StatusCode::OK, "setup: {}", setup.body);
        let owner = setup.session().unwrap();
        sqlx::query("UPDATE users SET email = 'paula@cook.test'")
            .execute(&app.pool)
            .await
            .unwrap();
        store_photo(&app).await;
        let dish = app
            .json("GET", &format!("/api/v1/dishes/{DISH}"), &owner, None)
            .await;
        app.json(
            "POST",
            "/api/v1/plan/entries",
            &owner,
            Some(json!({"variant_id": dish["default_variant"]})),
        )
        .await;
        let mut k = Kitchen {
            app,
            owner,
            device: String::new(),
            token_id: String::new(),
        };
        let (link, id) = k.issue("Kitchen Echo").await;
        k.device = k.open(&link).await.expect("a new link works");
        k.token_id = id;
        Some(k)
    }

    /// Issue a kitchen token as the owner: its link (a path, with no PUBLIC_BASE_URL set) and its id.
    async fn issue(&self, name: &str) -> (String, String) {
        let made = self
            .app
            .json(
                "POST",
                "/api/v1/kitchen-tokens",
                &self.owner,
                Some(json!({"name": name})),
            )
            .await;
        (
            made["link"].as_str().unwrap().to_string(),
            made["info"]["id"].as_str().unwrap().to_string(),
        )
    }

    /// Open a link as a fresh browser: the kitchen cookie it sets, or None when it sends the browser to sign in.
    async fn open(&self, link: &str) -> Option<String> {
        let reply = self.app.send("GET", link, As::Nobody, None).await;
        assert_eq!(reply.status, StatusCode::SEE_OTHER, "{link}");
        assert_eq!(reply.header(header::CACHE_CONTROL), Some("no-store"));
        assert_eq!(reply.header(header::REFERRER_POLICY), Some("no-referrer"));
        let to = reply.header(header::LOCATION).unwrap();
        match reply.cookie("mise_kitchen") {
            Some(c) => {
                assert_eq!(to, "/");
                Some(c.split(';').next().unwrap().to_string())
            }
            None => {
                assert!(to.starts_with("/login?error="), "{to}");
                None
            }
        }
    }

    /// GET as the device.
    async fn get(&self, path: &str) -> Reply {
        self.app
            .send("GET", path, As::Kitchen(&self.device), None)
            .await
    }
}

/// Draw a photo and give it to the tacos, as `mise-ingest`'s media pass would.
async fn store_photo(app: &TestApp) {
    let img = RgbImage::from_fn(64, 48, |x, y| Rgb([(x * 4) as u8, (y * 5) as u8, 90]));
    let mut png = std::io::Cursor::new(Vec::new());
    img.write_to(&mut png, ImageFormat::Png).unwrap();
    let archive = app.media_root.join("archive");
    std::fs::create_dir_all(&archive).unwrap();
    std::fs::write(archive.join("tacos.png"), png.into_inner()).unwrap();
    sqlx::query(
        "UPDATE variants v SET meta = v.meta || $2 FROM dishes d WHERE d.slug = $1 AND v.id = d.default_variant_id",
    )
    .bind(DISH)
    .bind(json!({"media": {"hero": "tacos.png"}}))
    .execute(&app.pool)
    .await
    .unwrap();
    let options = MediaOptions {
        archive_root: Some(archive),
        media_root: app.media_root.clone(),
        limit: None,
        jobs: 1,
    };
    store_and_link(&app.pool, &options).await.unwrap();
}

fn sha256(s: &str) -> String {
    hex::encode(Sha256::digest(s.as_bytes()))
}

fn assert_refused(reply: &Reply, what: &str) {
    assert_eq!(
        reply.status,
        StatusCode::FORBIDDEN,
        "{what}: {}",
        reply.body
    );
    assert_eq!(reply.body["code"], "forbidden", "{what}");
}

#[tokio::test]
async fn a_kitchen_link_works_once_and_sets_a_long_lived_cookie() {
    let k = test_app!(Kitchen::new());
    let app = &k.app;
    let (link, id) = k.issue("Pantry tablet").await;
    let code = link
        .strip_prefix("/api/v1/auth/kitchen?code=")
        .expect("a link on this install");
    assert_eq!(code.len(), 43, "a 256-bit code");

    let reply = app.send("GET", &link, As::Nobody, None).await;
    let cookie = reply
        .cookie("mise_kitchen")
        .expect("the link sets a cookie");
    for attribute in ["HttpOnly", "SameSite=Lax", "Path=/", "Max-Age=34560000"] {
        assert!(cookie.contains(attribute), "{attribute} in {cookie}");
    }
    let device = cookie.split(';').next().unwrap();
    assert_eq!(device.len(), 43);
    assert_ne!(device, code, "the cookie isn't the link's code");
    assert_eq!(k.open(&link).await, None, "a link works once");
    assert_eq!(
        app.send("GET", "/api/v1/plan", As::Kitchen(device), None)
            .await
            .status,
        StatusCode::OK
    );

    // Only hashes are stored, and the link's is gone once used.
    let (link_hash, token_hash): (Option<String>, Option<String>) =
        sqlx::query_as("SELECT link_hash, token_hash FROM kitchen_tokens WHERE id = $1::uuid")
            .bind(&id)
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert_eq!(link_hash, None);
    assert_eq!(token_hash, Some(sha256(device)));

    let list = app
        .json("GET", "/api/v1/kitchen-tokens", &k.owner, None)
        .await;
    let text = list.to_string();
    assert!(!text.contains(code) && !text.contains(device), "{text}");
    let listed = list
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == id.as_str())
        .unwrap();
    assert_eq!(listed["name"], "Pantry tablet");
    assert!(listed["linked_at"].is_string());
    assert!(listed["link_expires_at"].is_null());
}

#[tokio::test]
async fn an_expired_or_made_up_link_sets_no_cookie() {
    let k = test_app!(Kitchen::new());
    let (link, id) = k.issue("Late").await;
    sqlx::query(
        "UPDATE kitchen_tokens SET link_expires_at = now() - interval '1 second' WHERE id = $1::uuid",
    )
    .bind(&id)
    .execute(&k.app.pool)
    .await
    .unwrap();
    assert_eq!(k.open(&link).await, None, "expired");
    let listed = k
        .app
        .json("GET", "/api/v1/kitchen-tokens", &k.owner, None)
        .await;
    let late = listed
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == id.as_str())
        .unwrap();
    assert!(late["linked_at"].is_null(), "an expired link links nothing");

    for link in [
        "/api/v1/auth/kitchen",
        "/api/v1/auth/kitchen?code=",
        "/api/v1/auth/kitchen?code=made-up",
    ] {
        assert_eq!(k.open(link).await, None, "{link}");
    }
}

#[tokio::test]
async fn a_kitchen_session_reads_the_plan_and_recipes() {
    let k = test_app!(Kitchen::new());
    let app = &k.app;
    let plan = app.json("GET", "/api/v1/plan", &k.owner, None).await;
    let version = plan["entries"][0]["variant_id"].as_str().unwrap();
    let photo = plan["entries"][0]["image"]["id"]
        .as_str()
        .expect("the tacos have a photo");
    for path in [
        "/api/v1/plan".to_string(),
        "/api/v1/dishes?q=tacos".to_string(),
        format!("/api/v1/dishes/{DISH}"),
        format!("/api/v1/variants/{version}?servings=3"),
    ] {
        let reply = k.get(&path).await;
        assert_eq!(reply.status, StatusCode::OK, "{path}: {}", reply.body);
        let owners = app.json("GET", &path, &k.owner, None).await;
        assert_eq!(reply.body, owners, "{path} reads as the owner's");
    }
    let image = k.get(&format!("/api/v1/media/{photo}/original")).await;
    assert_eq!(image.status, StatusCode::OK);
    assert_eq!(image.header(header::CONTENT_TYPE), Some("image/png"));
    // HEAD, and the old addresses, read the same.
    let head = app
        .send("HEAD", "/api/v1/plan", As::Kitchen(&k.device), None)
        .await;
    assert_eq!(head.status, StatusCode::OK);
    assert_eq!(k.get("/api/plan").await.status, StatusCode::OK);
}

#[tokio::test]
async fn a_kitchen_session_is_refused_every_other_route() {
    let k = test_app!(Kitchen::new());
    let app = &k.app;
    let doc = app
        .json("GET", "/api/v1/openapi.json", &k.owner, None)
        .await;
    let plan = app.json("GET", "/api/v1/plan", &k.owner, None).await;
    let entry = plan["entries"][0]["id"].as_str().unwrap().to_string();
    let version = plan["entries"][0]["variant_id"].as_str().unwrap();
    let photo = plan["entries"][0]["image"]["id"].as_str().unwrap();
    let me = app.json("GET", "/api/v1/me", &k.owner, None).await;
    let owner_id = me["id"].as_str().unwrap();
    let api_token = app
        .json(
            "POST",
            "/api/v1/me/tokens",
            &k.owner,
            Some(json!({"name": "script", "scope": "read_write"})),
        )
        .await["info"]["id"]
        .as_str()
        .unwrap()
        .to_string();

    // Real ids where the route has them, so a route that let the device through would change something visible.
    let fill = |path: &str| -> String {
        let id = if path.starts_with("/api/v1/variants/") {
            version.to_string()
        } else if path.starts_with("/api/v1/media/") {
            photo.to_string()
        } else if path.starts_with("/api/v1/plan/entries/") {
            entry.clone()
        } else if path.starts_with("/api/v1/kitchen-tokens/") {
            k.token_id.clone()
        } else if path.starts_with("/api/v1/me/tokens/") {
            api_token.clone()
        } else if path.starts_with("/api/v1/users/") {
            owner_id.to_string()
        } else {
            SOME_ID.to_string()
        };
        path.replace("{slug}", DISH)
            .replace("{id}", &id)
            .replace("{food_id}", SOME_ID)
            .replace("{size}", "original")
    };

    let mut routes: Vec<(String, String)> = vec![("GET".into(), "/api/v1/openapi.json".into())];
    for (path, ops) in doc["paths"].as_object().expect("documented paths") {
        for method in ops.as_object().unwrap().keys() {
            routes.push((method.to_uppercase(), path.clone()));
        }
    }
    assert!(routes.len() > 30, "{} routes documented", routes.len());

    let (mut allowed, mut refused) = (0, 0);
    for (method, path) in &routes {
        let url = fill(path);
        let body = matches!(method.as_str(), "POST" | "PUT" | "PATCH").then(|| json!({}));
        let reply = app
            .send(method, &url, As::Kitchen(&k.device), body.clone())
            .await;
        if PUBLIC.contains(&path.as_str()) {
            let stranger = app.send(method, &url, As::Nobody, body).await;
            assert_eq!(reply.status, stranger.status, "{method} {path}");
            assert_eq!(reply.body, stranger.body, "{method} {path}");
        } else if ALLOWED.contains(&(method.as_str(), path.as_str())) {
            assert_eq!(
                reply.status,
                StatusCode::OK,
                "{method} {path}: {}",
                reply.body
            );
            allowed += 1;
        } else {
            assert_refused(&reply, &format!("{method} {path}"));
            refused += 1;
        }
        // An allowed path refuses every way of changing it.
        if ALLOWED.contains(&(method.as_str(), path.as_str())) {
            for other in ["POST", "PUT", "PATCH", "DELETE"] {
                let reply = app
                    .send(other, &url, As::Kitchen(&k.device), Some(json!({})))
                    .await;
                assert_refused(&reply, &format!("{other} {path}"));
            }
        }
    }
    assert_eq!(allowed, ALLOWED.len(), "every allowed route is documented");
    assert!(refused > 25, "{refused} routes refused");

    // Undocumented addresses, and the old ones, are refused too.
    for path in [
        "/api/v1/nope",
        "/api",
        "/api/v1",
        "/api/shopping",
        "/api/me/tokens",
        "/api/v1/plan/",
        "/api/v1/dishes//x",
    ] {
        assert_refused(&k.get(path).await, path);
    }
    // Nothing the device tried changed anything, and it still works.
    assert_eq!(app.json("GET", "/api/v1/plan", &k.owner, None).await, plan);
    assert_eq!(app.json("GET", "/api/v1/me", &k.owner, None).await, me);
    let tokens = app.json("GET", "/api/v1/me/tokens", &k.owner, None).await;
    assert_eq!(tokens.as_array().unwrap().len(), 1);
    assert_eq!(k.get("/api/v1/plan").await.status, StatusCode::OK);
}

#[tokio::test]
async fn me_reveals_nothing_personal_to_a_kitchen_device() {
    let k = test_app!(Kitchen::new());
    let owner = k.app.json("GET", "/api/v1/me", &k.owner, None).await;
    let reply = k.get("/api/v1/me").await;
    assert_eq!(reply.status, StatusCode::OK);
    let me = reply.body;
    assert_eq!(
        me,
        json!({
            "id": k.token_id,
            "username": "",
            "display_name": "Kitchen Echo",
            "email": null,
            "is_admin": false,
            "household": {"id": owner["household"]["id"], "name": ""},
            "has_password": false,
            "kitchen": true,
        })
    );
    let text = me.to_string();
    for personal in [
        OWNER,
        "Paula",
        "Cook",
        "paula@cook.test",
        owner["id"].as_str().unwrap(),
    ] {
        assert!(!text.contains(personal), "{personal} in {text}");
    }
    assert_eq!(owner["kitchen"], false);
}

#[tokio::test]
async fn a_revoked_kitchen_token_stops_working_at_once() {
    let k = test_app!(Kitchen::new());
    let app = &k.app;
    assert_eq!(k.get("/api/v1/plan").await.status, StatusCode::OK);
    let reply = app
        .send(
            "DELETE",
            &format!("/api/v1/kitchen-tokens/{}", k.token_id),
            As::Cookie(&k.owner),
            None,
        )
        .await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT);
    for path in ["/api/v1/plan", "/api/v1/me", "/api/v1/dishes"] {
        let reply = k.get(path).await;
        assert_eq!(reply.status, StatusCode::UNAUTHORIZED, "{path}");
        assert_eq!(reply.body["code"], "unauthorized");
    }
    let list = app
        .json("GET", "/api/v1/kitchen-tokens", &k.owner, None)
        .await;
    assert_eq!(list, json!([]));
    // A link revoked before it is opened doesn't work either.
    let (link, id) = k.issue("Never opened").await;
    app.send(
        "DELETE",
        &format!("/api/v1/kitchen-tokens/{id}"),
        As::Cookie(&k.owner),
        None,
    )
    .await;
    assert_eq!(k.open(&link).await, None);
}

#[tokio::test]
async fn only_a_households_admins_manage_its_kitchen_tokens() {
    let k = test_app!(Kitchen::new());
    let app = &k.app;
    // Someone in the same household who isn't an admin, and an admin of another household.
    let me = app.json("GET", "/api/v1/me", &k.owner, None).await;
    for (username, admin, household) in [
        ("sam", false, Some(me["household"]["id"].clone())),
        ("neighbour", true, None),
    ] {
        let mut user =
            json!({"username": username, "password": "another long password", "is_admin": admin});
        if let Some(h) = household {
            user["household_id"] = h;
        }
        app.json("POST", "/api/v1/users", &k.owner, Some(user))
            .await;
    }
    let sam = app.sign_in("sam", "another long password").await;
    let neighbour = app.sign_in("neighbour", "another long password").await;
    let token = format!("/api/v1/kitchen-tokens/{}", k.token_id);

    for (method, path, body) in [
        ("GET", "/api/v1/kitchen-tokens", None),
        (
            "POST",
            "/api/v1/kitchen-tokens",
            Some(json!({"name": "Mine"})),
        ),
        ("DELETE", token.as_str(), None),
    ] {
        let reply = app.send(method, path, As::Cookie(&sam), body).await;
        assert_refused(&reply, &format!("{method} {path} as a non-admin"));
    }
    // Another household's admin sees none of this household's tokens and can't revoke them.
    assert_eq!(
        app.json("GET", "/api/v1/kitchen-tokens", &neighbour, None)
            .await,
        json!([])
    );
    assert_eq!(
        app.send("DELETE", &token, As::Cookie(&neighbour), None)
            .await
            .status,
        StatusCode::NOT_FOUND
    );
    // Their own kitchen device sees their household's (empty) plan, not this one.
    let made = app
        .json(
            "POST",
            "/api/v1/kitchen-tokens",
            &neighbour,
            Some(json!({"name": "Next door"})),
        )
        .await;
    let theirs = k.open(made["link"].as_str().unwrap()).await.unwrap();
    let plan = app
        .send("GET", "/api/v1/plan", As::Kitchen(&theirs), None)
        .await;
    assert_eq!(plan.body["entries"], json!([]));

    // Issuing needs a browser session (one token can't mint another) and a name.
    let api = app
        .json(
            "POST",
            "/api/v1/me/tokens",
            &k.owner,
            Some(json!({"name": "script", "scope": "read_write"})),
        )
        .await;
    let reply = app
        .send(
            "POST",
            "/api/v1/kitchen-tokens",
            As::Bearer(api["token"].as_str().unwrap()),
            Some(json!({"name": "From a script"})),
        )
        .await;
    assert_refused(&reply, "issued with an API token");
    for name in ["", "   ", "x".repeat(101).as_str()] {
        let reply = app
            .send(
                "POST",
                "/api/v1/kitchen-tokens",
                As::Cookie(&k.owner),
                Some(json!({"name": name})),
            )
            .await;
        assert_eq!(reply.status, StatusCode::BAD_REQUEST, "{name:?}");
    }
}

#[tokio::test]
async fn the_kitchen_cookie_is_only_a_cookie_and_an_account_comes_first() {
    let k = test_app!(Kitchen::new());
    let app = &k.app;
    // It isn't an API token.
    for bearer in [k.device.clone(), format!("mise_{}", k.device)] {
        let reply = app
            .send("GET", "/api/v1/plan", As::Bearer(&bearer), None)
            .await;
        assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
    }
    // A browser signed in to an account acts as the account while it holds a kitchen cookie too.
    let both = As::Both {
        session: &k.owner,
        kitchen: &k.device,
    };
    let me = app.send("GET", "/api/v1/me", both, None).await;
    assert_eq!(me.body["username"], OWNER);
    assert_eq!(
        app.send("GET", "/api/v1/shopping", both, None).await.status,
        StatusCode::OK
    );
    // Once the session has ended, the kitchen cookie is used.
    app.send("POST", "/api/v1/auth/logout", As::Cookie(&k.owner), None)
        .await;
    let me = app.send("GET", "/api/v1/me", both, None).await;
    assert_eq!(me.body["kitchen"], true);
}

#[tokio::test]
async fn using_the_device_renews_its_cookie_at_most_hourly() {
    let k = test_app!(Kitchen::new());
    let app = &k.app;
    let first = k.get("/api/v1/plan").await;
    let renewed = first.cookie("mise_kitchen").expect("first use renews");
    assert!(renewed.starts_with(&format!("{};", k.device)), "{renewed}");
    assert!(renewed.contains("Max-Age=34560000"));
    assert_eq!(k.get("/api/v1/plan").await.cookie("mise_kitchen"), None);

    sqlx::query("UPDATE kitchen_tokens SET last_used_at = now() - interval '2 hours'")
        .execute(&app.pool)
        .await
        .unwrap();
    assert!(
        k.get("/api/v1/dishes")
            .await
            .cookie("mise_kitchen")
            .is_some()
    );
    // A refused request doesn't renew.
    sqlx::query("UPDATE kitchen_tokens SET last_used_at = now() - interval '2 hours'")
        .execute(&app.pool)
        .await
        .unwrap();
    let refused = k.get("/api/v1/shopping").await;
    assert_refused(&refused, "shopping");
    assert_eq!(refused.cookie("mise_kitchen"), None);
}

#[test]
fn the_api_documents_kitchen_tokens() {
    let doc = serde_json::to_value(<mise_api::ApiDoc as utoipa::OpenApi>::openapi()).unwrap();
    for path in [
        "/api/v1/kitchen-tokens",
        "/api/v1/kitchen-tokens/{id}",
        "/api/v1/auth/kitchen",
    ] {
        assert!(doc["paths"][path].is_object(), "{path}");
    }
    let me: &Value = &doc["components"]["schemas"]["Me"]["properties"]["kitchen"];
    assert_eq!(me["type"], "boolean");
}

/// A version `household` owns, as an import would add it (published at 2 servings with two lines), grouped into a
/// dish of its own by `mise-ingest`. Returns its id and its dish's slug.
async fn own_recipe(app: &TestApp, household: &str, title: &str) -> (String, String) {
    let id = uuid::Uuid::now_v7();
    sqlx::query(
        "INSERT INTO variants (id, origin, title, has_steps, content_hash, published_at, household_id)
         VALUES ($1, 'user', $2, false, $1::text, '2026-11-10T12:00:00Z', $3::uuid)",
    )
    .bind(id)
    .bind(title)
    .bind(household)
    .execute(&app.pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO variant_yields (variant_id, servings) VALUES ($1, 2)")
        .bind(id)
        .execute(&app.pool)
        .await
        .unwrap();
    for (i, name) in ["Red Lentils", "Coconut Milk"].iter().enumerate() {
        let line = uuid::Uuid::now_v7();
        sqlx::query("INSERT INTO ingredient_lines (id, variant_id, line_key, position, raw_name, shipped) VALUES ($1, $2, $3, $4, $5, true)")
            .bind(line)
            .bind(id)
            .bind(format!("l{i}"))
            .bind(i as i32)
            .bind(name)
            .execute(&app.pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO line_amounts (line_id, servings, amount, unit) VALUES ($1, 2, 1, 'cup')",
        )
        .bind(line)
        .execute(&app.pool)
        .await
        .unwrap();
    }
    mise_ingest::foods::match_foods(&app.pool).await.unwrap();
    mise_ingest::dishes::group_dishes(&app.pool).await.unwrap();
    mise_ingest::options::derive_options(&app.pool)
        .await
        .unwrap();
    let slug: String = sqlx::query_scalar(
        "SELECT d.slug FROM dishes d JOIN variants v ON v.dish_id = d.id WHERE v.id = $1",
    )
    .bind(id)
    .fetch_one(&app.pool)
    .await
    .unwrap();
    (id.to_string(), slug)
}

#[tokio::test]
async fn a_kitchen_device_sees_only_what_its_household_can_see() {
    let k = test_app!(Kitchen::new());
    let app = &k.app;
    let home = app.json("GET", "/api/v1/me", &k.owner, None).await["household"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let next_door = app
        .json(
            "POST",
            "/api/v1/households",
            &k.owner,
            Some(json!({"name": "Next door"})),
        )
        .await["id"]
        .as_str()
        .unwrap()
        .to_string();
    let (ours, our_dish) = own_recipe(app, &home, "Our Lentil Soup").await;
    let (theirs, their_dish) = own_recipe(app, &next_door, "Their Lentil Dal").await;
    // Next door's version on this household's plan, as no API call can put it: the plan shows only what it can see.
    sqlx::query(
        "INSERT INTO plan_entries (id, household_id, week, variant_id, servings)
         SELECT $1, $2::uuid, week, $3::uuid, 2 FROM plan_entries LIMIT 1",
    )
    .bind(uuid::Uuid::now_v7())
    .bind(&home)
    .bind(&theirs)
    .execute(&app.pool)
    .await
    .unwrap();

    // What the household can see, the device reads as the owner does.
    for path in [
        "/api/v1/dishes?q=lentil".to_string(),
        "/api/v1/dishes".to_string(),
        format!("/api/v1/dishes/{our_dish}"),
        format!("/api/v1/variants/{ours}"),
        "/api/v1/plan".to_string(),
    ] {
        let reply = k.get(&path).await;
        assert_eq!(reply.status, StatusCode::OK, "{path}: {}", reply.body);
        assert_eq!(
            reply.body,
            app.json("GET", &path, &k.owner, None).await,
            "{path}"
        );
    }
    let found = k.get("/api/v1/dishes?q=lentil").await.body.to_string();
    assert!(found.contains("Our Lentil Soup"), "{found}");
    assert!(!found.contains("Their Lentil Dal"), "{found}");
    let plan = k.get("/api/v1/plan").await.body;
    assert_eq!(plan["entries"].as_array().unwrap().len(), 1, "{plan}");
    assert!(!plan.to_string().contains(&theirs), "{plan}");

    // Next door's recipe isn't there for it, as for a dish or version that doesn't exist.
    for path in [
        format!("/api/v1/dishes/{their_dish}"),
        format!("/api/v1/variants/{theirs}"),
    ] {
        let reply = k.get(&path).await;
        assert_eq!(
            reply.status,
            StatusCode::NOT_FOUND,
            "{path}: {}",
            reply.body
        );
    }
}
