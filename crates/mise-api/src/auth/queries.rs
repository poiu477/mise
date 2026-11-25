//! The accounts' SQL: sessions, API tokens, accounts and households, kitchen devices and single sign-on. Each
//! statement is checked against the schema when mise is built.

use sqlx::PgExecutor;
use time::OffsetDateTime;
use uuid::Uuid;

use super::{
    kitchen::KitchenToken,
    routes::{Household, TokenInfo},
};

// Sessions and API tokens.

/// The account a secret signs in, as a request needs it.
pub struct SignedIn {
    /// The account.
    pub id: Uuid,
    /// What it signs in with.
    pub username: String,
    /// It is an admin.
    pub is_admin: bool,
    /// Its household.
    pub household_id: Uuid,
}

/// Starts a session for `user` lasting `days`, under the hash of its secret, after clearing the ended ones.
pub async fn add_session(
    db: &sqlx::PgPool,
    token_hash: &str,
    user: Uuid,
    days: i32,
    user_agent: Option<&str>,
) -> sqlx::Result<()> {
    sqlx::query!("DELETE FROM sessions WHERE expires_at < now()")
        .execute(db)
        .await?;
    sqlx::query!(
        "INSERT INTO sessions (token_hash, user_id, expires_at, user_agent)
         VALUES ($1, $2, now() + make_interval(days => $3::int), $4)",
        token_hash,
        user,
        days,
        user_agent
    )
    .execute(db)
    .await?;
    Ok(())
}

/// The account of the live session with this hash, unless the account is turned off, and when the session was
/// last seen.
pub async fn session(
    db: impl PgExecutor<'_>,
    token_hash: &str,
) -> sqlx::Result<Option<(SignedIn, OffsetDateTime)>> {
    Ok(sqlx::query!(
        "SELECT u.id, u.username, u.is_admin, u.household_id, s.last_seen_at
         FROM sessions s JOIN users u ON u.id = s.user_id
         WHERE s.token_hash = $1 AND s.expires_at > now() AND NOT u.disabled",
        token_hash
    )
    .fetch_optional(db)
    .await?
    .map(|r| {
        let user = SignedIn {
            id: r.id,
            username: r.username,
            is_admin: r.is_admin,
            household_id: r.household_id,
        };
        (user, r.last_seen_at)
    }))
}

/// Marks the session seen now and moves its end `days` on.
pub async fn renew_session(
    db: impl PgExecutor<'_>,
    token_hash: &str,
    days: i32,
) -> sqlx::Result<()> {
    sqlx::query!(
        "UPDATE sessions SET last_seen_at = now(), expires_at = now() + make_interval(days => $2::int)
         WHERE token_hash = $1",
        token_hash,
        days
    )
    .execute(db)
    .await?;
    Ok(())
}

/// Ends the session with this hash.
pub async fn end_session(db: impl PgExecutor<'_>, token_hash: &str) -> sqlx::Result<()> {
    sqlx::query!("DELETE FROM sessions WHERE token_hash = $1", token_hash)
        .execute(db)
        .await?;
    Ok(())
}

/// Ends every session of `user` but the one with hash `keep`.
pub async fn end_other_sessions(
    db: impl PgExecutor<'_>,
    user: Uuid,
    keep: &str,
) -> sqlx::Result<()> {
    sqlx::query!(
        "DELETE FROM sessions WHERE user_id = $1 AND token_hash <> $2",
        user,
        keep
    )
    .execute(db)
    .await?;
    Ok(())
}

/// The account of the API token with this hash, unless the account is turned off, and the token's scope. Marks the
/// token used, at most hourly.
pub async fn api_token(
    db: &sqlx::PgPool,
    token_hash: &str,
) -> sqlx::Result<Option<(SignedIn, String)>> {
    let Some(r) = sqlx::query!(
        "SELECT u.id, u.username, u.is_admin, u.household_id, t.scope
         FROM api_tokens t JOIN users u ON u.id = t.user_id
         WHERE t.token_hash = $1 AND NOT u.disabled",
        token_hash
    )
    .fetch_optional(db)
    .await?
    else {
        return Ok(None);
    };
    sqlx::query!(
        "UPDATE api_tokens SET last_used_at = now()
         WHERE token_hash = $1 AND (last_used_at IS NULL OR last_used_at < now() - interval '1 hour')",
        token_hash
    )
    .execute(db)
    .await?;
    let user = SignedIn {
        id: r.id,
        username: r.username,
        is_admin: r.is_admin,
        household_id: r.household_id,
    };
    Ok(Some((user, r.scope)))
}

