//! A household shares one feeding log. Its creator is the single owner; others join through invites
//! as editors (can log feedings) or viewers (read-only).

pub mod repository;
pub mod routes;

use std::collections::HashMap;

use axum::{
    extract::{FromRequestParts, Path},
    http::request::Parts,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{AppState, auth::AuthenticatedUser, error::ApiError};

const MAXIMUM_NAME_CHARACTERS: usize = 60;

pub const DEFAULT_FEEDING_INTERVAL_MINUTES: i64 = 180;
const MINIMUM_FEEDING_INTERVAL_MINUTES: i64 = 15;
const MAXIMUM_FEEDING_INTERVAL_MINUTES: i64 = 720;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Owner,
    Editor,
    Viewer,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Owner => "owner",
            Role::Editor => "editor",
            Role::Viewer => "viewer",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "owner" => Some(Role::Owner),
            "editor" => Some(Role::Editor),
            "viewer" => Some(Role::Viewer),
            _ => None,
        }
    }

    /// Roles that can be handed out through invites or role changes; ownership is never transferable.
    pub fn ensure_assignable(self) -> Result<Self, ApiError> {
        if self == Role::Owner {
            return Err(ApiError::Validation("role must be editor or viewer".into()));
        }

        Ok(self)
    }
}

#[derive(Debug, Serialize)]
pub struct MembershipSummary {
    pub household_id: Uuid,
    pub household_name: String,
    /// The baby's interval between feedings, shared by everyone in the household.
    pub feeding_interval_minutes: i64,
    pub role: Role,
    pub joined_at: DateTime<Utc>,
}

/// The caller's membership in the household named by the `{household_id}` path segment.
/// Non-members get 404 so household IDs cannot be probed.
pub struct HouseholdMember {
    pub household_id: Uuid,
    pub user_id: Uuid,
    pub role: Role,
}

impl HouseholdMember {
    pub fn require_editor(&self) -> Result<(), ApiError> {
        if self.role == Role::Viewer {
            return Err(ApiError::Forbidden("viewers cannot change feedings".into()));
        }

        Ok(())
    }

    pub fn require_owner(&self) -> Result<(), ApiError> {
        if self.role != Role::Owner {
            return Err(ApiError::Forbidden(
                "only the household owner can do this".into(),
            ));
        }

        Ok(())
    }
}

impl FromRequestParts<AppState> for HouseholdMember {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let user = parts
            .extensions
            .get::<AuthenticatedUser>()
            .cloned()
            .ok_or(ApiError::Unauthorized)?;

        let Path(path_parameters) =
            Path::<HashMap<String, String>>::from_request_parts(parts, state).await?;

        let household_id = path_parameters
            .get("household_id")
            .and_then(|raw_id| Uuid::parse_str(raw_id).ok())
            .ok_or_else(|| ApiError::BadRequest("household_id must be a UUID".into()))?;

        let role = repository::find_role(&state.pool, household_id, user.user_id)
            .await?
            .ok_or(ApiError::NotFound)?;

        Ok(HouseholdMember {
            household_id,
            user_id: user.user_id,
            role,
        })
    }
}

pub fn validate_feeding_interval(minutes: i64) -> Result<i64, ApiError> {
    if !(MINIMUM_FEEDING_INTERVAL_MINUTES..=MAXIMUM_FEEDING_INTERVAL_MINUTES).contains(&minutes) {
        return Err(ApiError::Validation(format!(
            "feeding_interval_minutes must be between {MINIMUM_FEEDING_INTERVAL_MINUTES} and {MAXIMUM_FEEDING_INTERVAL_MINUTES}"
        )));
    }

    Ok(minutes)
}

/// Trims and bounds a user-supplied name, rejecting control characters.
pub fn validate_name(field: &str, raw_name: &str) -> Result<String, ApiError> {
    let name = raw_name.trim();
    let character_count = name.chars().count();

    if character_count == 0 || character_count > MAXIMUM_NAME_CHARACTERS {
        return Err(ApiError::Validation(format!(
            "{field} must be 1 to {MAXIMUM_NAME_CHARACTERS} characters"
        )));
    }

    if name.chars().any(char::is_control) {
        return Err(ApiError::Validation(format!(
            "{field} must not contain control characters"
        )));
    }

    Ok(name.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_trimmed_and_bounded() {
        assert_eq!(validate_name("name", "  Burrito  ").unwrap(), "Burrito");
        assert!(validate_name("name", "   ").is_err());
        assert!(validate_name("name", &"x".repeat(61)).is_err());
        assert!(validate_name("name", "bad\u{0}name").is_err());
    }

    #[test]
    fn ownership_cannot_be_assigned() {
        assert!(Role::Owner.ensure_assignable().is_err());
        assert_eq!(Role::Viewer.ensure_assignable().unwrap(), Role::Viewer);
    }
}
