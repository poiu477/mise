//! Old dish addresses keep working: grouping retires a slug a dish stops using (it was renamed, lost its number,
//! or joined another dish), and the dish page answers to it with the dish under its current slug. On the demo
//! recipes, regrouped after changing them the way a new import or a decision would.

#[macro_use]
mod common;

use axum::http::StatusCode;
use common::{As, TestApp};
use sqlx::PgPool;
use uuid::Uuid;

/// Group dishes again and derive their options, as the `mise` command does after an import.
async fn regroup(pool: &PgPool) {
    mise_ingest::dishes::group_dishes(pool)
        .await
        .expect("group dishes");
    mise_ingest::options::derive_options(pool)
        .await
        .expect("derive options");
}

/// The id of the dish with this slug, live (not retired).
async fn dish_id(pool: &PgPool, slug: &str) -> Uuid {
    sqlx::query_scalar("SELECT id FROM dishes WHERE slug = $1")
        .bind(slug)
        .fetch_one(pool)
        .await
        .unwrap_or_else(|e| panic!("no dish at {slug}: {e}"))
}

/// The slug a dish answers to now.
async fn slug_of(pool: &PgPool, dish: Uuid) -> String {
    sqlx::query_scalar("SELECT slug FROM dishes WHERE id = $1")
        .bind(dish)
        .fetch_one(pool)
        .await
        .unwrap()
}

/// The slugs retired for a dish, in order.
async fn retired(pool: &PgPool, dish: Uuid) -> Vec<String> {
    sqlx::query_scalar("SELECT slug FROM retired_slugs WHERE dish_id = $1 ORDER BY slug")
        .bind(dish)
        .fetch_all(pool)
        .await
        .unwrap()
}

/// Give every version of a dish a new title, as a new import of the same recipes would.
async fn rename(pool: &PgPool, dish: Uuid, title: &str) {
    sqlx::query("UPDATE variants SET title = $2 WHERE dish_id = $1")
        .bind(dish)
        .bind(title)
        .execute(pool)
        .await
        .unwrap();
}

/// A dish a person made, holding a slug so grouping has to number around it.
async fn hold_slug(pool: &PgPool, slug: &str) -> Uuid {
    let id = Uuid::now_v7();
    sqlx::query("INSERT INTO dishes (id, title, slug, origin) VALUES ($1, $2, $2, 'user')")
        .bind(id)
        .bind(slug)
        .execute(pool)
        .await
        .unwrap();
    id
}

#[tokio::test]
async fn a_renamed_dish_takes_its_new_slug_and_keeps_the_old_ones() {
    let app = test_app!(TestApp::with_demo());
    let pool = &app.pool;
    let tacos = dish_id(pool, "black-bean-tacos").await;

    // Renamed while its new name's slug is taken: it takes a numbered copy, and the old slug is retired.
    let holder = hold_slug(pool, "smoky-black-bean-tacos").await;
    rename(pool, tacos, "Smoky Black Bean Tacos").await;
    regroup(pool).await;
    assert_eq!(slug_of(pool, tacos).await, "smoky-black-bean-tacos-2");
    assert_eq!(retired(pool, tacos).await, ["black-bean-tacos"]);

    // The plain slug comes free: the numbered copy becomes it, as before, and is retired too.
    sqlx::query("DELETE FROM dishes WHERE id = $1")
        .bind(holder)
        .execute(pool)
        .await
        .unwrap();
    regroup(pool).await;
    assert_eq!(slug_of(pool, tacos).await, "smoky-black-bean-tacos");
    assert_eq!(
        retired(pool, tacos).await,
        ["black-bean-tacos", "smoky-black-bean-tacos-2"]
    );

    // Renamed back: the dish takes its first slug again, so that one is no longer retired.
    rename(pool, tacos, "Black Bean Tacos").await;
    regroup(pool).await;
    assert_eq!(slug_of(pool, tacos).await, "black-bean-tacos");
    assert_eq!(
        retired(pool, tacos).await,
        ["smoky-black-bean-tacos", "smoky-black-bean-tacos-2"]
    );

    // Another run on the same data changes nothing.
    regroup(pool).await;
    assert_eq!(slug_of(pool, tacos).await, "black-bean-tacos");
    assert_eq!(
        retired(pool, tacos).await,
        ["smoky-black-bean-tacos", "smoky-black-bean-tacos-2"]
    );
}

