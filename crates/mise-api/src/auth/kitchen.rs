//! Kitchen devices: a screen in the kitchen whose browser can't sign in comfortably or send headers (an Echo
//! Show). A household admin issues a kitchen token with a name and gets a one-time link; opening the link on the
//! device sets a long-lived cookie (`mise_kitchen`). A kitchen session reads the household's plan, dishes, versions
//! and photos, and nothing else: [`may`] is the whole list, checked in `require_auth` before any handler runs.
//!
//! The link and the cookie are separate random 256-bit secrets, stored only as SHA-256 hashes. Opening the link
//! clears its hash and sets the cookie's in one statement, so a link works once; one never opened stops working
//! after [`LINK_MINUTES`]. Revoking a token deletes its row, so the device's next request is refused.

use axum::{
    Extension, Json,
    extract::State,
    http::{HeaderValue, Method, StatusCode, header},
    response::{IntoResponse, Redirect, Response},
};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use super::{AuthConfig, CurrentUser, Via, new_secret, secret_hash};
use crate::{
    AppState,
    error::{ApiError, ApiResult},
    extract::{JsonBody, Path, Query},
};

/// The cookie a kitchen device holds.
pub const COOKIE: &str = "mise_kitchen";

/// How long a link waits to be opened. Long enough to walk to the kitchen and type it; short enough that a link
/// left in a message is soon worth nothing.
pub const LINK_MINUTES: i32 = 60;

/// How long the device keeps its cookie, the most browsers allow. Using the device renews it (at most hourly).
const COOKIE_DAYS: i64 = 400;

/// Longest name a kitchen token can have.
const MAX_NAME: usize = 100;

/// What a kitchen session may call, after the API's address: reading the household's plan and recipes, and
/// `/me`, which tells the web app it is a kitchen device (with no personal data). Everything else is refused,
/// however the route is written in the router: this list is checked first.
pub fn may(method: &Method, rest: &str) -> bool {
    if *method != Method::GET && *method != Method::HEAD {
        return false;
    }
    let Some(rest) = rest.strip_prefix('/') else {
        return false;
    };
    let segments: Vec<&str> = rest.split('/').collect();
    if segments.iter().any(|s| s.is_empty()) {
        return false;
    }
    matches!(
        segments.as_slice(),
        ["me"] | ["plan"] | ["dishes"] | ["dishes", _] | ["variants", _] | ["media", _, _]
    )
}

/// The kitchen session this cookie belongs to, if its token still exists, and whether to renew the cookie.
pub(super) async fn from_cookie(
    pool: &PgPool,
    secret: &str,
) -> Result<Option<(CurrentUser, bool)>, ApiError> {
    let hash = secret_hash(secret);
    let row: Option<(Uuid, Uuid)> =
        sqlx::query_as("SELECT id, household_id FROM kitchen_tokens WHERE token_hash = $1")
            .bind(&hash)
            .fetch_optional(pool)
            .await?;
    let Some((id, household_id)) = row else {
        return Ok(None);
    };
    let renewed = sqlx::query("UPDATE kitchen_tokens SET last_used_at = now() WHERE id = $1 AND (last_used_at IS NULL OR last_used_at < now() - interval '1 hour')")
        .bind(id)
        .execute(pool)
        .await?
        .rows_affected()
        > 0;
    Ok(Some((
        CurrentUser {
            // The token's id: a kitchen session has no account. `may` keeps it to routes that read by household.
            id,
            username: String::new(),
            is_admin: false,
            household_id,
            via: Via::Kitchen,
        },
        renewed,
    )))
}

pub fn cookie(secret: String, config: &AuthConfig) -> Cookie<'static> {
    Cookie::build((COOKIE, secret))
        .path("/")
        .http_only(true)
        .same_site(SameSite::Lax)
        .secure(config.secure_cookies())
        .max_age(time::Duration::days(COOKIE_DAYS))
        .build()
}

/// What `/me` answers a kitchen device: the device's own name, its household's id, and no username, email or
/// household name.
pub(super) async fn me(pool: &PgPool, id: Uuid) -> ApiResult<super::routes::Me> {
    let (name, household_id): (String, Uuid) =
        sqlx::query_as("SELECT name, household_id FROM kitchen_tokens WHERE id = $1")
            .bind(id)
            .fetch_optional(pool)
            .await?
            .ok_or(ApiError::Unauthorized)?;
    Ok(super::routes::Me {
        id,
        username: String::new(),
        display_name: name,
        email: None,
        is_admin: false,
        household: super::routes::Household {
            id: household_id,
            name: String::new(),
        },
        has_password: false,
        kitchen: true,
    })
}

