pub mod accounts;
pub mod auth;
pub mod config;
pub mod db;
pub mod error;
pub mod extract;
pub mod feedings;
pub mod households;
pub mod routes;
pub mod stats;
pub mod tokens;

use std::sync::Arc;

use sqlx::SqlitePool;

use auth::apple::AppleIdentityVerifier;

pub use routes::build_router;

#[derive(Clone)]
pub struct AppState {
    pub pool: SqlitePool,
    pub apple_verifier: Arc<AppleIdentityVerifier>,
}

impl AppState {
    pub fn new(pool: SqlitePool, apple_verifier: AppleIdentityVerifier) -> Self {
        Self {
            pool,
            apple_verifier: Arc::new(apple_verifier),
        }
    }
}
