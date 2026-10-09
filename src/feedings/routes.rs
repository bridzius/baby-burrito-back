use axum::{
    Json,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use chrono::{DateTime, Duration as TimeSpan, Utc};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    AppState,
    error::ApiError,
    extract::{ApiJson, ApiPath, ApiQuery},
    feedings::{
        CreateFeedingRequest, Feeding, UpdateFeedingRequest,
        repository::{self, InsertOutcome, UpdateOutcome},
        timestamp_from_millis,
    },
    households::{self, HouseholdMember},
    stats::{self, FeedingTotals, PeriodStatistics},
};

fn conflicting_id_error(id: Uuid) -> ApiError {
    ApiError::Conflict(format!(
        "feeding {id} already exists with different content"
    ))
}

pub async fn create_feeding(
    State(state): State<AppState>,
    member: HouseholdMember,
    ApiJson(request): ApiJson<CreateFeedingRequest>,
) -> Result<Response, ApiError> {
    member.require_editor()?;

    let now = Utc::now();
    let new_feeding = request.validate(now)?;

    let outcome = repository::insert_feeding(
        &state.pool,
        member.household_id,
        member.user_id,
        &new_feeding,
        now,
    )
    .await?;

    let existing_feeding = match outcome {
        InsertOutcome::Created(feeding) => {
            return Ok((StatusCode::CREATED, Json(feeding)).into_response());
        }
        InsertOutcome::AlreadyExisted(None) => return Err(conflicting_id_error(new_feeding.id)),
        InsertOutcome::AlreadyExisted(Some(feeding)) => feeding,
    };

    if !existing_feeding.has_same_content_as(&new_feeding) {
        return Err(conflicting_id_error(new_feeding.id));
    }

    Ok((StatusCode::OK, Json(existing_feeding)).into_response())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListFeedingsQuery {
    before: Option<DateTime<Utc>>,
    limit: Option<i64>,
}

#[derive(Serialize)]
pub struct FeedingList {
    feedings: Vec<Feeding>,
}

pub async fn list_feedings(
    State(state): State<AppState>,
    member: HouseholdMember,
    ApiQuery(query): ApiQuery<ListFeedingsQuery>,
) -> Result<Json<FeedingList>, ApiError> {
    let limit = query.limit.unwrap_or(50);

    if !(1..=200).contains(&limit) {
        return Err(ApiError::Validation(
            "limit must be between 1 and 200".into(),
        ));
    }

    let feedings =
        repository::list_recent_feedings(&state.pool, member.household_id, query.before, limit)
            .await?;

    Ok(Json(FeedingList { feedings }))
}

pub async fn get_feeding(
    State(state): State<AppState>,
    member: HouseholdMember,
    ApiPath((_, feeding_id)): ApiPath<(Uuid, Uuid)>,
) -> Result<Json<Feeding>, ApiError> {
    let feeding = repository::find_feeding(&state.pool, member.household_id, feeding_id)
        .await?
        .ok_or(ApiError::NotFound)?;

    Ok(Json(feeding))
}

pub async fn update_feeding(
    State(state): State<AppState>,
    member: HouseholdMember,
    ApiPath((_, feeding_id)): ApiPath<(Uuid, Uuid)>,
    ApiJson(request): ApiJson<UpdateFeedingRequest>,
) -> Result<Json<Feeding>, ApiError> {
    member.require_editor()?;

    let existing_feeding = repository::find_feeding(&state.pool, member.household_id, feeding_id)
        .await?
        .ok_or(ApiError::NotFound)?;

    let now = Utc::now();
    let revision = request.apply_to(&existing_feeding, now)?;

    let outcome = repository::update_feeding(&state.pool, member.household_id, &existing_feeding, &revision, now).await?;

    match outcome {
        UpdateOutcome::Updated(feeding) => Ok(Json(feeding)),
        UpdateOutcome::NotFound => Err(ApiError::NotFound),
        UpdateOutcome::ModifiedConcurrently => {
            Err(ApiError::Conflict("feeding was changed by someone else; refetch and retry".into()))
        }
    }
}

pub async fn delete_feeding(
    State(state): State<AppState>,
    member: HouseholdMember,
    ApiPath((_, feeding_id)): ApiPath<(Uuid, Uuid)>,
) -> Result<StatusCode, ApiError> {
    member.require_editor()?;

    let was_deleted =
        repository::delete_feeding(&state.pool, member.household_id, feeding_id).await?;

    if !was_deleted {
        return Err(ApiError::NotFound);
    }

    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SummaryQuery {
    hours: Option<i64>,
}

/// Everything the home screen needs in one round trip.
#[derive(Serialize)]
pub struct Summary {
    window_start: DateTime<Utc>,
    window_end: DateTime<Utc>,
    #[serde(flatten)]
    totals: FeedingTotals,
    last_feeding: Option<Feeding>,
    feeding_interval_minutes: i64,
    /// `last_feeding.fed_at` plus the household's feeding interval.
    next_feeding_at: Option<DateTime<Utc>>,
}

pub async fn get_summary(
    State(state): State<AppState>,
    member: HouseholdMember,
    ApiQuery(query): ApiQuery<SummaryQuery>,
) -> Result<Json<Summary>, ApiError> {
    let hours = query.hours.unwrap_or(24);

    if !(1..=168).contains(&hours) {
        return Err(ApiError::Validation(
            "hours must be between 1 and 168".into(),
        ));
    }

    let window_end = timestamp_from_millis(Utc::now().timestamp_millis());
    let window_start = window_end - TimeSpan::hours(hours);

    let feedings_in_window = repository::list_feedings_between(
        &state.pool,
        member.household_id,
        window_start,
        window_end,
    )
    .await?;
    let last_feeding = repository::find_latest_feeding(&state.pool, member.household_id).await?;
    let feeding_interval_minutes =
        households::repository::find_feeding_interval_minutes(&state.pool, member.household_id).await?;

    let next_feeding_at = last_feeding
        .as_ref()
        .map(|feeding| feeding.fed_at + TimeSpan::minutes(feeding_interval_minutes));

    Ok(Json(Summary {
        window_start,
        window_end,
        totals: stats::sum_feedings(&feedings_in_window),
        last_feeding,
        feeding_interval_minutes,
        next_feeding_at,
    }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DailyStatisticsQuery {
    days: Option<i64>,
    timezone: Option<String>,
}

pub async fn get_daily_statistics(
    State(state): State<AppState>,
    member: HouseholdMember,
    ApiQuery(query): ApiQuery<DailyStatisticsQuery>,
) -> Result<Json<PeriodStatistics>, ApiError> {
    let day_count = query.days.unwrap_or(7);

    if !(1..=90).contains(&day_count) {
        return Err(ApiError::Validation("days must be between 1 and 90".into()));
    }

    let timezone_name = query.timezone.as_deref().unwrap_or("UTC");
    let timezone: Tz = timezone_name
        .parse()
        .map_err(|_| ApiError::Validation(format!("unknown timezone {timezone_name:?}")))?;

    let now = Utc::now();
    let last_day = now.with_timezone(&timezone).date_naive();
    let first_day = last_day - TimeSpan::days(day_count - 1);
    let period_start = stats::start_of_local_day(first_day, timezone);

    let feedings =
        repository::list_feedings_between(&state.pool, member.household_id, period_start, now)
            .await?;

    Ok(Json(stats::build_period_statistics(
        &feedings, timezone, first_day, last_day,
    )))
}
