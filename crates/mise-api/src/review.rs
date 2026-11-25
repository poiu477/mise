//! Review queues: what the import pipeline left for a person to decide (`review_items`), answered from the web
//! app by an admin. Three kinds are asked here: a food matched to a name that could mean more than one food, a
//! food that may be a kind of another, and a copied version too unlike its original to join its dish.
//!
//! A decision is stored exactly as the `mise` commands store it, so re-running a stage keeps it: a food decision is
//! a curation operation (`mise_ingest::curate`), which makes the food a person's own (`origin = 'user'`), and a dish
//! decision is a row in `dish_decisions` (`mise_ingest::dish_decisions`). `mise food export` and `mise dish export`
//! write them back out as curation files. The operation and the item's answer commit together.

use axum::{Extension, Json, extract::State};
use mise_ingest::curate::{self, Op, Refused};
use mise_ingest::dish_decisions::{self, Decision, Record};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{Postgres, Transaction};
use std::collections::HashMap;
use time::OffsetDateTime;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::{
    AppState,
    auth::CurrentUser,
    error::{ApiError, ApiResult},
    extract::{JsonBody, Path, Query},
};

/// What a review item asks.
#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReviewKind {
    /// A name that could mean more than one food was given a food of its own: is it right? ("pepper": black or
    /// bell?)
    FoodMatch,
    /// A new food may be a kind of another: "Garden Tomato" under "Tomato".
    FoodParent,
    /// A copied version is too unlike its original to join its dish without asking.
    DishMerge,
}

impl ReviewKind {
    /// As `review_items.kind` stores it.
    fn as_str(self) -> &'static str {
        match self {
            ReviewKind::FoodMatch => "food_match",
            ReviewKind::FoodParent => "food_parent",
            ReviewKind::DishMerge => "dish_merge",
        }
    }

    fn from_str(kind: &str) -> Option<ReviewKind> {
        [
            ReviewKind::FoodMatch,
            ReviewKind::FoodParent,
            ReviewKind::DishMerge,
        ]
        .into_iter()
        .find(|k| k.as_str() == kind)
    }

    /// The answers an item of this kind takes.
    fn verdicts(self) -> Vec<Verdict> {
        match self {
            ReviewKind::FoodMatch => vec![Verdict::Accept, Verdict::Edit],
            ReviewKind::FoodParent => vec![Verdict::Accept, Verdict::Reject, Verdict::Edit],
            ReviewKind::DishMerge => vec![Verdict::Accept, Verdict::Reject],
        }
    }
}

/// An answer to a review item.
#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// The proposal is right: keep the food as matched, make it a kind of the proposed food, join the two dishes.
    Accept,
    /// The proposal is wrong: the food stays where it is, the dishes stay apart (and aren't proposed again).
    Reject,
    /// Something else is right: the food is a kind of `parent`, or the same food as `same_as`.
    Edit,
}

/// One dish of a doubtful merge.
#[derive(Serialize, ToSchema)]
pub struct DishRef {
    /// The version's title, cleaned as dishes are named: "Pepper Steak Skillet".
    pub title: String,
    /// The address of the dish it is in now, when it still is in one.
    pub dish_slug: Option<String>,
}

/// The two sides of a doubtful dish merge, and how alike grouping found them.
#[derive(Serialize, ToSchema)]
pub struct DishPair {
    /// The copy.
    pub version: DishRef,
    /// The version it was copied from, and its dish.
    pub copied_from: DishRef,
    /// How alike the titles are, 0 to 1.
    pub title_similarity: f64,
    /// How many of the shipped ingredients they share, 0 to 1.
    pub shipped_overlap: f64,
}

/// One open item in the queue.
#[derive(Serialize, ToSchema)]
pub struct ReviewItem {
    /// The item's id, to answer it at `/api/v1/review-items/{id}/decision`.
    pub id: Uuid,
    /// What the item asks: a food match, a food's parent, or a dish merge.
    pub kind: ReviewKind,
    /// What it asks about: the food's name, or the copied version's title.
    pub subject: String,
    /// What the pipeline proposes, as a sentence: "Garden Tomato is a kind of Tomato".
    pub proposal: String,
    /// Why it asks, when the pipeline said: "black pepper or bell pepper (as a pantry item it is usually black
    /// pepper)".
    pub note: Option<String>,
    /// How sure the pipeline was, 0 to 1; items it was least sure of, or that touch few recipes, are low.
    pub confidence: Option<f32>,
    /// For a food: how many recipe ingredient lines use it.
    pub recipe_uses: Option<i64>,
    /// For a dish merge: the two dishes.
    pub dishes: Option<DishPair>,
    /// The answers this item takes.
    pub verdicts: Vec<Verdict>,
    /// When the import pipeline raised it.
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}

