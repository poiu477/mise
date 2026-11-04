//! Single sign-on end to end. A fake OpenID Connect provider runs inside each test (discovery, signing keys and a
//! token endpoint that checks PKCE) and signs its own ID tokens with RS256, ES256 and EdDSA, independently of the
//! openidconnect crate that mise verifies them with. Each test runs on a fresh database created on the dev server
//! from DATABASE_URL (skipped when it is not set; CI fails instead).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use axum::{
    Form, Json, Router,
    body::Body,
    extract::State,
    http::{Request, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use base64::Engine;
use http_body_util::BodyExt;
use mise_api::auth::{Auth, AuthConfig, oidc::OidcConfig};
use openidconnect::IssuerUrl;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use tower::ServiceExt;

const CLIENT_ID: &str = "mise-test";
const HOST: &str = "mise.test";

fn b64(bytes: impl AsRef<[u8]>) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

// --- The fake provider ----------------------------------------------------------------------------------------

/// The provider's signing keys, plus an impostor RSA key that claims the real key's id.
struct Keys {
    rsa: rsa::RsaPrivateKey,
    impostor: rsa::RsaPrivateKey,
    ec: p256::ecdsa::SigningKey,
    ed: ed25519_dalek::SigningKey,
}

/// RSA key generation is slow, so the keys are made once per test run.
fn keys() -> &'static Keys {
    static KEYS: OnceLock<Keys> = OnceLock::new();
    KEYS.get_or_init(|| {
        let mut rng = rsa::rand_core::OsRng;
        Keys {
            rsa: rsa::RsaPrivateKey::new(&mut rng, 2048).unwrap(),
            impostor: rsa::RsaPrivateKey::new(&mut rng, 2048).unwrap(),
            // Fixed test-only keys: any 32 bytes make a valid P-256 scalar here and an Ed25519 seed.
            ec: p256::ecdsa::SigningKey::from_slice(&[7u8; 32]).unwrap(),
            ed: ed25519_dalek::SigningKey::from_bytes(&[9u8; 32]),
        }
    })
}

#[derive(Clone, Copy, Debug)]
enum Alg {
    Rs256,
    Es256,
    EdDsa,
    /// RS256 under the real key's id, signed with a key the provider never published.
    ForgedRs256,
}

/// A signed JWT: `header.claims.signature`, each part base64url.
fn sign(alg: Alg, claims: &Value) -> String {
    let k = keys();
    let (name, kid) = match alg {
        Alg::Rs256 | Alg::ForgedRs256 => ("RS256", "rsa"),
        Alg::Es256 => ("ES256", "ec"),
        Alg::EdDsa => ("EdDSA", "ed"),
    };
    let header = json!({"alg": name, "kid": kid, "typ": "JWT"});
    let input = format!("{}.{}", b64(header.to_string()), b64(claims.to_string()));
    let signature: Vec<u8> = match alg {
        Alg::Rs256 | Alg::ForgedRs256 => {
            use rsa::signature::{SignatureEncoding, Signer};
            let key = if matches!(alg, Alg::Rs256) {
                &k.rsa
            } else {
                &k.impostor
            };
            rsa::pkcs1v15::SigningKey::<rsa::sha2::Sha256>::new(key.clone())
                .sign(input.as_bytes())
                .to_vec()
        }
        Alg::Es256 => {
            use p256::ecdsa::signature::Signer;
            let sig: p256::ecdsa::Signature = k.ec.sign(input.as_bytes());
            sig.to_bytes().to_vec()
        }
        Alg::EdDsa => {
            use ed25519_dalek::Signer;
            k.ed.sign(input.as_bytes()).to_bytes().to_vec()
        }
    };
    format!("{input}.{}", b64(signature))
}

fn jwks() -> Value {
    use rsa::traits::PublicKeyParts;
    let k = keys();
    // Uncompressed SEC1: 0x04, then x and y (32 bytes each for P-256).
    let point = k.ec.verifying_key().to_sec1_point(false);
    let (x, y) = point.as_bytes()[1..].split_at(32);
    json!({"keys": [
        {"kty": "RSA", "kid": "rsa", "use": "sig", "alg": "RS256",
         "n": b64(k.rsa.n().to_bytes_be()), "e": b64(k.rsa.e().to_bytes_be())},
        {"kty": "EC", "kid": "ec", "use": "sig", "alg": "ES256", "crv": "P-256",
         "x": b64(x), "y": b64(y)},
        {"kty": "OKP", "kid": "ed", "use": "sig", "alg": "EdDSA", "crv": "Ed25519",
         "x": b64(k.ed.verifying_key().to_bytes())},
    ]})
}

