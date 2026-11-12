//! Sign-in, account and admin endpoints.

use axum::{
    Extension, Json,
    extract::State,
    http::{HeaderMap, StatusCode, header},
};
use axum_extra::extract::cookie::CookieJar;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use super::{CurrentUser, SESSION_COOKIE, TOKEN_PREFIX, Via};
use crate::{
    AppState,
    error::{ApiError, ApiResult},
    extract::{JsonBody, Path},
};

#[derive(Serialize, ToSchema)]
pub struct AuthInfo {
    /// Username and password sign-in is on.
    pub local: bool,
    /// Label of the single-sign-on button, when single sign-on is on.
    pub oidc: Option<String>,
    /// No account exists yet: the first visitor creates the admin account.
    pub setup_required: bool,
}

/// How this install signs people in.
#[utoipa::path(get, path = "/api/v1/auth/config", operation_id = "auth_config", tag = "auth", responses((status = 200, description = "How this install signs people in", body = AuthInfo)))]
pub async fn config(State(s): State<AppState>) -> ApiResult<Json<AuthInfo>> {
    let users: i64 = sqlx::query_scalar("SELECT count(*) FROM users")
        .fetch_one(&s.pool)
        .await?;
    Ok(Json(AuthInfo {
        local: s.auth.config.local,
        oidc: s.auth.config.oidc.as_ref().map(|o| o.name.clone()),
        setup_required: users == 0,
    }))
}

#[derive(Serialize, ToSchema, Clone)]
pub struct Household {
    pub id: Uuid,
    pub name: String,
}

#[derive(Serialize, ToSchema)]
pub struct Me {
    pub id: Uuid,
    pub username: String,
    pub display_name: String,
    pub email: Option<String>,
    pub is_admin: bool,
    pub household: Household,
    /// The account can sign in with a password (single-sign-on-only accounts cannot).
    pub has_password: bool,
    /// This is a kitchen device, not an account: `id` is its kitchen token's, `display_name` its name, and the
    /// username, email and household name are empty.
    pub kitchen: bool,
}

pub(crate) async fn load_me(pool: &sqlx::PgPool, id: Uuid) -> ApiResult<Me> {
    let (username, display_name, email, is_admin, has_password, hid, hname): (String, String, Option<String>, bool, bool, Uuid, String) =
        sqlx::query_as(
            "SELECT u.username, u.display_name, u.email, u.is_admin, u.password_hash IS NOT NULL, h.id, h.name
             FROM users u JOIN households h ON h.id = u.household_id WHERE u.id = $1",
        )
        .bind(id)
        .fetch_optional(pool)
        .await?
        .ok_or(ApiError::Unauthorized)?;
    Ok(Me {
        id,
        username,
        display_name,
        email,
        is_admin,
        household: Household {
            id: hid,
            name: hname,
        },
        has_password,
        kitchen: false,
    })
}

#[derive(Deserialize, ToSchema)]
pub struct SetupRequest {
    pub username: String,
    pub password: String,
    pub display_name: Option<String>,
    /// Name for the first household (default "Home").
    pub household: Option<String>,
}

/// Create the first account (an admin) on a fresh install. Refused once any account exists.
#[utoipa::path(post, path = "/api/v1/auth/setup", operation_id = "setup", tag = "auth", request_body = SetupRequest,
    responses((status = 200, description = "The admin account, signed in", body = Me), (status = 409, description = "An account already exists", body = crate::error::ErrorBody)))]
