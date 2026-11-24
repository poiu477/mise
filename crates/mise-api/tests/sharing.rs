//! Sharing between households: a household's own recipes are seen by itself, by the households it has added as
//! partners, or by every household, as each recipe says; others read and fork them but never change them; the
//! install's cap holds every share to at most its level; and a share withdrawn leaves the forks made from it. Each
//! test runs on the demo recipes with three households: the owner's ("home"), next door's and across the road's.

#[macro_use]
mod common;

use std::ops::Deref;

use axum::http::StatusCode;
use common::{As, TestApp};
use mise_api::sharing::{Visibility, set_cap};
use serde_json::{Value, json};

/// The demo install with three households, each with someone signed in.
struct Street {
    app: TestApp,
    home: String,
    next_door: String,
    across: String,
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
        let mut others = vec![];
        for (household, user) in [("Next door", "neighbour"), ("Across the road", "opposite")] {
            let made = app
                .json(
                    "POST",
                    "/api/v1/households",
                    &home,
                    Some(json!({"name": household})),
                )
                .await;
            app.json(
                "POST",
                "/api/v1/users",
                &home,
                Some(json!({"username": user, "password": "another long password", "household_id": made["id"]})),
            )
            .await;
            others.push(app.sign_in(user, "another long password").await);
        }
        let across = others.pop().unwrap();
        let next_door = others.pop().unwrap();
        Some(Street {
            app,
            home,
            next_door,
            across,
        })
    }

    /// This browser's household's sharing.
    async fn household(&self, cookie: &str) -> Value {
        self.json("GET", "/api/v1/household", cookie, None).await
    }

    /// Add the household `of` as a partner of this browser's household.
    async fn add_partner(&self, cookie: &str, of: &str) {
        let id = self.household(of).await["id"].clone();
        self.json(
            "PUT",
            &format!("/api/v1/household/partners/{}", id.as_str().unwrap()),
            cookie,
            None,
        )
        .await;
    }

    /// Write a recipe titled `title` as this browser, shared as asked (the household's default when `None`).
    async fn write(&self, cookie: &str, title: &str, visibility: Option<&str>) -> Value {
        let mut recipe = json!({
            "title": title, "servings": 2,
            "ingredients": [{"text": "2 leeks, sliced"}, {"text": "1 TBSP butter"}],
            "steps": ["Soften the leeks in the butter."],
        });
        if let Some(v) = visibility {
            recipe["visibility"] = json!(v);
        }
        self.json("POST", "/api/v1/recipes", cookie, Some(recipe))
            .await
    }

    /// The status of the dish page at `slug` for this browser.
    async fn dish_status(&self, cookie: &str, slug: &Value) -> StatusCode {
        self.send(
            "GET",
            &format!("/api/v1/dishes/{}", slug.as_str().unwrap()),
            As::Cookie(cookie),
            None,
        )
        .await
        .status
    }

    /// Whether this browser sees the recipe: its dish page, the version by id, and in search.
    async fn sees(&self, cookie: &str, recipe: &Value) -> bool {
        let page = self.dish_status(cookie, &recipe["dish_slug"]).await;
        let version = self
            .send(
                "GET",
                &format!("/api/v1/variants/{}", recipe["id"].as_str().unwrap()),
                As::Cookie(cookie),
                None,
            )
            .await
            .status;
        let title = recipe["title"].as_str().unwrap();
        let found = self
            .json(
                "GET",
                &format!("/api/v1/dishes?q={}", title.replace(' ', "+")),
                cookie,
                None,
            )
            .await["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["slug"] == recipe["dish_slug"]);
        assert_eq!(
            page == StatusCode::OK,
            version == StatusCode::OK,
            "{title}: page and version agree"
        );
        assert_eq!(
            page == StatusCode::OK,
            found,
            "{title}: page and search agree"
        );
        page == StatusCode::OK
    }
}