/// What the provider will put in the ID token for one authorization code.
struct Grant {
    challenge: String,
    alg: Alg,
    claims: Value,
}

struct Provider {
    issuer: String,
    grants: Mutex<HashMap<String, Grant>>,
}

async fn discovery(State(p): State<Arc<Provider>>) -> Json<Value> {
    Json(json!({
        "issuer": p.issuer,
        "authorization_endpoint": format!("{}/authorize", p.issuer),
        "token_endpoint": format!("{}/token", p.issuer),
        "jwks_uri": format!("{}/jwks", p.issuer),
        "response_types_supported": ["code"],
        "subject_types_supported": ["public"],
        "id_token_signing_alg_values_supported": ["RS256", "ES256", "EdDSA"],
    }))
}

/// Swap a code for tokens, as a real provider would: once only, and only with the PKCE verifier that matches.
async fn token(State(p): State<Arc<Provider>>, Form(f): Form<HashMap<String, String>>) -> Response {
    let refuse = |why: &str| {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "invalid_grant", "error_description": why})),
        )
            .into_response()
    };
    if f.get("grant_type").map(String::as_str) != Some("authorization_code") {
        return refuse("not an authorization code grant");
    }
    let Some(grant) = f
        .get("code")
        .and_then(|c| p.grants.lock().unwrap().remove(c))
    else {
        return refuse("unknown or used code");
    };
    let verifier = f.get("code_verifier").cloned().unwrap_or_default();
    if b64(Sha256::digest(verifier.as_bytes())) != grant.challenge {
        return refuse("PKCE verifier doesn't match the challenge");
    }
    Json(json!({
        "access_token": "test-access-token",
        "token_type": "Bearer",
        "expires_in": 3600,
        "id_token": sign(grant.alg, &grant.claims),
    }))
    .into_response()
}

async fn provider() -> Arc<Provider> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let p = Arc::new(Provider {
        issuer: format!("http://{}", listener.local_addr().unwrap()),
        grants: Mutex::default(),
    });
    let app = Router::new()
        .route("/.well-known/openid-configuration", get(discovery))
        .route("/jwks", get(|| async { Json(jwks()) }))
        .route("/token", post(token))
        .with_state(p.clone());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    p
}

// --- mise, set up to use it -----------------------------------------------------------------------------------

struct TestApp {
    router: Router,
    pool: PgPool,
    admin: PgPool,
    db: String,
    idp: Arc<Provider>,
}

/// One sign-in as the browser sees it: where mise sent it, and the session it got.
struct Finished {
    status: StatusCode,
    location: String,
    cookie: Option<String>,
}

impl TestApp {
    async fn new() -> Option<TestApp> {
        dotenvy::from_path(concat!(env!("CARGO_MANIFEST_DIR"), "/../../.env")).ok();
        // Skipping is for a laptop without Postgres; in CI a missing or unreachable database must fail, not pass
        // quietly.
        let in_ci = std::env::var_os("CI").is_some();
        let url = match std::env::var("DATABASE_URL") {
            Ok(url) => url,
            Err(_) if in_ci => panic!("DATABASE_URL must be set in CI"),
            Err(_) => return None,
        };
        let admin = match mise_db::connect(&url).await {
            Ok(pool) => pool,
            Err(e) if in_ci => panic!("can't connect to DATABASE_URL in CI: {e}"),
            Err(_) => return None,
        };
        let db = format!("mise_test_{}", uuid::Uuid::now_v7().simple());
        // The name is ours (a UUID), not input.
        sqlx::query(sqlx::AssertSqlSafe(format!("CREATE DATABASE {db}")))
            .execute(&admin)
            .await
            .unwrap();
        let base = url.rsplit_once('/').unwrap().0;
        let pool = mise_db::connect(&format!("{base}/{db}")).await.unwrap();
        mise_db::migrate(&pool).await.unwrap();
        let idp = provider().await;
        let config = AuthConfig {
            local: true,
            oidc: Some(OidcConfig {
                issuer: IssuerUrl::new(idp.issuer.clone()).unwrap(),
                client_id: CLIENT_ID.into(),
                client_secret: Some("test-secret".into()),
                name: "Test sign-on".into(),
                scopes: vec!["email".into(), "profile".into()],
                admin_group: Some("mise-admins".into()),
                household: None,
            }),
            public_base_url: None,
            session_days: 30,
        };
        let state = mise_api::AppState {
            pool: pool.clone(),
            media_root: "./no-media".into(),
            auth: Arc::new(Auth::new(config)),
        };
        Some(TestApp {
            router: mise_api::router(state, None),
            pool,
            admin,
            db,
            idp,
        })
    }

