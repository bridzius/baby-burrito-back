use anyhow::anyhow;
use chrono::{DateTime, Utc};
use sqlx::{FromRow, SqlitePool};
use uuid::Uuid;

use crate::{
    db::parse_stored_uuid,
    error::ApiError,
    feedings::{
        BreastSide, Feeding, FeedingDetails, FeedingKind, FeedingRevision, NewFeeding,
        timestamp_from_millis,
    },
};

/// Expands to a string literal so it can be spliced into queries with `concat!`.
macro_rules! feeding_columns {
    () => {
        "id, fed_at_ms, kind, amount_ml, duration_minutes, breast_side, created_by_user_id, created_at_ms, updated_at_ms"
    };
}

#[derive(FromRow)]
struct FeedingRow {
    id: String,
    fed_at_ms: i64,
    kind: String,
    amount_ml: Option<i64>,
    duration_minutes: Option<i64>,
    breast_side: Option<String>,
    created_by_user_id: Option<String>,
    created_at_ms: i64,
    updated_at_ms: i64,
}

impl TryFrom<FeedingRow> for Feeding {
    type Error = ApiError;

    fn try_from(row: FeedingRow) -> Result<Self, Self::Error> {
        let kind = FeedingKind::parse(&row.kind)
            .ok_or_else(|| anyhow!("corrupt feeding kind {:?}", row.kind))?;

        let breast_side = row
            .breast_side
            .as_deref()
            .map(|stored| {
                BreastSide::parse(stored).ok_or_else(|| anyhow!("corrupt breast side {stored:?}"))
            })
            .transpose()?;

        Ok(Feeding {
            id: parse_stored_uuid(&row.id)?,
            fed_at: timestamp_from_millis(row.fed_at_ms),
            details: FeedingDetails {
                kind,
                amount_ml: row.amount_ml,
                duration_minutes: row.duration_minutes,
                breast_side,
            },
            created_by: row
                .created_by_user_id
                .as_deref()
                .map(parse_stored_uuid)
                .transpose()?,
            created_at: timestamp_from_millis(row.created_at_ms),
            updated_at: timestamp_from_millis(row.updated_at_ms),
        })
    }
}

fn rows_into_feedings(rows: Vec<FeedingRow>) -> Result<Vec<Feeding>, ApiError> {
    rows.into_iter().map(Feeding::try_from).collect()
}

pub enum InsertOutcome {
    Created(Feeding),
    /// The ID was already taken. `None` means it belongs to another household, which must not be revealed.
    AlreadyExisted(Option<Feeding>),
}

pub async fn insert_feeding(
    pool: &SqlitePool,
    household_id: Uuid,
    created_by: Uuid,
    new_feeding: &NewFeeding,
    now: DateTime<Utc>,
) -> Result<InsertOutcome, ApiError> {
    let details = &new_feeding.details;

    let inserted_row: Option<FeedingRow> = sqlx::query_as(concat!(
        "INSERT INTO feedings (id, household_id, fed_at_ms, kind, amount_ml, duration_minutes, breast_side,
                               created_by_user_id, created_at_ms, updated_at_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9)
         ON CONFLICT (id) DO NOTHING
         RETURNING ",
        feeding_columns!()
    ))
    .bind(new_feeding.id.to_string())
    .bind(household_id.to_string())
    .bind(new_feeding.fed_at.timestamp_millis())
    .bind(details.kind.as_str())
    .bind(details.amount_ml)
    .bind(details.duration_minutes)
    .bind(details.breast_side.map(BreastSide::as_str))
    .bind(created_by.to_string())
    .bind(now.timestamp_millis())
    .fetch_optional(pool)
    .await?;

    if let Some(row) = inserted_row {
        return Ok(InsertOutcome::Created(row.try_into()?));
    }

    let existing_feeding = find_feeding(pool, household_id, new_feeding.id).await?;

    Ok(InsertOutcome::AlreadyExisted(existing_feeding))
}

pub async fn find_feeding(
    pool: &SqlitePool,
    household_id: Uuid,
    id: Uuid,
) -> Result<Option<Feeding>, ApiError> {
    let row: Option<FeedingRow> = sqlx::query_as(concat!(
        "SELECT ",
        feeding_columns!(),
        " FROM feedings
          WHERE household_id = ?1 AND id = ?2"
    ))
    .bind(household_id.to_string())
    .bind(id.to_string())
    .fetch_optional(pool)
    .await?;

    row.map(Feeding::try_from).transpose()
}

