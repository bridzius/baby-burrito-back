use axum::{Extension, Json, extract::State, http::StatusCode};
use chrono::{DateTime, TimeDelta, Utc};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use subtle::ConstantTimeEq;

use crate::{
    AppState,
    accounts::{self, AccountOverview, validate_display_name},
    auth::session::{self, AuthenticatedUser},
    error::ApiError,
    extract::ApiJson,
    feedings::timestamp_from_millis,
    tokens::{generate_token, hash_token},
};

/// Long enough for the user to get through the Apple sheet (Face ID, account picker).
const CHALLENGE_LIFETIME: TimeDelta = TimeDelta::minutes(10);

const NONCE_BYTES: usize = 32;

#[derive(Serialize)]
pub struct Challenge {
    nonce: String,
    expires_at: DateTime<Utc>,
}

pub async fn create_challenge(
    State(state): State<AppState>,
) -> Result<(StatusCode, Json<Challenge>), ApiError> {
    let now = Utc::now();
    let nonce = generate_token(NONCE_BYTES)?;
    let expires_at = timestamp_from_millis((now + CHALLENGE_LIFETIME).timestamp_millis());

    sqlx::query("DELETE FROM sign_in_challenges WHERE expires_at_ms <= ?1")
        .bind(now.timestamp_millis())
        .execute(&state.pool)
        .await?;

    sqlx::query("INSERT INTO sign_in_challenges (nonce_hash, expires_at_ms) VALUES (?1, ?2)")
        .bind(hash_token(&nonce).as_slice())
        .bind(expires_at.timestamp_millis())
        .execute(&state.pool)
        .await?;

    Ok((StatusCode::CREATED, Json(Challenge { nonce, expires_at })))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignInWithAppleRequest {
    identity_token: String,
    nonce: String,
    /// Apple only reveals the user's name to the app on the very first authorization; the app may forward it.
    display_name: Option<String>,
}

#[derive(Serialize)]
pub struct SignInResponse {
    session_token: String,
    expires_at: DateTime<Utc>,
    #[serde(flatten)]
    account: AccountOverview,
}

pub async fn sign_in_with_apple(
    State(state): State<AppState>,
    ApiJson(request): ApiJson<SignInWithAppleRequest>,
) -> Result<Json<SignInResponse>, ApiError> {
    let now = Utc::now();
    let display_name = request
        .display_name
        .as_deref()
        .map(validate_display_name)
        .transpose()?;

    let identity = state.apple_verifier.verify(&request.identity_token).await?;

    let expected_nonce_hash = hex::encode(hash_token(&request.nonce));
    let token_nonce_hash = identity.nonce_hash.unwrap_or_default();
    let nonce_matches: bool = token_nonce_hash
        .as_bytes()
        .ct_eq(expected_nonce_hash.as_bytes())
        .into();

    if !nonce_matches {
        return Err(ApiError::Unauthorized);
    }

    consume_challenge(&state.pool, &request.nonce, now).await?;

    let user =
        accounts::upsert_user_by_apple_subject(&state.pool, &identity.subject, display_name, now)
            .await?;
    let issued_session = session::create_session(&state.pool, user.id, now).await?;
    let account = accounts::load_account_overview(&state.pool, user).await?;

    Ok(Json(SignInResponse {
        session_token: issued_session.token,
        expires_at: issued_session.expires_at,
        account,
    }))
}

/// Deleting the row is what makes the nonce single-use, even under concurrent requests.
async fn consume_challenge(
    pool: &SqlitePool,
    nonce: &str,
    now: DateTime<Utc>,
) -> Result<(), ApiError> {
    let result =
        sqlx::query("DELETE FROM sign_in_challenges WHERE nonce_hash = ?1 AND expires_at_ms > ?2")
            .bind(hash_token(nonce).as_slice())
            .bind(now.timestamp_millis())
            .execute(pool)
            .await?;

    if result.rows_affected() == 0 {
        return Err(ApiError::Unauthorized);
    }

    Ok(())
}

pub async fn sign_out(
    State(state): State<AppState>,
    Extension(user): Extension<AuthenticatedUser>,
) -> Result<StatusCode, ApiError> {
    session::delete_session(&state.pool, &user.session_token_hash).await?;

    Ok(StatusCode::NO_CONTENT)
}