pub async fn setup(
    State(s): State<AppState>,
    jar: CookieJar,
    headers: HeaderMap,
    JsonBody(r): JsonBody<SetupRequest>,
) -> ApiResult<(CookieJar, Json<Me>)> {
    let username = super::validate_username(&r.username)?;
    super::validate_password(&r.password)?;
    let hash = super::hash_password(&r.password)?;
    let mut tx = s.pool.begin().await?;
    // One setup at a time, so two first visitors cannot both become admin.
    sqlx::query("SELECT pg_advisory_xact_lock(724163)")
        .execute(&mut *tx)
        .await?;
    let users: i64 = sqlx::query_scalar("SELECT count(*) FROM users")
        .fetch_one(&mut *tx)
        .await?;
    if users > 0 {
        return Err(ApiError::Conflict(
            "this install is already set up; sign in instead".into(),
        ));
    }
    let household = Uuid::now_v7();
    sqlx::query("INSERT INTO households (id, name) VALUES ($1, $2)")
        .bind(household)
        .bind(non_empty(r.household).unwrap_or_else(|| "Home".into()))
        .execute(&mut *tx)
        .await?;
    let id = Uuid::now_v7();
    sqlx::query("INSERT INTO users (id, username, display_name, password_hash, is_admin, household_id) VALUES ($1, $2, $3, $4, true, $5)")
        .bind(id)
        .bind(&username)
        .bind(non_empty(r.display_name).unwrap_or_else(|| username.clone()))
        .bind(hash)
        .bind(household)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    tracing::info!(%username, "first account created");
    let token = super::create_session(
        &s.pool,
        id,
        user_agent(&headers),
        s.auth.config.session_days,
    )
    .await?;
    Ok((
        jar.add(super::session_cookie(token, &s.auth.config)),
        Json(load_me(&s.pool, id).await?),
    ))
}

#[derive(Deserialize, ToSchema)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

/// Sign in with a username and password. Sets the session cookie.
#[utoipa::path(post, path = "/api/v1/auth/login", operation_id = "login", tag = "auth", request_body = LoginRequest,
    responses((status = 200, description = "Signed in", body = Me), (status = 401, description = "Wrong username or password", body = crate::error::ErrorBody), (status = 429, description = "Too many attempts; try again after Retry-After seconds", body = crate::error::ErrorBody)))]
pub async fn login(
    State(s): State<AppState>,
    jar: CookieJar,
    headers: HeaderMap,
    JsonBody(r): JsonBody<LoginRequest>,
) -> ApiResult<(CookieJar, Json<Me>)> {
    if !s.auth.config.local {
        return Err(ApiError::Forbidden(
            "password sign-in is turned off; use single sign-on".into(),
        ));
    }
    let key = r.username.trim().to_lowercase();
    if let Some(wait) = s.auth.limiter.locked_for(&key) {
        return Err(ApiError::TooManyRequests(wait));
    }
    let row: Option<(Uuid, Option<String>, bool)> =
        sqlx::query_as("SELECT id, password_hash, disabled FROM users WHERE lower(username) = $1")
            .bind(&key)
            .fetch_optional(&s.pool)
            .await?;
    let password = r.password;
    let hash = row.as_ref().and_then(|r| r.1.clone());
    let ok = tokio::task::spawn_blocking(move || super::check_password(&password, hash.as_deref()))
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    let Some((id, _, disabled)) = row.filter(|_| ok) else {
        s.auth.limiter.failed(&key);
        return Err(ApiError::BadCredentials);
    };
    if disabled {
        return Err(ApiError::Forbidden("this account is turned off".into()));
    }
    s.auth.limiter.succeeded(&key);
    let token = super::create_session(
        &s.pool,
        id,
        user_agent(&headers),
        s.auth.config.session_days,
    )
    .await?;
    Ok((
        jar.add(super::session_cookie(token, &s.auth.config)),
        Json(load_me(&s.pool, id).await?),
    ))
}

/// Sign out of this browser.
#[utoipa::path(post, path = "/api/v1/auth/logout", operation_id = "logout", tag = "auth", responses((status = 204, description = "Signed out")))]
pub async fn logout(
    State(s): State<AppState>,
    jar: CookieJar,
) -> ApiResult<(CookieJar, StatusCode)> {
    if let Some(c) = jar.get(SESSION_COOKIE) {
        sqlx::query("DELETE FROM sessions WHERE token_hash = $1")
            .bind(super::secret_hash(c.value()))
            .execute(&s.pool)
            .await?;
    }
    Ok((
        jar.add(super::removal_cookie(&s.auth.config)),
        StatusCode::NO_CONTENT,
    ))
}

