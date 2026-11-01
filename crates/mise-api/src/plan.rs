//! The household's weekly plan: dishes with a chosen version and serving count.

use axum::{Extension, Json, extract::State};
use chrono::{Datelike, Days, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::{
    AppState,
    auth::CurrentUser,
    error::{ApiError, ApiResult},
    extract::{JsonBody, Path, Query},
    media::Image,
    variants::MAX_SERVINGS,
};

#[derive(Deserialize, IntoParams)]
pub struct WeekParams {
    /// Any day in the week (default: today).
    pub week: Option<NaiveDate>,
}

/// The Monday of the week holding `day`, today when none is given. Weeks are stored by their Monday.
pub fn monday(day: Option<NaiveDate>) -> NaiveDate {
    let d = day.unwrap_or_else(|| Utc::now().date_naive());
    d - Days::new(d.weekday().num_days_from_monday().into())
}

#[derive(Serialize, ToSchema)]
pub struct PlanEntry {
    pub id: Uuid,
    pub variant_id: Uuid,
    pub dish_slug: Option<String>,
    pub title: String,
    pub subtitle: Option<String>,
    pub servings: f64,
    /// Serving counts with published amounts.
    pub yields: Vec<f64>,
    pub image: Option<Image>,
}

#[derive(Serialize, ToSchema)]
pub struct Plan {
    /// The Monday.
    pub week: NaiveDate,
    pub entries: Vec<PlanEntry>,
}

type EntryRow = (Uuid, Uuid, Option<String>, String, Option<String>, f64, Vec<f64>, Option<Uuid>);

async fn load(s: &AppState, household: Uuid, week: NaiveDate) -> ApiResult<Plan> {
    let rows: Vec<EntryRow> = sqlx::query_as(
        "SELECT e.id, e.variant_id, d.slug, v.title, v.subtitle, e.servings::float8,
                ARRAY(SELECT y.servings::float8 FROM variant_yields y WHERE y.variant_id = v.id ORDER BY y.servings),
                (SELECT vm.media_id FROM variant_media vm WHERE vm.variant_id = v.id AND vm.role = 'hero' LIMIT 1)
         FROM plan_entries e JOIN variants v ON v.id = e.variant_id LEFT JOIN dishes d ON d.id = v.dish_id
         WHERE e.household_id = $1 AND e.week = $2 ORDER BY e.created_at, e.id",
    )
    .bind(household)
    .bind(week)
    .fetch_all(&s.pool)
    .await?;
    let entries = rows
        .into_iter()
        .map(|(id, variant_id, dish_slug, title, subtitle, servings, yields, hero)| PlanEntry {
            id,
            variant_id,
            dish_slug,
            title,
            subtitle,
            servings,
            yields,
            image: hero.map(Image::new),
        })
        .collect();
    Ok(Plan { week, entries })
}

fn check_servings(n: f64) -> ApiResult<()> {
    if n > 0.0 && n <= MAX_SERVINGS { Ok(()) } else { Err(ApiError::BadRequest(format!("servings must be more than 0 and at most {MAX_SERVINGS}"))) }
}

/// A week's plan.
#[utoipa::path(get, path = "/api/plan", operation_id = "get_plan", tag = "plan", params(WeekParams), responses((status = 200, body = Plan)))]
pub async fn get(State(s): State<AppState>, Extension(me): Extension<CurrentUser>, Query(p): Query<WeekParams>) -> ApiResult<Json<Plan>> {
    Ok(Json(load(&s, me.household_id, monday(p.week)).await?))
}

#[derive(Deserialize, ToSchema)]
pub struct NewEntry {
    /// Any day in the week (default: today).
    pub week: Option<NaiveDate>,
    pub variant_id: Uuid,
    /// Default: 2 where published, else the smallest published size.
    pub servings: Option<f64>,
}

/// Add a dish to a week. Answers with the week's plan.
#[utoipa::path(
    post, path = "/api/plan/entries", operation_id = "add_plan_entry", tag = "plan", request_body = NewEntry,
    responses((status = 200, body = Plan), (status = 404, body = crate::error::ErrorBody))
)]
pub async fn add(State(s): State<AppState>, Extension(me): Extension<CurrentUser>, JsonBody(b): JsonBody<NewEntry>) -> ApiResult<Json<Plan>> {
    let yields: Option<(Vec<f64>,)> = sqlx::query_as(
        "SELECT ARRAY(SELECT y.servings::float8 FROM variant_yields y WHERE y.variant_id = v.id ORDER BY y.servings) FROM variants v WHERE v.id = $1",
    )
    .bind(b.variant_id)
    .fetch_optional(&s.pool)
    .await?;
    let (yields,) = yields.ok_or(ApiError::NotFound)?;
    let servings = b.servings.unwrap_or_else(|| if yields.contains(&2.0) { 2.0 } else { yields.first().copied().unwrap_or(2.0) });
    check_servings(servings)?;
    let week = monday(b.week);
    sqlx::query("INSERT INTO plan_entries (id, household_id, week, variant_id, servings, added_by) VALUES ($1, $2, $3, $4, $5::float8::numeric, $6)")
        .bind(Uuid::now_v7())
        .bind(me.household_id)
        .bind(week)
        .bind(b.variant_id)
        .bind(servings)
        .bind(me.id)
        .execute(&s.pool)
        .await?;
    Ok(Json(load(&s, me.household_id, week).await?))
}