#[tokio::test]
async fn each_level_reaches_its_households_and_no_further() {
    let street = test_app!(Street::new());
    street.add_partner(&street.home, &street.next_door).await;
    let private = street
        .write(&street.home, "Private Leeks", Some("private"))
        .await;
    let partners = street
        .write(&street.home, "Partner Leeks", Some("partners"))
        .await;
    let install = street
        .write(&street.home, "Install Leeks", Some("install"))
        .await;
    assert_eq!(private["visibility"], "private");
    assert_eq!(partners["visibility"], "partners");
    assert_eq!(install["visibility"], "install");

    for (recipe, home, next_door, across) in [
        (&private, true, false, false),
        (&partners, true, true, false),
        (&install, true, true, true),
    ] {
        let title = &recipe["title"];
        assert_eq!(
            street.sees(&street.home, recipe).await,
            home,
            "home, {title}"
        );
        assert_eq!(
            street.sees(&street.next_door, recipe).await,
            next_door,
            "next door, {title}"
        );
        assert_eq!(
            street.sees(&street.across, recipe).await,
            across,
            "across the road, {title}"
        );
    }

    // Others read a shared recipe, named as whose it is, and can't change it or open it for editing.
    let home_name = street.household(&street.home).await["name"].clone();
    let page = street
        .json(
            "GET",
            &format!("/api/v1/dishes/{}", install["dish_slug"].as_str().unwrap()),
            &street.across,
            None,
        )
        .await;
    assert_eq!(page["variants"][0]["editable"], false);
    assert_eq!(page["variants"][0]["shared_by"], home_name);
    let mine = street
        .json(
            "GET",
            &format!("/api/v1/dishes/{}", install["dish_slug"].as_str().unwrap()),
            &street.home,
            None,
        )
        .await;
    assert_eq!(mine["variants"][0]["editable"], true);
    assert_eq!(mine["variants"][0]["shared_by"], Value::Null);
    let path = format!("/api/v1/recipes/{}", install["id"].as_str().unwrap());
    for (method, body) in [
        ("GET", None),
        (
            "PUT",
            Some(json!({"title": "Mine Now", "servings": 2, "ingredients": [], "steps": []})),
        ),
        ("DELETE", None),
    ] {
        let reply = street
            .send(method, &path, As::Cookie(&street.across), body)
            .await;
        assert_eq!(reply.status, StatusCode::NOT_FOUND, "{method} {path}");
    }
    assert!(street.sees(&street.across, &install).await, "still there");
}

#[tokio::test]
async fn a_partner_link_runs_one_way_until_the_other_household_adds_it_back() {
    let street = test_app!(Street::new());
    let theirs = street
        .write(&street.next_door, "Neighbour's Leeks", Some("partners"))
        .await;
    let ours = street
        .write(&street.home, "Our Leeks", Some("partners"))
        .await;

    // Home shares with next door: next door sees home's, but home doesn't see next door's.
    street.add_partner(&street.home, &street.next_door).await;
    assert!(street.sees(&street.next_door, &ours).await);
    assert!(!street.sees(&street.home, &theirs).await);
    let home = street.household(&street.home).await;
    let next_door = street.household(&street.next_door).await;
    assert_eq!(
        home["partners"],
        json!([{"id": next_door["id"], "name": "Next door"}])
    );
    assert_eq!(home["partner_of"], json!([]));
    assert_eq!(
        next_door["partner_of"],
        json!([{"id": home["id"], "name": home["name"]}])
    );
    let others: Vec<&Value> = home["households"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| &h["name"])
        .collect();
    assert_eq!(others, ["Across the road", "Next door"]);

    // Next door shares back: now both see each other's.
    street.add_partner(&street.next_door, &street.home).await;
    street.add_partner(&street.next_door, &street.home).await;
    assert!(street.sees(&street.home, &theirs).await);
    assert!(street.sees(&street.next_door, &ours).await);
    assert!(!street.sees(&street.across, &ours).await);

    // Home stops sharing: next door loses home's at once, and keeps sharing its own.
    let id = next_door["id"].as_str().unwrap();
    let reply = street
        .send(
            "DELETE",
            &format!("/api/v1/household/partners/{id}"),
            As::Cookie(&street.home),
            None,
        )
        .await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT);
    assert!(!street.sees(&street.next_door, &ours).await);
    assert!(street.sees(&street.home, &theirs).await);

    // A household isn't its own partner, and one that doesn't exist isn't anyone's.
    let own = home["id"].as_str().unwrap();
    for (path, status) in [
        (
            format!("/api/v1/household/partners/{own}"),
            StatusCode::BAD_REQUEST,
        ),
        (
            "/api/v1/household/partners/0190f4a2-0000-7000-8000-000000000000".to_string(),
            StatusCode::NOT_FOUND,
        ),
    ] {
        let reply = street
            .send("PUT", &path, As::Cookie(&street.home), None)
            .await;
        assert_eq!(reply.status, status, "{path}: {}", reply.body);
    }
}

