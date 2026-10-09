use std::{env, net::SocketAddr, path::PathBuf};

use anyhow::{Context, bail};

pub struct Config {
    pub bind_address: SocketAddr,
    pub database_path: PathBuf,
    /// Bundle IDs accepted as the `aud` of Apple identity tokens (e.g. release and debug builds).
    pub apple_bundle_ids: Vec<String>,
}

impl Config {
    pub fn from_environment() -> anyhow::Result<Self> {
        let bind_address = env::var("BIND_ADDRESS")
            .unwrap_or_else(|_| "127.0.0.1:8080".to_owned())
            .parse()
            .context("BIND_ADDRESS must be a socket address like 127.0.0.1:8080")?;

        let database_path = env::var("DATABASE_PATH")
            .unwrap_or_else(|_| "data/baby-burrito.db".to_owned())
            .into();

        let apple_bundle_ids: Vec<String> = env::var("APPLE_BUNDLE_IDS")
            .context("APPLE_BUNDLE_IDS must be set, e.g. com.example.babyburrito")?
            .split(',')
            .map(str::trim)
            .filter(|bundle_id| !bundle_id.is_empty())
            .map(str::to_owned)
            .collect();

        if apple_bundle_ids.is_empty() {
            bail!("APPLE_BUNDLE_IDS must contain at least one bundle ID");
        }

        Ok(Self {
            bind_address,
            database_path,
            apple_bundle_ids,
        })
    }
}
