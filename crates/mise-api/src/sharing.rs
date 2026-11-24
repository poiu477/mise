//! Sharing between households: how far a household's own recipes and forks are seen, and whom it shares with.
//!
//! A version a household owns is `private` (its own), `partners` (also the households it has added as partners) or
//! `install` (every household). Each household has a default for what it writes and forks, and any one recipe can
//! say otherwise. Others read a shared recipe but never change it; they can fork it, and a fork stays theirs when
//! the share is withdrawn. Plans, lists and preferences are never shared.
//!
//! The install's cap (`SHARING_MAX_LEVEL`) is the most open level allowed. The server writes it to the database when
//! it starts, where the SQL function `visible_to` reads it, so lowering it hides what was shared beyond it at once,
//! and asking for more is refused.

use axum::{Extension, Json, extract::State, http::StatusCode};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Postgres, Transaction};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::{
    AppState,
    auth::CurrentUser,
    error::{ApiError, ApiResult},
    extract::{JsonBody, Path},
};

/// How far a recipe is seen, from least to most.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum Visibility {
    /// The household alone.
    Private,
    /// The household and the households it has added as partners.
    Partners,
    /// Every household on the install.
    Install,
}

impl Visibility {
    /// As the database stores it.
    pub fn as_str(self) -> &'static str {
        match self {
            Visibility::Private => "private",
            Visibility::Partners => "partners",
            Visibility::Install => "install",
        }
    }

    /// Read back from the database, or from `SHARING_MAX_LEVEL`.
    pub fn parse(s: &str) -> Option<Visibility> {
        match s.trim() {
            "private" => Some(Visibility::Private),
            "partners" => Some(Visibility::Partners),
            "install" => Some(Visibility::Install),
            _ => None,
        }
    }
}

/// The cap `SHARING_MAX_LEVEL` sets: `install` when it isn't set.
pub fn cap_from_env() -> anyhow::Result<Visibility> {
    match std::env::var("SHARING_MAX_LEVEL") {
        Err(_) => Ok(Visibility::Install),
        Ok(v) if v.trim().is_empty() => Ok(Visibility::Install),
        Ok(v) => Visibility::parse(&v).ok_or_else(|| {
            anyhow::anyhow!("SHARING_MAX_LEVEL is {v:?}: use private, partners or install")
        }),
    }
}

/// Makes `cap` the install's cap, where `visible_to` reads it.
pub async fn set_cap(pool: &PgPool, cap: Visibility) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO sharing_cap (only_row, max_level) VALUES (true, $1)
         ON CONFLICT (only_row) DO UPDATE SET max_level = excluded.max_level",
    )
    .bind(cap.as_str())
    .execute(pool)
    .await?;
    Ok(())
}

/// The install's cap.
async fn cap(tx: &mut Transaction<'_, Postgres>) -> ApiResult<Visibility> {
    let stored: Option<String> = sqlx::query_scalar("SELECT max_level FROM sharing_cap")
        .fetch_optional(&mut **tx)
        .await?;
    Ok(stored
        .as_deref()
        .and_then(Visibility::parse)
        .unwrap_or(Visibility::Install))
}

/// Refuses a level the install's cap doesn't allow.
pub async fn allowed(tx: &mut Transaction<'_, Postgres>, asked: Visibility) -> ApiResult<()> {
    let cap = cap(tx).await?;
    if asked > cap {
        return Err(ApiError::BadRequest(format!(
            "this install shares recipes no further than {}",
            cap.as_str()
        )));
    }
    Ok(())
}

/// The visibility a new recipe or fork of `household` takes when it asks for none: the household's default, or the
/// cap when the cap has since come down below it.
pub async fn default_for(
    tx: &mut Transaction<'_, Postgres>,
    household: Uuid,
) -> ApiResult<Visibility> {
    let stored: String =
        sqlx::query_scalar("SELECT default_visibility FROM households WHERE id = $1")
            .bind(household)
            .fetch_one(&mut **tx)
            .await?;
    let default = Visibility::parse(&stored).unwrap_or(Visibility::Private);
    Ok(default.min(cap(tx).await?))
}

/// A household, by id and name.
#[derive(Serialize, ToSchema)]
pub struct HouseholdRef {
    /// The household's id.
    pub id: Uuid,
    /// The household's name.
    pub name: String,
}

/// The signed-in household's sharing.
#[derive(Serialize, ToSchema)]
pub struct HouseholdSharing {
    /// The signed-in household's id.
    pub id: Uuid,
    /// The signed-in household's name.
    pub name: String,
    /// What its new recipes and forks are shared with, unless one says otherwise.
    pub default_visibility: Visibility,
    /// The most open level this install allows.
    pub max_visibility: Visibility,
    /// The households it shares with at `partners`, by name.
    pub partners: Vec<HouseholdRef>,
    /// The households that share with it at `partners`, by name.
    pub partner_of: Vec<HouseholdRef>,
    /// Every other household on the install, by name, to choose partners from.
    pub households: Vec<HouseholdRef>,
}