/// `user`'s API tokens, oldest first.
pub async fn api_tokens(db: impl PgExecutor<'_>, user: Uuid) -> sqlx::Result<Vec<TokenInfo>> {
    sqlx::query_as!(
        TokenInfo,
        "SELECT id, name, scope, created_at, last_used_at FROM api_tokens WHERE user_id = $1 ORDER BY created_at",
        user
    )
    .fetch_all(db)
    .await
}

/// Adds an API token for `user`, under the hash of its secret; answers with when it was made.
pub async fn add_api_token(
    db: impl PgExecutor<'_>,
    id: Uuid,
    user: Uuid,
    name: &str,
    token_hash: &str,
    scope: &str,
) -> sqlx::Result<OffsetDateTime> {
    sqlx::query_scalar!(
        "INSERT INTO api_tokens (id, user_id, name, token_hash, scope) VALUES ($1, $2, $3, $4, $5)
         RETURNING created_at",
        id,
        user,
        name,
        token_hash,
        scope
    )
    .fetch_one(db)
    .await
}

/// Revokes `user`'s API token `id`; answers whether there was one.
pub async fn delete_api_token(db: impl PgExecutor<'_>, id: Uuid, user: Uuid) -> sqlx::Result<bool> {
    let done = sqlx::query!(
        "DELETE FROM api_tokens WHERE id = $1 AND user_id = $2",
        id,
        user
    )
    .execute(db)
    .await?;
    Ok(done.rows_affected() > 0)
}

// Accounts and households.

/// Waits for any other change to who is an admin, or to whether the install is set up; held until the transaction
/// ends, so two such changes can't each see the other's state from before.
pub async fn lock_accounts(db: impl PgExecutor<'_>, key: i64) -> sqlx::Result<()> {
    sqlx::query!("SELECT pg_advisory_xact_lock($1)", key)
        .execute(db)
        .await?;
    Ok(())
}

/// How many accounts there are.
pub async fn count_users(db: impl PgExecutor<'_>) -> sqlx::Result<i64> {
    sqlx::query_scalar!(r#"SELECT count(*) AS "count!" FROM users"#)
        .fetch_one(db)
        .await
}

/// An account, with its household.
pub struct AccountRow {
    /// The account.
    pub id: Uuid,
    /// What it signs in with.
    pub username: String,
    /// Its name as people read it.
    pub display_name: String,
    /// Its email address.
    pub email: Option<String>,
    /// It is an admin.
    pub is_admin: bool,
    /// It is turned off.
    pub disabled: bool,
    /// Its household.
    pub household_id: Uuid,
    /// Its household's name.
    pub household: String,
    /// It can sign in with a password.
    pub has_password: bool,
    /// Single-sign-on identities linked to it.
    pub identities: i64,
}

/// Account `id`.
pub async fn account(db: impl PgExecutor<'_>, id: Uuid) -> sqlx::Result<Option<AccountRow>> {
    sqlx::query_as!(
        AccountRow,
        r#"SELECT u.id, u.username, u.display_name, u.email, u.is_admin, u.disabled,
                  h.id AS household_id, h.name AS household,
                  u.password_hash IS NOT NULL AS "has_password!",
                  (SELECT count(*) FROM user_identities i WHERE i.user_id = u.id) AS "identities!"
           FROM users u JOIN households h ON h.id = u.household_id WHERE u.id = $1"#,
        id
    )
    .fetch_optional(db)
    .await
}

/// Every account, by username.
pub async fn accounts(db: impl PgExecutor<'_>) -> sqlx::Result<Vec<AccountRow>> {
    sqlx::query_as!(
        AccountRow,
        r#"SELECT u.id, u.username, u.display_name, u.email, u.is_admin, u.disabled,
                  h.id AS household_id, h.name AS household,
                  u.password_hash IS NOT NULL AS "has_password!",
                  (SELECT count(*) FROM user_identities i WHERE i.user_id = u.id) AS "identities!"
           FROM users u JOIN households h ON h.id = u.household_id ORDER BY lower(u.username)"#
    )
    .fetch_all(db)
    .await
}

/// The account whose username is `key`, matched without case: its id, password hash, and whether it is turned off.
pub struct LoginRow {
    /// The account.
    pub id: Uuid,
    /// Its password hash; none for an account that signs in only with single sign-on.
    pub password_hash: Option<String>,
    /// It is turned off.
    pub disabled: bool,
}

/// The account a password sign-in names, by its lowercased username.
pub async fn login(db: impl PgExecutor<'_>, key: &str) -> sqlx::Result<Option<LoginRow>> {
    sqlx::query_as!(
        LoginRow,
        "SELECT id, password_hash, disabled FROM users WHERE lower(username) = $1",
        key
    )
    .fetch_optional(db)
    .await
}

/// `user`'s password hash; none when it signs in only with single sign-on.
pub async fn password_hash(db: impl PgExecutor<'_>, user: Uuid) -> sqlx::Result<Option<String>> {
    sqlx::query_scalar!("SELECT password_hash FROM users WHERE id = $1", user)
        .fetch_one(db)
        .await
}

/// Sets `user`'s password hash.
pub async fn set_password_hash(
    db: impl PgExecutor<'_>,
    user: Uuid,
    hash: &str,
) -> sqlx::Result<()> {
    sqlx::query!(
        "UPDATE users SET password_hash = $2 WHERE id = $1",
        user,
        hash
    )
    .execute(db)
    .await?;
    Ok(())
}

/// An account to add.
pub struct NewAccount<'a> {
    /// Its id.
    pub id: Uuid,
    /// What it signs in with.
    pub username: &'a str,
    /// Its name as people read it.
    pub display_name: &'a str,
    /// Its email address.
    pub email: Option<&'a str>,
    /// Its password hash; none for one that signs in only with single sign-on.
    pub password_hash: Option<&'a str>,
    /// It is an admin.
    pub is_admin: bool,
    /// Its household.
    pub household: Uuid,
}