/// The signed-in account, or the kitchen device (`kitchen` true, with no personal data).
#[utoipa::path(get, path = "/api/v1/me", operation_id = "me", tag = "account", responses((status = 200, description = "The signed-in account, or the kitchen device", body = Me), (status = 401, description = "Not signed in", body = crate::error::ErrorBody)))]
pub async fn me(
    State(s): State<AppState>,
    Extension(u): Extension<CurrentUser>,
) -> ApiResult<Json<Me>> {
    if u.via == Via::Kitchen {
        return Ok(Json(super::kitchen::me(&s.pool, u.id).await?));
    }
    Ok(Json(load_me(&s.pool, u.id).await?))
}

#[derive(Deserialize, ToSchema)]
pub struct PasswordChange {
    /// Required when the account already has a password.
    pub current: Option<String>,
    pub new: String,
}

/// Set or change this account's password. Signs out every other browser.
#[utoipa::path(post, path = "/api/v1/me/password", operation_id = "change_password", tag = "account", request_body = PasswordChange,
    responses((status = 204, description = "Password changed; other sessions are signed out"), (status = 400, description = "The current password is wrong, or the new one has fewer than 8 characters", body = crate::error::ErrorBody), (status = 401, description = "Not signed in", body = crate::error::ErrorBody), (status = 403, description = "Called with an API token; only a browser session can change the password", body = crate::error::ErrorBody)))]
pub async fn change_password(
    State(s): State<AppState>,
    Extension(u): Extension<CurrentUser>,
    jar: CookieJar,
    JsonBody(r): JsonBody<PasswordChange>,
) -> ApiResult<StatusCode> {
    session_only(&u)?;
    super::validate_password(&r.new)?;
    let current: Option<String> =
        sqlx::query_scalar("SELECT password_hash FROM users WHERE id = $1")
            .bind(u.id)
            .fetch_one(&s.pool)
            .await?;
    if let Some(hash) = current {
        let given = r.current.unwrap_or_default();
        let ok = tokio::task::spawn_blocking(move || super::verify_password(&given, &hash))
            .await
            .map_err(|e| ApiError::Internal(e.into()))?;
        if !ok {
            return Err(ApiError::BadRequest("the current password is wrong".into()));
        }
    }
    let new = r.new;
    let hash = tokio::task::spawn_blocking(move || super::hash_password(&new))
        .await
        .map_err(|e| ApiError::Internal(e.into()))??;
    sqlx::query("UPDATE users SET password_hash = $2 WHERE id = $1")
        .bind(u.id)
        .bind(hash)
        .execute(&s.pool)
        .await?;
    let keep = jar
        .get(SESSION_COOKIE)
        .map(|c| super::secret_hash(c.value()))
        .unwrap_or_default();
    sqlx::query("DELETE FROM sessions WHERE user_id = $1 AND token_hash <> $2")
        .bind(u.id)
        .bind(keep)
        .execute(&s.pool)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize, ToSchema)]
pub struct TokenInfo {
    pub id: Uuid,
    pub name: String,
    /// read or read_write.
    pub scope: String,
    pub created_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
}

#[derive(Deserialize, ToSchema)]
pub struct TokenRequest {
    pub name: String,
    /// read or read_write.
    pub scope: String,
}

#[derive(Serialize, ToSchema)]
pub struct NewToken {
    /// Shown once: send it as `Authorization: Bearer <token>`.
    pub token: String,
    pub info: TokenInfo,
}

/// id, name, scope, created, last used.
type TokenRow = (Uuid, String, String, DateTime<Utc>, Option<DateTime<Utc>>);

/// This account's API tokens.
#[utoipa::path(get, path = "/api/v1/me/tokens", operation_id = "list_tokens", tag = "account", responses((status = 200, description = "This account's API tokens", body = [TokenInfo])))]
pub async fn list_tokens(
    State(s): State<AppState>,
    Extension(u): Extension<CurrentUser>,
) -> ApiResult<Json<Vec<TokenInfo>>> {
    let rows: Vec<TokenRow> =
        sqlx::query_as("SELECT id, name, scope, created_at, last_used_at FROM api_tokens WHERE user_id = $1 ORDER BY created_at")
            .bind(u.id)
            .fetch_all(&s.pool)
            .await?;
    Ok(Json(
        rows.into_iter()
            .map(|(id, name, scope, created_at, last_used_at)| TokenInfo {
                id,
                name,
                scope,
                created_at,
                last_used_at,
            })
            .collect(),
    ))
}