#[tokio::test]
async fn new_recipes_and_forks_take_the_households_default_and_keep_their_own() {
    let street = test_app!(Street::new());
    let home = street.household(&street.home).await;
    assert_eq!(home["default_visibility"], "private");
    assert_eq!(home["max_visibility"], "install");
    assert_eq!(
        street.write(&street.home, "Quiet Leeks", None).await["visibility"],
        "private"
    );

    let changed = street
        .json(
            "PUT",
            "/api/v1/household/sharing",
            &street.home,
            Some(json!({"default_visibility": "install"})),
        )
        .await;
    assert_eq!(changed["default_visibility"], "install");
    let shared = street.write(&street.home, "Open Leeks", None).await;
    assert_eq!(shared["visibility"], "install");
    assert!(street.sees(&street.across, &shared).await);

    // A fork takes the forking household's default too.
    let pasta = street
        .json(
            "GET",
            "/api/v1/dishes/weeknight-tomato-pasta",
            &street.home,
            None,
        )
        .await;
    let fork = street
        .json(
            "POST",
            &format!(
                "/api/v1/variants/{}/fork",
                pasta["default_variant"].as_str().unwrap()
            ),
            &street.home,
            None,
        )
        .await;
    assert_eq!(fork["visibility"], "install");

    // A change that doesn't say keeps the recipe's level; one that does sets it.
    let path = format!("/api/v1/recipes/{}", shared["id"].as_str().unwrap());
    let body = json!({"title": "Open Leeks", "servings": 2, "ingredients": [{"text": "3 leeks"}], "steps": []});
    assert_eq!(
        street
            .json("PUT", &path, &street.home, Some(body.clone()))
            .await["visibility"],
        "install"
    );
    let mut private = body;
    private["visibility"] = json!("private");
    assert_eq!(
        street.json("PUT", &path, &street.home, Some(private)).await["visibility"],
        "private"
    );
    assert!(!street.sees(&street.across, &shared).await);
}

#[tokio::test]
async fn the_cap_holds_every_share_to_its_level() {
    let street = test_app!(Street::new());
    street.add_partner(&street.home, &street.next_door).await;
    street
        .json(
            "PUT",
            "/api/v1/household/sharing",
            &street.home,
            Some(json!({"default_visibility": "install"})),
        )
        .await;
    let install = street
        .write(&street.home, "Install Leeks", Some("install"))
        .await;
    let partners = street
        .write(&street.home, "Partner Leeks", Some("partners"))
        .await;

    // Capped at partners: what was shared with the install now reaches partners only, and no more can be asked.
    set_cap(&street.pool, Visibility::Partners).await.unwrap();
    assert_eq!(
        street.household(&street.home).await["max_visibility"],
        "partners"
    );
    assert!(street.sees(&street.next_door, &install).await);
    assert!(!street.sees(&street.across, &install).await);
    let refused = street
        .send(
            "POST",
            "/api/v1/recipes",
            As::Cookie(&street.home),
            Some(json!({"title": "More Leeks", "servings": 2, "ingredients": [], "steps": [], "visibility": "install"})),
        )
        .await;
    assert_eq!(refused.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        refused.body["error"],
        "this install shares recipes no further than partners"
    );
    let refused = street
        .send(
            "PUT",
            "/api/v1/household/sharing",
            As::Cookie(&street.home),
            Some(json!({"default_visibility": "install"})),
        )
        .await;
    assert_eq!(refused.status, StatusCode::BAD_REQUEST);
    // A default set before the cap came down gives a new recipe no more than the cap.
    assert_eq!(
        street.write(&street.home, "Capped Leeks", None).await["visibility"],
        "partners"
    );

    // Capped at private: nothing is shared at all.
    set_cap(&street.pool, Visibility::Private).await.unwrap();
    assert!(!street.sees(&street.next_door, &install).await);
    assert!(!street.sees(&street.next_door, &partners).await);
    assert!(street.sees(&street.home, &partners).await);

    // Lifted again, every share is as it was.
    set_cap(&street.pool, Visibility::Install).await.unwrap();
    assert!(street.sees(&street.across, &install).await);
    assert!(!street.sees(&street.across, &partners).await);
    assert!(street.sees(&street.next_door, &partners).await);
}