/// Every open item, food matches first, then food parents, then dish merges; the surest first within each.
#[derive(Serialize, ToSchema)]
pub struct ReviewQueue {
    /// The open items.
    pub items: Vec<ReviewItem>,
}

/// Which items to list.
#[derive(Deserialize, IntoParams)]
pub struct QueueParams {
    /// Only items of this kind (default: every kind).
    pub kind: Option<ReviewKind>,
}

type ItemRow = (Uuid, String, Value, Value, Option<f32>, OffsetDateTime);

/// A text field of a stored item's JSON.
fn text(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::to_string)
}

/// An id field of a stored item's JSON.
fn id(v: &Value, key: &str) -> Option<Uuid> {
    v.get(key).and_then(Value::as_str)?.parse().ok()
}

/// The open items of `kinds`, as the page shows them. Food and dish names are read as they are now, so an item
/// follows a renamed food or a regrouped dish.
async fn load(s: &AppState, kinds: &[ReviewKind]) -> ApiResult<ReviewQueue> {
    let names: Vec<&str> = kinds.iter().map(|k| k.as_str()).collect();
    let rows: Vec<ItemRow> = sqlx::query_as(
        "SELECT id, kind, subject, proposal, confidence, created_at FROM review_items
         WHERE status = 'open' AND kind = ANY($1)
         ORDER BY array_position($1, kind), confidence DESC NULLS LAST, created_at, id",
    )
    .bind(&names)
    .fetch_all(&s.pool)
    .await?;

    let food_ids: Vec<Uuid> = rows
        .iter()
        .flat_map(|r| [id(&r.2, "food_id"), id(&r.3, "parent_id")])
        .flatten()
        .collect();
    let foods: HashMap<Uuid, String> =
        sqlx::query_as::<_, (Uuid, String)>("SELECT id, name FROM foods WHERE id = ANY($1)")
            .bind(&food_ids)
            .fetch_all(&s.pool)
            .await?
            .into_iter()
            .collect();
    let dish_ids: Vec<Uuid> = rows
        .iter()
        .flat_map(|r| [id(&r.2, "dish_id"), id(&r.3, "merge_into_dish")])
        .flatten()
        .collect();
    let dishes: HashMap<Uuid, String> =
        sqlx::query_as::<_, (Uuid, String)>("SELECT id, slug FROM dishes WHERE id = ANY($1)")
            .bind(&dish_ids)
            .fetch_all(&s.pool)
            .await?
            .into_iter()
            .collect();
    let food_name = |v: &Value, id_key: &str, name_key: &str| {
        id(v, id_key)
            .and_then(|f| foods.get(&f).cloned())
            .or_else(|| text(v, name_key))
            .unwrap_or_default()
    };

    let mut items = Vec::new();
    for (item_id, kind, subject, proposal, confidence, created_at) in rows {
        let Some(kind) = ReviewKind::from_str(&kind) else {
            continue;
        };
        let (name, sentence, note, recipe_uses, pair) = match kind {
            ReviewKind::FoodMatch | ReviewKind::FoodParent => {
                let name = food_name(&subject, "food_id", "name");
                let sentence = if kind == ReviewKind::FoodParent {
                    let parent = food_name(&proposal, "parent_id", "parent_name");
                    format!("{name} is a kind of {parent}")
                } else {
                    format!("{name} is a food of its own")
                };
                let uses = subject.get("recipe_uses").and_then(Value::as_i64);
                (name, sentence, text(&proposal, "note"), uses, None)
            }
            ReviewKind::DishMerge => {
                let title = text(&subject, "title").unwrap_or_default();
                let original = text(&proposal, "copied_from_title").unwrap_or_default();
                let slug = |v: &Value, key: &str| id(v, key).and_then(|d| dishes.get(&d).cloned());
                let number = |key: &str| proposal.get(key).and_then(Value::as_f64).unwrap_or(0.0);
                let pair = DishPair {
                    version: DishRef {
                        title: title.clone(),
                        dish_slug: slug(&subject, "dish_id"),
                    },
                    copied_from: DishRef {
                        title: original.clone(),
                        dish_slug: slug(&proposal, "merge_into_dish"),
                    },
                    title_similarity: number("title_similarity"),
                    shipped_overlap: number("shipped_overlap"),
                };
                let sentence = format!("{title} is a version of {original}");
                (title, sentence, None, None, Some(pair))
            }
        };
        items.push(ReviewItem {
            id: item_id,
            kind,
            subject: name,
            proposal: sentence,
            note,
            confidence,
            recipe_uses,
            dishes: pair,
            verdicts: kind.verdicts(),
            created_at,
        });
    }
    Ok(ReviewQueue { items })
}