#[derive(Serialize, ToSchema)]
pub struct KitchenToken {
    pub id: Uuid,
    pub name: String,
    pub created_at: DateTime<Utc>,
    /// When its link was opened on the device; null while the link waits.
    pub linked_at: Option<DateTime<Utc>>,
    /// When an unopened link stops working; null once it has been opened.
    pub link_expires_at: Option<DateTime<Utc>>,
    pub last_used_at: Option<DateTime<Utc>>,
}

#[derive(Deserialize, ToSchema)]
pub struct KitchenTokenRequest {
    /// What the device is called, such as "Kitchen Echo".
    pub name: String,
}

#[derive(Serialize, ToSchema)]
pub struct NewKitchenToken {
    /// Shown once: open it on the kitchen device within the hour. A full address when the install's
    /// PUBLIC_BASE_URL is set, else a path on this install.
    pub link: String,
    pub info: KitchenToken,
}

/// id, name, created, linked, link expires, last used.
type Row = (
    Uuid,
    String,
    DateTime<Utc>,
    Option<DateTime<Utc>>,
    Option<DateTime<Utc>>,
    Option<DateTime<Utc>>,
);

fn info((id, name, created_at, linked_at, link_expires_at, last_used_at): Row) -> KitchenToken {
    KitchenToken {
        id,
        name,
        created_at,
        linked_at,
        link_expires_at,
        last_used_at,
    }
}

/// The household's kitchen tokens (admins only).
#[utoipa::path(get, path = "/api/v1/kitchen-tokens", operation_id = "list_kitchen_tokens", tag = "kitchen",
    responses((status = 200, description = "The household's kitchen tokens", body = [KitchenToken]), (status = 403, description = "Not an admin", body = crate::error::ErrorBody)))]
pub async fn list(
    State(s): State<AppState>,
    Extension(u): Extension<CurrentUser>,
) -> ApiResult<Json<Vec<KitchenToken>>> {
    u.require_admin()?;
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT id, name, created_at, linked_at, link_expires_at, last_used_at FROM kitchen_tokens
         WHERE household_id = $1 ORDER BY created_at, id",
    )
    .bind(u.household_id)
    .fetch_all(&s.pool)
    .await?;
    Ok(Json(rows.into_iter().map(info).collect()))
}

/// Issue a kitchen token for the household (admins only, from a browser session). Answers with its one-time
/// link, shown only this once.
#[utoipa::path(post, path = "/api/v1/kitchen-tokens", operation_id = "create_kitchen_token", tag = "kitchen", request_body = KitchenTokenRequest,
    responses((status = 200, description = "The new token and its one-time link", body = NewKitchenToken), (status = 400, description = "No name, or one over 100 characters", body = crate::error::ErrorBody), (status = 403, description = "Not an admin, or not from a browser session", body = crate::error::ErrorBody)))]
pub async fn create(
    State(s): State<AppState>,
    Extension(u): Extension<CurrentUser>,
    JsonBody(r): JsonBody<KitchenTokenRequest>,
) -> ApiResult<Json<NewKitchenToken>> {
    super::routes::session_only(&u)?;
    u.require_admin()?;
    let name = r.name.trim().to_string();
    if name.is_empty() {
        return Err(ApiError::BadRequest("give the device a name".into()));
    }
    if name.chars().count() > MAX_NAME {
        return Err(ApiError::BadRequest(format!(
            "names can have at most {MAX_NAME} characters"
        )));
    }
    let secret = new_secret();
    let row: Row = sqlx::query_as(
        "INSERT INTO kitchen_tokens (id, household_id, created_by, name, link_hash, link_expires_at)
         VALUES ($1, $2, $3, $4, $5, now() + make_interval(mins => $6))
         RETURNING id, name, created_at, linked_at, link_expires_at, last_used_at",
    )
    .bind(Uuid::now_v7())
    .bind(u.household_id)
    .bind(u.id)
    .bind(&name)
    .bind(secret_hash(&secret))
    .bind(LINK_MINUTES)
    .fetch_one(&s.pool)
    .await?;
    let path = format!("{}/auth/kitchen?code={secret}", crate::API);
    let link = match &s.auth.config.public_base_url {
        Some(base) => base
            .join(&path)
            .map_err(|e| ApiError::Internal(e.into()))?
            .to_string(),
        None => path,
    };
    Ok(Json(NewKitchenToken {
        link,
        info: info(row),
    }))
}

