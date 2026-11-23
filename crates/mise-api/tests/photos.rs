//! Photos through the router: a version's photo linked from search, the dish page and the version, served at each
//! size, and the answers when one is missing. The photo is drawn by the test, stored by `mise-ingest`'s media
//! passes into a media folder of the test's own (removed when it ends), and given to the newest Black Bean Tacos.
//!
//! Photos follow their version's visibility: a household's own version's photo is served to that household and its
//! kitchen devices and is not found for every other, while the demo recipes' photos and foods' pictures, which no
//! household owns, are served to all (the `Street` tests).

#[macro_use]
mod common;

use std::path::Path;

use axum::{
    body::Bytes,
    http::{HeaderMap, StatusCode, header},
};
use common::{As, TestApp};
use http_body_util::BodyExt;
use image::{ImageFormat, Rgb, RgbImage};
use mise_ingest::media::{MediaOptions, make_derivatives, store_and_link};
use serde_json::{Value, json};
use uuid::Uuid;

/// Size of the drawn photo: wider than the small web copy, narrower than the large one.
const WIDTH: u32 = 800;
const HEIGHT: u32 = 600;

/// The dish the photo belongs to; it goes on the version the dish opens on.
const DISH: &str = "black-bean-tacos";

/// A photo's response, read through as bytes.
struct Fetched {
    status: StatusCode,
    headers: HeaderMap,
    body: Bytes,
}

impl Fetched {
    fn content_type(&self) -> &str {
        self.headers
            .get(header::CONTENT_TYPE)
            .and_then(|h| h.to_str().ok())
            .unwrap_or_default()
    }

    /// Width and height of the image in the body.
    fn dimensions(&self) -> (u32, u32) {
        let img = image::load_from_memory(&self.body).expect("the body is an image");
        (img.width(), img.height())
    }

    /// The JSON error message, for a response that isn't an image.
    fn error(&self) -> Value {
        serde_json::from_slice::<Value>(&self.body).expect("a JSON body")["error"].clone()
    }
}

/// The demo install with its owner signed in, and the drawn photo's bytes.
struct Kitchen {
    app: TestApp,
    cookie: String,
    photo: Vec<u8>,
}

impl Kitchen {
    /// The demo recipes with the photo stored and linked; with `web_copies`, its web sizes made too.
    async fn new(web_copies: bool) -> Option<Kitchen> {
        let app = TestApp::with_demo().await?;
        let cookie = app.setup_owner().await;
        let photo = draw(TACOS);
        let archive = app.media_root.join("archive");
        std::fs::create_dir_all(&archive).unwrap();
        std::fs::write(archive.join("tacos.png"), &photo).unwrap();
        // What a seed bundle records for a version's photo: a path relative to the bundle's media root.
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
        let stored = store_and_link(&app.pool, &options).await.unwrap();
        assert_eq!((stored.new_media, stored.hero_links), (1, 1), "{stored:?}");
        if web_copies {
            let made = make_derivatives(&app.pool, &options).await.unwrap();
            assert_eq!(made.done, 1, "{made:?}");
        }
        Some(Kitchen { app, cookie, photo })
    }

    /// GET as the owner and expect 200 OK.
    async fn get(&self, path: &str) -> Value {
        self.app.json("GET", path, &self.cookie, None).await
    }

    /// The photo's media id, as the dish's search result links it.
    async fn photo_id(&self) -> Uuid {
        let page = self.get("/api/v1/dishes?q=tacos").await;
        let id = page["items"][0]["image"]["id"]
            .as_str()
            .expect("the tacos have a photo");
        id.parse().unwrap()
    }

    /// GET a photo as `from`, keeping the body as bytes.
    async fn fetch_as(&self, path: &str, from: As<'_>) -> Fetched {
        let req = TestApp::request("GET", path, from)
            .body(axum::body::Body::empty())
            .unwrap();
        let (parts, body) = self.app.oneshot(req).await.into_parts();
        Fetched {
            status: parts.status,
            headers: parts.headers,
            body: body.collect().await.unwrap().to_bytes(),
        }
    }

