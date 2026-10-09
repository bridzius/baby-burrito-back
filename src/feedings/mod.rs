pub mod repository;
pub mod routes;

use chrono::{DateTime, Duration, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::ApiError;

pub const MINIMUM_AMOUNT_ML: i64 = 1;
pub const MAXIMUM_AMOUNT_ML: i64 = 500;

pub const MINIMUM_NURSING_MINUTES: i64 = 1;
pub const MAXIMUM_NURSING_MINUTES: i64 = 120;

/// Clock skew allowance between the phone and the server.
const MAXIMUM_FUTURE_DRIFT: Duration = Duration::minutes(5);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeedingKind {
    /// At the breast: has a duration and a side, no amount.
    Nursing,
    /// A bottle of expressed breast milk: has an amount.
    BreastMilk,
    /// A bottle of formula: has an amount.
    Formula,
}

impl FeedingKind {
    pub fn as_str(self) -> &'static str {
        match self {
            FeedingKind::Nursing => "nursing",
            FeedingKind::BreastMilk => "breast_milk",
            FeedingKind::Formula => "formula",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "nursing" => Some(FeedingKind::Nursing),
            "breast_milk" => Some(FeedingKind::BreastMilk),
            "formula" => Some(FeedingKind::Formula),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BreastSide {
    Left,
    Right,
    Both,
}

impl BreastSide {
    pub fn as_str(self) -> &'static str {
        match self {
            BreastSide::Left => "left",
            BreastSide::Right => "right",
            BreastSide::Both => "both",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "left" => Some(BreastSide::Left),
            "right" => Some(BreastSide::Right),
            "both" => Some(BreastSide::Both),
            _ => None,
        }
    }
}

/// What was fed. Only values that passed [`FeedingDetails::validate`] are stored, so each kind carries
/// exactly its own fields: nursing has `duration_minutes` and `breast_side`, bottles have `amount_ml`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct FeedingDetails {
    pub kind: FeedingKind,
    pub amount_ml: Option<i64>,
    pub duration_minutes: Option<i64>,
    pub breast_side: Option<BreastSide>,
}

impl FeedingDetails {
    fn empty(kind: FeedingKind) -> Self {
        Self {
            kind,
            amount_ml: None,
            duration_minutes: None,
            breast_side: None,
        }
    }

    pub fn validate(self) -> Result<Self, ApiError> {
        match self.kind {
            FeedingKind::Nursing => validate_nursing(self),
            FeedingKind::BreastMilk | FeedingKind::Formula => validate_bottle(self),
        }
    }
}

fn validate_nursing(details: FeedingDetails) -> Result<FeedingDetails, ApiError> {
    if details.amount_ml.is_some() {
        return Err(ApiError::Validation(
            "amount_ml is only for bottle feedings".into(),
        ));
    }

    let duration_minutes = details
        .duration_minutes
        .ok_or_else(|| ApiError::Validation("nursing requires duration_minutes".into()))?;

    if !(MINIMUM_NURSING_MINUTES..=MAXIMUM_NURSING_MINUTES).contains(&duration_minutes) {
        return Err(ApiError::Validation(format!(
            "duration_minutes must be between {MINIMUM_NURSING_MINUTES} and {MAXIMUM_NURSING_MINUTES}"
        )));
    }

    if details.breast_side.is_none() {
        return Err(ApiError::Validation("nursing requires breast_side".into()));
    }

    Ok(details)
}

fn validate_bottle(details: FeedingDetails) -> Result<FeedingDetails, ApiError> {
    if details.duration_minutes.is_some() || details.breast_side.is_some() {
        return Err(ApiError::Validation(
            "duration_minutes and breast_side are only for nursing".into(),
        ));
    }

    let amount_ml = details
        .amount_ml
        .ok_or_else(|| ApiError::Validation("bottle feedings require amount_ml".into()))?;

    if !(MINIMUM_AMOUNT_ML..=MAXIMUM_AMOUNT_ML).contains(&amount_ml) {
        return Err(ApiError::Validation(format!(
            "amount_ml must be between {MINIMUM_AMOUNT_ML} and {MAXIMUM_AMOUNT_ML}"
        )));
    }

    Ok(details)
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Feeding {
    pub id: Uuid,
    /// When the feeding started; intervals between feedings are measured start to start.
    pub fed_at: DateTime<Utc>,
    #[serde(flatten)]
    pub details: FeedingDetails,
    /// Who logged it; `None` once that user has deleted their account.
    pub created_by: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// The client may supply `id` (a UUID it generated) so retried uploads are idempotent.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateFeedingRequest {
    pub id: Option<Uuid>,
    pub fed_at: DateTime<Utc>,
    pub kind: FeedingKind,
    pub amount_ml: Option<i64>,
    pub duration_minutes: Option<i64>,
    pub breast_side: Option<BreastSide>,
}

/// Partial update. Changing `kind` clears the previous kind's fields, so the new kind's fields must be
/// sent along (e.g. `{"kind": "formula", "amount_ml": 90}` turns a nursing entry into a bottle).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateFeedingRequest {
    pub fed_at: Option<DateTime<Utc>>,
    pub kind: Option<FeedingKind>,
    pub amount_ml: Option<i64>,
    pub duration_minutes: Option<i64>,
    pub breast_side: Option<BreastSide>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NewFeeding {
    pub id: Uuid,
    pub fed_at: DateTime<Utc>,
    pub details: FeedingDetails,
}

/// The complete new state of an edited feeding.
#[derive(Debug, Clone, PartialEq)]
pub struct FeedingRevision {
    pub fed_at: DateTime<Utc>,
    pub details: FeedingDetails,
}

impl Feeding {
    pub fn has_same_content_as(&self, new_feeding: &NewFeeding) -> bool {
        self.fed_at == new_feeding.fed_at && self.details == new_feeding.details
    }
}

impl CreateFeedingRequest {
    pub fn validate(self, now: DateTime<Utc>) -> Result<NewFeeding, ApiError> {
        let details = FeedingDetails {
            kind: self.kind,
            amount_ml: self.amount_ml,
            duration_minutes: self.duration_minutes,
            breast_side: self.breast_side,
        };

        Ok(NewFeeding {
            id: self.id.unwrap_or_else(Uuid::now_v7),
            fed_at: validate_fed_at(self.fed_at, now)?,
            details: details.validate()?,
        })
    }
}

impl UpdateFeedingRequest {
    pub fn apply_to(
        self,
        existing: &Feeding,
        now: DateTime<Utc>,
    ) -> Result<FeedingRevision, ApiError> {
        let nothing_to_change = self.fed_at.is_none()
            && self.kind.is_none()
            && self.amount_ml.is_none()
            && self.duration_minutes.is_none()
            && self.breast_side.is_none();

        if nothing_to_change {
            return Err(ApiError::Validation(
                "provide at least one of fed_at, kind, amount_ml, duration_minutes, breast_side"
                    .into(),
            ));
        }

        let base_details = match self.kind {
            Some(new_kind) if new_kind != existing.details.kind => FeedingDetails::empty(new_kind),
            _ => existing.details,
        };

        let details = FeedingDetails {
            kind: base_details.kind,
            amount_ml: self.amount_ml.or(base_details.amount_ml),
            duration_minutes: self.duration_minutes.or(base_details.duration_minutes),
            breast_side: self.breast_side.or(base_details.breast_side),
        };

        let fed_at = match self.fed_at {
            Some(fed_at) => validate_fed_at(fed_at, now)?,
            None => existing.fed_at,
        };

        Ok(FeedingRevision {
            fed_at,
            details: details.validate()?,
        })
    }
}

/// Rejects implausible times and truncates to the millisecond precision the database stores.
fn validate_fed_at(fed_at: DateTime<Utc>, now: DateTime<Utc>) -> Result<DateTime<Utc>, ApiError> {
    if fed_at > now + MAXIMUM_FUTURE_DRIFT {
        return Err(ApiError::Validation(
            "fed_at must not be in the future".into(),
        ));
    }

    let earliest_plausible = Utc.with_ymd_and_hms(2020, 1, 1, 0, 0, 0).unwrap();

    if fed_at < earliest_plausible {
        return Err(ApiError::Validation("fed_at is implausibly old".into()));
    }

    Ok(timestamp_from_millis(fed_at.timestamp_millis()))
}

pub fn timestamp_from_millis(millis: i64) -> DateTime<Utc> {
    DateTime::from_timestamp_millis(millis).unwrap_or(DateTime::UNIX_EPOCH)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bottle_request(amount_ml: i64, fed_at: DateTime<Utc>) -> CreateFeedingRequest {
        CreateFeedingRequest {
            id: None,
            fed_at,
            kind: FeedingKind::Formula,
            amount_ml: Some(amount_ml),
            duration_minutes: None,
            breast_side: None,
        }
    }

    fn nursing_request(
        duration_minutes: Option<i64>,
        breast_side: Option<BreastSide>,
    ) -> CreateFeedingRequest {
        CreateFeedingRequest {
            id: None,
            fed_at: Utc::now(),
            kind: FeedingKind::Nursing,
            amount_ml: None,
            duration_minutes,
            breast_side,
        }
    }

    fn empty_update() -> UpdateFeedingRequest {
        UpdateFeedingRequest {
            fed_at: None,
            kind: None,
            amount_ml: None,
            duration_minutes: None,
            breast_side: None,
        }
    }

    fn existing_nursing_feeding() -> Feeding {
        let now = Utc::now();

        Feeding {
            id: Uuid::now_v7(),
            fed_at: timestamp_from_millis(now.timestamp_millis()),
            details: FeedingDetails {
                kind: FeedingKind::Nursing,
                amount_ml: None,
                duration_minutes: Some(15),
                breast_side: Some(BreastSide::Left),
            },
            created_by: None,
            created_at: now,
            updated_at: now,
        }
    }

    #[test]
    fn rejects_bottle_amounts_outside_the_allowed_range() {
        let now = Utc::now();

        assert!(bottle_request(0, now).validate(now).is_err());
        assert!(bottle_request(501, now).validate(now).is_err());
        assert!(bottle_request(120, now).validate(now).is_ok());
    }

    #[test]
    fn nursing_requires_duration_and_side_and_no_amount() {
        let now = Utc::now();
        let mut with_amount = nursing_request(Some(15), Some(BreastSide::Left));
        with_amount.amount_ml = Some(60);

        assert!(
            nursing_request(Some(15), Some(BreastSide::Left))
                .validate(now)
                .is_ok()
        );
        assert!(
            nursing_request(None, Some(BreastSide::Left))
                .validate(now)
                .is_err()
        );
        assert!(nursing_request(Some(15), None).validate(now).is_err());
        assert!(
            nursing_request(Some(121), Some(BreastSide::Both))
                .validate(now)
                .is_err()
        );
        assert!(with_amount.validate(now).is_err());
    }

    #[test]
    fn bottles_reject_nursing_fields() {
        let now = Utc::now();
        let mut with_side = bottle_request(90, now);
        with_side.breast_side = Some(BreastSide::Right);

        assert!(with_side.validate(now).is_err());
    }

    #[test]
    fn rejects_feedings_in_the_future_beyond_clock_drift() {
        let now = Utc::now();

        assert!(
            bottle_request(90, now + Duration::minutes(4))
                .validate(now)
                .is_ok()
        );
        assert!(
            bottle_request(90, now + Duration::minutes(6))
                .validate(now)
                .is_err()
        );
    }

    #[test]
    fn truncates_fed_at_to_milliseconds() {
        let now = Utc::now();
        let precise_time = Utc.timestamp_opt(1_780_000_000, 123_456_789).unwrap();

        let new_feeding = bottle_request(90, precise_time).validate(now).unwrap();

        assert_eq!(new_feeding.fed_at.timestamp_subsec_nanos(), 123_000_000);
    }

    #[test]
    fn update_without_fields_is_rejected() {
        assert!(
            empty_update()
                .apply_to(&existing_nursing_feeding(), Utc::now())
                .is_err()
        );
    }

    #[test]
    fn update_within_the_same_kind_keeps_other_fields() {
        let update = UpdateFeedingRequest {
            breast_side: Some(BreastSide::Right),
            ..empty_update()
        };

        let revision = update
            .apply_to(&existing_nursing_feeding(), Utc::now())
            .unwrap();

        assert_eq!(revision.details.duration_minutes, Some(15));
        assert_eq!(revision.details.breast_side, Some(BreastSide::Right));
    }

    #[test]
    fn changing_kind_clears_the_previous_kinds_fields() {
        let to_bottle = UpdateFeedingRequest {
            kind: Some(FeedingKind::Formula),
            amount_ml: Some(90),
            ..empty_update()
        };
        let to_bottle_without_amount = UpdateFeedingRequest {
            kind: Some(FeedingKind::Formula),
            ..empty_update()
        };

        let revision = to_bottle
            .apply_to(&existing_nursing_feeding(), Utc::now())
            .unwrap();

        assert_eq!(revision.details.amount_ml, Some(90));
        assert_eq!(revision.details.duration_minutes, None);
        assert_eq!(revision.details.breast_side, None);
        assert!(
            to_bottle_without_amount
                .apply_to(&existing_nursing_feeding(), Utc::now())
                .is_err()
        );
    }
}
