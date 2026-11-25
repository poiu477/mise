//! Single sign-on with any OpenID Connect provider (Authentik, Keycloak, Nextcloud…): authorization code flow
//! with PKCE and a nonce, finished only in the browser that started it (a cookie holds the sign-in's state). A
//! returning person is found by their provider identity, else linked to an account with the same verified email
//! (never an admin's), else given a new account when the install lets anyone in (OIDC_OPEN_SIGNUP) or they are in
//! OIDC_ALLOWED_GROUP. With OIDC_ALLOWED_GROUP set, only its members (and OIDC_ADMIN_GROUP's) sign in at all.
//! Membership of OIDC_ADMIN_GROUP makes them admin.

use anyhow::{Context, anyhow};
use axum::{
    extract::State,
    http::HeaderMap,
    response::{IntoResponse, Redirect, Response},
};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
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

use super::{queries, secret_hash};
use crate::{AppState, extract::Query};

/// OIDC_ISSUER, OIDC_CLIENT_ID, OIDC_CLIENT_SECRET, OIDC_NAME, OIDC_SCOPES, OIDC_ADMIN_GROUP, OIDC_HOUSEHOLD.
pub struct OidcConfig {
    /// The provider's issuer URL, where its discovery document is.
    pub issuer: IssuerUrl,
    /// mise's client id at the provider.
    pub client_id: String,
    /// mise's client secret, for a confidential client.
    pub client_secret: Option<String>,
    /// Label on the sign-in button.
    pub name: String,
    /// Scopes to ask for besides `openid` (default `email profile`).
    pub scopes: Vec<String>,
    /// Members of this group (from the `groups` claim) are admins; checked at every sign-in.
    pub admin_group: Option<String>,
    /// New accounts join the household with this name; unset, each gets a household of their own.
    pub household: Option<String>,
    /// Only members of this group (from the `groups` claim), and of `admin_group`, may sign in; a member without an
    /// account gets one. Unset, only people with an account here already sign in, unless `open_signup` is on.
    pub allowed_group: Option<String>,
    /// Anyone the provider signs in gets an account at their first sign-in. Only for a provider whose every user
    /// belongs here (a household's own Authentik), never a public one such as Google.
    pub open_signup: bool,
}

impl OidcConfig {
    /// Reads the provider's settings from the environment; fails, saying which, when one is missing or wrong.
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
            allowed_group: var("OIDC_ALLOWED_GROUP"),
            open_signup: match var("OIDC_OPEN_SIGNUP").as_deref() {
                None | Some("false" | "0") => false,
                Some("true" | "1") => true,
                Some(other) => anyhow::bail!("OIDC_OPEN_SIGNUP is {other:?}: use true or false"),
            },
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

/// Where to go once signed in.
#[derive(Deserialize)]
pub struct StartParams {
    /// Path to return to after signing in.
    pub return_to: Option<String>,
}

/// Only paths on this site, so the callback can't be used to send people elsewhere. A browser drops tabs and line
/// breaks from an address and reads `\` as `/`, so `/\t/evil.example` would leave the site: anything with those, or
/// that a browser would resolve to another host, goes home instead.
fn safe_return(p: Option<String>) -> String {
    const HERE: &str = "http://mise.invalid/";
    p.filter(|p| {
        p.starts_with('/')
            && !p.starts_with("//")
            && !p.contains('\\')
            && !p.chars().any(|c| c.is_control() || c.is_whitespace())
            && url::Url::parse(HERE)
                .and_then(|here| here.join(p))
                .is_ok_and(|to| to.origin() == url::Url::parse(HERE).expect("a URL").origin())
    })
    .unwrap_or_else(|| "/".into())
}

/// The cookie that ties a sign-in to the browser that started it: the state sent to the provider, which the
/// callback must be given back by the same browser, so nobody can finish their own sign-in in someone else's.
const STATE_COOKIE: &str = "mise_oidc_state";

fn state_cookie(state: &str, config: &super::AuthConfig) -> Cookie<'static> {
    Cookie::build((STATE_COOKIE, state.to_owned()))
        .path("/api/auth/oidc")
        .http_only(true)
        // The provider sends the browser back with a top-level GET, which Lax lets the cookie go with.
        .same_site(SameSite::Lax)
        .secure(config.secure_cookies())
        .max_age(time::Duration::minutes(15))
        .build()
}