const EVERY_KIND: [ReviewKind; 3] = [
    ReviewKind::FoodMatch,
    ReviewKind::FoodParent,
    ReviewKind::DishMerge,
];

/// The open review items (admins only).
#[utoipa::path(get, path = "/api/v1/review-items", operation_id = "list_review_items", tag = "review", params(QueueParams),
    responses(
        (status = 200, description = "The open items", body = ReviewQueue),
        (status = 400, description = "`kind` isn't one of the kinds asked here", body = crate::error::ErrorBody),
        (status = 403, description = "Not an admin, or called from a kitchen device", body = crate::error::ErrorBody),
        crate::error::SignedIn
    )
)]
pub async fn list(
    State(s): State<AppState>,
    Extension(me): Extension<CurrentUser>,
    Query(p): Query<QueueParams>,
) -> ApiResult<Json<ReviewQueue>> {
    me.require_admin()?;
    let kinds = match p.kind {
        Some(k) => vec![k],
        None => EVERY_KIND.to_vec(),
    };
    Ok(Json(load(&s, &kinds).await?))
}

/// An answer to one item.
#[derive(Deserialize, ToSchema)]
pub struct ReviewDecision {
    /// The answer: one of the item's `verdicts`.
    pub decision: Verdict,
    /// With `edit` on a food: the food it is a kind of, by name ("Tomato").
    pub parent: Option<String>,
    /// With `edit` on a food: the food it is the same as, by name ("Black Pepper"); the two become one.
    pub same_as: Option<String>,
    /// Why, in a few words, kept with the decision: "renamed: same recipe".
    pub note: Option<String>,
}

/// Longest name or note a decision takes.
const MAX_TEXT: usize = 200;

/// `text` trimmed, refused when empty, too long, or holding a NUL (which Postgres can't store).
fn checked(text: Option<&str>, what: &str) -> ApiResult<Option<String>> {
    let Some(t) = text.map(str::trim) else {
        return Ok(None);
    };
    if t.is_empty() || t.chars().count() > MAX_TEXT || t.contains('\0') {
        return Err(ApiError::BadRequest(format!(
            "{what} must be 1 to {MAX_TEXT} characters"
        )));
    }
    Ok(Some(t.to_string()))
}

/// The record a version came from, as dish decisions name it; None for a version written in the app.
async fn record_of(tx: &mut Transaction<'_, Postgres>, variant: Uuid) -> ApiResult<Option<Record>> {
    Ok(sqlx::query_as::<_, (String, String)>(
        "SELECT s.key, sr.external_id FROM variants v JOIN source_records sr ON sr.id = v.source_record_id
         JOIN sources s ON s.id = sr.source_id WHERE v.id = $1",
    )
    .bind(variant)
    .fetch_optional(&mut **tx)
    .await?
    .map(|(source, external)| Record(source, external)))
}

/// The food a stored item names, by its slug (unique, unlike names), or refused when it is gone.
async fn food_slug(tx: &mut Transaction<'_, Postgres>, v: &Value, key: &str) -> ApiResult<String> {
    let found: Option<String> =
        sqlx::query_scalar!("SELECT slug FROM foods WHERE id = $1", id(v, key))
            .fetch_optional(&mut **tx)
            .await?;
    found.ok_or_else(|| {
        ApiError::BadRequest("a food this item names is gone; run match-foods again".into())
    })
}

/// A curation operation applied inside the transaction; one refused as asked (a name that matches no food, a
/// loop) is the person's to put right.
async fn curate_in(tx: &mut Transaction<'_, Postgres>, op: &Op) -> ApiResult<String> {
    curate::apply_in(tx, op)
        .await
        .map_err(|e| match e.downcast_ref::<Refused>() {
            Some(r) => ApiError::BadRequest(r.0.clone()),
            None => ApiError::Internal(e),
        })
}

