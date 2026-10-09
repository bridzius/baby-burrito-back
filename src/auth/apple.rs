//! Verification of the identity token (a JWT) the iOS app receives from Sign in with Apple.

use std::{
    collections::HashMap,
    sync::{Mutex, RwLock},
    time::{Duration, Instant},
};

use anyhow::Context;
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header, jwk::JwkSet};
use serde::Deserialize;

use crate::error::ApiError;

const APPLE_ISSUER: &str = "https://appleid.apple.com";
const APPLE_KEYS_URL: &str = "https://appleid.apple.com/auth/keys";

/// Unknown key IDs trigger a refetch (Apple rotates keys), but never more often than this.
const MINIMUM_KEY_REFRESH_INTERVAL: Duration = Duration::from_secs(60);

pub struct AppleIdentity {
    /// Stable, app-scoped, opaque user identifier. Not the Apple ID itself.
    pub subject: String,
    /// The value the app put in the authorization request: hex SHA-256 of the raw nonce.
    pub nonce_hash: Option<String>,
}

#[derive(Deserialize)]
struct AppleClaims {
    sub: String,
    nonce: Option<String>,
}

enum KeySource {
    Apple { http_client: reqwest::Client },
    Fixed,
}

pub struct AppleIdentityVerifier {
    validation: Validation,
    key_source: KeySource,
    keys_by_id: RwLock<HashMap<String, DecodingKey>>,
    last_key_refresh: Mutex<Option<Instant>>,
}

impl AppleIdentityVerifier {
    /// Verifies against Apple's published signing keys, fetched lazily.
    pub fn for_apple(bundle_ids: &[String]) -> anyhow::Result<Self> {
        let http_client = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .context("building HTTP client")?;

        Ok(Self::new(
            bundle_ids,
            KeySource::Apple { http_client },
            HashMap::new(),
        ))
    }

    /// Verifies against a fixed key set and never touches the network. Meant for tests.
    pub fn with_fixed_keys(
        bundle_ids: &[String],
        keys_by_id: HashMap<String, DecodingKey>,
    ) -> Self {
        Self::new(bundle_ids, KeySource::Fixed, keys_by_id)
    }

    fn new(
        bundle_ids: &[String],
        key_source: KeySource,
        keys_by_id: HashMap<String, DecodingKey>,
    ) -> Self {
        let mut validation = Validation::new(Algorithm::RS256);
        validation.set_issuer(&[APPLE_ISSUER]);
        validation.set_audience(bundle_ids);
        validation.set_required_spec_claims(&["exp", "iss", "aud", "sub"]);

        Self {
            validation,
            key_source,
            keys_by_id: RwLock::new(keys_by_id),
            last_key_refresh: Mutex::new(None),
        }
    }

    pub async fn verify(&self, identity_token: &str) -> Result<AppleIdentity, ApiError> {
        let header = decode_header(identity_token).map_err(|_| ApiError::Unauthorized)?;
        let key_id = header.kid.ok_or(ApiError::Unauthorized)?;
        let signing_key = self
            .find_signing_key(&key_id)
            .await?
            .ok_or(ApiError::Unauthorized)?;

        let token_data = decode::<AppleClaims>(identity_token, &signing_key, &self.validation)
            .map_err(|error| {
                tracing::info!(%error, "rejected Apple identity token");
                ApiError::Unauthorized
            })?;

        Ok(AppleIdentity {
            subject: token_data.claims.sub,
            nonce_hash: token_data.claims.nonce,
        })
    }

    async fn find_signing_key(&self, key_id: &str) -> Result<Option<DecodingKey>, ApiError> {
        if let Some(key) = self.cached_key(key_id) {
            return Ok(Some(key));
        }

        let KeySource::Apple { http_client } = &self.key_source else {
            return Ok(None);
        };

        if !self.claim_key_refresh() {
            return Ok(None);
        }

        let fetched_keys = fetch_apple_keys(http_client).await?;
        *self.keys_by_id.write().expect("key cache lock poisoned") = fetched_keys;

        Ok(self.cached_key(key_id))
    }

    fn cached_key(&self, key_id: &str) -> Option<DecodingKey> {
        self.keys_by_id
            .read()
            .expect("key cache lock poisoned")
            .get(key_id)
            .cloned()
    }

    /// Returns true if this caller may refetch keys now, recording the attempt.
    fn claim_key_refresh(&self) -> bool {
        let mut last_refresh = self
            .last_key_refresh
            .lock()
            .expect("key refresh lock poisoned");

        let refreshed_recently =
            last_refresh.is_some_and(|instant| instant.elapsed() < MINIMUM_KEY_REFRESH_INTERVAL);

        if refreshed_recently {
            return false;
        }

        *last_refresh = Some(Instant::now());

        true
    }
}

async fn fetch_apple_keys(
    http_client: &reqwest::Client,
) -> Result<HashMap<String, DecodingKey>, ApiError> {
    let key_set: JwkSet = http_client
        .get(APPLE_KEYS_URL)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .context("fetching Apple signing keys")?
        .json()
        .await
        .context("parsing Apple signing keys")?;

    let keys_by_id = key_set
        .keys
        .iter()
        .filter_map(|jwk| {
            let key_id = jwk.common.key_id.clone()?;
            let key = DecodingKey::from_jwk(jwk).ok()?;

            Some((key_id, key))
        })
        .collect();

    Ok(keys_by_id)
}