#[tokio::test]
async fn a_dish_that_joins_another_hands_on_its_slug() {
    let app = test_app!(TestApp::with_demo());
    let pool = &app.pool;
    let tacos = dish_id(pool, "black-bean-tacos").await;
    let bowl = dish_id(pool, "garlic-rice-bowl").await;

    // A person decides the rice bowl and the tacos are one dish; the tacos have more versions, so they keep theirs.
    let record = |slug: &'static str| async move {
        sqlx::query_as::<_, (String, String)>(
            "SELECT s.key, sr.external_id FROM dishes d JOIN variants v ON v.dish_id = d.id
             JOIN source_records sr ON sr.id = v.source_record_id JOIN sources s ON s.id = sr.source_id
             WHERE d.slug = $1 ORDER BY v.id LIMIT 1",
        )
        .bind(slug)
        .fetch_one(pool)
        .await
        .unwrap()
    };
    let (a, b) = (
        record("black-bean-tacos").await,
        record("garlic-rice-bowl").await,
    );
    sqlx::query(
        "INSERT INTO dish_decisions (a_source, a_external, b_source, b_external, decision, decided_by)
         VALUES ($1, $2, $3, $4, 'merge', 'user')",
    )
    .bind(&a.0)
    .bind(&a.1)
    .bind(&b.0)
    .bind(&b.1)
    .execute(pool)
    .await
    .unwrap();
    regroup(pool).await;

    assert_eq!(slug_of(pool, tacos).await, "black-bean-tacos");
    let gone: i64 = sqlx::query_scalar("SELECT count(*) FROM dishes WHERE id = $1")
        .bind(bowl)
        .fetch_one(pool)
        .await
        .unwrap();
    assert_eq!(gone, 0, "the rice bowl is part of the tacos now");
    assert_eq!(retired(pool, tacos).await, ["garlic-rice-bowl"]);
}

#[tokio::test]
async fn an_old_slug_answers_with_the_dish_under_its_current_one() {
    let app = test_app!(TestApp::with_demo());
    let cookie = app.setup_owner().await;
    let pool = &app.pool;
    let tacos = dish_id(pool, "black-bean-tacos").await;
    rename(pool, tacos, "Smoky Black Bean Tacos").await;
    regroup(pool).await;

    for slug in ["black-bean-tacos", "smoky-black-bean-tacos"] {
        let dish = app
            .json("GET", &format!("/api/v1/dishes/{slug}"), &cookie, None)
            .await;
        assert_eq!(dish["id"], tacos.to_string(), "{slug}");
        assert_eq!(dish["slug"], "smoky-black-bean-tacos", "{slug}");
        assert_eq!(dish["title"], "Smoky Black Bean Tacos", "{slug}");
        assert_eq!(dish["variants"].as_array().map(Vec::len), Some(2), "{slug}");
    }

    // A slug no dish ever had is still not found, and an old one is still private.
    let reply = app
        .send(
            "GET",
            "/api/v1/dishes/no-such-dish",
            As::Cookie(&cookie),
            None,
        )
        .await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
    let reply = app
        .send("GET", "/api/v1/dishes/black-bean-tacos", As::Nobody, None)
        .await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn a_live_dish_wins_over_a_retired_slug() {
    let app = test_app!(TestApp::with_demo());
    let cookie = app.setup_owner().await;
    let pool = &app.pool;
    let pasta = dish_id(pool, "weeknight-tomato-pasta").await;
    let tacos = dish_id(pool, "black-bean-tacos").await;
    // A stray row grouping would never leave: the tacos' live slug retired for the pasta.
    sqlx::query("INSERT INTO retired_slugs (slug, dish_id) VALUES ('black-bean-tacos', $1)")
        .bind(pasta)
        .execute(pool)
        .await
        .unwrap();

    let dish = app
        .json("GET", "/api/v1/dishes/black-bean-tacos", &cookie, None)
        .await;
    assert_eq!(dish["id"], tacos.to_string());

    // The next grouping run clears it.
    regroup(pool).await;
    assert!(retired(pool, pasta).await.is_empty());
}
