#![allow(dead_code)]

use std::collections::HashMap;

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode, header},
};
use baby_burrito::{
    AppState, auth::apple::AppleIdentityVerifier, build_router, db, tokens::hash_token,
};
use chrono::Utc;
use http_body_util::BodyExt;
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header};
use serde_json::{Value, json};
use tower::ServiceExt;

pub const BUNDLE_ID: &str = "com.example.babyburrito";
pub const TEST_KEY_ID: &str = "test-key";

const PRIVATE_KEY_PEM: &[u8] = include_bytes!("../fixtures/test_apple_signing_key.pem");
const PUBLIC_KEY_PEM: &[u8] = include_bytes!("../fixtures/test_apple_signing_key.pub.pem");

pub struct TestApp {
    router: Router,
}

pub struct Response {
    pub status: StatusCode,
    pub body: Value,
}

impl TestApp {
    pub async fn new() -> Self {
        let pool = db::open_in_memory_database().await.unwrap();

        let keys_by_id = HashMap::from([(
            TEST_KEY_ID.to_owned(),
            DecodingKey::from_rsa_pem(PUBLIC_KEY_PEM).unwrap(),
        )]);
        let verifier = AppleIdentityVerifier::with_fixed_keys(&[BUNDLE_ID.to_owned()], keys_by_id);

        Self {
            router: build_router(AppState::new(pool, verifier)),
        }
    }

    pub async fn request(
        &self,
        method: &str,
        uri: &str,
        session: Option<&str>,
        body: Option<Value>,
    ) -> Response {
        let mut request = Request::builder().method(method).uri(uri);

        if let Some(session_token) = session {
            request = request.header(header::AUTHORIZATION, format!("Bearer {session_token}"));
        }

        let request = match body {
            Some(json_body) => request
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(json_body.to_string())),
            None => request.body(Body::empty()),
        }
        .unwrap();

        let response = self.router.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();

        Response {
            status,
            body: serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        }
    }

    pub async fn get(&self, uri: &str, session: &str) -> Response {
        self.request("GET", uri, Some(session), None).await
    }

    pub async fn post(&self, uri: &str, session: &str, body: Value) -> Response {
        self.request("POST", uri, Some(session), Some(body)).await
    }

    pub async fn patch(&self, uri: &str, session: &str, body: Value) -> Response {
        self.request("PATCH", uri, Some(session), Some(body)).await
    }

    pub async fn delete(&self, uri: &str, session: &str) -> Response {
        self.request("DELETE", uri, Some(session), None).await
    }

    pub async fn new_challenge_nonce(&self) -> String {
        let response = self.request("POST", "/v1/auth/challenge", None, None).await;

        assert_eq!(response.status, StatusCode::CREATED);

        response.body["nonce"].as_str().unwrap().to_owned()
    }

    /// Runs the full challenge + Sign in with Apple flow and returns the session token.
    pub async fn sign_in(&self, apple_subject: &str) -> String {
        let nonce = self.new_challenge_nonce().await;
        let identity_token = sign_identity_token(&valid_claims(apple_subject, &nonce));

        let response = self
            .request(
                "POST",
                "/v1/auth/apple",
                None,
                Some(json!({ "identity_token": identity_token, "nonce": nonce })),
            )
            .await;

        assert_eq!(
            response.status,
            StatusCode::OK,
            "sign-in failed: {}",
            response.body
        );

        response.body["session_token"].as_str().unwrap().to_owned()
    }

    /// Creates a household owned by `owner_session`, returning its ID.
    pub async fn create_household(&self, owner_session: &str) -> String {
        let response = self
            .post(
                "/v1/households",
                owner_session,
                json!({ "name": "Burrito" }),
            )
            .await;

        assert_eq!(response.status, StatusCode::CREATED);

        response.body["household_id"].as_str().unwrap().to_owned()
    }

    /// Invites a fresh user into the household with `role` and returns their session token.
    pub async fn add_member(
        &self,
        owner_session: &str,
        household_id: &str,
        apple_subject: &str,
        role: &str,
    ) -> String {
        let invite = self
            .post(
                &format!("/v1/households/{household_id}/invites"),
                owner_session,
                json!({ "role": role }),
            )
            .await;
        let member_session = self.sign_in(apple_subject).await;

        let accepted = self
            .post(
                "/v1/invites/accept",
                &member_session,
                json!({ "code": invite.body["code"] }),
            )
            .await;

        assert_eq!(
            accepted.status,
            StatusCode::OK,
            "accept failed: {}",
            accepted.body
        );

        member_session
    }
}

pub fn nonce_hash(nonce: &str) -> String {
    hex::encode(hash_token(nonce))
}

pub fn valid_claims(apple_subject: &str, nonce: &str) -> Value {
    let now = Utc::now().timestamp();

    json!({
        "iss": "https://appleid.apple.com",
        "aud": BUNDLE_ID,
        "sub": apple_subject,
        "iat": now,
        "exp": now + 600,
        "nonce": nonce_hash(nonce),
    })
}

pub fn sign_identity_token(claims: &Value) -> String {
    sign_identity_token_with_key_id(claims, TEST_KEY_ID)
}

pub fn sign_identity_token_with_key_id(claims: &Value, key_id: &str) -> String {
    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some(key_id.to_owned());

    jsonwebtoken::encode(
        &header,
        claims,
        &EncodingKey::from_rsa_pem(PRIVATE_KEY_PEM).unwrap(),
    )
    .unwrap()
}
