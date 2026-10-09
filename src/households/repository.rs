use anyhow::anyhow;
use chrono::{DateTime, Utc};
use sqlx::{FromRow, SqlitePool};
use uuid::Uuid;

use crate::{
    db::parse_stored_uuid,
    error::ApiError,
    feedings::timestamp_from_millis,
    households::{MembershipSummary, Role},
    tokens::TokenHash,
};

fn parse_stored_role(stored: &str) -> Result<Role, ApiError> {
    Role::parse(stored)
        .ok_or_else(|| ApiError::Internal(anyhow!("corrupt role in database: {stored:?}")))
}

pub async fn create_household(
    pool: &SqlitePool,
    owner_id: Uuid,
    name: &str,
    feeding_interval_minutes: i64,
    now: DateTime<Utc>,
) -> Result<MembershipSummary, ApiError> {
    let household_id = Uuid::now_v7();
    let now_ms = now.timestamp_millis();
    let mut transaction = pool.begin().await?;

    sqlx::query(
        "INSERT INTO households (id, name, feeding_interval_minutes, created_at_ms)
         VALUES (?1, ?2, ?3, ?4)",
    )
    .bind(household_id.to_string())
    .bind(name)
    .bind(feeding_interval_minutes)
    .bind(now_ms)
    .execute(&mut *transaction)
    .await?;

    sqlx::query(
        "INSERT INTO memberships (household_id, user_id, role, joined_at_ms)
         VALUES (?1, ?2, 'owner', ?3)",
    )
    .bind(household_id.to_string())
    .bind(owner_id.to_string())
    .bind(now_ms)
    .execute(&mut *transaction)
    .await?;

    transaction.commit().await?;

    Ok(MembershipSummary {
        household_id,
        household_name: name.to_owned(),
        feeding_interval_minutes,
        role: Role::Owner,
        joined_at: timestamp_from_millis(now_ms),
    })
}

/// Changes whichever of `name` and `feeding_interval_minutes` are given.
pub async fn update_household(
    pool: &SqlitePool,
    household_id: Uuid,
    name: Option<&str>,
    feeding_interval_minutes: Option<i64>,
) -> Result<(), ApiError> {
    sqlx::query(
        "UPDATE households
         SET name = COALESCE(?2, name),
             feeding_interval_minutes = COALESCE(?3, feeding_interval_minutes)
         WHERE id = ?1",
    )
    .bind(household_id.to_string())
    .bind(name)
    .bind(feeding_interval_minutes)
    .execute(pool)
    .await?;

    Ok(())
}

pub async fn find_feeding_interval_minutes(pool: &SqlitePool, household_id: Uuid) -> Result<i64, ApiError> {
    let feeding_interval_minutes: Option<i64> =
        sqlx::query_scalar("SELECT feeding_interval_minutes FROM households WHERE id = ?1")
            .bind(household_id.to_string())
            .fetch_optional(pool)
            .await?;

    feeding_interval_minutes.ok_or(ApiError::NotFound)
}

pub async fn delete_household(pool: &SqlitePool, household_id: Uuid) -> Result<(), ApiError> {
    sqlx::query("DELETE FROM households WHERE id = ?1")
        .bind(household_id.to_string())
        .execute(pool)
        .await?;

    Ok(())
}

pub async fn find_role(
    pool: &SqlitePool,
    household_id: Uuid,
    user_id: Uuid,
) -> Result<Option<Role>, ApiError> {
    let stored_role: Option<String> =
        sqlx::query_scalar("SELECT role FROM memberships WHERE household_id = ?1 AND user_id = ?2")
            .bind(household_id.to_string())
            .bind(user_id.to_string())
            .fetch_optional(pool)
            .await?;

    stored_role.as_deref().map(parse_stored_role).transpose()
}

#[derive(FromRow)]
struct MembershipSummaryRow {
    household_id: String,
    household_name: String,
    feeding_interval_minutes: i64,
    role: String,
    joined_at_ms: i64,
}

impl TryFrom<MembershipSummaryRow> for MembershipSummary {
    type Error = ApiError;

    fn try_from(row: MembershipSummaryRow) -> Result<Self, Self::Error> {
        Ok(MembershipSummary {
            household_id: parse_stored_uuid(&row.household_id)?,
            household_name: row.household_name,
            feeding_interval_minutes: row.feeding_interval_minutes,
            role: parse_stored_role(&row.role)?,
            joined_at: timestamp_from_millis(row.joined_at_ms),
        })
    }
}

/// Expands to a string literal so membership queries can share it via `concat!`.
macro_rules! membership_summary_query {
    () => {
        "SELECT memberships.household_id, households.name AS household_name, households.feeding_interval_minutes,
                memberships.role, memberships.joined_at_ms
         FROM memberships
         JOIN households ON households.id = memberships.household_id
         WHERE memberships.user_id = ?1"
    };
}

pub async fn list_memberships_for_user(pool: &SqlitePool, user_id: Uuid) -> Result<Vec<MembershipSummary>, ApiError> {
    let rows: Vec<MembershipSummaryRow> =
        sqlx::query_as(concat!(membership_summary_query!(), " ORDER BY memberships.joined_at_ms ASC"))
            .bind(user_id.to_string())
            .fetch_all(pool)
            .await?;

    rows.into_iter().map(MembershipSummary::try_from).collect()
}

