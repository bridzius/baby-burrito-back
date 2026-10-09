use std::{path::Path, time::Duration};

use anyhow::{Context, anyhow};
use sqlx::{
    SqlitePool,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous},
};
use uuid::Uuid;

use crate::error::ApiError;

pub async fn open_file_database(path: &Path) -> anyhow::Result<SqlitePool> {
    if let Some(directory) = path.parent() {
        std::fs::create_dir_all(directory)
            .with_context(|| format!("creating database directory {}", directory.display()))?;
    }

    let options = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .foreign_keys(true)
        .busy_timeout(Duration::from_secs(5));

    let pool = SqlitePoolOptions::new()
        .max_connections(4)
        .connect_with(options)
        .await
        .with_context(|| format!("opening database {}", path.display()))?;

    run_migrations(&pool).await?;

    Ok(pool)
}

/// A single-connection in-memory database; the connection is pinned so the data outlives idle periods.
pub async fn open_in_memory_database() -> anyhow::Result<SqlitePool> {
    let options = SqliteConnectOptions::new()
        .in_memory(true)
        .foreign_keys(true);

    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .min_connections(1)
        .idle_timeout(None)
        .max_lifetime(None)
        .connect_with(options)
        .await
        .context("opening in-memory database")?;

    run_migrations(&pool).await?;

    Ok(pool)
}

async fn run_migrations(pool: &SqlitePool) -> anyhow::Result<()> {
    sqlx::migrate!("./migrations")
        .run(pool)
        .await
        .context("running database migrations")
}

pub fn parse_stored_uuid(stored: &str) -> Result<Uuid, ApiError> {
    Uuid::parse_str(stored)
        .map_err(|_| ApiError::Internal(anyhow!("corrupt UUID in database: {stored:?}")))
}