/// Adds an account.
pub async fn add_account(db: impl PgExecutor<'_>, a: NewAccount<'_>) -> sqlx::Result<()> {
    sqlx::query!(
        "INSERT INTO users (id, username, display_name, email, password_hash, is_admin, household_id)
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
        a.id,
        a.username,
        a.display_name,
        a.email,
        a.password_hash,
        a.is_admin,
        a.household
    )
    .execute(db)
    .await?;
    Ok(())
}

/// Whether an account signs in as `username`, or has `email`, matched without case.
pub async fn name_taken(
    db: impl PgExecutor<'_>,
    username: &str,
    email: &str,
) -> sqlx::Result<bool> {
    sqlx::query_scalar!(
        r#"SELECT EXISTS (SELECT 1 FROM users
                          WHERE lower(username) = lower($1) OR (email IS NOT NULL AND lower(email) = lower($2)))
                  AS "taken!""#,
        username,
        email
    )
    .fetch_one(db)
    .await
}

/// Whether an account is an admin and whether it is turned off.
pub struct RoleRow {
    /// It is an admin.
    pub is_admin: bool,
    /// It is turned off.
    pub disabled: bool,
}

/// Account `id`'s role.
pub async fn role(db: impl PgExecutor<'_>, id: Uuid) -> sqlx::Result<Option<RoleRow>> {
    sqlx::query_as!(
        RoleRow,
        "SELECT is_admin, disabled FROM users WHERE id = $1",
        id
    )
    .fetch_optional(db)
    .await
}

/// How many admins, other than account `id`, can sign in.
pub async fn other_admins(db: impl PgExecutor<'_>, id: Uuid) -> sqlx::Result<i64> {
    sqlx::query_scalar!(
        r#"SELECT count(*) AS "count!" FROM users WHERE is_admin AND NOT disabled AND id <> $1"#,
        id
    )
    .fetch_one(db)
    .await
}

/// Changes account `id`: its household and password only when given.
pub async fn update_account(
    db: impl PgExecutor<'_>,
    id: Uuid,
    is_admin: bool,
    disabled: bool,
    household: Option<Uuid>,
    password_hash: Option<&str>,
) -> sqlx::Result<()> {
    sqlx::query!(
        "UPDATE users SET is_admin = $2, disabled = $3, household_id = coalesce($4, household_id),
                          password_hash = coalesce($5, password_hash)
         WHERE id = $1",
        id,
        is_admin,
        disabled,
        household,
        password_hash
    )
    .execute(db)
    .await?;
    Ok(())
}

/// Makes account `id` an admin, or not.
pub async fn set_admin(db: impl PgExecutor<'_>, id: Uuid, is_admin: bool) -> sqlx::Result<()> {
    sqlx::query!("UPDATE users SET is_admin = $2 WHERE id = $1", id, is_admin)
        .execute(db)
        .await?;
    Ok(())
}