    /// GET a photo as the owner.
    async fn fetch(&self, path: &str) -> Fetched {
        self.fetch_as(path, As::Cookie(&self.cookie)).await
    }
}

/// How much blue each drawn photo has, so no two are the same file: photos are stored once by content.
const TACOS: u8 = 120;
const STEW: u8 = 30;
const BEANS: u8 = 200;
const LENTILS: u8 = 250;

/// An opaque PNG drawn here (a colour gradient), so the repository holds no photo and the web copy is a JPEG.
fn draw(blue: u8) -> Vec<u8> {
    let img = RgbImage::from_fn(WIDTH, HEIGHT, |x, y| {
        Rgb([(x * 255 / WIDTH) as u8, (y * 255 / HEIGHT) as u8, blue])
    });
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, ImageFormat::Png).unwrap();
    out.into_inner()
}

/// Remove every file under `dir`, keeping the folders: the photo is still in the database but gone from disk.
fn empty_files(dir: &Path) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            empty_files(&path);
        } else {
            std::fs::remove_file(&path).unwrap();
        }
    }
}

#[tokio::test]
async fn a_photo_is_linked_from_search_the_dish_and_the_version() {
    let kitchen = test_app!(Kitchen::new(true));
    let id = kitchen.photo_id().await;
    let links = json!({
        "id": id,
        "small": format!("/api/v1/media/{id}/400"),
        "large": format!("/api/v1/media/{id}/1200"),
        "original": format!("/api/v1/media/{id}/original"),
    });

    let page = kitchen.get("/api/v1/dishes").await;
    for item in page["items"].as_array().unwrap() {
        let expected = if item["slug"] == DISH {
            &links
        } else {
            &Value::Null
        };
        assert_eq!(&item["image"], expected, "{}", item["title"]);
    }

    let dish = kitchen.get(&format!("/api/v1/dishes/{DISH}")).await;
    assert_eq!(dish["variants"][0]["id"], dish["default_variant"]);
    assert_eq!(dish["variants"][0]["image"], links);
    assert_eq!(
        dish["variants"][1]["image"],
        Value::Null,
        "the older version has no photo"
    );

    let version_id = dish["default_variant"].as_str().unwrap();
    for servings in ["2", "3"] {
        let version = kitchen
            .get(&format!(
                "/api/v1/variants/{version_id}?servings={servings}"
            ))
            .await;
        assert_eq!(version["image"], links, "{servings} servings");
    }
}

#[tokio::test]
async fn a_photo_is_served_at_each_size() {
    let kitchen = test_app!(Kitchen::new(true));
    let id = kitchen.photo_id().await;

    let small = kitchen.fetch(&format!("/api/v1/media/{id}/400")).await;
    assert_eq!(small.status, StatusCode::OK);
    assert_eq!(small.content_type(), "image/jpeg");
    assert_eq!(small.dimensions(), (400, 300));

    // The photo already fits the large size, so it is never made bigger.
    let large = kitchen.fetch(&format!("/api/v1/media/{id}/1200")).await;
    assert_eq!(large.status, StatusCode::OK);
    assert_eq!(large.dimensions(), (WIDTH, HEIGHT));

    let original = kitchen.fetch(&format!("/api/v1/media/{id}/original")).await;
    assert_eq!(original.status, StatusCode::OK);
    assert_eq!(original.content_type(), "image/png");
    assert_eq!(
        original.body, kitchen.photo,
        "the original is the stored file"
    );

    for photo in [&small, &large, &original] {
        assert_eq!(
            photo.headers.get(header::CACHE_CONTROL).unwrap(),
            "private, max-age=31536000, immutable",
            "a media id never changes its bytes, and only the browser that asked may keep them"
        );
    }
}