#[tokio::test]
async fn a_withdrawn_share_leaves_the_forks_made_from_it() {
    let street = test_app!(Street::new());
    let shared = street
        .write(&street.home, "Shared Leeks", Some("install"))
        .await;

    // Across the road makes its own version of home's recipe, which names home's as its original.
    let fork = street
        .json(
            "POST",
            &format!("/api/v1/variants/{}/fork", shared["id"].as_str().unwrap()),
            &street.across,
            None,
        )
        .await;
    assert_eq!(fork["dish_slug"], shared["dish_slug"]);
    let slug = shared["dish_slug"].as_str().unwrap();
    let page = street
        .json(
            "GET",
            &format!("/api/v1/dishes/{slug}"),
            &street.across,
            None,
        )
        .await;
    assert_eq!(page["variants"].as_array().unwrap().len(), 2);
    let theirs = page["variants"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["id"] == fork["id"])
        .unwrap();
    assert_eq!(theirs["original"]["id"], shared["id"]);
    assert_eq!(
        theirs["original"]["by"],
        street.household(&street.home).await["name"]
    );
    // Home doesn't see across the road's fork: it's theirs, private by default.
    let home_page = street
        .json("GET", &format!("/api/v1/dishes/{slug}"), &street.home, None)
        .await;
    assert_eq!(home_page["variants"].as_array().unwrap().len(), 1);

    // Home withdraws the share: its recipe is gone for across the road, whose fork stays, no longer naming it.
    let mut private = json!({
        "title": "Shared Leeks", "servings": 2,
        "ingredients": [{"text": "2 leeks, sliced"}, {"text": "1 TBSP butter"}],
        "steps": ["Soften the leeks in the butter."], "visibility": "private",
    });
    street
        .json(
            "PUT",
            &format!("/api/v1/recipes/{}", shared["id"].as_str().unwrap()),
            &street.home,
            Some(private.take()),
        )
        .await;
    // The dish stays, through the fork; home's version is gone from it and by its id.
    let gone = street
        .send(
            "GET",
            &format!("/api/v1/variants/{}", shared["id"].as_str().unwrap()),
            As::Cookie(&street.across),
            None,
        )
        .await;
    assert_eq!(gone.status, StatusCode::NOT_FOUND);
    let page = street
        .json(
            "GET",
            &format!("/api/v1/dishes/{slug}"),
            &street.across,
            None,
        )
        .await;
    assert_eq!(page["variants"].as_array().unwrap().len(), 1);
    assert_eq!(page["variants"][0]["id"], fork["id"]);
    assert_eq!(page["variants"][0]["original"], Value::Null);
    assert_eq!(page["variants"][0]["editable"], true);
    let kept = street
        .json(
            "GET",
            &format!("/api/v1/recipes/{}", fork["id"].as_str().unwrap()),
            &street.across,
            None,
        )
        .await;
    assert_eq!(kept["ingredients"].as_array().unwrap().len(), 2);
}
