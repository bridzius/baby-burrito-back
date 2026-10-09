mod common;

use axum::http::StatusCode;
use chrono::Utc;
use common::{TestApp, sign_identity_token, sign_identity_token_with_key_id, valid_claims};
use serde_json::{Value, json};

async fn attempt_sign_in(app: &TestApp, identity_token: String, nonce: &str) -> StatusCode {
    let body = json!({ "identity_token": identity_token, "nonce": nonce });

    app.request("POST", "/v1/auth/apple", None, Some(body))
        .await
        .status
}

#[tokio::test]
async fn health_needs_no_session() {
    let app = TestApp::new().await;

    let response = app.request("GET", "/health", None, None).await;

    assert_eq!(response.status, StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn sign_in_issues_a_session_and_reuses_the_account() {
    let app = TestApp::new().await;

    let first_session = app.sign_in("apple-subject-mom").await;
    let second_session = app.sign_in("apple-subject-mom").await;

    let first_account = app.get("/v1/me", &first_session).await;
    let second_account = app.get("/v1/me", &second_session).await;

    assert_eq!(first_account.status, StatusCode::OK);
    assert_ne!(first_session, second_session);
    assert_eq!(
        first_account.body["user"]["id"],
        second_account.body["user"]["id"]
    );
    assert_eq!(first_account.body["memberships"], json!([]));
}

#[tokio::test]
async fn sign_in_stores_the_display_name_apple_shares_once() {
    let app = TestApp::new().await;
    let nonce = app.new_challenge_nonce().await;
    let identity_token = sign_identity_token(&valid_claims("apple-subject-mom", &nonce));

    let body = json!({ "identity_token": identity_token, "nonce": nonce, "display_name": "Mom" });
    let response = app
        .request("POST", "/v1/auth/apple", None, Some(body))
        .await;

    let later_session = app.sign_in("apple-subject-mom").await;
    let account = app.get("/v1/me", &later_session).await;

    assert_eq!(response.body["user"]["display_name"], "Mom");
    assert_eq!(account.body["user"]["display_name"], "Mom");
}

#[tokio::test]
async fn rejects_tokens_with_wrong_audience_issuer_or_expiry() {
    let app = TestApp::new().await;

    let tamperings: [(&str, Value); 3] = [
        ("aud", json!("com.attacker.app")),
        ("iss", json!("https://evil.example")),
        ("exp", json!(Utc::now().timestamp() - 3600)),
    ];

    for (claim, tampered_value) in tamperings {
        let nonce = app.new_challenge_nonce().await;
        let mut claims = valid_claims("apple-subject-mom", &nonce);
        claims[claim] = tampered_value;

        let status = attempt_sign_in(&app, sign_identity_token(&claims), &nonce).await;

        assert_eq!(
            status,
            StatusCode::UNAUTHORIZED,
            "tampered {claim} was accepted"
        );
    }
}

#[tokio::test]
async fn rejects_tokens_signed_with_an_unknown_key_or_garbage() {
    let app = TestApp::new().await;
    let nonce = app.new_challenge_nonce().await;
    let claims = valid_claims("apple-subject-mom", &nonce);

    let unknown_key_status = attempt_sign_in(
        &app,
        sign_identity_token_with_key_id(&claims, "other-key"),
        &nonce,
    )
    .await;
    let garbage_status = attempt_sign_in(&app, "not.a.jwt".to_owned(), &nonce).await;

    assert_eq!(unknown_key_status, StatusCode::UNAUTHORIZED);
    assert_eq!(garbage_status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn rejects_a_nonce_that_does_not_match_the_token() {
    let app = TestApp::new().await;
    let issued_nonce = app.new_challenge_nonce().await;
    let identity_token =
        sign_identity_token(&valid_claims("apple-subject-mom", "some-other-nonce"));

    let status = attempt_sign_in(&app, identity_token, &issued_nonce).await;

    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn rejects_a_nonce_the_server_never_issued() {
    let app = TestApp::new().await;
    let invented_nonce = "invented-by-the-client";
    let identity_token = sign_identity_token(&valid_claims("apple-subject-mom", invented_nonce));

    let status = attempt_sign_in(&app, identity_token, invented_nonce).await;

    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn a_challenge_can_only_be_used_once() {
    let app = TestApp::new().await;
    let nonce = app.new_challenge_nonce().await;
    let identity_token = sign_identity_token(&valid_claims("apple-subject-mom", &nonce));

    let first_status = attempt_sign_in(&app, identity_token.clone(), &nonce).await;
    let replay_status = attempt_sign_in(&app, identity_token, &nonce).await;

    assert_eq!(first_status, StatusCode::OK);
    assert_eq!(replay_status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn protected_routes_reject_missing_or_unknown_sessions() {
    let app = TestApp::new().await;

    let missing = app.request("GET", "/v1/me", None, None).await;
    let unknown = app.get("/v1/me", "made-up-session-token").await;

    assert_eq!(missing.status, StatusCode::UNAUTHORIZED);
    assert_eq!(missing.body["error"]["code"], "unauthorized");
    assert_eq!(unknown.status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn sign_out_revokes_only_that_session() {
    let app = TestApp::new().await;
    let phone_session = app.sign_in("apple-subject-mom").await;
    let ipad_session = app.sign_in("apple-subject-mom").await;

    let sign_out = app
        .request("POST", "/v1/auth/sign-out", Some(&phone_session), None)
        .await;

    assert_eq!(sign_out.status, StatusCode::NO_CONTENT);
    assert_eq!(
        app.get("/v1/me", &phone_session).await.status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        app.get("/v1/me", &ipad_session).await.status,
        StatusCode::OK
    );
}

#[tokio::test]
async fn display_name_can_be_changed_and_is_validated() {
    let app = TestApp::new().await;
    let session = app.sign_in("apple-subject-mom").await;

    let renamed = app
        .patch("/v1/me", &session, json!({ "display_name": "  Mama " }))
        .await;
    let blank = app
        .patch("/v1/me", &session, json!({ "display_name": "   " }))
        .await;

    assert_eq!(renamed.body["display_name"], "Mama");
    assert_eq!(blank.status, StatusCode::UNPROCESSABLE_ENTITY);
}
