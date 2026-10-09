use axum::{
    extract::{Request, State},
    http::header::AUTHORIZATION,
    middleware::Next,
    response::Response,
};
use chrono::{DateTime, TimeDelta, Utc};
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::{
    AppState,
    db::parse_stored_uuid,
    error::ApiError,
    feedings::timestamp_from_millis,
    tokens::{TokenHash, generate_token, hash_token},
};

const SESSION_LIFETIME: TimeDelta = TimeDelta::days(90);

/// Sliding expiry is pushed forward at most once per this interval, to avoid a write on every request.
const SESSION_EXTENSION_GRANULARITY: TimeDelta = TimeDelta::days(1);

const SESSION_TOKEN_BYTES: usize = 32;

/// Inserted into request extensions by [`require_session`].
#[derive(Clone)]
pub struct AuthenticatedUser {
    pub user_id: Uuid,
    pub session_token_hash: TokenHash,
}

pub struct IssuedSession {
    pub token: String,
    pub expires_at: DateTime<Utc>,
}

pub async fn create_session(
    pool: &SqlitePool,
    user_id: Uuid,
    now: DateTime<Utc>,
) -> Result<IssuedSession, ApiError> {
    let token = generate_token(SESSION_TOKEN_BYTES)?;
    let expires_at = timestamp_from_millis((now + SESSION_LIFETIME).timestamp_millis());

    sqlx::query("DELETE FROM sessions WHERE expires_at_ms <= ?1")
        .bind(now.timestamp_millis())
        .execute(pool)
        .await?;

    sqlx::query(
        "INSERT INTO sessions (token_hash, user_id, created_at_ms, expires_at_ms)
         VALUES (?1, ?2, ?3, ?4)",
    )
    .bind(hash_token(&token).as_slice())
    .bind(user_id.to_string())
    .bind(now.timestamp_millis())
    .bind(expires_at.timestamp_millis())
    .execute(pool)
    .await?;

    Ok(IssuedSession { token, expires_at })
}

pub async fn delete_session(pool: &SqlitePool, token_hash: &TokenHash) -> Result<(), ApiError> {
    sqlx::query("DELETE FROM sessions WHERE token_hash = ?1")
        .bind(token_hash.as_slice())
        .execute(pool)
        .await?;

    Ok(())
}

pub async fn require_session(
    State(state): State<AppState>,
    mut request: Request,
    next: Next,
) -> Result<Response, ApiError> {
    let presented_token = request
        .headers()
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .ok_or(ApiError::Unauthorized)?;

    let session_token_hash = hash_token(presented_token);
    let user_id = find_and_extend_session(&state.pool, &session_token_hash, Utc::now())
        .await?
        .ok_or(ApiError::Unauthorized)?;

    request.extensions_mut().insert(AuthenticatedUser {
        user_id,
        session_token_hash,
    });

    Ok(next.run(request).await)
}

/// Lookup is by digest through the primary key index, so timing reveals nothing about valid tokens.
async fn find_and_extend_session(
    pool: &SqlitePool,
    token_hash: &TokenHash,
    now: DateTime<Utc>,
) -> Result<Option<Uuid>, ApiError> {
    let session: Option<(String, i64)> = sqlx::query_as(
        "SELECT user_id, expires_at_ms
         FROM sessions
         WHERE token_hash = ?1 AND expires_at_ms > ?2",
    )
    .bind(token_hash.as_slice())
    .bind(now.timestamp_millis())
    .fetch_optional(pool)
    .await?;

    let Some((stored_user_id, expires_at_ms)) = session else {
        return Ok(None);
    };

    let extended_expiry_ms = (now + SESSION_LIFETIME).timestamp_millis();
    let extension_is_due =
        extended_expiry_ms - expires_at_ms >= SESSION_EXTENSION_GRANULARITY.num_milliseconds();

    if extension_is_due {
        sqlx::query("UPDATE sessions SET expires_at_ms = ?2 WHERE token_hash = ?1")
            .bind(token_hash.as_slice())
            .bind(extended_expiry_ms)
            .execute(pool)
            .await?;
    }

    Ok(Some(parse_stored_uuid(&stored_user_id)?))
}