/// Make an API token. Only from a browser session, so one token cannot mint another.
#[utoipa::path(post, path = "/api/v1/me/tokens", operation_id = "create_token", tag = "account", request_body = TokenRequest, responses((status = 200, description = "The new token, shown only this once", body = NewToken)))]
pub async fn create_token(
    State(s): State<AppState>,
    Extension(u): Extension<CurrentUser>,
    JsonBody(r): JsonBody<TokenRequest>,
) -> ApiResult<Json<NewToken>> {
    session_only(&u)?;
    if r.scope != "read" && r.scope != "read_write" {
        return Err(ApiError::BadRequest(
            "scope must be read or read_write".into(),
        ));
    }
    let name = non_empty(Some(r.name))
        .ok_or_else(|| ApiError::BadRequest("give the token a name".into()))?;
    let secret = super::new_secret();
    let id = Uuid::now_v7();
    let created_at: DateTime<Utc> =
        sqlx::query_scalar("INSERT INTO api_tokens (id, user_id, name, token_hash, scope) VALUES ($1, $2, $3, $4, $5) RETURNING created_at")
            .bind(id)
            .bind(u.id)
            .bind(&name)
            .bind(super::secret_hash(&secret))
            .bind(&r.scope)
            .fetch_one(&s.pool)
            .await?;
    Ok(Json(NewToken {
        token: format!("{TOKEN_PREFIX}{secret}"),
        info: TokenInfo {
            id,
            name,
            scope: r.scope,
            created_at,
            last_used_at: None,
        },
    }))
}

/// Revoke one of this account's tokens.
#[utoipa::path(delete, path = "/api/v1/me/tokens/{id}", operation_id = "delete_token", tag = "account", params(("id" = Uuid, Path)),
    responses((status = 204, description = "Revoked"), (status = 404, description = "No such token", body = crate::error::ErrorBody)))]
pub async fn delete_token(
    State(s): State<AppState>,
    Extension(u): Extension<CurrentUser>,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    let n = sqlx::query("DELETE FROM api_tokens WHERE id = $1 AND user_id = $2")
        .bind(id)
        .bind(u.id)
        .execute(&s.pool)
        .await?
        .rows_affected();
    if n == 0 {
        Err(ApiError::NotFound)
    } else {
        Ok(StatusCode::NO_CONTENT)
    }
}

#[derive(Serialize, ToSchema)]
pub struct UserInfo {
    pub id: Uuid,
    pub username: String,
    pub display_name: String,
    pub email: Option<String>,
    pub is_admin: bool,
    pub disabled: bool,
    pub household: Household,
    pub has_password: bool,
    /// Single-sign-on identities linked to the account.
    pub identities: i64,
}

/// Every account (admins only).
#[utoipa::path(get, path = "/api/v1/users", operation_id = "list_users", tag = "admin", responses((status = 200, description = "Every account", body = [UserInfo])))]
pub async fn list_users(
    State(s): State<AppState>,
    Extension(u): Extension<CurrentUser>,
) -> ApiResult<Json<Vec<UserInfo>>> {
    u.require_admin()?;
    type Row = (
        Uuid,
        String,
        String,
        Option<String>,
        bool,
        bool,
        Uuid,
        String,
        bool,
        i64,
    );
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT u.id, u.username, u.display_name, u.email, u.is_admin, u.disabled, h.id, h.name, u.password_hash IS NOT NULL,
                (SELECT count(*) FROM user_identities i WHERE i.user_id = u.id)
         FROM users u JOIN households h ON h.id = u.household_id ORDER BY lower(u.username)",
    )
    .fetch_all(&s.pool)
    .await?;
    Ok(Json(
        rows.into_iter()
            .map(
                |(
                    id,
                    username,
                    display_name,
                    email,
                    is_admin,
                    disabled,
                    hid,
                    hname,
                    has_password,
                    identities,
                )| UserInfo {
                    id,
                    username,
                    display_name,
                    email,
                    is_admin,
                    disabled,
                    household: Household {
                        id: hid,
                        name: hname,
                    },
                    has_password,
                    identities,
                },
            )
            .collect(),
    ))
}

