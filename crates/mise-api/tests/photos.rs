//! Photos through the router: a version's photo linked from search, the dish page and the version, served at each
//! size, and the answers when one is missing. The photo is drawn by the test, stored by `mise-ingest`'s media
//! passes into a media folder of the test's own (removed when it ends), and given to the newest Black Bean Tacos.

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
        let photo = draw();
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

/// An opaque PNG drawn here (a colour gradient), so the repository holds no photo and the web copy is a JPEG.
fn draw() -> Vec<u8> {
    let img = RgbImage::from_fn(WIDTH, HEIGHT, |x, y| {
        Rgb([(x * 255 / WIDTH) as u8, (y * 255 / HEIGHT) as u8, 120])
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
            "public, max-age=31536000, immutable",
            "a media id never changes its bytes"
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