/// Answer one item (admins only), stored as the `mise` commands store it. Answers with the queue as it now stands:
/// one food decision can answer other items about the same food.
#[utoipa::path(
    post, path = "/api/v1/review-items/{id}/decision", operation_id = "decide_review_item", tag = "review",
    params(("id" = Uuid, Path)), request_body = ReviewDecision,
    responses(
        (status = 200, description = "Decided; the open items left", body = ReviewQueue),
        (status = 400, description = "The id isn't a UUID; the answer isn't one this kind of item takes; a food named doesn't exist, or the change would make a loop; or the body isn't JSON of the right shape", body = crate::error::ErrorBody),
        (status = 403, description = "Not an admin, or called with a read-only API token or from a kitchen device", body = crate::error::ErrorBody),
        (status = 404, description = "No such item", body = crate::error::ErrorBody),
        (status = 409, description = "The item was decided already", body = crate::error::ErrorBody),
        crate::error::SignedIn,
        crate::error::TooLarge
    )
)]
pub async fn decide(
    State(s): State<AppState>,
    Extension(me): Extension<CurrentUser>,
    Path(item): Path<Uuid>,
    JsonBody(d): JsonBody<ReviewDecision>,
) -> ApiResult<Json<ReviewQueue>> {
    me.require_admin()?;
    let parent = checked(d.parent.as_deref(), "the parent's name")?;
    let same_as = checked(d.same_as.as_deref(), "the food's name")?;
    let note = checked(d.note.as_deref(), "a note")?;

    let mut tx = s.pool.begin().await?;
    let row: Option<(String, String, Value, Value)> = sqlx::query_as(
        "SELECT kind, status, subject, proposal FROM review_items WHERE id = $1 FOR UPDATE",
    )
    .bind(item)
    .fetch_optional(&mut *tx)
    .await?;
    let (kind, status, subject, proposal) = row.ok_or(ApiError::NotFound)?;
    let kind = ReviewKind::from_str(&kind).ok_or(ApiError::NotFound)?;
    if status != "open" {
        return Err(ApiError::Conflict("that item was decided already".into()));
    }
    if !kind.verdicts().contains(&d.decision) {
        return Err(ApiError::BadRequest(match kind {
            ReviewKind::FoodMatch => {
                "to say it's another food, edit it and name the food it's the same as".into()
            }
            _ => "a dish merge is accepted or rejected".into(),
        }));
    }

    let done = match kind {
        ReviewKind::FoodMatch | ReviewKind::FoodParent => {
            let food = food_slug(&mut tx, &subject, "food_id").await?;
            let op = match (d.decision, parent, same_as) {
                (Verdict::Accept, _, _) if kind == ReviewKind::FoodParent => Op::SetParent {
                    food,
                    parent: food_slug(&mut tx, &proposal, "parent_id").await?,
                },
                (Verdict::Accept | Verdict::Reject, _, _) => Op::Keep { food },
                (Verdict::Edit, Some(parent), None) => Op::SetParent { food, parent },
                (Verdict::Edit, None, Some(into)) => Op::Merge { from: food, into },
                (Verdict::Edit, _, _) => {
                    return Err(ApiError::BadRequest(
                        "name either the food it's a kind of, or the food it's the same as".into(),
                    ));
                }
            };
            curate_in(&mut tx, &op).await?;
            json!({ "operation": op })
        }
        ReviewKind::DishMerge => {
            let (Some(copy), Some(original)) = (
                id(&subject, "variant_id"),
                id(&proposal, "copied_from_variant"),
            ) else {
                return Err(ApiError::Internal(anyhow::anyhow!(
                    "dish_merge item {item} names no versions"
                )));
            };
            let (Some(a), Some(b)) = (
                record_of(&mut tx, copy).await?,
                record_of(&mut tx, original).await?,
            ) else {
                return Err(ApiError::BadRequest(
                    "one of these versions was written in the app, and only imported versions can be decided here"
                        .into(),
                ));
            };
            let decision = Decision {
                decision: if d.decision == Verdict::Accept {
                    "merge"
                } else {
                    "keep_apart"
                }
                .into(),
                a,
                b,
                by: "user".into(),
                note: note.clone(),
            };
            dish_decisions::record(&mut tx, &decision)
                .await
                .map_err(ApiError::Internal)?;
            json!({ "dish_decision": decision })
        }
    };

    let status = match d.decision {
        Verdict::Accept => "accepted",
        Verdict::Reject => "rejected",
        Verdict::Edit => "edited",
    };
    let mut decision = done;
    decision["by"] = json!(me.username);
    if let Some(n) = note {
        decision["note"] = json!(n);
    }
    sqlx::query!(
        "UPDATE review_items SET status = $2, decision = $3, decided_at = now() WHERE id = $1",
        item,
        status,
        decision
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(Json(load(&s, &EVERY_KIND).await?))
}
