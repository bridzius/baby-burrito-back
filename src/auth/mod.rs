//! Sign in with Apple flow:
//!
//! 1. `POST /v1/auth/challenge` returns a single-use `nonce`.
//! 2. The app starts Sign in with Apple with `request.nonce = hex(sha256(nonce))`.
//! 3. `POST /v1/auth/apple` sends Apple's identity token plus the raw `nonce`; the server verifies the
//!    token signature, issuer, audience and expiry, checks the nonce, burns the challenge and issues
//!    an opaque session token for the `Authorization: Bearer` header.

pub mod apple;
pub mod routes;
pub mod session;

pub use session::{AuthenticatedUser, require_session};
