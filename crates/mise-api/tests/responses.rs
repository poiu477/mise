//! Every route answers with the statuses the API document lists for it, and only those. Each route is called in
//! the ways that give each status it documents: signed out, for another account, with a read-only token, with a
//! body too large or not JSON, with a path or query that can't be read, with values its handler refuses, and with
//! the database gone. An answer the document doesn't list for that route fails the test at once; a documented
//! status no call produced fails it at the end.

#[macro_use]
mod common;

use std::collections::{BTreeMap, BTreeSet};

use axum::{
    body::Body,
    http::{Request, header},
};
use common::{As, Reply, TestApp};
use serde_json::{Value, json};

/// A made-up id, for what isn't there.
const SOME_ID: &str = "0190f4a2-0000-7000-8000-000000000000";

/// The dish the calls read, from the demo recipes.
const DISH: &str = "black-bean-tacos";

/// A far-off week, for plans and lists.
const WEEK: &str = "2027-01-04";

/// Documented statuses no call here can produce, with why.
const UNPRODUCED: &[(&str, &str, u16, &str)] = &[(
    "GET",
    "/api/auth/oidc/callback",
    500,
    "only when the database fails between the provider's answer and saving the session",
)];

/// The route `method path` documents `status` but no call here can produce it.
fn unproduced(method: &str, path: &str, status: u16) -> bool {
    UNPRODUCED
        .iter()
        .any(|&(m, p, s, _)| (m, p, s) == (method, path, status))
}

/// A route as the document names it: the method in capitals and the path as written, `{id}` and all.
type Route = (String, String);

/// What the document says about a route.
struct Documented {
    statuses: BTreeSet<u16>,
    /// `security: [{}]`: anyone may call it.
    public: bool,
    takes_body: bool,
    takes_query: bool,
}

/// The routes the document lists, read from the server's own `/api/v1/openapi.json`.
fn routes(doc: &Value) -> BTreeMap<Route, Documented> {
    let mut routes = BTreeMap::new();
    for (path, ops) in doc["paths"].as_object().expect("documented paths") {
        for (method, op) in ops.as_object().unwrap() {
            let statuses = op["responses"]
                .as_object()
                .unwrap()
                .keys()
                .map(|s| s.parse().unwrap())
                .collect();
            let takes_query = op["parameters"]
                .as_array()
                .is_some_and(|ps| ps.iter().any(|p| p["in"] == "query"));
            routes.insert(
                (method.to_uppercase(), path.clone()),
                Documented {
                    statuses,
                    public: op["security"] == json!([{}]),
                    takes_body: op.get("requestBody").is_some(),
                    takes_query,
                },
            );
        }
    }
    routes
}

/// `path` with its parameters filled in with values that read: the demo dish, a made-up id, a size that exists.
fn fill(path: &str) -> String {
    path.replace("{slug}", DISH)
        .replace("{id}", SOME_ID)
        .replace("{food_id}", SOME_ID)
        .replace("{size}", "400")
}

/// `path` with a parameter that can't be read: an id that isn't a UUID, a slug with a NUL in it.
fn unreadable(path: &str) -> Option<String> {
    if !path.contains('{') {
        return None;
    }
    Some(
        fill(
            &path
                .replace("{id}", "not-a-uuid")
                .replace("{food_id}", "not-a-uuid"),
        )
        .replace(DISH, "a%00b"),
    )
}

/// A request with a body of raw text sent as JSON, and optionally the account it's meant for.
fn raw(
    method: &str,
    path: &str,
    from: As<'_>,
    body: String,
    account: Option<&str>,
) -> Request<Body> {
    let mut b = TestApp::request(method, path, from);
    if let Some(a) = account {
        b = b.header("x-mise-account", a);
    }
    if !body.is_empty() {
        b = b.header(header::CONTENT_TYPE, "application/json");
    }
    b.body(Body::from(body)).unwrap()
}