async fn sharing(pool: &PgPool, household: Uuid) -> ApiResult<HouseholdSharing> {
    let mut tx = pool.begin().await?;
    let (name, default): (String, String) =
        sqlx::query_as("SELECT name, default_visibility FROM households WHERE id = $1")
            .bind(household)
            .fetch_one(&mut *tx)
            .await?;
    let refs = |rows: Vec<(Uuid, String)>| {
        rows.into_iter()
            .map(|(id, name)| HouseholdRef { id, name })
            .collect::<Vec<_>>()
    };
    let partners = refs(
        sqlx::query_as(
            "SELECT h.id, h.name FROM household_partners p JOIN households h ON h.id = p.partner_id
             WHERE p.household_id = $1 ORDER BY h.name, h.id",
        )
        .bind(household)
        .fetch_all(&mut *tx)
        .await?,
    );
    let partner_of = refs(
        sqlx::query_as(
            "SELECT h.id, h.name FROM household_partners p JOIN households h ON h.id = p.household_id
             WHERE p.partner_id = $1 ORDER BY h.name, h.id",
        )
        .bind(household)
        .fetch_all(&mut *tx)
        .await?,
    );
    let households = refs(
        sqlx::query_as("SELECT id, name FROM households WHERE id <> $1 ORDER BY name, id")
            .bind(household)
            .fetch_all(&mut *tx)
            .await?,
    );
    let max_visibility = cap(&mut tx).await?;
    Ok(HouseholdSharing {
        id: household,
        name,
        default_visibility: Visibility::parse(&default).unwrap_or(Visibility::Private),
        max_visibility,
        partners,
        partner_of,
        households,
    })
}

/// The signed-in household's sharing: its default, the install's cap, its partners, and the other households to
/// choose them from.
#[utoipa::path(
    get, path = "/api/v1/household", operation_id = "get_household_sharing", tag = "sharing",
    responses(
        (status = 200, description = "The household's sharing", body = HouseholdSharing),
        crate::error::SignedIn,
        crate::error::KitchenDevice
    )
)]
pub async fn get(
    State(s): State<AppState>,
    Extension(me): Extension<CurrentUser>,
) -> ApiResult<Json<HouseholdSharing>> {
    Ok(Json(sharing(&s.pool, me.household_id).await?))
}

/// A household's default visibility.
#[derive(Deserialize, ToSchema)]
pub struct SharingDefault {
    /// What new recipes and forks are shared with, unless one says otherwise; no more than the install allows.
    pub default_visibility: Visibility,
}

/// Set what the household's new recipes and forks are shared with. Recipes it already has keep theirs.
#[utoipa::path(
    put, path = "/api/v1/household/sharing", operation_id = "set_household_sharing", tag = "sharing",
    request_body = SharingDefault,
    responses(
        (status = 200, description = "The household's sharing", body = HouseholdSharing),
        (status = 400, description = "The level is more open than the install allows, or the body isn't JSON of the right shape", body = crate::error::ErrorBody),
        crate::error::SignedIn,
        crate::error::ReadOnlyToken,
        crate::error::TooLarge
    )
)]
pub async fn set_default(
    State(s): State<AppState>,
    Extension(me): Extension<CurrentUser>,
    JsonBody(input): JsonBody<SharingDefault>,
) -> ApiResult<Json<HouseholdSharing>> {
    let mut tx = s.pool.begin().await?;
    allowed(&mut tx, input.default_visibility).await?;
    sqlx::query("UPDATE households SET default_visibility = $2 WHERE id = $1")
        .bind(me.household_id)
        .bind(input.default_visibility.as_str())
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(Json(sharing(&s.pool, me.household_id).await?))
}

/// Share with another household: from now on it sees what this household shares with its partners. It doesn't
/// share back until it adds this one too. Adding one already added changes nothing.
#[utoipa::path(
    put, path = "/api/v1/household/partners/{id}", operation_id = "add_partner", tag = "sharing",
    params(("id" = Uuid, Path, description = "The other household")),
    responses(
        (status = 200, description = "The household's sharing", body = HouseholdSharing),
        (status = 400, description = "The id isn't a UUID, or is this household's own", body = crate::error::ErrorBody),
        (status = 404, description = "No such household", body = crate::error::ErrorBody),
        crate::error::SignedIn,
        crate::error::ReadOnlyToken
    )
)]
pub async fn add_partner(
    State(s): State<AppState>,
    Extension(me): Extension<CurrentUser>,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<HouseholdSharing>> {
    if id == me.household_id {
        return Err(ApiError::BadRequest(
            "a household can't be its own partner".into(),
        ));
    }
    let added = sqlx::query(
        "INSERT INTO household_partners (household_id, partner_id)
         SELECT $1, h.id FROM households h WHERE h.id = $2
         ON CONFLICT DO NOTHING",
    )
    .bind(me.household_id)
    .bind(id)
    .execute(&s.pool)
    .await?;
    if added.rows_affected() == 0 {
        let exists: bool =
            sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM households WHERE id = $1)")
                .bind(id)
                .fetch_one(&s.pool)
                .await?;
        if !exists {
            return Err(ApiError::NotFound);
        }
    }
    Ok(Json(sharing(&s.pool, me.household_id).await?))
}

/// Stop sharing with a household: what this household shares with its partners is hidden from it at once, while any
/// forks it made stay its own. Removing one that isn't a partner changes nothing.
#[utoipa::path(
    delete, path = "/api/v1/household/partners/{id}", operation_id = "remove_partner", tag = "sharing",
    params(("id" = Uuid, Path, description = "The other household")),
    responses(
        (status = 204, description = "No longer a partner"),
        (status = 400, description = "The id isn't a UUID", body = crate::error::ErrorBody),
        crate::error::SignedIn,
        crate::error::ReadOnlyToken
    )
)]
pub async fn remove_partner(
    State(s): State<AppState>,
    Extension(me): Extension<CurrentUser>,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    sqlx::query("DELETE FROM household_partners WHERE household_id = $1 AND partner_id = $2")
        .bind(me.household_id)
        .bind(id)
        .execute(&s.pool)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
