//! Accounts and sign-in. Every API route except health and sign-in itself needs a signed-in user: a browser
//! session (an HttpOnly cookie) or a personal API token (`Authorization: Bearer mise_…`), or a kitchen device's
//! cookie, which reads the household's plan and recipes only (see `kitchen`). Passwords are argon2id; sessions and
//! tokens are random 256-bit values stored only as SHA-256 hashes.

pub mod kitchen;
pub mod oidc;
pub mod routes;

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, bail};
use argon2::{Argon2, PasswordHasher, PasswordVerifier};
use axum::{
    extract::{Request, State},
    http::{HeaderValue, Method, header},
    middleware::Next,
    response::{IntoResponse, Response},
};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use base64::Engine;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use time::OffsetDateTime;
use url::Url;
use uuid::Uuid;

use crate::{AppState, error::ApiError};

/// The cookie a browser's session travels in.
pub const SESSION_COOKIE: &str = "mise_session";
/// What every API token starts with, so one pasted somewhere is recognisable.
pub const TOKEN_PREFIX: &str = "mise_";

/// Routes anyone may call, after the API's address (`/api/v1` or the old `/api`): health, and what signing in
/// needs. Single sign-on's redirects sit at `/api/auth/oidc/…` only, so they read the same under either.
const PUBLIC: &[&str] = &[
    "/health",
    "/ready",
    "/openapi.json",
    "/auth/config",
    "/auth/setup",
    "/auth/login",
    "/auth/kitchen",
    "/auth/oidc/start",
    "/auth/oidc/callback",
];

/// Sign-in settings (AUTH_MODES, OIDC_*, PUBLIC_BASE_URL, SESSION_DAYS).
pub struct AuthConfig {
    /// Username and password sign-in.
    pub local: bool,
    /// Single sign-on, when OIDC_ISSUER is set.
    pub oidc: Option<oidc::OidcConfig>,
    /// Where the install is reached from outside (behind a reverse proxy); unset on a home-network install.
    pub public_base_url: Option<Url>,
    /// How long a sign-in lasts (SESSION_DAYS, default 30).
    pub session_days: i64,
}

impl AuthConfig {
    /// Reads the settings from the environment; fails, saying which, when one is wrong or nobody could sign in.
    pub fn from_env() -> anyhow::Result<Self> {
        let modes = std::env::var("AUTH_MODES").unwrap_or_else(|_| "local".into());
        let modes: Vec<String> = modes
            .split(',')
            .map(|m| m.trim().to_lowercase())
            .filter(|m| !m.is_empty())
            .collect();
        for m in &modes {
            if m != "local" && m != "oidc" {
                bail!("AUTH_MODES: unknown mode {m:?} (expected local, oidc)");
            }
        }
        if modes.is_empty() {
            bail!("AUTH_MODES is empty: nobody could sign in");
        }
        let public_base_url = match std::env::var("PUBLIC_BASE_URL") {
            Ok(u) if !u.trim().is_empty() => {
                Some(Url::parse(u.trim()).context("PUBLIC_BASE_URL is not a URL")?)
            }
            _ => None,
        };
        let oidc = if modes.iter().any(|m| m == "oidc") {
            Some(oidc::OidcConfig::from_env()?)
        } else {
            None
        };
        let session_days = std::env::var("SESSION_DAYS")
            .ok()
            .and_then(|d| d.parse().ok())
            .filter(|d| *d > 0)
            .unwrap_or(30);
        Ok(AuthConfig {
            local: modes.iter().any(|m| m == "local"),
            oidc,
            public_base_url,
            session_days,
        })
    }

    /// Cookies are marked Secure when the install is served over HTTPS.
    pub fn secure_cookies(&self) -> bool {
        self.public_base_url
            .as_ref()
            .is_some_and(|u| u.scheme() == "https")
    }
}

/// Everything sign-in needs at run time.
pub struct Auth {
    /// The settings.
    pub config: AuthConfig,
    /// Failed password sign-ins, per username.
    pub limiter: LoginLimiter,
    /// The single sign-on provider, discovered on first use.
    pub oidc: oidc::Provider,
}

impl Auth {
    /// Sign-in with these settings, nobody throttled yet.
    pub fn new(config: AuthConfig) -> Self {
        Auth {
            config,
            limiter: LoginLimiter::default(),
            oidc: oidc::Provider::default(),
        }
    }
}

