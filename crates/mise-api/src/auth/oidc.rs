//! Single sign-on with any OpenID Connect provider (Authentik, Keycloak, Nextcloud…): authorization code flow
//! with PKCE and a nonce. A returning person is found by their provider identity, else linked to an account
//! with the same verified email, else given a new account. Membership of OIDC_ADMIN_GROUP makes them admin.

use anyhow::{Context, anyhow};
use axum::{
    extract::State,
    http::HeaderMap,
    response::{IntoResponse, Redirect, Response},
};
use axum_extra::extract::cookie::CookieJar;
use base64::Engine;
use openidconnect::core::{
    CoreAuthenticationFlow, CoreClient, CoreJwsSigningAlgorithm, CoreProviderMetadata,
};
use openidconnect::{
    AuthorizationCode, ClientId, ClientSecret, CsrfToken, IssuerUrl, Nonce, PkceCodeChallenge,
    PkceCodeVerifier, RedirectUrl, Scope,
};
use serde::Deserialize;
use tokio::sync::OnceCell;
use uuid::Uuid;

use super::secret_hash;
use crate::{AppState, extract::Query};

/// OIDC_ISSUER, OIDC_CLIENT_ID, OIDC_CLIENT_SECRET, OIDC_NAME, OIDC_SCOPES, OIDC_ADMIN_GROUP, OIDC_HOUSEHOLD.
pub struct OidcConfig {
    pub issuer: IssuerUrl,
    pub client_id: String,
    pub client_secret: Option<String>,
    /// Label on the sign-in button.
    pub name: String,
    pub scopes: Vec<String>,
    /// Members of this group (from the `groups` claim) are admins; checked at every sign-in.
    pub admin_group: Option<String>,
    /// New accounts join the household with this name; unset, each gets a household of their own.
    pub household: Option<String>,
}

impl OidcConfig {
    pub fn from_env() -> anyhow::Result<Self> {
        let var = |k: &str| {
            std::env::var(k)
                .ok()
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
        };
        let issuer =
            var("OIDC_ISSUER").context("AUTH_MODES includes oidc, so OIDC_ISSUER must be set")?;
        Ok(OidcConfig {
            issuer: IssuerUrl::new(issuer).context("OIDC_ISSUER is not a URL")?,
            client_id: var("OIDC_CLIENT_ID")
                .context("AUTH_MODES includes oidc, so OIDC_CLIENT_ID must be set")?,
            client_secret: var("OIDC_CLIENT_SECRET"),
            name: var("OIDC_NAME").unwrap_or_else(|| "Single sign-on".into()),
            scopes: var("OIDC_SCOPES")
                .unwrap_or_else(|| "email profile".into())
                .split([' ', ','])
                .filter(|s| !s.is_empty() && *s != "openid")
                .map(str::to_string)
                .collect(),
            admin_group: var("OIDC_ADMIN_GROUP"),
            household: var("OIDC_HOUSEHOLD"),
        })
    }
}

/// The provider's metadata, discovered on first use and kept.
#[derive(Default)]
pub struct Provider {
    metadata: OnceCell<CoreProviderMetadata>,
}

fn http() -> anyhow::Result<openidconnect::reqwest::Client> {
    openidconnect::reqwest::ClientBuilder::new()
        .redirect(openidconnect::reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .context("building the HTTP client")
}

/// Where the provider sends people back: PUBLIC_BASE_URL when set, else the address this request came to. The
/// path stays `/api/auth/oidc/callback`, outside `/api/v1`: providers have it on record as the redirect address.
fn redirect_url(s: &AppState, headers: &HeaderMap) -> anyhow::Result<RedirectUrl> {
    let base = match &s.auth.config.public_base_url {
        Some(u) => u.as_str().trim_end_matches('/').to_string(),
        None => {
            let host = headers
                .get("host")
                .and_then(|h| h.to_str().ok())
                .ok_or_else(|| anyhow!("no Host header"))?;
            let proto = headers
                .get("x-forwarded-proto")
                .and_then(|h| h.to_str().ok())
                .unwrap_or("http");
            format!("{proto}://{host}")
        }
    };
    Ok(RedirectUrl::new(format!("{base}/api/auth/oidc/callback"))?)
}

async fn metadata<'a>(
    s: &'a AppState,
    cfg: &OidcConfig,
) -> anyhow::Result<&'a CoreProviderMetadata> {
    s.auth
        .oidc
        .metadata
        .get_or_try_init(|| async {
            CoreProviderMetadata::discover_async(cfg.issuer.clone(), &http()?)
                .await
                .context("discovering the OIDC provider")
        })
        .await
}

