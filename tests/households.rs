mod common;

use axum::http::StatusCode;
use chrono::{Duration, Utc};
use common::TestApp;
use serde_json::{Value, json};

fn bottle_feeding() -> Value {
    json!({ "fed_at": (Utc::now() - Duration::minutes(5)).to_rfc3339(), "amount_ml": 90, "kind": "formula" })
}

#[tokio::test]
async fn creator_becomes_the_owner() {
    let app = TestApp::new().await;
    let mom = app.sign_in("mom").await;

    let household_id = app.create_household(&mom).await;
    let account = app.get("/v1/me", &mom).await;

    assert_eq!(account.body["memberships"][0]["household_id"], household_id);
    assert_eq!(account.body["memberships"][0]["role"], "owner");
    assert_eq!(account.body["memberships"][0]["household_name"], "Burrito");
}

#[tokio::test]
async fn viewer_can_read_but_not_write_until_promoted() {
    let app = TestApp::new().await;
    let mom = app.sign_in("mom").await;
    let household_id = app.create_household(&mom).await;
    let dad = app.add_member(&mom, &household_id, "dad", "viewer").await;
    let feedings_uri = format!("/v1/households/{household_id}/feedings");

    app.post(&feedings_uri, &mom, bottle_feeding()).await;

    let dad_reads = app.get(&feedings_uri, &dad).await;
    let dad_writes = app.post(&feedings_uri, &dad, bottle_feeding()).await;

    assert_eq!(dad_reads.status, StatusCode::OK);
    assert_eq!(dad_reads.body["feedings"].as_array().unwrap().len(), 1);
    assert_eq!(dad_writes.status, StatusCode::FORBIDDEN);

    let members = app
        .get(&format!("/v1/households/{household_id}/members"), &mom)
        .await;
    let dad_user_id = members.body["members"][1]["user_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let member_uri = format!("/v1/households/{household_id}/members/{dad_user_id}");

    let promotion = app
        .patch(&member_uri, &mom, json!({ "role": "editor" }))
        .await;
    let dad_writes_again = app.post(&feedings_uri, &dad, bottle_feeding()).await;

    assert_eq!(promotion.status, StatusCode::NO_CONTENT);
    assert_eq!(dad_writes_again.status, StatusCode::CREATED);
    assert_eq!(dad_writes_again.body["created_by"], dad_user_id);
}

#[tokio::test]
async fn outsiders_cannot_see_that_a_household_exists() {
    let app = TestApp::new().await;
    let mom = app.sign_in("mom").await;
    let stranger = app.sign_in("stranger").await;
    let household_id = app.create_household(&mom).await;

    let summary = app
        .get(&format!("/v1/households/{household_id}/summary"), &stranger)
        .await;
    let feedings = app
        .get(
            &format!("/v1/households/{household_id}/feedings"),
            &stranger,
        )
        .await;

    assert_eq!(summary.status, StatusCode::NOT_FOUND);
    assert_eq!(feedings.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn only_the_owner_manages_invites_and_roles() {
    let app = TestApp::new().await;
    let mom = app.sign_in("mom").await;
    let household_id = app.create_household(&mom).await;
    let dad = app.add_member(&mom, &household_id, "dad", "editor").await;

    let dad_invites = app
        .post(
            &format!("/v1/households/{household_id}/invites"),
            &dad,
            json!({ "role": "viewer" }),
        )
        .await;
    let owner_invite = app
        .post(
            &format!("/v1/households/{household_id}/invites"),
            &mom,
            json!({ "role": "owner" }),
        )
        .await;
    let dad_renames = app
        .patch(
            &format!("/v1/households/{household_id}"),
            &dad,
            json!({ "name": "Mine" }),
        )
        .await;
    let dad_deletes = app
        .delete(&format!("/v1/households/{household_id}"), &dad)
        .await;

    assert_eq!(dad_invites.status, StatusCode::FORBIDDEN);
    assert_eq!(owner_invite.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(dad_renames.status, StatusCode::FORBIDDEN);
    assert_eq!(dad_deletes.status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn invites_are_single_use() {
    let app = TestApp::new().await;
    let mom = app.sign_in("mom").await;
    let dad = app.sign_in("dad").await;
    let grandma = app.sign_in("grandma").await;
    let household_id = app.create_household(&mom).await;

    let invite = app
        .post(
            &format!("/v1/households/{household_id}/invites"),
            &mom,
            json!({ "role": "viewer" }),
        )
        .await;
    let code = invite.body["code"].clone();

    let dad_accepts = app
        .post("/v1/invites/accept", &dad, json!({ "code": code }))
        .await;
    let grandma_reuses = app
        .post("/v1/invites/accept", &grandma, json!({ "code": code }))
        .await;
    let bogus_code = app
        .post("/v1/invites/accept", &grandma, json!({ "code": "guessed" }))
        .await;

    assert_eq!(invite.status, StatusCode::CREATED);
    assert_eq!(dad_accepts.status, StatusCode::OK);
    assert_eq!(dad_accepts.body["role"], "viewer");
    assert_eq!(grandma_reuses.status, StatusCode::NOT_FOUND);
    assert_eq!(bogus_code.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn accepting_an_invite_as_an_existing_member_does_not_burn_it() {
    let app = TestApp::new().await;
    let mom = app.sign_in("mom").await;
    let dad = app.sign_in("dad").await;
    let household_id = app.create_household(&mom).await;

    let invite = app
        .post(
            &format!("/v1/households/{household_id}/invites"),
            &mom,
            json!({ "role": "editor" }),
        )
        .await;
    let code = invite.body["code"].clone();

    let mom_accepts_own_invite = app
        .post("/v1/invites/accept", &mom, json!({ "code": code }))
        .await;
    let dad_accepts = app
        .post("/v1/invites/accept", &dad, json!({ "code": code }))
        .await;

    assert_eq!(mom_accepts_own_invite.status, StatusCode::CONFLICT);
    assert_eq!(dad_accepts.status, StatusCode::OK);
}

#[tokio::test]
async fn members_can_leave_but_the_owner_cannot() {
    let app = TestApp::new().await;
    let mom = app.sign_in("mom").await;
    let household_id = app.create_household(&mom).await;
    let dad = app.add_member(&mom, &household_id, "dad", "editor").await;

    let mom_user_id = app.get("/v1/me", &mom).await.body["user"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let dad_user_id = app.get("/v1/me", &dad).await.body["user"]["id"]
        .as_str()
        .unwrap()
        .to_owned();

    let dad_removes_mom = app
        .delete(
            &format!("/v1/households/{household_id}/members/{mom_user_id}"),
            &dad,
        )
        .await;
    let mom_leaves = app
        .delete(
            &format!("/v1/households/{household_id}/members/{mom_user_id}"),
            &mom,
        )
        .await;
    let dad_leaves = app
        .delete(
            &format!("/v1/households/{household_id}/members/{dad_user_id}"),
            &dad,
        )
        .await;
    let dad_after_leaving = app
        .get(&format!("/v1/households/{household_id}/feedings"), &dad)
        .await;

    assert_eq!(dad_removes_mom.status, StatusCode::FORBIDDEN);
    assert_eq!(mom_leaves.status, StatusCode::FORBIDDEN);
    assert_eq!(dad_leaves.status, StatusCode::NO_CONTENT);
    assert_eq!(dad_after_leaving.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn owner_can_remove_a_member() {
    let app = TestApp::new().await;
    let mom = app.sign_in("mom").await;
    let household_id = app.create_household(&mom).await;
    let dad = app.add_member(&mom, &household_id, "dad", "viewer").await;
    let dad_user_id = app.get("/v1/me", &dad).await.body["user"]["id"]
        .as_str()
        .unwrap()
        .to_owned();

    let removal = app
        .delete(
            &format!("/v1/households/{household_id}/members/{dad_user_id}"),
            &mom,
        )
        .await;
    let members = app
        .get(&format!("/v1/households/{household_id}/members"), &mom)
        .await;

    assert_eq!(removal.status, StatusCode::NO_CONTENT);
    assert_eq!(members.body["members"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn deleting_the_owner_account_erases_the_household() {
    let app = TestApp::new().await;
    let mom = app.sign_in("mom").await;
    let household_id = app.create_household(&mom).await;
    let dad = app.add_member(&mom, &household_id, "dad", "editor").await;

    app.post(
        &format!("/v1/households/{household_id}/feedings"),
        &dad,
        bottle_feeding(),
    )
    .await;

    let deletion = app.delete("/v1/me", &mom).await;
    let mom_session_after = app.get("/v1/me", &mom).await;
    let dad_account = app.get("/v1/me", &dad).await;
    let dad_feedings = app
        .get(&format!("/v1/households/{household_id}/feedings"), &dad)
        .await;

    assert_eq!(deletion.status, StatusCode::NO_CONTENT);
    assert_eq!(mom_session_after.status, StatusCode::UNAUTHORIZED);
    assert_eq!(dad_account.body["memberships"], json!([]));
    assert_eq!(dad_feedings.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn deleting_a_member_account_keeps_their_feedings_anonymously() {
    let app = TestApp::new().await;
    let mom = app.sign_in("mom").await;
    let household_id = app.create_household(&mom).await;
    let dad = app.add_member(&mom, &household_id, "dad", "editor").await;
    let feedings_uri = format!("/v1/households/{household_id}/feedings");

    app.post(&feedings_uri, &dad, bottle_feeding()).await;
    app.delete("/v1/me", &dad).await;

    let feedings = app.get(&feedings_uri, &mom).await;

    assert_eq!(feedings.body["feedings"].as_array().unwrap().len(), 1);
    assert!(feedings.body["feedings"][0]["created_by"].is_null());
}

#[tokio::test]
async fn feeding_interval_belongs_to_the_household_and_editors_can_change_it() {
    let app = TestApp::new().await;
    let mom = app.sign_in("mom").await;
    let household_id = app.create_household(&mom).await;
    let dad = app.add_member(&mom, &household_id, "dad", "editor").await;
    let grandma = app.add_member(&mom, &household_id, "grandma", "viewer").await;
    let household_uri = format!("/v1/households/{household_id}");

    let default_interval = app.get("/v1/me", &mom).await.body["memberships"][0]["feeding_interval_minutes"].clone();
    let dad_changes = app.patch(&household_uri, &dad, json!({ "feeding_interval_minutes": 150 })).await;
    let grandma_changes = app.patch(&household_uri, &grandma, json!({ "feeding_interval_minutes": 60 })).await;
    let too_short = app.patch(&household_uri, &mom, json!({ "feeding_interval_minutes": 5 })).await;
    let empty = app.patch(&household_uri, &mom, json!({})).await;

    let seen_by_mom = app.get("/v1/me", &mom).await.body["memberships"][0]["feeding_interval_minutes"].clone();
    let summary = app.get(&format!("{household_uri}/summary"), &grandma).await;

    assert_eq!(default_interval, 180);
    assert_eq!(dad_changes.status, StatusCode::OK);
    assert_eq!(dad_changes.body["feeding_interval_minutes"], 150);
    assert_eq!(grandma_changes.status, StatusCode::FORBIDDEN);
    assert_eq!(too_short.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(empty.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(seen_by_mom, 150);
    assert_eq!(summary.body["feeding_interval_minutes"], 150);
    assert!(summary.body["next_feeding_at"].is_null());
}

#[tokio::test]
async fn household_can_be_created_with_a_custom_interval() {
    let app = TestApp::new().await;
    let mom = app.sign_in("mom").await;

    let created = app.post("/v1/households", &mom, json!({ "name": "Burrito", "feeding_interval_minutes": 120 })).await;
    let renamed = app
        .patch(&format!("/v1/households/{}", created.body["household_id"].as_str().unwrap()), &mom, json!({ "name": "Taco" }))
        .await;

    assert_eq!(created.status, StatusCode::CREATED);
    assert_eq!(created.body["feeding_interval_minutes"], 120);
    assert_eq!(renamed.status, StatusCode::OK);
    assert_eq!(renamed.body["household_name"], "Taco");
    assert_eq!(renamed.body["feeding_interval_minutes"], 120);
}