/// Who is calling, attached to every authenticated request. A kitchen device has no account: its `id` is its
/// kitchen token's, and `require_auth` lets it reach only the routes `kitchen::may` lists.
#[derive(Clone, Debug)]
pub struct CurrentUser {
    /// The account's id, or a kitchen device's token id.
    pub id: Uuid,
    /// The account's username; empty for a kitchen device.
    pub username: String,
    /// The account manages accounts, households and kitchen devices.
    pub is_admin: bool,
    /// The household every query is scoped to.
    pub household_id: Uuid,
    /// How the request proved who it is.
    pub via: Via,
}

/// How a request proved who it is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Via {
    /// A browser's session cookie.
    Session,
    /// An API token in `Authorization: Bearer`.
    Token {
        /// The token may only read.
        read_only: bool,
    },
    /// A kitchen device's cookie.
    Kitchen,
}

impl CurrentUser {
    /// Refuses anyone but an admin.
    pub fn require_admin(&self) -> Result<(), ApiError> {
        if self.is_admin {
            Ok(())
        } else {
            Err(ApiError::Forbidden("admins only".into()))
        }
    }
}

/// The password's Argon2id hash, as stored.
pub fn hash_password(password: &str) -> anyhow::Result<String> {
    Argon2::default()
        .hash_password(password.as_bytes())
        .map(|h| h.to_string())
        .map_err(|e| anyhow::anyhow!("hashing a password: {e}"))
}

/// Whether the password matches a stored hash.
pub fn verify_password(password: &str, hash: &str) -> bool {
    Argon2::default()
        .verify_password(password.as_bytes(), hash)
        .is_ok()
}

/// Verified against when the username does not exist, so a miss takes as long as a wrong password.
static DUMMY_HASH: LazyLock<String> =
    LazyLock::new(|| hash_password("not a real password").expect("argon2 works"));

/// Whether the password matches; with no hash (no such account), it still takes as long as a check, so a wrong username
/// can't be told from a wrong password by the time taken.
pub fn check_password(password: &str, hash: Option<&str>) -> bool {
    match hash {
        Some(h) => verify_password(password, h),
        None => {
            let _ = verify_password(password, &DUMMY_HASH);
            false
        }
    }
}

/// Password rules: long enough to resist guessing, short enough to hash quickly.
pub fn validate_password(password: &str) -> Result<(), ApiError> {
    let n = password.chars().count();
    if n < 8 {
        return Err(ApiError::BadRequest(
            "passwords need at least 8 characters".into(),
        ));
    }
    if n > 256 {
        return Err(ApiError::BadRequest(
            "passwords can have at most 256 characters".into(),
        ));
    }
    Ok(())
}

/// The username trimmed, when it is 1 to 64 letters, digits or `._-@+`; refused otherwise.
pub fn validate_username(username: &str) -> Result<String, ApiError> {
    let u = username.trim();
    let ok = (1..=64).contains(&u.chars().count())
        && u.chars()
            .all(|c| c.is_alphanumeric() || "._-@+".contains(c));
    if ok {
        Ok(u.to_string())
    } else {
        Err(ApiError::BadRequest(
            "usernames are 1–64 letters, digits or . _ - @ +".into(),
        ))
    }
}

/// A random 256-bit secret, URL-safe.
pub fn new_secret() -> String {
    let mut bytes = [0u8; 32];
    rand::fill(&mut bytes);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// How a secret is stored: its SHA-256, in hex. Sessions, tokens and kitchen links are kept only this way.
pub fn secret_hash(secret: &str) -> String {
    hex::encode(Sha256::digest(secret.as_bytes()))
}

/// Starts a session for `user` lasting `days`, clearing ended ones; answers with the secret for its cookie.
pub async fn create_session(
    pool: &PgPool,
    user: Uuid,
    user_agent: Option<&str>,
    days: i64,
) -> Result<String, ApiError> {
    let token = new_secret();
    sqlx::query!("DELETE FROM sessions WHERE expires_at < now()")
        .execute(pool)
        .await?;
    sqlx::query!(
        "INSERT INTO sessions (token_hash, user_id, expires_at, user_agent) VALUES ($1, $2, now() + make_interval(days => $3::int), $4)",
        secret_hash(&token),
        user,
        days as i32,
        user_agent.map(|u| u.chars().take(200).collect::<String>())
    )
    .execute(pool)
    .await?;
    Ok(token)
}

/// The session cookie: HttpOnly, SameSite=Lax, Secure when the install is served over HTTPS.
pub fn session_cookie(token: String, config: &AuthConfig) -> Cookie<'static> {
    Cookie::build((SESSION_COOKIE, token))
        .path("/")
        .http_only(true)
        .same_site(SameSite::Lax)
        .secure(config.secure_cookies())
        .max_age(time::Duration::days(config.session_days))
        .build()
}