#[derive(Deserialize)]
pub struct StartParams {
    /// Path to return to after signing in.
    pub return_to: Option<String>,
}

/// Only paths on this site, so the callback can't be used to send people elsewhere.
fn safe_return(p: Option<String>) -> String {
    p.filter(|p| p.starts_with('/') && !p.starts_with("//") && !p.contains('\\'))
        .unwrap_or_else(|| "/".into())
}

fn to_login(error: &str) -> Response {
    Redirect::to(&format!(
        "/login?error={}",
        url::form_urlencoded::byte_serialize(error.as_bytes()).collect::<String>()
    ))
    .into_response()
}

/// Begin single sign-on: redirect to the provider.
#[utoipa::path(get, path = "/api/auth/oidc/start", operation_id = "oidc_start", tag = "auth",
    params(("return_to" = Option<String>, Query)), responses((status = 303, description = "To the provider")))]
pub async fn start(
    State(s): State<AppState>,
    headers: HeaderMap,
    Query(p): Query<StartParams>,
) -> Response {
    match start_inner(&s, &headers, p).await {
        Ok(url) => Redirect::to(&url).into_response(),
        Err(e) => {
            tracing::warn!(error = ?e, "single sign-on could not start");
            to_login("Single sign-on isn't available right now.")
        }
    }
}

async fn start_inner(s: &AppState, headers: &HeaderMap, p: StartParams) -> anyhow::Result<String> {
    let cfg = s
        .auth
        .config
        .oidc
        .as_ref()
        .ok_or_else(|| anyhow!("single sign-on is not configured"))?;
    let meta = metadata(s, cfg).await?;
    let client = CoreClient::from_provider_metadata(
        meta.clone(),
        ClientId::new(cfg.client_id.clone()),
        cfg.client_secret.clone().map(ClientSecret::new),
    )
    .set_redirect_uri(redirect_url(s, headers)?);
    let (challenge, verifier) = PkceCodeChallenge::new_random_sha256();
    let mut req = client.authorize_url(
        CoreAuthenticationFlow::AuthorizationCode,
        CsrfToken::new_random,
        Nonce::new_random,
    );
    for scope in &cfg.scopes {
        req = req.add_scope(Scope::new(scope.clone()));
    }
    let (url, state, nonce) = req.set_pkce_challenge(challenge).url();
    sqlx::query("DELETE FROM oidc_logins WHERE created_at < now() - interval '15 minutes'")
        .execute(&s.pool)
        .await?;
    sqlx::query("INSERT INTO oidc_logins (state_hash, nonce, pkce_verifier, return_to) VALUES ($1, $2, $3, $4)")
        .bind(secret_hash(state.secret()))
        .bind(nonce.secret())
        .bind(verifier.secret())
        .bind(safe_return(p.return_to))
        .execute(&s.pool)
        .await?;
    Ok(url.to_string())
}

#[derive(Deserialize)]
pub struct CallbackParams {
    pub code: Option<String>,
    pub state: Option<String>,
    pub error: Option<String>,
    pub error_description: Option<String>,
}

/// The provider sends people back here; sign them in and continue where they started.
#[utoipa::path(get, path = "/api/auth/oidc/callback", operation_id = "oidc_callback", tag = "auth",
    responses((status = 303, description = "Signed in, back to the app")))]
pub async fn callback(
    State(s): State<AppState>,
    jar: CookieJar,
    headers: HeaderMap,
    Query(p): Query<CallbackParams>,
) -> Response {
    if let Some(e) = &p.error {
        tracing::info!(error = %e, description = ?p.error_description, "provider refused sign-in");
        return to_login("The sign-in provider refused the sign-in.");
    }
    match callback_inner(&s, &headers, p).await {
        Ok(Outcome::SignedIn { user, return_to }) => {
            match super::create_session(
                &s.pool,
                user,
                super::routes::user_agent(&headers),
                s.auth.config.session_days,
            )
            .await
            {
                Ok(token) => (
                    jar.add(super::session_cookie(token, &s.auth.config)),
                    Redirect::to(&return_to),
                )
                    .into_response(),
                Err(e) => e.into_response(),
            }
        }
        Ok(Outcome::Refused(why)) => to_login(why),
        Err(e) => {
            tracing::warn!(error = ?e, "single sign-on failed");
            to_login("Single sign-on didn't work. Try again.")
        }
    }
}