    async fn get(&self, path: &str, cookie: Option<&str>) -> axum::response::Response {
        let mut b = Request::builder().uri(path).header(header::HOST, HOST);
        if let Some(c) = cookie {
            b = b.header(header::COOKIE, format!("mise_session={c}"));
        }
        self.router
            .clone()
            .oneshot(b.body(Body::empty()).unwrap())
            .await
            .unwrap()
    }

    async fn json(&self, method: &str, path: &str, cookie: &str, body: Option<Value>) -> Value {
        let b = Request::builder()
            .method(method)
            .uri(path)
            .header(header::HOST, HOST)
            .header(header::COOKIE, format!("mise_session={cookie}"));
        let req = match body {
            Some(v) => b
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(v.to_string())),
            None => b.body(Body::empty()),
        };
        let res = self.router.clone().oneshot(req.unwrap()).await.unwrap();
        assert!(
            res.status().is_success(),
            "{method} {path}: {}",
            res.status()
        );
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    }

    /// Start a sign-in and check the redirect to the provider; returns the state and nonce it carries.
    async fn begin(&self, return_to: &str) -> (String, String) {
        let res = self
            .get(&format!("/api/auth/oidc/start?return_to={return_to}"), None)
            .await;
        assert_eq!(res.status(), StatusCode::SEE_OTHER);
        let to = url::Url::parse(res.headers()[header::LOCATION].to_str().unwrap()).unwrap();
        assert_eq!(
            to.as_str().split('?').next().unwrap(),
            format!("{}/authorize", self.idp.issuer)
        );
        let q: HashMap<String, String> = to.query_pairs().into_owned().collect();
        assert_eq!(q["client_id"], CLIENT_ID);
        assert_eq!(q["response_type"], "code");
        assert_eq!(q["code_challenge_method"], "S256");
        assert_eq!(
            q["redirect_uri"],
            format!("http://{HOST}/api/auth/oidc/callback")
        );
        let scopes: Vec<&str> = q["scope"].split(' ').collect();
        assert!(
            ["openid", "email", "profile"]
                .iter()
                .all(|s| scopes.contains(s))
        );
        // The challenge is kept for the token endpoint to check the verifier against.
        self.idp.grants.lock().unwrap().insert(
            format!("code-for-{}", q["state"]),
            Grant {
                challenge: q["code_challenge"].clone(),
                alg: Alg::Rs256,
                claims: Value::Null,
            },
        );
        (q["state"].clone(), q["nonce"].clone())
    }

    /// The provider sends the browser back with a code; mise exchanges it for an ID token with these claims.
    async fn finish(&self, state: &str, alg: Alg, claims: Value) -> Finished {
        let code = format!("code-for-{state}");
        if let Some(g) = self.idp.grants.lock().unwrap().get_mut(&code) {
            g.alg = alg;
            g.claims = claims;
        }
        let res = self
            .get(
                &format!("/api/auth/oidc/callback?code={code}&state={state}"),
                None,
            )
            .await;
        let cookie = res
            .headers()
            .get_all(header::SET_COOKIE)
            .iter()
            .filter_map(|h| h.to_str().ok())
            .find_map(|c| {
                c.strip_prefix("mise_session=")
                    .map(|v| v.split(';').next().unwrap().to_string())
            })
            .filter(|v| !v.is_empty());
        Finished {
            status: res.status(),
            location: res
                .headers()
                .get(header::LOCATION)
                .map(|l| l.to_str().unwrap().to_string())
                .unwrap_or_default(),
            cookie,
        }
    }

    /// The whole round trip with the provider's usual claims for this person.
    async fn sign_in(&self, alg: Alg, person: Value) -> Finished {
        let (state, nonce) = self.begin("/plan").await;
        self.finish(&state, alg, self.claims(&nonce, person)).await
    }

    /// Standard claims for this provider and client, with the person's own on top.
    fn claims(&self, nonce: &str, person: Value) -> Value {
        let now = chrono::Utc::now().timestamp();
        let mut c = json!({
            "iss": self.idp.issuer, "aud": CLIENT_ID, "iat": now, "exp": now + 300, "nonce": nonce,
        });
        c.as_object_mut()
            .unwrap()
            .extend(person.as_object().unwrap().clone());
        c
    }

    async fn users(&self) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM users")
            .fetch_one(&self.pool)
            .await
            .unwrap()
    }

    async fn drop(self) {
        self.pool.close().await;
        let _ = sqlx::query(sqlx::AssertSqlSafe(format!(
            "DROP DATABASE IF EXISTS {} WITH (FORCE)",
            self.db
        )))
        .execute(&self.admin)
        .await;
    }
}

