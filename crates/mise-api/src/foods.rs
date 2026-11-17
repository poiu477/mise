//! Foods, to pick from: the search a recipe's writer uses to say which food an ingredient line is.
//!
//! Foods are shared by the whole install, and matching makes one for any ingredient name it hasn't seen, so a food's
//! name can come from a recipe only one household can see. The search therefore offers a household the foods mise
//! was given or a person edited, and of the matched ones only those a version it can see uses.

use axum::{Extension, Json, extract::State};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use crate::{
    AppState,
    auth::CurrentUser,
    error::{ApiError, ApiResult},
    extract::Query,
    recipes::FoodRef,
};

/// The most foods one search answers with.
const MAX_LIMIT: i64 = 50;

#[derive(Deserialize, IntoParams)]
pub struct FoodParams {
    /// Part of the food's name, 1 to 100 characters: "onion", "parm". Small typos still match.
    pub q: String,
    /// How many to answer with (default 10, at most 50).
    pub limit: Option<i64>,
}

/// Foods matching a search, the closest names first.
#[derive(Serialize, ToSchema)]
pub struct FoodList {
    pub items: Vec<FoodRef>,
}

/// Search the foods by name, to pick the one an ingredient line means. Answers with the foods every household
/// knows and those this household's recipes use, never one known only from another household's recipe.
#[utoipa::path(
    get, path = "/api/v1/foods", operation_id = "search_foods", tag = "recipes", params(FoodParams),
    responses(
        (status = 200, description = "Matching foods, the closest names first", body = FoodList),
        (status = 400, description = "`q` is missing, empty or over 100 characters, or `limit` isn't a number", body = crate::error::ErrorBody),
        crate::error::SignedIn,
        crate::error::KitchenDevice
    )
)]
pub async fn search(
    State(s): State<AppState>,
    Extension(me): Extension<CurrentUser>,
    Query(p): Query<FoodParams>,
) -> ApiResult<Json<FoodList>> {
    let q = p.q.trim();
    if q.is_empty() || q.chars().count() > 100 {
        return Err(ApiError::BadRequest(
            "q must be between 1 and 100 characters".into(),
        ));
    }
    let items = sqlx::query_as::<_, (uuid::Uuid, String)>(
        "SELECT f.id, f.name FROM foods f
         WHERE f.kind <> 'category' AND (f.name ILIKE '%' || $2 || '%' OR f.name % $2)
           AND (f.origin <> 'matched' OR EXISTS (
                 SELECT 1 FROM ingredient_lines il JOIN variants v ON v.id = il.variant_id
                 WHERE il.food_id = f.id AND visible_to(v, $1)))
         ORDER BY f.name ILIKE $2 || '%' DESC, similarity(f.name, $2) DESC, f.name
         LIMIT $3",
    )
    .bind(me.household_id)
    .bind(q)
    .bind(p.limit.unwrap_or(10).clamp(1, MAX_LIMIT))
    .fetch_all(&s.pool)
    .await?
    .into_iter()
    .map(|(id, name)| FoodRef { id, name })
    .collect();
    Ok(Json(FoodList { items }))
}
