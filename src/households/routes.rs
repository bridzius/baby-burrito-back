use axum::{Extension, Json, extract::State, http::StatusCode};
use chrono::{DateTime, TimeDelta, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    AppState,
    auth::AuthenticatedUser,
    error::ApiError,
    extract::{ApiJson, ApiPath},
    feedings::timestamp_from_millis,
    households::{
        HouseholdMember, MembershipSummary, Role,
        repository::{self, InviteAcceptance, Member, NewInvite},
        DEFAULT_FEEDING_INTERVAL_MINUTES, validate_feeding_interval, validate_name,
    },
    tokens::{generate_token, hash_token},
};

const INVITE_LIFETIME: TimeDelta = TimeDelta::hours(72);

/// 128 bits: unguessable, still short enough for a link or QR code.
const INVITE_CODE_BYTES: usize = 16;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateHouseholdRequest {
    name: String,
    feeding_interval_minutes: Option<i64>,
}

pub async fn create_household(
    State(state): State<AppState>,
    Extension(user): Extension<AuthenticatedUser>,
    ApiJson(request): ApiJson<CreateHouseholdRequest>,
) -> Result<(StatusCode, Json<MembershipSummary>), ApiError> {
    let name = validate_name("name", &request.name)?;
    let feeding_interval_minutes =
        validate_feeding_interval(request.feeding_interval_minutes.unwrap_or(DEFAULT_FEEDING_INTERVAL_MINUTES))?;

    let membership =
        repository::create_household(&state.pool, user.user_id, &name, feeding_interval_minutes, Utc::now()).await?;

    Ok((StatusCode::CREATED, Json(membership)))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateHouseholdRequest {
    name: Option<String>,
    feeding_interval_minutes: Option<i64>,
}

/// Renaming is for the owner; the feeding interval is part of daily care, so editors may change it too.
pub async fn update_household(
    State(state): State<AppState>,
    member: HouseholdMember,
    ApiJson(request): ApiJson<UpdateHouseholdRequest>,
) -> Result<Json<MembershipSummary>, ApiError> {
    if request.name.is_none() && request.feeding_interval_minutes.is_none() {
        return Err(ApiError::Validation("provide name and/or feeding_interval_minutes".into()));
    }

    if request.name.is_some() {
        member.require_owner()?;
    }

    if request.feeding_interval_minutes.is_some() {
        member.require_editor()?;
    }

    let name = request.name.as_deref().map(|raw_name| validate_name("name", raw_name)).transpose()?;
    let feeding_interval_minutes = request.feeding_interval_minutes.map(validate_feeding_interval).transpose()?;

    repository::update_household(&state.pool, member.household_id, name.as_deref(), feeding_interval_minutes).await?;

    let membership = repository::find_membership_summary(&state.pool, member.household_id, member.user_id)
        .await?
        .ok_or(ApiError::NotFound)?;

    Ok(Json(membership))
}

pub async fn delete_household(
    State(state): State<AppState>,
    member: HouseholdMember,
) -> Result<StatusCode, ApiError> {
    member.require_owner()?;

    repository::delete_household(&state.pool, member.household_id).await?;

    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize)]
pub struct MemberList {
    members: Vec<Member>,
}

pub async fn list_members(
    State(state): State<AppState>,
    member: HouseholdMember,
) -> Result<Json<MemberList>, ApiError> {
    let members = repository::list_members(&state.pool, member.household_id).await?;

    Ok(Json(MemberList { members }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoleRequest {
    role: Role,
}

pub async fn change_member_role(
    State(state): State<AppState>,
    member: HouseholdMember,
    ApiPath((_, target_user_id)): ApiPath<(Uuid, Uuid)>,
    ApiJson(request): ApiJson<RoleRequest>,
) -> Result<StatusCode, ApiError> {
    member.require_owner()?;

    let role = request.role.ensure_assignable()?;
    let was_changed =
        repository::change_member_role(&state.pool, member.household_id, target_user_id, role)
            .await?;

    if !was_changed {
        return Err(ApiError::NotFound);
    }

    Ok(StatusCode::NO_CONTENT)
}

/// The owner can remove anyone else; any other member can remove themselves (leave).
pub async fn remove_member(
    State(state): State<AppState>,
    member: HouseholdMember,
    ApiPath((_, target_user_id)): ApiPath<(Uuid, Uuid)>,
) -> Result<StatusCode, ApiError> {
    let is_leaving = target_user_id == member.user_id;

    if is_leaving && member.role == Role::Owner {
        return Err(ApiError::Forbidden(
            "the owner cannot leave; delete the household instead".into(),
        ));
    }

    if !is_leaving {
        member.require_owner()?;
    }

    let was_removed =
        repository::remove_member(&state.pool, member.household_id, target_user_id).await?;

    if !was_removed {
        return Err(ApiError::NotFound);
    }

    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize)]
pub struct CreatedInvite {
    code: String,
    role: Role,
    expires_at: DateTime<Utc>,
}

/// The plaintext code is returned once and only its digest is stored. Share it via iMessage/AirDrop.
pub async fn create_invite(
    State(state): State<AppState>,
    member: HouseholdMember,
    ApiJson(request): ApiJson<RoleRequest>,
) -> Result<(StatusCode, Json<CreatedInvite>), ApiError> {
    member.require_owner()?;

    let role = request.role.ensure_assignable()?;
    let now = Utc::now();
    let code = generate_token(INVITE_CODE_BYTES)?;
    let expires_at = timestamp_from_millis((now + INVITE_LIFETIME).timestamp_millis());

    let invite = NewInvite {
        code_hash: &hash_token(&code),
        household_id: member.household_id,
        role,
        created_by: member.user_id,
        expires_at,
    };

    repository::create_invite(&state.pool, &invite, now).await?;

    Ok((
        StatusCode::CREATED,
        Json(CreatedInvite {
            code,
            role,
            expires_at,
        }),
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AcceptInviteRequest {
    code: String,
}

pub async fn accept_invite(
    State(state): State<AppState>,
    Extension(user): Extension<AuthenticatedUser>,
    ApiJson(request): ApiJson<AcceptInviteRequest>,
) -> Result<Json<MembershipSummary>, ApiError> {
    let acceptance = repository::accept_invite(
        &state.pool,
        &hash_token(&request.code),
        user.user_id,
        Utc::now(),
    )
    .await?;

    let household_id = match acceptance {
        InviteAcceptance::Joined(household_id) => household_id,
        InviteAcceptance::InvalidOrExpired => return Err(ApiError::NotFound),
        InviteAcceptance::AlreadyMember => {
            return Err(ApiError::Conflict(
                "you are already a member of this household".into(),
            ));
        }
    };

    let membership = repository::find_membership_summary(&state.pool, household_id, user.user_id)
        .await?
        .ok_or(ApiError::NotFound)?;

    Ok(Json(membership))
}