/// A cookie that removes the session cookie, signing the browser out.
pub fn removal_cookie(config: &AuthConfig) -> Cookie<'static> {
    let mut c = Cookie::build((SESSION_COOKIE, ""))
        .path("/")
        .http_only(true)
        .same_site(SameSite::Lax)
        .secure(config.secure_cookies())
        .build();
    c.make_removal();
    c
}

/// Counts failed sign-ins per username; after five, the account is locked for a while, doubling each time
/// (30 seconds up to 15 minutes). A success clears the count.
#[derive(Default)]
pub struct LoginLimiter {
    failures: Mutex<HashMap<String, (u32, Instant)>>,
}

const FREE_ATTEMPTS: u32 = 5;
const MAX_LOCK: Duration = Duration::from_secs(15 * 60);

impl LoginLimiter {
    /// Seconds until `key` may try again, if it is locked.
    pub fn locked_for(&self, key: &str) -> Option<u64> {
        let map = self.failures.lock().unwrap();
        let (n, at) = map.get(key)?;
        if *n < FREE_ATTEMPTS {
            return None;
        }
        let lock = Duration::from_secs(30)
            .saturating_mul(1 << (n - FREE_ATTEMPTS).min(10))
            .min(MAX_LOCK);
        let left = lock.checked_sub(at.elapsed())?;
        Some(left.as_secs().max(1))
    }

    /// Counts a failed sign-in for `key` (a username).
    pub fn failed(&self, key: &str) {
        let mut map = self.failures.lock().unwrap();
        // Forget old entries so the map cannot grow without bound.
        if map.len() > 10_000 {
            map.retain(|_, (_, at)| at.elapsed() < MAX_LOCK);
        }
        let e = map.entry(key.to_string()).or_insert((0, Instant::now()));
        e.0 += 1;
        e.1 = Instant::now();
    }

    /// Clears `key`'s failures after a sign-in that worked.
    pub fn succeeded(&self, key: &str) {
        self.failures.lock().unwrap().remove(key);
    }
}

async fn from_session(
    pool: &PgPool,
    token: &str,
    days: i64,
) -> Result<Option<CurrentUser>, ApiError> {
    let hash = secret_hash(token);
    let row: Option<(Uuid, String, bool, Uuid, OffsetDateTime)> = sqlx::query!(
        r#"SELECT u.id AS "id!",
                  u.username AS "username!",
                  u.is_admin AS "is_admin!",
                  u.household_id AS "household_id!",
                  s.last_seen_at AS "last_seen_at!"
           FROM sessions s JOIN users u ON u.id = s.user_id
           WHERE s.token_hash = $1 AND s.expires_at > now() AND NOT u.disabled"#,
        &hash
    )
    .fetch_optional(pool)
    .await
    .map(|row| row.map(|r| (r.id, r.username, r.is_admin, r.household_id, r.last_seen_at)))?;
    let Some((id, username, is_admin, household_id, seen)) = row else {
        return Ok(None);
    };
    // Sliding expiry, written at most hourly.
    if OffsetDateTime::now_utc() - seen > time::Duration::hours(1) {
        sqlx::query!(
            "UPDATE sessions SET last_seen_at = now(), expires_at = now() + make_interval(days => $2::int) WHERE token_hash = $1",
            &hash,
            days as i32
        )
            .execute(pool)
            .await?;
    }
    Ok(Some(CurrentUser {
        id,
        username,
        is_admin,
        household_id,
        via: Via::Session,
    }))
}

async fn from_token(pool: &PgPool, token: &str) -> Result<Option<CurrentUser>, ApiError> {
    let hash = secret_hash(token);
    let row: Option<(Uuid, String, bool, Uuid, String)> = sqlx::query!(
        r#"SELECT u.id AS "id!",
                  u.username AS "username!",
                  u.is_admin AS "is_admin!",
                  u.household_id AS "household_id!",
                  t.scope AS "scope!"
           FROM api_tokens t JOIN users u ON u.id = t.user_id
           WHERE t.token_hash = $1 AND NOT u.disabled"#,
        &hash
    )
    .fetch_optional(pool)
    .await
    .map(|row| row.map(|r| (r.id, r.username, r.is_admin, r.household_id, r.scope)))?;
    let Some((id, username, is_admin, household_id, scope)) = row else {
        return Ok(None);
    };
    sqlx::query!(
        "UPDATE api_tokens SET last_used_at = now() WHERE token_hash = $1 AND (last_used_at IS NULL OR last_used_at < now() - interval '1 hour')",
        &hash
    )
        .execute(pool)
        .await?;
    Ok(Some(CurrentUser {
        id,
        username,
        is_admin,
        household_id,
        via: Via::Token {
            read_only: scope == "read",
        },
    }))
}