#[tokio::test]
async fn a_size_not_made_yet_falls_back_to_the_original() {
    let kitchen = test_app!(Kitchen::new(false));
    let id = kitchen.photo_id().await;
    for size in ["400", "1200"] {
        let photo = kitchen.fetch(&format!("/api/v1/media/{id}/{size}")).await;
        assert_eq!(photo.status, StatusCode::OK, "{size}");
        assert_eq!(photo.content_type(), "image/png", "{size}");
        assert_eq!(photo.body, kitchen.photo, "{size}");
    }
}

#[tokio::test]
async fn a_photo_that_does_not_exist_is_not_found() {
    let kitchen = test_app!(Kitchen::new(true));
    let unknown = kitchen
        .fetch(&format!("/api/v1/media/{}/original", Uuid::now_v7()))
        .await;
    assert_eq!(unknown.status, StatusCode::NOT_FOUND);
    assert!(unknown.error().is_string());

    // Still recorded, but its files are gone from the media folder.
    let id = kitchen.photo_id().await;
    empty_files(&kitchen.app.media_root);
    for size in ["400", "1200", "original"] {
        let gone = kitchen.fetch(&format!("/api/v1/media/{id}/{size}")).await;
        assert_eq!(gone.status, StatusCode::NOT_FOUND, "{size}");
        assert!(gone.error().is_string(), "{size}");
    }
}

