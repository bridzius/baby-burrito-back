use axum::{Extension, Json, extract::State, http::StatusCode};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::{
    AppState,
    auth::AuthenticatedUser,
    db::parse_stored_uuid,
    error::ApiError,
    extract::ApiJson,
    households::{self, MembershipSummary, validate_name},
};

#[derive(Debug, Clone, Serialize)]
pub struct User {
    pub id: Uuid,
    pub display_name: Option<String>,
}

#[derive(Serialize)]
pub struct AccountOverview {
    user: User,
    memberships: Vec<MembershipSummary>,
}

pub fn validate_display_name(display_name: &str) -> Result<String, ApiError> {
    validate_name("display_name", display_name)
}

pub async fn upsert_user_by_apple_subject(
    pool: &SqlitePool,
    apple_subject: &str,
    display_name: Option<String>,
    now: DateTime<Utc>,
) -> Result<User, ApiError> {
    let (stored_id, stored_display_name): (String, Option<String>) = sqlx::query_as(
        "INSERT INTO users (id, apple_subject, display_name, created_at_ms)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT (apple_subject) DO UPDATE SET display_name = COALESCE(excluded.display_name, users.display_name)
         RETURNING id, display_name",
    )
    .bind(Uuid::now_v7().to_string())
    .bind(apple_subject)
    .bind(display_name)
    .bind(now.timestamp_millis())
    .fetch_one(pool)
    .await?;

    Ok(User {
        id: parse_stored_uuid(&stored_id)?,
        display_name: stored_display_name,
    })
}

async fn find_user(pool: &SqlitePool, user_id: Uuid) -> Result<User, ApiError> {
    let display_name: Option<Option<String>> =
        sqlx::query_scalar("SELECT display_name FROM users WHERE id = ?1")
            .bind(user_id.to_string())
            .fetch_optional(pool)
            .await?;

    let display_name = display_name.ok_or(ApiError::Unauthorized)?;

    Ok(User {
        id: user_id,
        display_name,
    })
}

pub async fn load_account_overview(
    pool: &SqlitePool,
    user: User,
) -> Result<AccountOverview, ApiError> {
    let memberships = households::repository::list_memberships_for_user(pool, user.id).await?;

    Ok(AccountOverview { user, memberships })
}

pub async fn get_account(
    State(state): State<AppState>,
    Extension(authenticated): Extension<AuthenticatedUser>,
) -> Result<Json<AccountOverview>, ApiError> {
    let user = find_user(&state.pool, authenticated.user_id).await?;

    Ok(Json(load_account_overview(&state.pool, user).await?))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateAccountRequest {
    display_name: String,
}

pub async fn update_account(
    State(state): State<AppState>,
    Extension(authenticated): Extension<AuthenticatedUser>,
    ApiJson(request): ApiJson<UpdateAccountRequest>,
) -> Result<Json<User>, ApiError> {
    let display_name = validate_display_name(&request.display_name)?;

    sqlx::query("UPDATE users SET display_name = ?2 WHERE id = ?1")
        .bind(authenticated.user_id.to_string())
        .bind(&display_name)
        .execute(&state.pool)
        .await?;

    Ok(Json(User {
        id: authenticated.user_id,
        display_name: Some(display_name),
    }))
}

/// Deletes the user, their sessions and memberships, and every household they own (with its feedings).
/// Feedings they logged in other households stay, with `created_by` cleared.
pub async fn delete_account(
    State(state): State<AppState>,
    Extension(authenticated): Extension<AuthenticatedUser>,
) -> Result<StatusCode, ApiError> {
    let user_id = authenticated.user_id.to_string();
    let mut transaction = state.pool.begin().await?;

    sqlx::query(
        "DELETE FROM households
         WHERE id IN (SELECT household_id FROM memberships WHERE user_id = ?1 AND role = 'owner')",
    )
    .bind(&user_id)
    .execute(&mut *transaction)
    .await?;

    sqlx::query("DELETE FROM users WHERE id = ?1")
        .bind(&user_id)
        .execute(&mut *transaction)
        .await?;

    transaction.commit().await?;

    Ok(StatusCode::NO_CONTENT)
}