/// Names the account a request is made for. The web app sends it with every request, so work a page started as
/// one person (a queued tick, say) is refused with 412 rather than applied to whoever signed in since, in another
/// tab, with the shared cookie. Scripts and integrations can leave it out.
pub const ACCOUNT_HEADER: &str = "x-mise-account";

/// Resolve the caller and refuse what they may not do. Paths outside the API (the web app) and the public
/// routes pass through untouched. A bearer token comes first, then an account's session cookie, then a kitchen
/// device's cookie.
pub async fn require_auth(
    State(s): State<AppState>,
    jar: CookieJar,
    mut req: Request,
    next: Next,
) -> Response {
    let rest = match crate::api_path(req.uri().path()) {
        Some(rest) if !PUBLIC.contains(&rest) => rest.to_string(),
        _ => return next.run(req).await,
    };
    let bearer = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "))
        .map(str::trim)
        .map(str::to_string);
    // The kitchen cookie to set again on the way out, renewing its lifetime, when this request renewed its use.
    let mut renew = None;
    let user = match (bearer, jar.get(SESSION_COOKIE)) {
        (Some(t), _) => match t.strip_prefix(TOKEN_PREFIX) {
            Some(t) => from_token(&s.pool, t).await,
            None => Ok(None),
        },
        (None, session) => {
            let user = match session {
                Some(c) => from_session(&s.pool, c.value(), s.auth.config.session_days).await,
                None => Ok(None),
            };
            match (user, jar.get(kitchen::COOKIE)) {
                (Ok(None), Some(c)) => match kitchen::from_cookie(&s.pool, c.value()).await {
                    Ok(Some((u, renewed))) => {
                        renew = renewed.then(|| c.value().to_string());
                        Ok(Some(u))
                    }
                    Ok(None) => Ok(None),
                    Err(e) => Err(e),
                },
                (user, _) => user,
            }
        }
    };
    let user = match user {
        Ok(Some(u)) => u,
        Ok(None) => return ApiError::Unauthorized.into_response(),
        Err(e) => return e.into_response(),
    };
    if user.via == Via::Kitchen && !kitchen::may(req.method(), &rest) {
        return ApiError::Forbidden("a kitchen device can only read the plan and recipes".into())
            .into_response();
    }
    if user.via == (Via::Token { read_only: true })
        && !matches!(*req.method(), Method::GET | Method::HEAD)
    {
        return ApiError::Forbidden("this token can only read".into()).into_response();
    }
    if let Some(meant) = req.headers().get(ACCOUNT_HEADER)
        && meant.to_str().ok().and_then(|m| m.parse::<Uuid>().ok()) != Some(user.id)
    {
        return ApiError::AccountChanged.into_response();
    }
    req.extensions_mut().insert(user);
    let mut res = next.run(req).await;
    if let Some(cookie) = renew
        .map(|secret| kitchen::cookie(secret, &s.auth.config).to_string())
        .and_then(|c| HeaderValue::from_str(&c).ok())
    {
        res.headers_mut().append(header::SET_COOKIE, cookie);
    }
    res
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passwords_round_trip() {
        let h = hash_password("correct horse battery").unwrap();
        assert!(h.starts_with("$argon2id$"));
        assert!(verify_password("correct horse battery", &h));
        assert!(!verify_password("correct horse batterY", &h));
        assert!(!check_password("anything", None));
    }

    #[test]
    fn secrets_are_random_and_hashed() {
        let (a, b) = (new_secret(), new_secret());
        assert_ne!(a, b);
        assert_eq!(a.len(), 43);
        assert_eq!(secret_hash(&a).len(), 64);
    }

    #[test]
    fn limiter_locks_after_five_failures() {
        let l = LoginLimiter::default();
        for _ in 0..4 {
            l.failed("paul");
        }
        assert_eq!(l.locked_for("paul"), None);
        l.failed("paul");
        assert!(l.locked_for("paul").is_some_and(|s| s <= 30));
        assert_eq!(l.locked_for("someone-else"), None);
        l.succeeded("paul");
        assert_eq!(l.locked_for("paul"), None);
    }

    #[test]
    fn usernames_and_passwords_are_checked() {
        assert!(validate_username("paul.h").is_ok());
        assert!(validate_username("").is_err());
        assert!(validate_username("a b").is_err());
        assert!(validate_password("short").is_err());
        assert!(validate_password("long enough").is_ok());
    }
}