#[derive(Deserialize, ToSchema)]
pub struct NewUser {
    pub username: String,
    /// Leave out for an account that only signs in with single sign-on.
    pub password: Option<String>,
    pub display_name: Option<String>,
    pub email: Option<String>,
    #[serde(default)]
    pub is_admin: bool,
    /// Join this household; leave out (with `household_name`) to start a new one.
    pub household_id: Option<Uuid>,
    pub household_name: Option<String>,
}

/// Add an account (admins only).
#[utoipa::path(post, path = "/api/v1/users", operation_id = "create_user", tag = "admin", request_body = NewUser,
    responses((status = 200, description = "The new account", body = Me), (status = 409, description = "That username or email is taken", body = crate::error::ErrorBody)))]
pub async fn create_user(
    State(s): State<AppState>,
    Extension(u): Extension<CurrentUser>,
    JsonBody(r): JsonBody<NewUser>,
) -> ApiResult<Json<Me>> {
    u.require_admin()?;
    let id = add_user(&s.pool, r).await?;
    Ok(Json(load_me(&s.pool, id).await?))
}

/// Shared by the API and `mise user add`.
pub async fn add_user(pool: &sqlx::PgPool, r: NewUser) -> ApiResult<Uuid> {
    let username = super::validate_username(&r.username)?;
    let hash = match &r.password {
        Some(p) => {
            super::validate_password(p)?;
            Some(super::hash_password(p)?)
        }
        None => None,
    };
    let mut tx = pool.begin().await?;
    let taken: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM users WHERE lower(username) = lower($1) OR (email IS NOT NULL AND lower(email) = lower($2)))")
        .bind(&username)
        .bind(r.email.as_deref().unwrap_or(""))
        .fetch_one(&mut *tx)
        .await?;
    if taken {
        return Err(ApiError::Conflict(
            "that username or email is already in use".into(),
        ));
    }
    let household = match (r.household_id, non_empty(r.household_name)) {
        (Some(h), _) => {
            let exists: bool =
                sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM households WHERE id = $1)")
                    .bind(h)
                    .fetch_one(&mut *tx)
                    .await?;
            if !exists {
                return Err(ApiError::BadRequest("no such household".into()));
            }
            h
        }
        (None, name) => {
            let h = Uuid::now_v7();
            sqlx::query("INSERT INTO households (id, name) VALUES ($1, $2)")
                .bind(h)
                .bind(name.unwrap_or_else(|| format!("{username}'s household")))
                .execute(&mut *tx)
                .await?;
            h
        }
    };
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO users (id, username, display_name, email, password_hash, is_admin, household_id) VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(id)
    .bind(&username)
    .bind(non_empty(r.display_name).unwrap_or_else(|| username.clone()))
    .bind(non_empty(r.email))
    .bind(hash)
    .bind(r.is_admin)
    .bind(household)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(id)
}

#[derive(Deserialize, ToSchema)]
pub struct UserUpdate {
    pub is_admin: Option<bool>,
    pub disabled: Option<bool>,
    pub household_id: Option<Uuid>,
    /// Set a new password (and sign the account out everywhere).
    pub password: Option<String>,
}

/// Change an account (admins only). An admin cannot remove the last admin or turn off their own account.
#[utoipa::path(patch, path = "/api/v1/users/{id}", operation_id = "update_user", tag = "admin", params(("id" = Uuid, Path)), request_body = UserUpdate,
    responses((status = 200, description = "The updated account", body = Me), (status = 404, description = "No such account", body = crate::error::ErrorBody)))]