pub async fn find_membership_summary(
    pool: &SqlitePool,
    household_id: Uuid,
    user_id: Uuid,
) -> Result<Option<MembershipSummary>, ApiError> {
    let row: Option<MembershipSummaryRow> =
        sqlx::query_as(concat!(membership_summary_query!(), " AND memberships.household_id = ?2"))
            .bind(user_id.to_string())
            .bind(household_id.to_string())
            .fetch_optional(pool)
            .await?;

    row.map(MembershipSummary::try_from).transpose()
}

#[derive(Debug, serde::Serialize)]
pub struct Member {
    pub user_id: Uuid,
    pub display_name: Option<String>,
    pub role: Role,
    pub joined_at: DateTime<Utc>,
}

#[derive(FromRow)]
struct MemberRow {
    user_id: String,
    display_name: Option<String>,
    role: String,
    joined_at_ms: i64,
}

impl TryFrom<MemberRow> for Member {
    type Error = ApiError;

    fn try_from(row: MemberRow) -> Result<Self, Self::Error> {
        Ok(Member {
            user_id: parse_stored_uuid(&row.user_id)?,
            display_name: row.display_name,
            role: parse_stored_role(&row.role)?,
            joined_at: timestamp_from_millis(row.joined_at_ms),
        })
    }
}

pub async fn list_members(pool: &SqlitePool, household_id: Uuid) -> Result<Vec<Member>, ApiError> {
    let rows: Vec<MemberRow> = sqlx::query_as(
        "SELECT memberships.user_id, users.display_name, memberships.role, memberships.joined_at_ms
         FROM memberships
         JOIN users ON users.id = memberships.user_id
         WHERE memberships.household_id = ?1
         ORDER BY memberships.joined_at_ms ASC",
    )
    .bind(household_id.to_string())
    .fetch_all(pool)
    .await?;

    rows.into_iter().map(Member::try_from).collect()
}

/// Never touches the owner's membership. Returns false if no such non-owner member exists.
pub async fn change_member_role(
    pool: &SqlitePool,
    household_id: Uuid,
    user_id: Uuid,
    role: Role,
) -> Result<bool, ApiError> {
    let result = sqlx::query(
        "UPDATE memberships SET role = ?3
         WHERE household_id = ?1 AND user_id = ?2 AND role != 'owner'",
    )
    .bind(household_id.to_string())
    .bind(user_id.to_string())
    .bind(role.as_str())
    .execute(pool)
    .await?;

    Ok(result.rows_affected() > 0)
}

/// Never removes the owner. Returns false if no such non-owner member exists.
pub async fn remove_member(
    pool: &SqlitePool,
    household_id: Uuid,
    user_id: Uuid,
) -> Result<bool, ApiError> {
    let result = sqlx::query(
        "DELETE FROM memberships
         WHERE household_id = ?1 AND user_id = ?2 AND role != 'owner'",
    )
    .bind(household_id.to_string())
    .bind(user_id.to_string())
    .execute(pool)
    .await?;

    Ok(result.rows_affected() > 0)
}

pub struct NewInvite<'a> {
    pub code_hash: &'a TokenHash,
    pub household_id: Uuid,
    pub role: Role,
    pub created_by: Uuid,
    pub expires_at: DateTime<Utc>,
}

pub async fn create_invite(
    pool: &SqlitePool,
    invite: &NewInvite<'_>,
    now: DateTime<Utc>,
) -> Result<(), ApiError> {
    sqlx::query(
        "INSERT INTO invites (code_hash, household_id, role, created_by_user_id, created_at_ms, expires_at_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
    )
    .bind(invite.code_hash.as_slice())
    .bind(invite.household_id.to_string())
    .bind(invite.role.as_str())
    .bind(invite.created_by.to_string())
    .bind(now.timestamp_millis())
    .bind(invite.expires_at.timestamp_millis())
    .execute(pool)
    .await?;

    Ok(())
}

pub enum InviteAcceptance {
    Joined(Uuid),
    InvalidOrExpired,
    AlreadyMember,
}

/// Claims the invite and adds the membership in one transaction. The invite is claimed first so the
/// transaction takes SQLite's write lock up front instead of upgrading from a read.
pub async fn accept_invite(
    pool: &SqlitePool,
    code_hash: &TokenHash,
    user_id: Uuid,
    now: DateTime<Utc>,
) -> Result<InviteAcceptance, ApiError> {
    let now_ms = now.timestamp_millis();
    let mut transaction = pool.begin().await?;

    let claimed_invite: Option<(String, String)> = sqlx::query_as(
        "UPDATE invites SET accepted_by_user_id = ?2, accepted_at_ms = ?3
         WHERE code_hash = ?1 AND accepted_at_ms IS NULL AND expires_at_ms > ?3
         RETURNING household_id, role",
    )
    .bind(code_hash.as_slice())
    .bind(user_id.to_string())
    .bind(now_ms)
    .fetch_optional(&mut *transaction)
    .await?;

    let Some((stored_household_id, stored_role)) = claimed_invite else {
        return Ok(InviteAcceptance::InvalidOrExpired);
    };

    let membership_result = sqlx::query(
        "INSERT INTO memberships (household_id, user_id, role, joined_at_ms)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT (household_id, user_id) DO NOTHING",
    )
    .bind(&stored_household_id)
    .bind(user_id.to_string())
    .bind(&stored_role)
    .bind(now_ms)
    .execute(&mut *transaction)
    .await?;

    // Rolling back leaves the invite unclaimed for whoever it was really meant for.
    if membership_result.rows_affected() == 0 {
        transaction.rollback().await?;
        return Ok(InviteAcceptance::AlreadyMember);
    }

    transaction.commit().await?;

    Ok(InviteAcceptance::Joined(parse_stored_uuid(
        &stored_household_id,
    )?))
}
