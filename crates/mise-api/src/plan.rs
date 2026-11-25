//! The household's weekly plan: dishes with a chosen version and serving count.

use axum::{Extension, Json, extract::State};
use serde::{Deserialize, Serialize};
use time::{Date, OffsetDateTime};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

mod queries;

use crate::{
    AppState,
    auth::CurrentUser,
    error::{ApiError, ApiResult},
    extract::{JsonBody, Path, Query},
    media::Image,
    variants::max_servings,
};

/// Which week to answer for.
#[derive(Deserialize, IntoParams)]
pub struct WeekParams {
    /// Any day in the week (default: today).
    pub week: Option<Date>,
}

/// The Monday of the week holding `day`, today when none is given. Weeks are stored by their Monday.
pub fn monday(day: Option<Date>) -> Date {
    let d = day.unwrap_or_else(|| OffsetDateTime::now_utc().date());
    d - time::Duration::days(d.weekday().number_days_from_monday().into())
}

/// One dish planned for the week, at a serving count.
#[derive(Serialize, ToSchema)]
pub struct PlanEntry {
    /// The entry's id, to change or remove it.
    pub id: Uuid,
    /// The version planned: `/api/v1/variants/{id}` shows it.
    pub variant_id: Uuid,
    /// Its dish's slug, for `/api/v1/dishes/{slug}`.
    pub dish_slug: Option<String>,
    /// The version's title.
    pub title: String,
    /// The version's subtitle ("with Lemon Rice").
    pub subtitle: Option<String>,
    /// How many it is planned for.
    pub servings: f64,
    /// Serving counts with published amounts.
    pub yields: Vec<f64>,
    /// The version's photo.
    pub image: Option<Image>,
}

/// A household's plan for one week.
#[derive(Serialize, ToSchema)]
pub struct Plan {
    /// The Monday.
    pub week: Date,
    /// The dishes planned, in the order they were added.
    pub entries: Vec<PlanEntry>,
}

/// A week's plan, with only the versions the household can see: one it can no longer see drops out rather than
/// showing another household's recipe.
async fn load(s: &AppState, household: Uuid, week: Date) -> ApiResult<Plan> {
    let entries = queries::entries(&s.pool, household, week)
        .await?
        .into_iter()
        .map(|e| PlanEntry {
            id: e.id,
            variant_id: e.variant_id,
            dish_slug: e.dish_slug,
            title: e.title,
            subtitle: e.subtitle,
            servings: e.servings,
            yields: e.yields,
            image: e.hero.map(Image::new),
        })
        .collect();
    Ok(Plan { week, entries })
}

/// Refuses a serving count the version can't be shown at: at most 24, or its largest published yield when that is
/// more (a 40-serving roast is planned at 40).
fn check_servings(n: f64, yields: &[f64]) -> ApiResult<()> {
    let max = max_servings(yields);
    if n > 0.0 && n <= max {
        Ok(())
    } else {
        Err(ApiError::BadRequest(format!(
            "servings must be more than 0 and at most {max}"
        )))
    }
}

/// A week's plan.
#[utoipa::path(get, path = "/api/v1/plan", operation_id = "get_plan", tag = "plan", params(WeekParams),
    responses(
        (status = 200, description = "The week's plan", body = Plan),
        (status = 400, description = "`week` isn't a date", body = crate::error::ErrorBody),
        crate::error::SignedIn
    )
)]
pub async fn get(
    State(s): State<AppState>,
    Extension(me): Extension<CurrentUser>,
    Query(p): Query<WeekParams>,
) -> ApiResult<Json<Plan>> {
    Ok(Json(load(&s, me.household_id, monday(p.week)).await?))
}

/// A dish to add to a week's plan.
#[derive(Deserialize, ToSchema)]
pub struct NewEntry {
    /// Any day in the week (default: today).
    pub week: Option<Date>,
    /// The version to plan.
    pub variant_id: Uuid,
    /// Default: 2 where published, else the smallest published size.
    pub servings: Option<f64>,
}