#[derive(Deserialize, ToSchema)]
pub struct EntryChange {
    pub servings: f64,
}

/// Change a planned dish's serving count. Answers with the week's plan.
#[utoipa::path(
    patch, path = "/api/plan/entries/{id}", operation_id = "update_plan_entry", tag = "plan", params(("id" = Uuid, Path)), request_body = EntryChange,
    responses((status = 200, body = Plan), (status = 404, body = crate::error::ErrorBody))
)]
pub async fn update(State(s): State<AppState>, Extension(me): Extension<CurrentUser>, Path(id): Path<Uuid>, JsonBody(b): JsonBody<EntryChange>) -> ApiResult<Json<Plan>> {
    check_servings(b.servings)?;
    let week: NaiveDate = sqlx::query_scalar("UPDATE plan_entries SET servings = $3::float8::numeric WHERE id = $1 AND household_id = $2 RETURNING week")
        .bind(id)
        .bind(me.household_id)
        .bind(b.servings)
        .fetch_optional(&s.pool)
        .await?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(load(&s, me.household_id, week).await?))
}

/// Take a dish off the plan. Answers with the week's plan.
#[utoipa::path(
    delete, path = "/api/plan/entries/{id}", operation_id = "delete_plan_entry", tag = "plan", params(("id" = Uuid, Path)),
    responses((status = 200, body = Plan), (status = 404, body = crate::error::ErrorBody))
)]
pub async fn remove(State(s): State<AppState>, Extension(me): Extension<CurrentUser>, Path(id): Path<Uuid>) -> ApiResult<Json<Plan>> {
    let week: NaiveDate = sqlx::query_scalar("DELETE FROM plan_entries WHERE id = $1 AND household_id = $2 RETURNING week")
        .bind(id)
        .bind(me.household_id)
        .fetch_optional(&s.pool)
        .await?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(load(&s, me.household_id, week).await?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weeks_start_on_monday() {
        let d = |y, m, dd| NaiveDate::from_ymd_opt(y, m, dd);
        assert_eq!(monday(d(2026, 11, 1)), d(2026, 10, 26).unwrap()); // a Sunday
        assert_eq!(monday(d(2026, 11, 2)), d(2026, 11, 2).unwrap()); // a Monday
        assert_eq!(monday(d(2026, 1, 1)), d(2025, 12, 29).unwrap()); // across a year
    }
}