/// Revoke one of the household's kitchen tokens (admins only). The device is refused from its next request.
#[utoipa::path(delete, path = "/api/v1/kitchen-tokens/{id}", operation_id = "delete_kitchen_token", tag = "kitchen", params(("id" = Uuid, Path)),
    responses((status = 204, description = "Revoked"), (status = 403, description = "Not an admin", body = crate::error::ErrorBody), (status = 404, description = "No such token in this household", body = crate::error::ErrorBody)))]
pub async fn delete(
    State(s): State<AppState>,
    Extension(u): Extension<CurrentUser>,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    u.require_admin()?;
    let n = sqlx::query("DELETE FROM kitchen_tokens WHERE id = $1 AND household_id = $2")
        .bind(id)
        .bind(u.household_id)
        .execute(&s.pool)
        .await?
        .rows_affected();
    if n == 0 {
        Err(ApiError::NotFound)
    } else {
        Ok(StatusCode::NO_CONTENT)
    }
}

#[derive(Deserialize, IntoParams)]
pub struct LinkParams {
    /// The link's one-time code.
    pub code: Option<String>,
}

/// Open a kitchen token's one-time link: sets the device's cookie and goes to the web app. A link works once and
/// for an hour; a used, expired or unknown one goes to the sign-in page with an explanation. A browser signed in to
/// an account stays signed in, and its account comes first until it signs out.
#[utoipa::path(get, path = "/api/v1/auth/kitchen", operation_id = "open_kitchen_link", tag = "kitchen", params(LinkParams),
    responses((status = 303, description = "To the web app with the kitchen cookie set, or to the sign-in page when the link doesn't work")))]
pub async fn open_link(
    State(s): State<AppState>,
    jar: CookieJar,
    Query(p): Query<LinkParams>,
) -> Response {
    let secret = new_secret();
    let opened: Result<Option<Uuid>, sqlx::Error> = match p.code.as_deref() {
        Some(code) if !code.is_empty() => {
            sqlx::query_scalar(
                "UPDATE kitchen_tokens SET link_hash = NULL, link_expires_at = NULL, token_hash = $2, linked_at = now()
                 WHERE link_hash = $1 AND link_expires_at > now() RETURNING id",
            )
            .bind(secret_hash(code))
            .bind(secret_hash(&secret))
            .fetch_optional(&s.pool)
            .await
        }
        _ => Ok(None),
    };
    let mut res = match opened {
        Ok(Some(id)) => {
            tracing::info!(%id, "kitchen link opened");
            (jar.add(cookie(secret, &s.auth.config)), Redirect::to("/")).into_response()
        }
        Ok(None) => super::oidc::to_login(
            "That kitchen link has been used or has expired. An admin can make a new one on the account page.",
        ),
        Err(e) => return ApiError::from(e).into_response(),
    };
    // The address held the code: keep it out of caches and out of the Referer of what the page loads next.
    let headers = res.headers_mut();
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    res
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kitchen_reads_the_plan_and_recipes_only() {
        for path in [
            "/me",
            "/plan",
            "/dishes",
            "/dishes/tomato-soup",
            "/variants/0190",
            "/media/0190/web",
        ] {
            assert!(may(&Method::GET, path), "GET {path}");
            assert!(may(&Method::HEAD, path), "HEAD {path}");
            for m in [Method::POST, Method::PUT, Method::PATCH, Method::DELETE] {
                assert!(!may(&m, path), "{m} {path}");
            }
        }
        for path in [
            "",
            "/",
            "/me/",
            "/me/tokens",
            "/me/password",
            "/plan/entries",
            "/shopping",
            "/shopping/settings",
            "/users",
            "/households",
            "/kitchen-tokens",
            "/dishes/",
            "/dishes//",
            "/dishes/a/b",
            "/media/a",
            "/media/a/b/c",
            "/variants",
            "plan",
            "/PLAN",
        ] {
            assert!(!may(&Method::GET, path), "GET {path}");
        }
    }
}