pub async fn update_user(
    State(s): State<AppState>,
    Extension(u): Extension<CurrentUser>,
    Path(id): Path<Uuid>,
    JsonBody(r): JsonBody<UserUpdate>,
) -> ApiResult<Json<Me>> {
    u.require_admin()?;
    let mut tx = s.pool.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(724164)")
        .execute(&mut *tx)
        .await?;
    let (is_admin, disabled): (bool, bool) =
        sqlx::query_as("SELECT is_admin, disabled FROM users WHERE id = $1")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(ApiError::NotFound)?;
    let (new_admin, new_disabled) = (
        r.is_admin.unwrap_or(is_admin),
        r.disabled.unwrap_or(disabled),
    );
    if id == u.id && new_disabled {
        return Err(ApiError::BadRequest(
            "you can't turn off your own account".into(),
        ));
    }
    if is_admin && !disabled && (!new_admin || new_disabled) {
        let others: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM users WHERE is_admin AND NOT disabled AND id <> $1",
        )
        .bind(id)
        .fetch_one(&mut *tx)
        .await?;
        if others == 0 {
            return Err(ApiError::BadRequest("this is the last admin".into()));
        }
    }
    if let Some(h) = r.household_id {
        let exists: bool =
            sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM households WHERE id = $1)")
                .bind(h)
                .fetch_one(&mut *tx)
                .await?;
        if !exists {
            return Err(ApiError::BadRequest("no such household".into()));
        }
    }
    let hash = match &r.password {
        Some(p) => {
            super::validate_password(p)?;
            Some(super::hash_password(p)?)
        }
        None => None,
    };
    sqlx::query(
        "UPDATE users SET is_admin = $2, disabled = $3, household_id = coalesce($4, household_id), password_hash = coalesce($5, password_hash) WHERE id = $1",
    )
    .bind(id)
    .bind(new_admin)
    .bind(new_disabled)
    .bind(r.household_id)
    .bind(&hash)
    .execute(&mut *tx)
    .await?;
    if new_disabled || hash.is_some() {
        sqlx::query("DELETE FROM sessions WHERE user_id = $1")
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(Json(load_me(&s.pool, id).await?))
}

/// Every household (admins only).
#[utoipa::path(get, path = "/api/v1/households", operation_id = "list_households", tag = "admin", responses((status = 200, description = "Every household", body = [Household])))]
pub async fn list_households(
    State(s): State<AppState>,
    Extension(u): Extension<CurrentUser>,
) -> ApiResult<Json<Vec<Household>>> {
    u.require_admin()?;
    let rows: Vec<(Uuid, String)> = sqlx::query_as("SELECT id, name FROM households ORDER BY name")
        .fetch_all(&s.pool)
        .await?;
    Ok(Json(
        rows.into_iter()
            .map(|(id, name)| Household { id, name })
            .collect(),
    ))
}

#[derive(Deserialize, ToSchema)]
pub struct NewHousehold {
    pub name: String,
}

/// Add a household (admins only).
#[utoipa::path(post, path = "/api/v1/households", operation_id = "create_household", tag = "admin", request_body = NewHousehold, responses((status = 200, description = "The new household", body = Household)))]
pub async fn create_household(
    State(s): State<AppState>,
    Extension(u): Extension<CurrentUser>,
    JsonBody(r): JsonBody<NewHousehold>,
) -> ApiResult<Json<Household>> {
    u.require_admin()?;
    let name = non_empty(Some(r.name))
        .ok_or_else(|| ApiError::BadRequest("give the household a name".into()))?;
    let id = Uuid::now_v7();
    sqlx::query("INSERT INTO households (id, name) VALUES ($1, $2)")
        .bind(id)
        .bind(&name)
        .execute(&s.pool)
        .await?;
    Ok(Json(Household { id, name }))
}

pub(super) fn session_only(u: &CurrentUser) -> ApiResult<()> {
    if u.via == Via::Session {
        Ok(())
    } else {
        Err(ApiError::Forbidden(
            "sign in from a browser to do this".into(),
        ))
    }
}

fn non_empty(s: Option<String>) -> Option<String> {
    s.map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

pub(crate) fn user_agent(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::USER_AGENT)
        .and_then(|h| h.to_str().ok())
}