fn clear_state_cookie(jar: CookieJar) -> CookieJar {
    jar.remove(Cookie::build(STATE_COOKIE).path("/api/auth/oidc"))
}

pub(super) fn to_login(error: &str) -> Response {
    Redirect::to(&format!(
        "/login?error={}",
        url::form_urlencoded::byte_serialize(error.as_bytes()).collect::<String>()
    ))
    .into_response()
}

/// Begin single sign-on: redirect to the provider.
#[utoipa::path(get, path = "/api/auth/oidc/start", operation_id = "oidc_start", tag = "auth",
    params(("return_to" = Option<String>, Query)), security(()),
    responses(
        (status = 303, description = "To the provider, or back to the sign-in page saying why single sign-on isn't available"),
        (status = 400, description = "The query can't be read", body = crate::error::ErrorBody)
    )
)]
pub async fn start(
    State(s): State<AppState>,
    jar: CookieJar,
    headers: HeaderMap,
    Query(p): Query<StartParams>,
) -> Response {
    match start_inner(&s, &headers, p).await {
        Ok((url, state)) => (
            jar.add(state_cookie(&state, &s.auth.config)),
            Redirect::to(&url),
        )
            .into_response(),
        Err(e) => {
            tracing::warn!(error = ?e, "single sign-on could not start");
            to_login("Single sign-on isn't available right now.")
        }
    }
}

/// The provider's sign-in address, and the state it carries.
async fn start_inner(
    s: &AppState,
    headers: &HeaderMap,
    p: StartParams,
) -> anyhow::Result<(String, String)> {
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
    queries::add_oidc_login(
        &s.pool,
        &secret_hash(state.secret()),
        nonce.secret(),
        verifier.secret(),
        &safe_return(p.return_to),
    )
    .await?;
    Ok((url.to_string(), state.secret().clone()))
}

/// What the provider sends back to the callback.
#[derive(Deserialize)]
pub struct CallbackParams {
    /// The authorization code to exchange for tokens.
    pub code: Option<String>,
    /// The state mise sent, to match the sign-in it started.
    pub state: Option<String>,
    /// Why the provider didn't sign the person in, when it didn't.
    pub error: Option<String>,
    /// The provider's words for that.
    pub error_description: Option<String>,
}