pub async fn find_latest_feeding(
    pool: &SqlitePool,
    household_id: Uuid,
) -> Result<Option<Feeding>, ApiError> {
    let row: Option<FeedingRow> = sqlx::query_as(concat!(
        "SELECT ",
        feeding_columns!(),
        " FROM feedings
          WHERE household_id = ?1
          ORDER BY fed_at_ms DESC, id DESC
          LIMIT 1"
    ))
    .bind(household_id.to_string())
    .fetch_optional(pool)
    .await?;

    row.map(Feeding::try_from).transpose()
}

/// Newest first. `before` is an exclusive cursor for paging back through history.
pub async fn list_recent_feedings(
    pool: &SqlitePool,
    household_id: Uuid,
    before: Option<DateTime<Utc>>,
    limit: i64,
) -> Result<Vec<Feeding>, ApiError> {
    let rows: Vec<FeedingRow> = sqlx::query_as(concat!(
        "SELECT ",
        feeding_columns!(),
        " FROM feedings
          WHERE household_id = ?1 AND (?2 IS NULL OR fed_at_ms < ?2)
          ORDER BY fed_at_ms DESC, id DESC
          LIMIT ?3"
    ))
    .bind(household_id.to_string())
    .bind(before.map(|timestamp| timestamp.timestamp_millis()))
    .bind(limit)
    .fetch_all(pool)
    .await?;

    rows_into_feedings(rows)
}

/// Oldest first, both bounds inclusive.
pub async fn list_feedings_between(
    pool: &SqlitePool,
    household_id: Uuid,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> Result<Vec<Feeding>, ApiError> {
    let rows: Vec<FeedingRow> = sqlx::query_as(concat!(
        "SELECT ",
        feeding_columns!(),
        " FROM feedings
          WHERE household_id = ?1 AND fed_at_ms BETWEEN ?2 AND ?3
          ORDER BY fed_at_ms ASC, id ASC"
    ))
    .bind(household_id.to_string())
    .bind(start.timestamp_millis())
    .bind(end.timestamp_millis())
    .fetch_all(pool)
    .await?;

    rows_into_feedings(rows)
}

pub enum UpdateOutcome {
    Updated(Feeding),
    NotFound,
    /// Someone else changed the feeding after `existing` was read; the client should refetch and retry.
    ModifiedConcurrently,
}

/// Writes the complete revision, but only if the row is still the version `existing` was read from
/// (optimistic concurrency on `updated_at`), so a merge of stale fields never overwrites a newer edit.
pub async fn update_feeding(
    pool: &SqlitePool,
    household_id: Uuid,
    existing: &Feeding,
    revision: &FeedingRevision,
    now: DateTime<Utc>,
) -> Result<UpdateOutcome, ApiError> {
    let previous_updated_at_ms = existing.updated_at.timestamp_millis();
    let updated_at_ms = now.timestamp_millis().max(previous_updated_at_ms + 1);
    let details = &revision.details;

    let row: Option<FeedingRow> = sqlx::query_as(concat!(
        "UPDATE feedings
         SET fed_at_ms = ?4, kind = ?5, amount_ml = ?6, duration_minutes = ?7, breast_side = ?8, updated_at_ms = ?9
         WHERE household_id = ?1 AND id = ?2 AND updated_at_ms = ?3
         RETURNING ",
        feeding_columns!()
    ))
    .bind(household_id.to_string())
    .bind(existing.id.to_string())
    .bind(previous_updated_at_ms)
    .bind(revision.fed_at.timestamp_millis())
    .bind(details.kind.as_str())
    .bind(details.amount_ml)
    .bind(details.duration_minutes)
    .bind(details.breast_side.map(BreastSide::as_str))
    .bind(updated_at_ms)
    .fetch_optional(pool)
    .await?;

    if let Some(updated_row) = row {
        return Ok(UpdateOutcome::Updated(updated_row.try_into()?));
    }

    let still_exists = find_feeding(pool, household_id, existing.id)
        .await?
        .is_some();

    if !still_exists {
        return Ok(UpdateOutcome::NotFound);
    }

    Ok(UpdateOutcome::ModifiedConcurrently)
}

pub async fn delete_feeding(
    pool: &SqlitePool,
    household_id: Uuid,
    id: Uuid,
) -> Result<bool, ApiError> {
    let result = sqlx::query("DELETE FROM feedings WHERE household_id = ?1 AND id = ?2")
        .bind(household_id.to_string())
        .bind(id.to_string())
        .execute(pool)
        .await?;

    Ok(result.rows_affected() > 0)
}