/// Signs account `id` out everywhere.
pub async fn end_sessions(db: impl PgExecutor<'_>, id: Uuid) -> sqlx::Result<()> {
    sqlx::query!("DELETE FROM sessions WHERE user_id = $1", id)
        .execute(db)
        .await?;
    Ok(())
}

/// Adds household `id`.
pub async fn add_household(db: impl PgExecutor<'_>, id: Uuid, name: &str) -> sqlx::Result<()> {
    sqlx::query!(
        "INSERT INTO households (id, name) VALUES ($1, $2)",
        id,
        name
    )
    .execute(db)
    .await?;
    Ok(())
}

/// Whether household `id` exists.
pub async fn household_exists(db: impl PgExecutor<'_>, id: Uuid) -> sqlx::Result<bool> {
    sqlx::query_scalar!(
        r#"SELECT EXISTS (SELECT 1 FROM households WHERE id = $1) AS "exists!""#,
        id
    )
    .fetch_one(db)
    .await
}

/// Every household, by name.
pub async fn households(db: impl PgExecutor<'_>) -> sqlx::Result<Vec<Household>> {
    sqlx::query_as!(Household, "SELECT id, name FROM households ORDER BY name")
        .fetch_all(db)
        .await
}

// Kitchen devices.

/// The kitchen token whose cookie has this hash, and its household; marks it used, at most hourly, answering
/// whether it did (when the cookie is renewed too).
pub async fn kitchen_session(
    db: &sqlx::PgPool,
    token_hash: &str,
) -> sqlx::Result<Option<(Uuid, Uuid, bool)>> {
    let Some(t) = sqlx::query!(
        "SELECT id, household_id FROM kitchen_tokens WHERE token_hash = $1",
        token_hash
    )
    .fetch_optional(db)
    .await?
    else {
        return Ok(None);
    };
    let renewed = sqlx::query!(
        "UPDATE kitchen_tokens SET last_used_at = now()
         WHERE id = $1 AND (last_used_at IS NULL OR last_used_at < now() - interval '1 hour')",
        t.id
    )
    .execute(db)
    .await?
    .rows_affected()
        > 0;
    Ok(Some((t.id, t.household_id, renewed)))
}

/// Kitchen token `id`'s name and household.
pub async fn kitchen_device(
    db: impl PgExecutor<'_>,
    id: Uuid,
) -> sqlx::Result<Option<(String, Uuid)>> {
    Ok(sqlx::query!(
        "SELECT name, household_id FROM kitchen_tokens WHERE id = $1",
        id
    )
    .fetch_optional(db)
    .await?
    .map(|t| (t.name, t.household_id)))
}

/// The household's kitchen tokens, oldest first.
pub async fn kitchen_tokens(
    db: impl PgExecutor<'_>,
    household: Uuid,
) -> sqlx::Result<Vec<KitchenToken>> {
    sqlx::query_as!(
        KitchenToken,
        "SELECT id, name, created_at, linked_at, link_expires_at, last_used_at
         FROM kitchen_tokens WHERE household_id = $1 ORDER BY created_at, id",
        household
    )
    .fetch_all(db)
    .await
}

/// Adds a kitchen token for the household, made by account `by`, whose one-time link (by the hash of its code)
/// works for `minutes`.
pub async fn add_kitchen_token(
    db: impl PgExecutor<'_>,
    household: Uuid,
    by: Uuid,
    name: &str,
    link_hash: &str,
    minutes: i32,
) -> sqlx::Result<KitchenToken> {
    sqlx::query_as!(
        KitchenToken,
        "INSERT INTO kitchen_tokens (id, household_id, created_by, name, link_hash, link_expires_at)
         VALUES ($1, $2, $3, $4, $5, now() + make_interval(mins => $6))
         RETURNING id, name, created_at, linked_at, link_expires_at, last_used_at",
        Uuid::now_v7(),
        household,
        by,
        name,
        link_hash,
        minutes
    )
    .fetch_one(db)
    .await
}

/// Revokes the household's kitchen token `id`; answers whether there was one.
pub async fn delete_kitchen_token(
    db: impl PgExecutor<'_>,
    id: Uuid,
    household: Uuid,
) -> sqlx::Result<bool> {
    let done = sqlx::query!(
        "DELETE FROM kitchen_tokens WHERE id = $1 AND household_id = $2",
        id,
        household
    )
    .execute(db)
    .await?;
    Ok(done.rows_affected() > 0)
}