/// The provider sends people back here; sign them in and continue where they started.
#[utoipa::path(get, path = "/api/auth/oidc/callback", operation_id = "oidc_callback", tag = "auth", security(()),
    responses(
        (status = 303, description = "Signed in and back to the app, or back to the sign-in page saying why not"),
        (status = 400, description = "The query can't be read", body = crate::error::ErrorBody),
        (status = 500, description = "Signed in at the provider, but the server failed to start the session; the details are in its log", body = crate::error::ErrorBody)
    )
)]
pub async fn callback(
    State(s): State<AppState>,
    jar: CookieJar,
    headers: HeaderMap,
    Query(p): Query<CallbackParams>,
) -> Response {
    let started_here = jar.get(STATE_COOKIE).map(|c| c.value().to_owned());
    let jar = clear_state_cookie(jar);
    if let Some(e) = &p.error {
        tracing::info!(error = %e, description = ?p.error_description, "provider refused sign-in");
        return (jar, to_login("The sign-in provider refused the sign-in.")).into_response();
    }
    match callback_inner(&s, &headers, p, started_here.as_deref()).await {
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
        Ok(Outcome::Refused(why)) => (jar, to_login(why)).into_response(),
        Err(e) => {
            tracing::warn!(error = ?e, "single sign-on failed");
            (jar, to_login("Single sign-on didn't work. Try again.")).into_response()
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
    started_here: Option<&str>,
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
    // Compared as hashes, which take the same time however much of the two agrees.
    if started_here.map(secret_hash) != Some(secret_hash(&state)) {
        return Ok(Outcome::Refused(
            "That sign-in didn't start in this browser. Try again.",
        ));
    }
    let pending = queries::take_oidc_login(&s.pool, &secret_hash(&state))
        .await?
        .map(|l| (l.nonce, l.pkce_verifier, l.return_to));
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

    let member = |group: &Option<String>| group.as_ref().is_some_and(|g| groups.contains(g));
    if cfg.allowed_group.is_some() && !member(&cfg.allowed_group) && !member(&cfg.admin_group) {
        return Ok(Outcome::Refused(
            "Your account at the sign-in provider isn't one this install lets in.",
        ));
    }
    let may_create = cfg.open_signup || cfg.allowed_group.is_some();
    let user = match find_or_create(
        s,
        cfg,
        &issuer,
        &subject,
        email.as_deref(),
        email_verified,
        name,
        preferred,
        may_create,
    )
    .await?
    {
        Account::Active(user) => user,
        Account::Disabled => return Ok(Outcome::Refused("This account is turned off.")),
        Account::None => {
            return Ok(Outcome::Refused(
                "There's no account here for you yet: ask an admin to add one with your email address.",
            ));
        }
        Account::AdminEmail => {
            return Ok(Outcome::Refused(
                "That email address belongs to an admin account, which single sign-on doesn't take over: sign in with its password.",
            ));
        }
    };
    if let Some(group) = &cfg.admin_group {
        queries::set_admin(&s.pool, user, groups.iter().any(|g| g == group)).await?;
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

/// What signing on found for an identity.
enum Account {
    /// An account that may sign in.
    Active(Uuid),
    /// The account is turned off.
    Disabled,
    /// No account, and the install doesn't make one at sign-in.
    None,
    /// The verified email is an admin account's, which signing on never links to: an admin's rights would go to
    /// whoever controls that address at the provider.
    AdminEmail,
}

/// The account for this identity: the one it signed in to before, else an account with its verified email, else a
/// new one when `may_create`.
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
    may_create: bool,
) -> anyhow::Result<Account> {
    let mut tx = s.pool.begin().await?;
    let known = queries::identity(&mut *tx, issuer, subject).await?;
    if let Some((id, disabled)) = known {
        return Ok(if disabled {
            Account::Disabled
        } else {
            Account::Active(id)
        });
    }
    // Link to an existing account only on an email the provider vouches for, and never to an admin's.
    let by_email = match (email, email_verified) {
        (Some(e), true) => queries::by_email(&mut *tx, e)
            .await?
            .map(|u| (u.id, u.disabled, u.is_admin)),
        _ => None,
    };
    let (id, disabled) = match by_email {
        Some((_, _, true)) => return Ok(Account::AdminEmail),
        Some((id, disabled, false)) => (id, disabled),
        None if !may_create => return Ok(Account::None),
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
            while queries::username_taken(&mut *tx, &username).await? {
                username = format!("{base}{n}");
                n += 1;
            }
            let household = match &cfg.household {
                Some(h) => match queries::household_named(&mut *tx, h).await? {
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
            let display_name = name.unwrap_or_else(|| username.clone());
            let account = queries::NewAccount {
                id,
                username: &username,
                display_name: &display_name,
                email: if email_verified { email } else { None },
                password_hash: None,
                is_admin: false,
                household,
            };
            queries::add_account(&mut *tx, account).await?;
            tracing::info!(%username, "account created by single sign-on");
            (id, false)
        }
    };
    queries::add_identity(&mut *tx, issuer, subject, id).await?;
    tx.commit().await?;
    Ok(if disabled {
        Account::Disabled
    } else {
        Account::Active(id)
    })
}

async fn new_household(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    name: &str,
) -> anyhow::Result<Uuid> {
    let id = Uuid::now_v7();
    queries::add_household(&mut **tx, id, name).await?;
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
        // Browsers drop tabs and line breaks, which would turn these into //evil.example.
        for sneaky in [
            "/\t/evil.example",
            "/\n/evil.example",
            "/\r/evil.example",
            "/ /evil.example",
        ] {
            assert_eq!(safe_return(Some(sneaky.into())), "/", "{sneaky:?}");
        }
        // Encoded, they are only a path here.
        assert_eq!(safe_return(Some("/%09/x".into())), "/%09/x");
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