/// Calls routes and keeps track of which documented statuses they gave.
struct Checker<'a> {
    app: &'a TestApp,
    routes: BTreeMap<Route, Documented>,
    seen: BTreeMap<Route, BTreeSet<u16>>,
}

impl Checker<'_> {
    /// Send `req` to the route `method path` and note its status. Fails at once if the route answers with a status
    /// its document doesn't list, or an error without the error body.
    async fn answer(&mut self, route: (&str, &str), req: Request<Body>, why: &str) -> Reply {
        let key = (route.0.to_string(), route.1.to_string());
        let documented = &self
            .routes
            .get(&key)
            .unwrap_or_else(|| panic!("{} {} isn't documented", route.0, route.1))
            .statuses;
        let reply = self.app.read(req).await;
        let got = reply.status.as_u16();
        assert!(
            documented.contains(&got),
            "{} {} answered {got} ({why}), which its document doesn't list (it lists {documented:?}): {}",
            route.0,
            route.1,
            reply.body
        );
        if got >= 400 {
            assert!(
                reply.body["code"].is_string() && reply.body["error"].is_string(),
                "{} {} {got} ({why}) has no error body: {}",
                route.0,
                route.1,
                reply.body
            );
        }
        self.seen.entry(key).or_default().insert(got);
        reply
    }

    /// Send `req` to the route `method path` and expect `status`, as `answer` checks it.
    async fn expect_request(
        &mut self,
        route: (&str, &str),
        req: Request<Body>,
        status: u16,
        why: &str,
    ) -> Reply {
        let reply = self.answer(route, req, why).await;
        assert_eq!(
            reply.status.as_u16(),
            status,
            "{} {} ({why}): {}",
            route.0,
            route.1,
            reply.body
        );
        reply
    }

    /// Send to the route `method path` at `url`, with a JSON body if given, and expect `status`.
    async fn expect(
        &mut self,
        route: (&str, &str),
        url: &str,
        from: As<'_>,
        body: Option<Value>,
        status: u16,
        why: &str,
    ) -> Reply {
        let body = body.map(|b| b.to_string()).unwrap_or_default();
        let req = raw(route.0, url, from, body, None);
        self.expect_request(route, req, status, why).await
    }

    /// Call the route at its own address with `from` and expect `status`; the shortest form, for routes without
    /// parameters.
    async fn call(
        &mut self,
        route: (&str, &str),
        from: As<'_>,
        body: Option<Value>,
        status: u16,
        why: &str,
    ) -> Reply {
        self.expect(route, route.1, from, body, status, why).await
    }
}