enum Outcome {
    SignedIn { user: Uuid, return_to: String },
    Refused(&'static str),
}

async fn callback_inner(
    s: &AppState,
    headers: &HeaderMap,
    p: CallbackParams,
) -> anyhow::Result<Outcome> {
    let cfg = s
        .auth
        .config
        .oidc
        .as_ref()
        .ok_or_else(|| anyhow!("single sign-on is not configured"))?;
    let (Some(code), Some(state)) = (p.code, p.state) else {
        return Ok(Outcome::Refused("The sign-in reply was incomplete."));
    };
    let pending: Option<(String, String, String)> = sqlx::query_as(
        "DELETE FROM oidc_logins WHERE state_hash = $1 AND created_at > now() - interval '15 minutes' RETURNING nonce, pkce_verifier, return_to",
    )
    .bind(secret_hash(&state))
    .fetch_optional(&s.pool)
    .await?;
    let Some((nonce, verifier, return_to)) = pending else {
        return Ok(Outcome::Refused("That sign-in expired. Try again."));
    };

    let meta = metadata(s, cfg).await?;
    let client = CoreClient::from_provider_metadata(
        meta.clone(),
        ClientId::new(cfg.client_id.clone()),
        cfg.client_secret.clone().map(ClientSecret::new),
    )
    .set_redirect_uri(redirect_url(s, headers)?);
    let tokens = client
        .exchange_code(AuthorizationCode::new(code))?
        .set_pkce_verifier(PkceCodeVerifier::new(verifier))
        .request_async(&http()?)
        .await
        .context("exchanging the code")?;
    use openidconnect::TokenResponse as _;
    let id_token = tokens
        .id_token()
        .ok_or_else(|| anyhow!("the provider sent no ID token"))?;
    let mut algs = vec![
        CoreJwsSigningAlgorithm::RsaSsaPkcs1V15Sha256,
        CoreJwsSigningAlgorithm::RsaSsaPkcs1V15Sha384,
        CoreJwsSigningAlgorithm::RsaSsaPkcs1V15Sha512,
        CoreJwsSigningAlgorithm::RsaSsaPssSha256,
        CoreJwsSigningAlgorithm::EcdsaP256Sha256,
        CoreJwsSigningAlgorithm::EcdsaP384Sha384,
        CoreJwsSigningAlgorithm::EdDsa,
    ];
    if cfg.client_secret.is_some() {
        algs.push(CoreJwsSigningAlgorithm::HmacSha256);
    }
    let verifier = client.id_token_verifier().set_allowed_algs(algs);
    let claims = id_token
        .claims(&verifier, &Nonce::new(nonce))
        .context("verifying the ID token")?;

    let issuer = claims.issuer().as_str().to_string();
    let subject = claims.subject().as_str().to_string();
    let email = claims.email().map(|e| e.as_str().to_lowercase());
    let email_verified = claims.email_verified().unwrap_or(false);
    let name = claims
        .name()
        .and_then(|n| n.get(None))
        .map(|n| n.as_str().to_string());
    let preferred = claims.preferred_username().map(|u| u.as_str().to_string());
    // The signature is verified above; other claims (groups) come from the same payload.
    let groups = groups_claim(&id_token.to_string());

    let user = find_or_create(
        s,
        cfg,
        &issuer,
        &subject,
        email.as_deref(),
        email_verified,
        name,
        preferred,
    )
    .await?;
    let Some(user) = user else {
        return Ok(Outcome::Refused("This account is turned off."));
    };
    if let Some(group) = &cfg.admin_group {
        sqlx::query("UPDATE users SET is_admin = $2 WHERE id = $1")
            .bind(user)
            .bind(groups.iter().any(|g| g == group))
            .execute(&s.pool)
            .await?;
    }
    Ok(Outcome::SignedIn { user, return_to })
}

fn groups_claim(jwt: &str) -> Vec<String> {
    let Some(payload) = jwt.split('.').nth(1) else {
        return vec![];
    };
    let Ok(bytes) =
        base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(payload.trim_end_matches('='))
    else {
        return vec![];
    };
    let Ok(v) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return vec![];
    };
    match v.get("groups") {
        Some(serde_json::Value::Array(a)) => a
            .iter()
            .filter_map(|g| g.as_str().map(str::to_string))
            .collect(),
        Some(serde_json::Value::String(g)) => vec![g.clone()],
        _ => vec![],
    }
}