#[tokio::test]
async fn a_size_that_is_never_made_is_refused() {
    let kitchen = test_app!(Kitchen::new(true));
    let id = kitchen.photo_id().await;
    for size in ["800", "small", "0"] {
        let photo = kitchen.fetch(&format!("/api/v1/media/{id}/{size}")).await;
        assert_eq!(photo.status, StatusCode::BAD_REQUEST, "{size}");
        assert_eq!(photo.error(), "size must be 400, 1200 or original");
    }
    let bad_id = kitchen.fetch("/api/v1/media/not-a-uuid/400").await;
    assert_eq!(bad_id.status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn photos_need_a_signed_in_person() {
    let kitchen = test_app!(Kitchen::new(true));
    let id = kitchen.photo_id().await;
    for size in ["400", "1200", "original"] {
        let photo = kitchen
            .fetch_as(&format!("/api/v1/media/{id}/{size}"), As::Nobody)
            .await;
        assert_eq!(photo.status, StatusCode::UNAUTHORIZED, "{size}");
        assert!(
            image::load_from_memory(&photo.body).is_err(),
            "{size} sends no image"
        );
    }
}

/// The sizes a photo is asked for in.
const SIZES: [&str; 3] = ["400", "1200", "original"];

/// The demo install with two households, home (the owner's) and next door, each with an admin signed in and a
/// kitchen device.
struct Street {
    app: TestApp,
    home: Household,
    next_door: Household,
}

/// A household's id, its admin's session cookie and its kitchen device's cookie.
struct Household {
    id: Uuid,
    admin: String,
    device: String,
}

impl Household {
    /// Everyone who asks as this household: its admin's browser and its kitchen device.
    fn everyone(&self) -> [(&'static str, As<'_>); 2] {
        [
            ("an account", As::Cookie(&self.admin)),
            ("a kitchen device", As::Kitchen(&self.device)),
        ]
    }
}

impl Street {
    /// None when there is no database (outside CI).
    async fn new() -> Option<Street> {
        let app = TestApp::with_demo().await?;
        let owner = app.setup_owner().await;
        let home_id = app.json("GET", "/api/v1/me", &owner, None).await["household"]["id"].clone();
        let neighbour = app
            .json(
                "POST",
                "/api/v1/users",
                &owner,
                Some(json!({
                    "username": "neighbour",
                    "password": "another long password",
                    "is_admin": true,
                    "household_name": "Next door",
                })),
            )
            .await;
        let admin = app.sign_in("neighbour", "another long password").await;
        let home = Household {
            id: uuid(&home_id),
            device: kitchen_device(&app, &owner).await,
            admin: owner,
        };
        let next_door = Household {
            id: uuid(&neighbour["household"]["id"]),
            device: kitchen_device(&app, &admin).await,
            admin,
        };
        assert_ne!(home.id, next_door.id);
        Some(Street {
            app,
            home,
            next_door,
        })
    }

    /// A version `household` owns, as an import would add it, with the archive file `photo` as its photo once
    /// `store` runs.
    async fn own_recipe(&self, household: Uuid, title: &str, photo: &str) -> Uuid {
        let id = Uuid::now_v7();
        sqlx::query(
            "INSERT INTO variants (id, origin, title, has_steps, content_hash, published_at, household_id, meta)
             VALUES ($1, 'user', $2, false, $1::text, '2026-11-10T12:00:00Z', $3, $4)",
        )
        .bind(id)
        .bind(title)
        .bind(household)
        .bind(json!({"media": {"hero": photo}}))
        .execute(&self.app.pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO variant_yields (variant_id, servings) VALUES ($1, 2)")
            .bind(id)
            .execute(&self.app.pool)
            .await
            .unwrap();
        id
    }

    /// The newest Black Bean Tacos, a demo version no household owns.
    async fn tacos(&self) -> Uuid {
        sqlx::query_scalar("SELECT default_variant_id FROM dishes WHERE slug = $1")
            .bind(DISH)
            .fetch_one(&self.app.pool)
            .await
            .unwrap()
    }

    /// Give version `variant` the archive file `photo` as its photo, once `store` runs.
    async fn set_photo(&self, variant: Uuid, photo: &str) {
        sqlx::query("UPDATE variants SET meta = meta || $2 WHERE id = $1")
            .bind(variant)
            .bind(json!({"media": {"hero": photo}}))
            .execute(&self.app.pool)
            .await
            .unwrap();
    }

    /// Put `files` (name and bytes) in an archive and run `mise-ingest`'s media passes over everything the versions
    /// and ingredient lines reference: store, link, web copies.
    async fn store(&self, files: &[(&str, Vec<u8>)]) -> mise_ingest::media::StoreStats {
        let archive = self.app.media_root.join("archive");
        std::fs::create_dir_all(&archive).unwrap();
        for (name, bytes) in files {
            std::fs::write(archive.join(name), bytes).unwrap();
        }
        let options = MediaOptions {
            archive_root: Some(archive),
            media_root: self.app.media_root.clone(),
            limit: None,
            jobs: 1,
        };
        let stored = store_and_link(&self.app.pool, &options).await.unwrap();
        make_derivatives(&self.app.pool, &options).await.unwrap();
        stored
    }

    /// The id of version `variant`'s photo, as the version links it for `household`.
    async fn photo_of(&self, variant: Uuid, household: &Household) -> Uuid {
        let version = self.version(variant, household).await;
        uuid(&version["image"]["id"])
    }

    /// Version `variant` as `household`'s admin reads it.
    async fn version(&self, variant: Uuid, household: &Household) -> Value {
        let path = format!("/api/v1/variants/{variant}");
        self.app.json("GET", &path, &household.admin, None).await
    }

    /// The media id stored for the archive file `name`.
    async fn stored(&self, name: &str) -> Uuid {
        sqlx::query_scalar("SELECT media_id FROM media_sources WHERE path = $1")
            .bind(name)
            .fetch_one(&self.app.pool)
            .await
            .unwrap()
    }

    /// GET as `from`, keeping the body as bytes.
    async fn fetch(&self, path: &str, from: As<'_>) -> Fetched {
        let req = TestApp::request("GET", path, from)
            .body(axum::body::Body::empty())
            .unwrap();
        let (parts, body) = self.app.oneshot(req).await.into_parts();
        Fetched {
            status: parts.status,
            headers: parts.headers,
            body: body.collect().await.unwrap().to_bytes(),
        }
    }

    /// Photo `id` is served to everyone in `household`, in every size, at the API's address and its old one.
    async fn assert_served(&self, id: Uuid, household: &Household, what: &str) {
        for (who, from) in household.everyone() {
            for api in ["/api/v1", "/api"] {
                for size in SIZES {
                    let photo = self.fetch(&format!("{api}/media/{id}/{size}"), from).await;
                    let what = format!("{what}: {who}, {api}, {size}");
                    assert_eq!(photo.status, StatusCode::OK, "{what}");
                    assert!(photo.content_type().starts_with("image/"), "{what}");
                    let width = size.parse().map_or(WIDTH, |side: u32| side.min(WIDTH));
                    assert_eq!(photo.dimensions().0, width, "{what}");
                }
            }
        }
    }

    /// Photo `id` is not found for anyone in `household`, in any size, at the API's address or its old one: the
    /// answer is the one for an id that was never stored, to the letter, and carries no image.
    async fn assert_not_found(&self, id: Uuid, household: &Household, what: &str) {
        for (who, from) in household.everyone() {
            for api in ["/api/v1", "/api"] {
                for size in SIZES {
                    let hidden = self.fetch(&format!("{api}/media/{id}/{size}"), from).await;
                    let never = self
                        .fetch(&format!("{api}/media/{}/{size}", Uuid::now_v7()), from)
                        .await;
                    let what = format!("{what}: {who}, {api}, {size}");
                    assert_eq!(hidden.status, StatusCode::NOT_FOUND, "{what}");
                    assert_eq!(hidden.body, never.body, "{what}");
                    assert_eq!(hidden.content_type(), never.content_type(), "{what}");
                    assert_eq!(
                        hidden.headers.get(header::CACHE_CONTROL),
                        never.headers.get(header::CACHE_CONTROL),
                        "{what}"
                    );
                    assert!(image::load_from_memory(&hidden.body).is_err(), "{what}");
                }
            }
        }
    }
}

/// The UUID in a JSON string.
fn uuid(v: &Value) -> Uuid {
    v.as_str().expect("an id").parse().unwrap()
}

/// A kitchen device for `admin`'s household: issue a kitchen token, open its one-time link as the device would, and
/// return the cookie it sets.
async fn kitchen_device(app: &TestApp, admin: &str) -> String {
    let made = app
        .json(
            "POST",
            "/api/v1/kitchen-tokens",
            admin,
            Some(json!({"name": "Kitchen screen"})),
        )
        .await;
    let opened = app
        .send("GET", made["link"].as_str().unwrap(), As::Nobody, None)
        .await;
    assert_eq!(opened.status, StatusCode::SEE_OTHER);
    let cookie = opened.cookie("mise_kitchen").expect("a kitchen cookie");
    cookie.split(';').next().unwrap().to_string()
}

#[tokio::test]
async fn a_household_s_own_photo_is_served_only_to_that_household() {
    let street = test_app!(Street::new());
    let (home, next_door) = (&street.home, &street.next_door);
    let stew = street
        .own_recipe(home.id, "Coconut Lentil Stew", "stew.png")
        .await;
    let photo = draw(STEW);
    street.store(&[("stew.png", photo.clone())]).await;
    let id = street.photo_of(stew, home).await;
    let original = format!("/api/v1/media/{id}/original");

    street.assert_served(id, home, "home's own photo").await;
    let ours = street.fetch(&original, As::Cookie(&home.admin)).await;
    assert_eq!(ours.body, photo, "the owner gets the stored file");

    // Next door can't see the version, so its photo isn't there for them either, whoever asks and at any size.
    let version = street
        .fetch(
            &format!("/api/v1/variants/{stew}"),
            As::Cookie(&next_door.admin),
        )
        .await;
    assert_eq!(version.status, StatusCode::NOT_FOUND);
    street
        .assert_not_found(id, next_door, "home's photo, next door")
        .await;

    // A script next door, with an API token, is no better placed.
    let token = street
        .app
        .json(
            "POST",
            "/api/v1/me/tokens",
            &next_door.admin,
            Some(json!({"name": "script", "scope": "read"})),
        )
        .await;
    let script = street
        .fetch(&original, As::Bearer(token["token"].as_str().unwrap()))
        .await;
    assert_eq!(script.status, StatusCode::NOT_FOUND);

    // Signed out, the answer is the one every photo gives: sign in.
    let nobody = street.fetch(&original, As::Nobody).await;
    assert_eq!(nobody.status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn a_seed_photo_is_served_to_every_household() {
    let street = test_app!(Street::new());
    let tacos = street.tacos().await;
    street.set_photo(tacos, "tacos.png").await;
    street.store(&[("tacos.png", draw(TACOS))]).await;
    let id = street.photo_of(tacos, &street.home).await;
    assert_eq!(id, street.photo_of(tacos, &street.next_door).await);

    for household in [&street.home, &street.next_door] {
        street
            .assert_served(id, household, "the demo tacos' photo")
            .await;
    }
}

#[tokio::test]
async fn a_photo_follows_its_version_s_visibility() {
    let street = test_app!(Street::new());
    let (home, next_door) = (&street.home, &street.next_door);
    let stew = street
        .own_recipe(home.id, "Coconut Lentil Stew", "stew.png")
        .await;
    street.store(&[("stew.png", draw(STEW))]).await;
    let id = street.photo_of(stew, home).await;
    street.assert_served(id, home, "while it is home's").await;
    street
        .assert_not_found(id, next_door, "while it is home's")
        .await;

    // The version changes hands (as sharing will widen and withdraw who sees it): the photo goes with it.
    sqlx::query("UPDATE variants SET household_id = $2 WHERE id = $1")
        .bind(stew)
        .bind(next_door.id)
        .execute(&street.app.pool)
        .await
        .unwrap();
    street
        .assert_served(id, next_door, "once it is next door's")
        .await;
    street
        .assert_not_found(id, home, "once it is next door's")
        .await;

    // A version no household owns is everyone's, and so is its photo.
    sqlx::query("UPDATE variants SET household_id = NULL WHERE id = $1")
        .bind(stew)
        .execute(&street.app.pool)
        .await
        .unwrap();
    for household in [home, next_door] {
        street
            .assert_served(id, household, "once no household owns it")
            .await;
    }
}

#[tokio::test]
async fn a_photo_two_versions_use_is_served_to_whoever_sees_either() {
    let street = test_app!(Street::new());
    let (home, next_door) = (&street.home, &street.next_door);
    // The same file under two names: stored once, and each household's version links the one photo.
    let ours = street
        .own_recipe(home.id, "Coconut Lentil Stew", "ours.png")
        .await;
    let theirs = street
        .own_recipe(next_door.id, "Lentil Dal", "theirs.png")
        .await;
    let stored = street
        .store(&[("ours.png", draw(STEW)), ("theirs.png", draw(STEW))])
        .await;
    assert_eq!((stored.new_media, stored.hero_links), (1, 2), "{stored:?}");
    let id = street.photo_of(ours, home).await;
    assert_eq!(id, street.photo_of(theirs, next_door).await);
    for household in [home, next_door] {
        street
            .assert_served(id, household, "a photo both households use")
            .await;
    }

    // Next door's version goes: the photo is home's alone again.
    sqlx::query("DELETE FROM variants WHERE id = $1")
        .bind(theirs)
        .execute(&street.app.pool)
        .await
        .unwrap();
    street.assert_served(id, home, "home still uses it").await;
    street
        .assert_not_found(id, next_door, "next door no longer uses it")
        .await;

    // A fork keeps its parent's photo: what a version everyone sees uses stays everyone's, whoever else uses it.
    let tacos = street.tacos().await;
    street.set_photo(tacos, "ours.png").await;
    street.store(&[]).await;
    for household in [home, next_door] {
        street
            .assert_served(id, household, "a photo a demo version uses too")
            .await;
    }
}

#[tokio::test]
async fn a_photo_no_version_uses_is_not_found() {
    let street = test_app!(Street::new());
    let stew = street
        .own_recipe(street.home.id, "Coconut Lentil Stew", "stew.png")
        .await;
    street.store(&[("stew.png", draw(STEW))]).await;
    let id = street.photo_of(stew, &street.home).await;

    // Its only version is deleted: the photo is still stored, and nobody's.
    sqlx::query("DELETE FROM variants WHERE id = $1")
        .bind(stew)
        .execute(&street.app.pool)
        .await
        .unwrap();
    let kept: i64 = sqlx::query_scalar("SELECT count(*) FROM media WHERE id = $1")
        .bind(id)
        .fetch_one(&street.app.pool)
        .await
        .unwrap();
    assert_eq!(kept, 1);
    for household in [&street.home, &street.next_door] {
        street
            .assert_not_found(id, household, "a photo no version uses")
            .await;
    }
}

/// The id of the picture `version` shows for its ingredient matched to `food`; null when it shows none.
fn picture(version: &Value, food: Uuid) -> Value {
    let line = version["ingredients"]
        .as_array()
        .expect("ingredients")
        .iter()
        .find(|i| i["food_id"] == food.to_string())
        .expect("an ingredient with that food");
    line["image"]["id"].clone()
}

#[tokio::test]
async fn a_food_s_picture_is_everyone_s_and_never_a_household_s_own_photo() {
    let street = test_app!(Street::new());
    let (home, next_door) = (&street.home, &street.next_door);
    let tacos = street.tacos().await;
    // Two of the tacos' ingredients, matched to foods. The demo version gets a picture on the first; home's own
    // version uses the second, with a photo of home's own on its line.
    let lines: Vec<(Uuid, Uuid)> = sqlx::query_as(
        "SELECT DISTINCT ON (food_id) id, food_id FROM ingredient_lines WHERE variant_id = $1 AND food_id IS NOT NULL
         ORDER BY food_id LIMIT 2",
    )
    .bind(tacos)
    .fetch_all(&street.app.pool)
    .await
    .unwrap();
    let [(pictured_line, pictured), (_, unpictured)] = lines[..] else {
        panic!("the tacos have two matched ingredients: {lines:?}");
    };
    sqlx::query("UPDATE ingredient_lines SET source_food = $2 WHERE id = $1")
        .bind(pictured_line)
        .bind(json!({"image": "beans.png"}))
        .execute(&street.app.pool)
        .await
        .unwrap();
    let stew = street
        .own_recipe(home.id, "Coconut Lentil Stew", "stew.png")
        .await;
    sqlx::query(
        "INSERT INTO ingredient_lines (id, variant_id, line_key, position, raw_name, food_id, source_food)
         VALUES ($1, $2, 'l0', 0, 'Red Lentils', $3, $4)",
    )
    .bind(Uuid::now_v7())
    .bind(stew)
    .bind(unpictured)
    .bind(json!({"image": "lentils.png"}))
    .execute(&street.app.pool)
    .await
    .unwrap();
    let stored = street
        .store(&[
            ("beans.png", draw(BEANS)),
            ("lentils.png", draw(LENTILS)),
            ("stew.png", draw(STEW)),
        ])
        .await;
    assert_eq!((stored.new_media, stored.food_images), (3, 1), "{stored:?}");

    // The demo version's ingredient shows its food's picture, and both households are served it.
    let version = street.version(tacos, next_door).await;
    let beans = uuid(&picture(&version, pictured));
    assert_eq!(beans, street.stored("beans.png").await);
    for household in [home, next_door] {
        street
            .assert_served(beans, household, "a food's picture")
            .await;
    }

    // Home's own photo of an ingredient didn't become the food's picture, so no one is shown it or served it:
    // foods are everyone's, and what pictures them comes only from versions no household owns.
    assert_eq!(picture(&version, unpictured), Value::Null);
    let ours = street.version(stew, home).await;
    assert_eq!(picture(&ours, unpictured), Value::Null);
    let lentils = street.stored("lentils.png").await;
    for household in [home, next_door] {
        street
            .assert_not_found(lentils, household, "a household's own ingredient photo")
            .await;
    }
}