macro_rules! app {
    () => {
        match TestApp::new().await {
            Some(a) => a,
            None => {
                eprintln!("DATABASE_URL not set; skipping");
                return;
            }
        }
    };
}

fn alice(groups: &[&str]) -> Value {
    json!({
        "sub": "alice-1", "email": "Alice@Example.test", "email_verified": true,
        "name": "Alice Example", "preferred_username": "alice", "groups": groups,
    })
}

fn signed_in(f: &Finished, to: &str) -> String {
    assert_eq!(f.status, StatusCode::SEE_OTHER);
    assert_eq!(f.location, to, "should continue where the sign-in started");
    f.cookie.clone().expect("a session cookie")
}

/// Sent back to the sign-in page with this message, and no session.
fn refused(f: &Finished, message: &str) {
    assert_eq!(f.status, StatusCode::SEE_OTHER);
    let shown = f.location.strip_prefix("/login?error=").map(|e| {
        url::form_urlencoded::parse(format!("e={e}").as_bytes())
            .next()
            .unwrap()
            .1
            .into_owned()
    });
    assert_eq!(shown.as_deref(), Some(message), "went to {:?}", f.location);
    assert!(
        f.cookie.is_none(),
        "a refused sign-in must not get a session"
    );
}

const DIDNT_WORK: &str = "Single sign-on didn't work. Try again.";
const EXPIRED: &str = "That sign-in expired. Try again.";

// --- The tests ------------------------------------------------------------------------------------------------

#[tokio::test]
async fn a_new_person_signs_in_and_comes_back_to_the_same_account() {
    let app = app!();
    let first = signed_in(
        &app.sign_in(Alg::Rs256, alice(&["cooks", "mise-admins"]))
            .await,
        "/plan",
    );
    let me = app.json("GET", "/api/me", &first, None).await;
    assert_eq!(me["username"], "alice");
    assert_eq!(me["display_name"], "Alice Example");
    assert_eq!(
        me["email"], "alice@example.test",
        "emails are kept in lower case"
    );
    assert_eq!(
        me["is_admin"], true,
        "a member of OIDC_ADMIN_GROUP is an admin"
    );
    assert_eq!(me["household"]["name"], "Alice Example's household");
    assert_eq!(me["has_password"], false);

    // The same identity again finds the same account, and admin follows the groups at every sign-in.
    let again = signed_in(&app.sign_in(Alg::Rs256, alice(&["cooks"])).await, "/plan");
    let me2 = app.json("GET", "/api/me", &again, None).await;
    assert_eq!(me2["id"], me["id"]);
    assert_eq!(me2["is_admin"], false);
    assert_eq!(app.users().await, 1);
    app.drop().await;
}

#[tokio::test]
async fn tokens_signed_with_each_accepted_algorithm_verify() {
    let app = app!();
    for (alg, sub) in [(Alg::Rs256, "rs"), (Alg::Es256, "es"), (Alg::EdDsa, "ed")] {
        let person = json!({"sub": sub, "preferred_username": sub});
        let cookie = signed_in(&app.sign_in(alg, person).await, "/plan");
        let me = app.json("GET", "/api/me", &cookie, None).await;
        assert_eq!(me["username"], sub, "{alg:?}");
    }
    assert_eq!(app.users().await, 3);
    app.drop().await;
}