const CONFIG: (&str, &str) = ("GET", "/api/v1/auth/config");
const SETUP: (&str, &str) = ("POST", "/api/v1/auth/setup");
const LOGIN: (&str, &str) = ("POST", "/api/v1/auth/login");
const LOGOUT: (&str, &str) = ("POST", "/api/v1/auth/logout");
const ME: (&str, &str) = ("GET", "/api/v1/me");
const PASSWORD: (&str, &str) = ("POST", "/api/v1/me/password");
const TOKENS: (&str, &str) = ("GET", "/api/v1/me/tokens");
const NEW_TOKEN: (&str, &str) = ("POST", "/api/v1/me/tokens");
const REVOKE: (&str, &str) = ("DELETE", "/api/v1/me/tokens/{id}");
const USERS: (&str, &str) = ("GET", "/api/v1/users");
const NEW_USER: (&str, &str) = ("POST", "/api/v1/users");
const CHANGE_USER: (&str, &str) = ("PATCH", "/api/v1/users/{id}");
const HOUSEHOLDS: (&str, &str) = ("GET", "/api/v1/households");
const NEW_HOUSEHOLD: (&str, &str) = ("POST", "/api/v1/households");
const SEARCH: (&str, &str) = ("GET", "/api/v1/dishes");
const DISH_PAGE: (&str, &str) = ("GET", "/api/v1/dishes/{slug}");
const VERSION: (&str, &str) = ("GET", "/api/v1/variants/{id}");
const PHOTO: (&str, &str) = ("GET", "/api/v1/media/{id}/{size}");
const PLAN: (&str, &str) = ("GET", "/api/v1/plan");
const PLAN_ADD: (&str, &str) = ("POST", "/api/v1/plan/entries");
const PLAN_CHANGE: (&str, &str) = ("PATCH", "/api/v1/plan/entries/{id}");
const PLAN_REMOVE: (&str, &str) = ("DELETE", "/api/v1/plan/entries/{id}");
const LIST: (&str, &str) = ("GET", "/api/v1/shopping");
const MARK: (&str, &str) = ("PUT", "/api/v1/shopping/marks");
const EXTRA_ADD: (&str, &str) = ("POST", "/api/v1/shopping/extras");
const EXTRA_CHANGE: (&str, &str) = ("PATCH", "/api/v1/shopping/extras/{id}");
const EXTRA_REMOVE: (&str, &str) = ("DELETE", "/api/v1/shopping/extras/{id}");
const STAPLE_ADD: (&str, &str) = ("POST", "/api/v1/pantry-staples");
const STAPLE_REMOVE: (&str, &str) = ("DELETE", "/api/v1/pantry-staples/{food_id}");
const SETTINGS: (&str, &str) = ("GET", "/api/v1/shopping/settings");
const SETTINGS_CHANGE: (&str, &str) = ("PUT", "/api/v1/shopping/settings");
const FOOD_WEIGHTS: (&str, &str) = ("PUT", "/api/v1/shopping/food-weights");
const HEALTH: (&str, &str) = ("GET", "/api/v1/health");
const SSO_START: (&str, &str) = ("GET", "/api/auth/oidc/start");
const SSO_CALLBACK: (&str, &str) = ("GET", "/api/auth/oidc/callback");

/// A body that sets up or signs in successfully, for the routes that read it before the database.
fn sign_in_body(route: (&str, &str)) -> Option<Value> {
    (route == SETUP || route == LOGIN)
        .then(|| json!({"username": common::OWNER, "password": common::OWNER_PASSWORD}))
}