/// Add a dish to a week. Answers with the week's plan; a version the household can't see is not found.
#[utoipa::path(
    post, path = "/api/v1/plan/entries", operation_id = "add_plan_entry", tag = "plan", request_body = NewEntry,
    responses(
        (status = 200, description = "The week's plan with the dish added", body = Plan),
        (status = 400, description = "The serving count is out of range, or the body isn't JSON of the right shape", body = crate::error::ErrorBody),
        (status = 404, description = "No such version, or not one this household can see", body = crate::error::ErrorBody),
        crate::error::SignedIn,
        crate::error::ReadOnlyToken,
        crate::error::TooLarge
    )
)]
pub async fn add(
    State(s): State<AppState>,
    Extension(me): Extension<CurrentUser>,
    JsonBody(b): JsonBody<NewEntry>,
) -> ApiResult<Json<Plan>> {
    let yields = queries::version_yields(&s.pool, b.variant_id, me.household_id)
        .await?
        .ok_or(ApiError::NotFound)?;
    let servings = b.servings.unwrap_or_else(|| {
        if yields.contains(&2.0) {
            2.0
        } else {
            yields.first().copied().unwrap_or(2.0)
        }
    });
    check_servings(servings, &yields)?;
    let week = monday(b.week);
    queries::add(
        &s.pool,
        me.household_id,
        week,
        b.variant_id,
        servings,
        me.id,
    )
    .await?;
    Ok(Json(load(&s, me.household_id, week).await?))
}

/// A new serving count for a planned dish.
#[derive(Deserialize, ToSchema)]
pub struct EntryChange {
    /// How many to plan it for: more than 0, and at most 24, or the version's largest published yield when that is
    /// more.
    pub servings: f64,
}

/// Change a planned dish's serving count. Answers with the week's plan.
#[utoipa::path(
    patch, path = "/api/v1/plan/entries/{id}", operation_id = "update_plan_entry", tag = "plan", params(("id" = Uuid, Path)), request_body = EntryChange,
    responses(
        (status = 200, description = "The week's plan", body = Plan),
        (status = 400, description = "The id isn't a UUID, the serving count is out of range, or the body isn't JSON of the right shape", body = crate::error::ErrorBody),
        (status = 404, description = "No such entry in this household's plan", body = crate::error::ErrorBody),
        crate::error::SignedIn,
        crate::error::ReadOnlyToken,
        crate::error::TooLarge
    )
)]
pub async fn update(
    State(s): State<AppState>,
    Extension(me): Extension<CurrentUser>,
    Path(id): Path<Uuid>,
    JsonBody(b): JsonBody<EntryChange>,
) -> ApiResult<Json<Plan>> {
    let yields = queries::entry_yields(&s.pool, id, me.household_id)
        .await?
        .ok_or(ApiError::NotFound)?;
    check_servings(b.servings, &yields)?;
    let week = queries::set_servings(&s.pool, id, me.household_id, b.servings)
        .await?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(load(&s, me.household_id, week).await?))
}

/// Take a dish off the plan. Answers with the week's plan.
#[utoipa::path(
    delete, path = "/api/v1/plan/entries/{id}", operation_id = "delete_plan_entry", tag = "plan", params(("id" = Uuid, Path)),
    responses(
        (status = 200, description = "The week's plan without it", body = Plan),
        (status = 400, description = "The id isn't a UUID", body = crate::error::ErrorBody),
        (status = 404, description = "No such entry in this household's plan", body = crate::error::ErrorBody),
        crate::error::SignedIn,
        crate::error::ReadOnlyToken
    )
)]
pub async fn remove(
    State(s): State<AppState>,
    Extension(me): Extension<CurrentUser>,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Plan>> {
    let week = queries::remove(&s.pool, id, me.household_id)
        .await?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(load(&s, me.household_id, week).await?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weeks_start_on_monday() {
        use time::macros::date;
        assert_eq!(monday(Some(date!(2026 - 11 - 01))), date!(2026 - 10 - 26)); // a Sunday
        assert_eq!(monday(Some(date!(2026 - 11 - 02))), date!(2026 - 11 - 02)); // a Monday
        assert_eq!(monday(Some(date!(2026 - 01 - 01))), date!(2025 - 12 - 29)); // across a year
    }
}