#[tokio::test]
async fn only_a_verified_email_links_to_an_existing_account() {
    let app = app!();
    // An admin made through first-run setup adds Bea, who has a local password and an email.
    let res = app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/auth/setup")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({"username": "paul", "password": "a long password"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let admin = res
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .find_map(|h| {
            h.to_str()
                .ok()?
                .strip_prefix("mise_session=")
                .map(|v| v.split(';').next().unwrap().to_string())
        })
        .unwrap();
    let bea = app
        .json(
            "POST",
            "/api/users",
            &admin,
            Some(json!({"username": "bea", "password": "bea's long password", "email": "bea@example.test"})),
        )
        .await;

    // An unverified email proves nothing: a new account, without the email.
    let unverified = json!({"sub": "bea-x", "email": "bea@example.test", "email_verified": false, "preferred_username": "bea"});
    let cookie = signed_in(&app.sign_in(Alg::Es256, unverified).await, "/plan");
    let me = app.json("GET", "/api/me", &cookie, None).await;
    assert_ne!(me["id"], bea["id"]);
    assert_eq!(me["username"], "bea2", "a taken username gets a number");
    assert_eq!(me["email"], Value::Null);

    // A verified one links to Bea's own account.
    let verified = json!({"sub": "bea-1", "email": "BEA@example.test", "email_verified": true});
    let cookie = signed_in(&app.sign_in(Alg::Es256, verified).await, "/plan");
    let me = app.json("GET", "/api/me", &cookie, None).await;
    assert_eq!(me["id"], bea["id"]);
    assert_eq!(me["has_password"], true, "linking keeps the local password");
    app.drop().await;
}

#[tokio::test]
async fn forged_or_mismatched_tokens_are_refused() {
    let app = app!();
    let person = || json!({"sub": "mallory", "preferred_username": "mallory"});

    // Signed with a key the provider never published, under the real key's id.
    refused(&app.sign_in(Alg::ForgedRs256, person()).await, DIDNT_WORK);

    let now = chrono::Utc::now().timestamp();
    // Each case: what's wrong, and the claims that make it so (laid over an otherwise good token).
    let cases = [
        ("another sign-in's nonce", json!({"nonce": "not-the-nonce"})),
        ("another client's token", json!({"aud": "someone-else"})),
        (
            "another issuer's token",
            json!({"iss": "https://elsewhere.example"}),
        ),
        (
            "an expired token",
            json!({"iat": now - 7200, "exp": now - 3600}),
        ),
    ];
    for (what, wrong) in cases {
        let (state, nonce) = app.begin("/plan").await;
        let mut claims = app.claims(&nonce, person());
        claims
            .as_object_mut()
            .unwrap()
            .extend(wrong.as_object().unwrap().clone());
        let f = app.finish(&state, Alg::Rs256, claims).await;
        assert!(f.cookie.is_none(), "{what} must not sign in");
        refused(&f, DIDNT_WORK);
    }
    assert_eq!(app.users().await, 0);
    app.drop().await;
}

#[tokio::test]
async fn a_sign_in_finishes_once_and_only_with_its_own_state() {
    let app = app!();
    let (state, nonce) = app.begin("/dishes/x").await;
    let claims = app.claims(&nonce, alice(&[]));
    signed_in(
        &app.finish(&state, Alg::Rs256, claims.clone()).await,
        "/dishes/x",
    );
    // Replaying the same reply (code and state) is refused: the state was used up.
    refused(&app.finish(&state, Alg::Rs256, claims).await, EXPIRED);
    // So is a state mise never issued.
    refused(
        &app.finish("made-up-state", Alg::Rs256, alice(&[])).await,
        EXPIRED,
    );
    // And a reply with no code.
    let res = app
        .get(&format!("/api/auth/oidc/callback?state={state}"), None)
        .await;
    assert_eq!(
        res.headers()[header::LOCATION].to_str().unwrap(),
        "/login?error=The+sign-in+reply+was+incomplete."
    );
    app.drop().await;
}

#[tokio::test]
async fn a_turned_off_account_cannot_sign_in() {
    let app = app!();
    signed_in(&app.sign_in(Alg::EdDsa, alice(&[])).await, "/plan");
    sqlx::query("UPDATE users SET disabled = true")
        .execute(&app.pool)
        .await
        .unwrap();
    refused(
        &app.sign_in(Alg::EdDsa, alice(&[])).await,
        "This account is turned off.",
    );
    app.drop().await;
}