#[tokio::test]
async fn every_route_answers_only_with_the_statuses_it_documents() {
    let app = test_app!(TestApp::with_demo());
    let doc = app
        .read(raw(
            "GET",
            "/api/v1/openapi.json",
            As::Nobody,
            String::new(),
            None,
        ))
        .await
        .body;
    let mut c = Checker {
        app: &app,
        routes: routes(&doc),
        seen: BTreeMap::new(),
    };
    assert!(c.routes.len() > 30, "{} routes documented", c.routes.len());

    // First run: anyone may ask how to sign in and set the install up, once.
    c.call(HEALTH, As::Nobody, None, 200, "the health check")
        .await;
    c.call(CONFIG, As::Nobody, None, 200, "sign-in settings")
        .await;
    let bad = json!({"username": "two words", "password": common::OWNER_PASSWORD});
    c.call(SETUP, As::Nobody, Some(bad), 400, "a username with a space")
        .await;
    let owner = json!({"username": common::OWNER, "password": common::OWNER_PASSWORD});
    let cookie = c
        .call(SETUP, As::Nobody, Some(owner.clone()), 200, "setup")
        .await
        .session()
        .unwrap();
    c.call(SETUP, As::Nobody, Some(owner), 409, "a second setup")
        .await;
    let me = As::Cookie(&cookie);

    // Tokens: only a browser makes them.
    let mut tokens = Vec::new();
    for scope in ["read_write", "read", "read_write"] {
        let made = c
            .call(
                NEW_TOKEN,
                me,
                Some(json!({"name": scope, "scope": scope})),
                200,
                "a token",
            )
            .await;
        tokens.push((
            made.body["token"].as_str().unwrap().to_string(),
            made.body["info"]["id"].clone(),
        ));
    }
    let (writer, reader) = (tokens[0].0.clone(), tokens[1].0.clone());
    let wrong = json!({"name": "x", "scope": "everything"});
    c.call(NEW_TOKEN, me, Some(wrong), 400, "an unknown scope")
        .await;
    let another = json!({"name": "x", "scope": "read"});
    c.call(
        NEW_TOKEN,
        As::Bearer(&writer),
        Some(another),
        403,
        "a token making a token",
    )
    .await;
    c.call(TOKENS, me, None, 200, "the tokens").await;
    let spare = format!("/api/v1/me/tokens/{}", tokens[2].1.as_str().unwrap());
    c.expect(REVOKE, &spare, me, None, 204, "revoking a token")
        .await;
    c.expect(REVOKE, &spare, me, None, 404, "revoking it again")
        .await;

    // Accounts and households: admins only.
    let cook =
        json!({"username": "cook", "password": "a cook's password", "household_name": "Next door"});
    c.call(NEW_USER, me, Some(cook.clone()), 200, "a new account")
        .await;
    c.call(NEW_USER, me, Some(cook), 409, "the same username again")
        .await;
    let homeless = json!({"username": "lost", "household_id": SOME_ID});
    c.call(
        NEW_USER,
        me,
        Some(homeless),
        400,
        "a household that doesn't exist",
    )
    .await;
    let retired = json!({"username": "retired", "password": "a retired password"});
    let retired = c
        .call(NEW_USER, me, Some(retired), 200, "another account")
        .await
        .body["id"]
        .clone();
    let retired = format!("/api/v1/users/{}", retired.as_str().unwrap());
    c.expect(
        CHANGE_USER,
        &retired,
        me,
        Some(json!({"disabled": true})),
        200,
        "turning an account off",
    )
    .await;
    let owner_id = c
        .call(ME, me, None, 200, "the signed-in account")
        .await
        .body["id"]
        .clone();
    let own = format!("/api/v1/users/{}", owner_id.as_str().unwrap());
    c.expect(
        CHANGE_USER,
        &own,
        me,
        Some(json!({"disabled": true})),
        400,
        "turning yourself off",
    )
    .await;
    c.expect(
        CHANGE_USER,
        &fill(CHANGE_USER.1),
        me,
        Some(json!({})),
        404,
        "an account that isn't there",
    )
    .await;
    c.call(USERS, me, None, 200, "every account").await;
    c.call(HOUSEHOLDS, me, None, 200, "every household").await;
    c.call(
        NEW_HOUSEHOLD,
        me,
        Some(json!({"name": "Upstairs"})),
        200,
        "a household",
    )
    .await;
    c.call(
        NEW_HOUSEHOLD,
        me,
        Some(json!({"name": " "})),
        400,
        "a household without a name",
    )
    .await;
    let cook = app.sign_in("cook", "a cook's password").await;
    let cook = As::Cookie(&cook);
    c.call(USERS, cook, None, 403, "not an admin").await;
    c.call(HOUSEHOLDS, cook, None, 403, "not an admin").await;
    c.call(
        NEW_USER,
        cook,
        Some(json!({"username": "x"})),
        403,
        "not an admin",
    )
    .await;
    c.expect(
        CHANGE_USER,
        &own,
        cook,
        Some(json!({})),
        403,
        "not an admin",
    )
    .await;
    c.call(
        NEW_HOUSEHOLD,
        cook,
        Some(json!({"name": "Mine"})),
        403,
        "not an admin",
    )
    .await;
    c.call(LOGOUT, cook, None, 204, "signing out").await;

    // Signing in.
    let owner = json!({"username": common::OWNER, "password": common::OWNER_PASSWORD});
    c.call(LOGIN, As::Nobody, Some(owner), 200, "the right password")
        .await;
    let guess = json!({"username": "nobody", "password": "a guess at it"});
    for _ in 0..5 {
        c.call(
            LOGIN,
            As::Nobody,
            Some(guess.clone()),
            401,
            "a wrong password",
        )
        .await;
    }
    c.call(
        LOGIN,
        As::Nobody,
        Some(guess),
        429,
        "a sixth wrong password",
    )
    .await;
    let off = json!({"username": "retired", "password": "a retired password"});
    c.call(LOGIN, As::Nobody, Some(off), 403, "a turned-off account")
        .await;

    // Single sign-on isn't set up here, so both of its addresses send people back to the sign-in page.
    c.call(
        SSO_START,
        As::Nobody,
        None,
        303,
        "single sign-on turned off",
    )
    .await;
    c.call(
        SSO_CALLBACK,
        As::Nobody,
        None,
        303,
        "single sign-on turned off",
    )
    .await;
    let nul = "/api/auth/oidc/callback?code=a%00b";
    c.expect(
        SSO_CALLBACK,
        nul,
        As::Nobody,
        None,
        400,
        "a NUL in the reply",
    )
    .await;

    // Dishes, versions and photos.
    c.call(SEARCH, me, None, 200, "search").await;
    c.expect(
        SEARCH,
        "/api/v1/dishes?limit=lots",
        me,
        None,
        400,
        "a page size that isn't a number",
    )
    .await;
    let dish = c
        .expect(DISH_PAGE, &fill(DISH_PAGE.1), me, None, 200, "a dish")
        .await
        .body;
    c.expect(
        DISH_PAGE,
        "/api/v1/dishes/no-such-dish",
        me,
        None,
        404,
        "a dish that isn't there",
    )
    .await;
    let version = format!(
        "/api/v1/variants/{}",
        dish["default_variant"].as_str().unwrap()
    );
    c.expect(VERSION, &version, me, None, 200, "a version")
        .await;
    c.expect(
        VERSION,
        &format!("{version}?servings=0"),
        me,
        None,
        400,
        "no servings",
    )
    .await;
    c.expect(
        VERSION,
        &fill(VERSION.1),
        me,
        None,
        404,
        "a version that isn't there",
    )
    .await;
    let photo = uuid::Uuid::now_v7();
    std::fs::write(app.media_root.join("photo.png"), b"a photo").unwrap();
    sqlx::query("INSERT INTO media (id, sha256, mime, bytes, path) VALUES ($1, 'responses', 'image/png', 7, 'photo.png')")
        .bind(photo)
        .execute(&app.pool)
        .await
        .unwrap();
    c.expect(
        PHOTO,
        &format!("/api/v1/media/{photo}/original"),
        me,
        None,
        200,
        "a photo",
    )
    .await;
    c.expect(
        PHOTO,
        &format!("/api/v1/media/{photo}/999"),
        me,
        None,
        400,
        "a size never made",
    )
    .await;
    c.expect(
        PHOTO,
        &fill(PHOTO.1),
        me,
        None,
        404,
        "a photo that isn't there",
    )
    .await;

    // The plan.
    let plan = format!("/api/v1/plan?week={WEEK}");
    c.expect(PLAN, &plan, me, None, 200, "the plan").await;
    let variant = dish["default_variant"].clone();
    let add = json!({"week": WEEK, "variant_id": variant});
    let planned = c
        .call(PLAN_ADD, me, Some(add), 200, "planning a dish")
        .await
        .body;
    let entry = format!(
        "/api/v1/plan/entries/{}",
        planned["entries"][0]["id"].as_str().unwrap()
    );
    let none = json!({"week": WEEK, "variant_id": SOME_ID});
    c.call(PLAN_ADD, me, Some(none), 404, "a version that isn't there")
        .await;
    let zero = json!({"week": WEEK, "variant_id": variant, "servings": 0});
    c.call(PLAN_ADD, me, Some(zero), 400, "no servings").await;
    c.expect(
        PLAN_CHANGE,
        &entry,
        me,
        Some(json!({"servings": 4})),
        200,
        "more servings",
    )
    .await;
    c.expect(
        PLAN_CHANGE,
        &entry,
        me,
        Some(json!({"servings": 99})),
        400,
        "too many servings",
    )
    .await;
    let missing = fill(PLAN_CHANGE.1);
    c.expect(
        PLAN_CHANGE,
        &missing,
        me,
        Some(json!({"servings": 4})),
        404,
        "an entry that isn't there",
    )
    .await;
    c.expect(
        PLAN_REMOVE,
        &missing,
        me,
        None,
        404,
        "an entry that isn't there",
    )
    .await;

    // The shopping list.
    let list = format!("/api/v1/shopping?week={WEEK}");
    c.expect(LIST, &list, me, None, 200, "the list").await;
    c.call(
        MARK,
        me,
        Some(json!({"week": WEEK, "key": "rice", "checked": true})),
        200,
        "a tick",
    )
    .await;
    c.call(
        MARK,
        me,
        Some(json!({"week": WEEK, "key": " "})),
        400,
        "a blank key",
    )
    .await;
    let added = c
        .call(
            EXTRA_ADD,
            me,
            Some(json!({"week": WEEK, "name": "candles"})),
            200,
            "an extra",
        )
        .await;
    let extras = added.body["extras"].as_array().expect("the list's extras");
    let extra = format!(
        "/api/v1/shopping/extras/{}",
        extras[0]["id"].as_str().unwrap()
    );
    c.call(
        EXTRA_ADD,
        me,
        Some(json!({"week": WEEK, "name": ""})),
        400,
        "a blank extra",
    )
    .await;
    c.expect(
        EXTRA_CHANGE,
        &extra,
        me,
        Some(json!({"checked": true})),
        200,
        "ticking an extra",
    )
    .await;
    let missing = fill(EXTRA_CHANGE.1);
    c.expect(
        EXTRA_CHANGE,
        &missing,
        me,
        Some(json!({"checked": true})),
        404,
        "an extra that isn't there",
    )
    .await;
    c.expect(
        EXTRA_REMOVE,
        &missing,
        me,
        None,
        404,
        "an extra that isn't there",
    )
    .await;
    c.expect(EXTRA_REMOVE, &extra, me, None, 200, "removing an extra")
        .await;
    let food: uuid::Uuid = sqlx::query_scalar("SELECT id FROM foods ORDER BY id LIMIT 1")
        .fetch_one(&app.pool)
        .await
        .unwrap();
    c.call(
        STAPLE_ADD,
        me,
        Some(json!({"week": WEEK, "food_id": food})),
        200,
        "a staple",
    )
    .await;
    c.call(
        STAPLE_ADD,
        me,
        Some(json!({"food_id": SOME_ID})),
        404,
        "a food that isn't there",
    )
    .await;
    let staple = format!("/api/v1/pantry-staples/{food}?week={WEEK}");
    c.expect(
        STAPLE_REMOVE,
        &staple,
        me,
        None,
        200,
        "a staple back on the list",
    )
    .await;
    c.call(SETTINGS, me, None, 200, "the settings").await;
    c.call(
        SETTINGS_CHANGE,
        me,
        Some(json!({"weight_unit": "metric"})),
        200,
        "grams",
    )
    .await;
    c.call(
        SETTINGS_CHANGE,
        me,
        Some(json!({"weight_unit": "stone"})),
        400,
        "stones",
    )
    .await;
    let weights = json!({"week": WEEK, "food_id": food, "weight_unit": "oz"});
    c.call(FOOD_WEIGHTS, me, Some(weights), 200, "a food in ounces")
        .await;
    let stones = json!({"food_id": food, "weight_unit": "stone"});
    c.call(FOOD_WEIGHTS, me, Some(stones), 400, "a food in stones")
        .await;
    let missing = json!({"food_id": SOME_ID, "weight_unit": "oz"});
    c.call(
        FOOD_WEIGHTS,
        me,
        Some(missing),
        404,
        "a food that isn't there",
    )
    .await;
    c.expect(PLAN_REMOVE, &entry, me, None, 200, "taking the dish off")
        .await;

    // The password, last of the owner's own changes: it signs out the owner's other browsers.
    let wrong = json!({"current": "not the password", "new": "a newer password"});
    c.call(PASSWORD, me, Some(wrong), 400, "a wrong current password")
        .await;
    let change = json!({"current": common::OWNER_PASSWORD, "new": "a newer password"});
    c.call(
        PASSWORD,
        As::Bearer(&writer),
        Some(change.clone()),
        403,
        "a token changing the password",
    )
    .await;
    c.call(PASSWORD, me, Some(change), 204, "a new password")
        .await;

    // What every route answers before its handler runs.
    let routes: Vec<(Route, bool, bool, bool)> = c
        .routes
        .iter()
        .map(|(r, d)| (r.clone(), d.public, d.takes_body, d.takes_query))
        .collect();
    for ((method, path), public, takes_body, takes_query) in &routes {
        let route = (method.as_str(), path.as_str());
        let url = fill(path);
        let body = if *takes_body {
            "{}".to_string()
        } else {
            String::new()
        };
        if !public {
            let req = raw(method, &url, As::Nobody, body.clone(), None);
            c.expect_request(route, req, 401, "signed out").await;
            let req = raw(method, &url, me, body.clone(), Some(SOME_ID));
            c.expect_request(route, req, 412, "made for another account")
                .await;
            if method != "GET" {
                let req = raw(method, &url, As::Bearer(&reader), body.clone(), None);
                c.expect_request(route, req, 403, "a read-only token").await;
            }
        }
        if *takes_body {
            let req = raw(method, &url, me, "{\"name\": ".into(), None);
            c.expect_request(route, req, 400, "broken JSON").await;
            let big = format!("{{\"name\": \"{}\"}}", "a".repeat(3 << 20));
            c.expect_request(route, raw(method, &url, me, big, None), 413, "3 MB")
                .await;
        }
        if let Some(bad) = unreadable(path) {
            let req = raw(method, &bad, me, body.clone(), None);
            c.expect_request(route, req, 400, "a parameter that can't be read")
                .await;
        }
        if *takes_query {
            let req = raw(method, &format!("{url}?week=a%00b"), me, body, None);
            c.expect_request(route, req, 400, "a NUL in the query")
                .await;
        }
    }

    // With the database gone, every route that needs it fails as documented; the others answer as before.
    app.pool.close().await;
    for (key, _, takes_body, _) in &routes {
        let (method, path) = key;
        let route = (method.as_str(), path.as_str());
        let body = sign_in_body(route).or_else(|| takes_body.then(|| json!({})));
        let body = body.map(|b| b.to_string()).unwrap_or_default();
        let req = raw(method, &fill(path), me, body, None);
        let reply = c.answer(route, req, "without a database").await;
        if c.routes[key].statuses.contains(&500) && !unproduced(method, path, 500) {
            assert_eq!(
                reply.status.as_u16(),
                500,
                "{method} {path} without a database: {}",
                reply.body
            );
        }
    }

    // Every documented status was produced by some call above.
    let mut gaps = Vec::new();
    for (key, d) in &c.routes {
        let (method, path) = key;
        let seen = c.seen.get(key);
        for &status in &d.statuses {
            if !seen.is_some_and(|s| s.contains(&status)) && !unproduced(method, path, status) {
                gaps.push(format!("{method} {path} {status}"));
            }
        }
    }
    assert!(gaps.is_empty(), "documented but never produced: {gaps:#?}");
}
