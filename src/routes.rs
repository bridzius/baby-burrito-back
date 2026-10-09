use std::time::Duration;

use axum::{
    Router,
    extract::State,
    http::StatusCode,
    middleware,
    routing::{get, patch, post},
};
use tower_http::{limit::RequestBodyLimitLayer, timeout::TimeoutLayer, trace::TraceLayer};

use crate::{AppState, accounts, auth, error::ApiError, feedings, households};

const MAXIMUM_REQUEST_BODY_BYTES: usize = 16 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

pub fn build_router(state: AppState) -> Router {
    let public_routes = Router::new()
        .route("/auth/challenge", post(auth::routes::create_challenge))
        .route("/auth/apple", post(auth::routes::sign_in_with_apple));

    let authenticated_routes = Router::new()
        .route("/auth/sign-out", post(auth::routes::sign_out))
        .route(
            "/me",
            get(accounts::get_account)
                .patch(accounts::update_account)
                .delete(accounts::delete_account),
        )
        .route("/invites/accept", post(households::routes::accept_invite))
        .route("/households", post(households::routes::create_household))
        .route(
            "/households/{household_id}",
            patch(households::routes::update_household)
                .delete(households::routes::delete_household),
        )
        .route(
            "/households/{household_id}/members",
            get(households::routes::list_members),
        )
        .route(
            "/households/{household_id}/members/{user_id}",
            patch(households::routes::change_member_role).delete(households::routes::remove_member),
        )
        .route(
            "/households/{household_id}/invites",
            post(households::routes::create_invite),
        )
        .route(
            "/households/{household_id}/feedings",
            get(feedings::routes::list_feedings).post(feedings::routes::create_feeding),
        )
        .route(
            "/households/{household_id}/feedings/{feeding_id}",
            get(feedings::routes::get_feeding)
                .patch(feedings::routes::update_feeding)
                .delete(feedings::routes::delete_feeding),
        )
        .route(
            "/households/{household_id}/summary",
            get(feedings::routes::get_summary),
        )
        .route(
            "/households/{household_id}/stats/daily",
            get(feedings::routes::get_daily_statistics),
        )
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            auth::require_session,
        ));

    Router::new()
        .route("/health", get(health))
        .nest("/v1", public_routes.merge(authenticated_routes))
        .layer(RequestBodyLimitLayer::new(MAXIMUM_REQUEST_BODY_BYTES))
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            REQUEST_TIMEOUT,
        ))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

async fn health(State(state): State<AppState>) -> Result<StatusCode, ApiError> {
    sqlx::query("SELECT 1").execute(&state.pool).await?;

    Ok(StatusCode::NO_CONTENT)
}