/// Uses the live one-time link with this code hash: the token now signs in by `token_hash` and the link is gone.
/// Answers with the token's id, or none for a used, expired or unknown link.
pub async fn open_kitchen_link(
    db: impl PgExecutor<'_>,
    link_hash: &str,
    token_hash: &str,
) -> sqlx::Result<Option<Uuid>> {
    sqlx::query_scalar!(
        "UPDATE kitchen_tokens SET link_hash = NULL, link_expires_at = NULL, token_hash = $2, linked_at = now()
         WHERE link_hash = $1 AND link_expires_at > now() RETURNING id",
        link_hash,
        token_hash
    )
    .fetch_optional(db)
    .await
}

// Single sign-on.

/// Keeps a sign-in started with the provider, by the hash of its state, after clearing those over 15 minutes old.
pub async fn add_oidc_login(
    db: &sqlx::PgPool,
    state_hash: &str,
    nonce: &str,
    pkce_verifier: &str,
    return_to: &str,
) -> sqlx::Result<()> {
    sqlx::query!("DELETE FROM oidc_logins WHERE created_at < now() - interval '15 minutes'")
        .execute(db)
        .await?;
    sqlx::query!(
        "INSERT INTO oidc_logins (state_hash, nonce, pkce_verifier, return_to) VALUES ($1, $2, $3, $4)",
        state_hash,
        nonce,
        pkce_verifier,
        return_to
    )
    .execute(db)
    .await?;
    Ok(())
}

/// A sign-in started with the provider.
pub struct OidcLogin {
    /// The nonce the ID token must carry.
    pub nonce: String,
    /// The PKCE verifier for the code exchange.
    pub pkce_verifier: String,
    /// Where to go after signing in.
    pub return_to: String,
}

/// Takes the sign-in started with this state hash in the last 15 minutes, so it can be finished only once.
pub async fn take_oidc_login(
    db: impl PgExecutor<'_>,
    state_hash: &str,
) -> sqlx::Result<Option<OidcLogin>> {
    sqlx::query_as!(
        OidcLogin,
        "DELETE FROM oidc_logins WHERE state_hash = $1 AND created_at > now() - interval '15 minutes'
         RETURNING nonce, pkce_verifier, return_to",
        state_hash
    )
    .fetch_optional(db)
    .await
}

/// The account linked to this identity at the provider, and whether it is turned off.
pub async fn identity(
    db: impl PgExecutor<'_>,
    issuer: &str,
    subject: &str,
) -> sqlx::Result<Option<(Uuid, bool)>> {
    Ok(sqlx::query!(
        "SELECT u.id, u.disabled FROM user_identities i JOIN users u ON u.id = i.user_id
         WHERE i.issuer = $1 AND i.subject = $2",
        issuer,
        subject
    )
    .fetch_optional(db)
    .await?
    .map(|u| (u.id, u.disabled)))
}

/// The account with this email address, matched without case.
pub struct EmailMatch {
    /// The account.
    pub id: Uuid,
    /// It is turned off.
    pub disabled: bool,
    /// It is an admin.
    pub is_admin: bool,
}

/// The account whose email is `email`, lowercased.
pub async fn by_email(db: impl PgExecutor<'_>, email: &str) -> sqlx::Result<Option<EmailMatch>> {
    sqlx::query_as!(
        EmailMatch,
        "SELECT id, disabled, is_admin FROM users WHERE lower(email) = $1",
        email
    )
    .fetch_optional(db)
    .await
}

/// Whether an account signs in as `username`, matched without case.
pub async fn username_taken(db: impl PgExecutor<'_>, username: &str) -> sqlx::Result<bool> {
    sqlx::query_scalar!(
        r#"SELECT EXISTS (SELECT 1 FROM users WHERE lower(username) = lower($1)) AS "taken!""#,
        username
    )
    .fetch_one(db)
    .await
}

/// The oldest household named `name`.
pub async fn household_named(db: impl PgExecutor<'_>, name: &str) -> sqlx::Result<Option<Uuid>> {
    sqlx::query_scalar!(
        "SELECT id FROM households WHERE name = $1 ORDER BY created_at LIMIT 1",
        name
    )
    .fetch_optional(db)
    .await
}

/// Links this identity at the provider to account `user`.
pub async fn add_identity(
    db: impl PgExecutor<'_>,
    issuer: &str,
    subject: &str,
    user: Uuid,
) -> sqlx::Result<()> {
    sqlx::query!(
        "INSERT INTO user_identities (issuer, subject, user_id) VALUES ($1, $2, $3)",
        issuer,
        subject,
        user
    )
    .execute(db)
    .await?;
    Ok(())
}
