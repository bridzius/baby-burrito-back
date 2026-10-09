//! Random secrets handed to clients (session tokens, nonces, invite codes) and their stored digests.

use anyhow::anyhow;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use sha2::{Digest, Sha256};

use crate::error::ApiError;

pub type TokenHash = [u8; 32];

/// A URL-safe string carrying `byte_count` bytes from the OS CSPRNG.
pub fn generate_token(byte_count: usize) -> Result<String, ApiError> {
    let mut bytes = vec![0u8; byte_count];

    getrandom::fill(&mut bytes)
        .map_err(|error| anyhow!("system randomness unavailable: {error}"))?;

    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

pub fn hash_token(token: &str) -> TokenHash {
    Sha256::digest(token.as_bytes()).into()
}