/// The account for this identity, or None when it is turned off.
#[allow(clippy::too_many_arguments)]
async fn find_or_create(
    s: &AppState,
    cfg: &OidcConfig,
    issuer: &str,
    subject: &str,
    email: Option<&str>,
    email_verified: bool,
    name: Option<String>,
    preferred: Option<String>,
) -> anyhow::Result<Option<Uuid>> {
    let mut tx = s.pool.begin().await?;
    let known: Option<(Uuid, bool)> = sqlx::query_as(
        "SELECT u.id, u.disabled FROM user_identities i JOIN users u ON u.id = i.user_id WHERE i.issuer = $1 AND i.subject = $2",
    )
    .bind(issuer)
    .bind(subject)
    .fetch_optional(&mut *tx)
    .await?;
    if let Some((id, disabled)) = known {
        return Ok((!disabled).then_some(id));
    }
    // Link to an existing account only on an email the provider vouches for.
    let by_email: Option<(Uuid, bool)> = match (email, email_verified) {
        (Some(e), true) => {
            sqlx::query_as("SELECT id, disabled FROM users WHERE lower(email) = $1")
                .bind(e)
                .fetch_optional(&mut *tx)
                .await?
        }
        _ => None,
    };
    let (id, disabled) = match by_email {
        Some(found) => found,
        None => {
            let base = preferred
                .or_else(|| email.and_then(|e| e.split('@').next()).map(str::to_string))
                .map(|u| {
                    u.chars()
                        .filter(|c| c.is_alphanumeric() || "._-".contains(*c))
                        .take(48)
                        .collect::<String>()
                })
                .filter(|u| !u.is_empty())
                .unwrap_or_else(|| "user".into());
            let mut username = base.clone();
            let mut n = 2;
            while sqlx::query_scalar::<_, bool>(
                "SELECT EXISTS (SELECT 1 FROM users WHERE lower(username) = lower($1))",
            )
            .bind(&username)
            .fetch_one(&mut *tx)
            .await?
            {
                username = format!("{base}{n}");
                n += 1;
            }
            let household = match &cfg.household {
                Some(h) => match sqlx::query_scalar::<_, Uuid>(
                    "SELECT id FROM households WHERE name = $1 ORDER BY created_at LIMIT 1",
                )
                .bind(h)
                .fetch_optional(&mut *tx)
                .await?
                {
                    Some(id) => id,
                    None => new_household(&mut tx, h).await?,
                },
                None => {
                    new_household(
                        &mut tx,
                        &format!("{}'s household", name.as_deref().unwrap_or(&username)),
                    )
                    .await?
                }
            };
            let id = Uuid::now_v7();
            sqlx::query("INSERT INTO users (id, username, display_name, email, household_id) VALUES ($1, $2, $3, $4, $5)")
                .bind(id)
                .bind(&username)
                .bind(name.unwrap_or_else(|| username.clone()))
                .bind(if email_verified { email } else { None })
                .bind(household)
                .execute(&mut *tx)
                .await?;
            tracing::info!(%username, "account created by single sign-on");
            (id, false)
        }
    };
    sqlx::query("INSERT INTO user_identities (issuer, subject, user_id) VALUES ($1, $2, $3)")
        .bind(issuer)
        .bind(subject)
        .bind(id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok((!disabled).then_some(id))
}

async fn new_household(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    name: &str,
) -> anyhow::Result<Uuid> {
    let id = Uuid::now_v7();
    sqlx::query("INSERT INTO households (id, name) VALUES ($1, $2)")
        .bind(id)
        .bind(name)
        .execute(&mut **tx)
        .await?;
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn return_paths_stay_on_this_site() {
        assert_eq!(safe_return(Some("/dishes/x?s=4".into())), "/dishes/x?s=4");
        assert_eq!(safe_return(Some("//evil.example".into())), "/");
        assert_eq!(safe_return(Some("https://evil.example".into())), "/");
        assert_eq!(safe_return(Some("/\\evil".into())), "/");
        assert_eq!(safe_return(None), "/");
    }

    #[test]
    fn reads_the_groups_claim() {
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(br#"{"sub":"1","groups":["cooks","mise-admins"]}"#);
        assert_eq!(
            groups_claim(&format!("h.{payload}.s")),
            vec!["cooks", "mise-admins"]
        );
        assert!(groups_claim("garbage").is_empty());
    }
}
