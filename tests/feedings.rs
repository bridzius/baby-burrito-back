mod common;

use axum::http::StatusCode;
use chrono::{Duration, Utc};
use common::TestApp;
use serde_json::{Value, json};
use uuid::Uuid;

/// A signed-in owner with a fresh household; `send` targets paths relative to that household.
struct Household {
    app: TestApp,
    session: String,
    base_uri: String,
}

impl Household {
    async fn new() -> Self {
        let app = TestApp::new().await;
        let session = app.sign_in("mom").await;
        let household_id = app.create_household(&session).await;

        Self { app, session, base_uri: format!("/v1/households/{household_id}") }
    }
}

async fn send(household: &Household, method: &str, path: &str, body: Option<Value>) -> (StatusCode, Value) {
    let uri = format!("{}{path}", household.base_uri);
    let response = household.app.request(method, &uri, Some(&household.session), body).await;

    (response.status, response.body)
}

fn minutes_ago(minutes: i64) -> String {
    (Utc::now() - Duration::minutes(minutes)).to_rfc3339()
}

#[tokio::test]
async fn creates_and_lists_feedings_newest_first() {
    let app = Household::new().await;

    let older = json!({ "fed_at": minutes_ago(180), "amount_ml": 90, "kind": "formula" });
    let newer = json!({ "fed_at": minutes_ago(10), "amount_ml": 60, "kind": "breast_milk" });

    let (created_status, created) = send(&app, "POST", "/feedings", Some(older)).await;
    send(&app, "POST", "/feedings", Some(newer)).await;

    assert_eq!(created_status, StatusCode::CREATED);
    assert_eq!(created["amount_ml"], 90);
    assert!(Uuid::parse_str(created["id"].as_str().unwrap()).is_ok());

    let (status, list) = send(&app, "GET", "/feedings?limit=10", None).await;
    let amounts: Vec<i64> = list["feedings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|feeding| feeding["amount_ml"].as_i64().unwrap())
        .collect();

    assert_eq!(status, StatusCode::OK);
    assert_eq!(amounts, vec![60, 90]);
}

#[tokio::test]
async fn pages_with_before_cursor() {
    let app = Household::new().await;

    for minutes in [30, 20, 10] {
        let feeding =
            json!({ "fed_at": minutes_ago(minutes), "amount_ml": minutes, "kind": "formula" });
        send(&app, "POST", "/feedings", Some(feeding)).await;
    }

    let (_, first_page) = send(&app, "GET", "/feedings?limit=2", None).await;
    let cursor = first_page["feedings"][1]["fed_at"].as_str().unwrap();
    let encoded_cursor = cursor.replace('+', "%2B");

    let (_, second_page) = send(
        &app,
        "GET",
        &format!("/feedings?limit=2&before={encoded_cursor}"),
        None,
    )
    .await;

    assert_eq!(second_page["feedings"].as_array().unwrap().len(), 1);
    assert_eq!(second_page["feedings"][0]["amount_ml"], 30);
}

#[tokio::test]
async fn retrying_a_create_with_the_same_id_is_idempotent() {
    let app = Household::new().await;
    let id = Uuid::now_v7();
    let fed_at = minutes_ago(5);
    let feeding = json!({ "id": id, "fed_at": fed_at, "amount_ml": 100, "kind": "formula" });

    let (first_status, _) = send(&app, "POST", "/feedings", Some(feeding.clone())).await;
    let (retry_status, retried) = send(&app, "POST", "/feedings", Some(feeding)).await;

    let conflicting = json!({ "id": id, "fed_at": fed_at, "amount_ml": 120, "kind": "formula" });
    let (conflict_status, _) = send(&app, "POST", "/feedings", Some(conflicting)).await;

    let (_, list) = send(&app, "GET", "/feedings", None).await;

    assert_eq!(first_status, StatusCode::CREATED);
    assert_eq!(retry_status, StatusCode::OK);
    assert_eq!(retried["id"], id.to_string());
    assert_eq!(conflict_status, StatusCode::CONFLICT);
    assert_eq!(list["feedings"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn rejects_invalid_input_with_json_errors() {
    let app = Household::new().await;

    let too_much = json!({ "fed_at": minutes_ago(1), "amount_ml": 9000, "kind": "formula" });
    let unknown_kind = json!({ "fed_at": minutes_ago(1), "amount_ml": 90, "kind": "juice" });
    let future = json!({ "fed_at": (Utc::now() + Duration::hours(1)).to_rfc3339(), "amount_ml": 90, "kind": "formula" });

    let (too_much_status, too_much_body) = send(&app, "POST", "/feedings", Some(too_much)).await;
    let (unknown_kind_status, unknown_kind_body) =
        send(&app, "POST", "/feedings", Some(unknown_kind)).await;
    let (future_status, _) = send(&app, "POST", "/feedings", Some(future)).await;
    let (bad_id_status, _) = send(&app, "GET", "/feedings/not-a-uuid", None).await;

    assert_eq!(too_much_status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(too_much_body["error"]["code"], "validation_failed");
    assert_eq!(unknown_kind_status, StatusCode::BAD_REQUEST);
    assert_eq!(unknown_kind_body["error"]["code"], "bad_request");
    assert_eq!(future_status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(bad_id_status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn updates_and_deletes_a_feeding() {
    let app = Household::new().await;
    let feeding = json!({ "fed_at": minutes_ago(30), "amount_ml": 80, "kind": "formula" });

    let (_, created) = send(&app, "POST", "/feedings", Some(feeding)).await;
    let feeding_uri = format!("/feedings/{}", created["id"].as_str().unwrap());

    let (update_status, updated) = send(
        &app,
        "PATCH",
        &feeding_uri,
        Some(json!({ "amount_ml": 95, "kind": "breast_milk" })),
    )
    .await;

    assert_eq!(update_status, StatusCode::OK);
    assert_eq!(updated["amount_ml"], 95);
    assert_eq!(updated["kind"], "breast_milk");
    assert_eq!(updated["fed_at"], created["fed_at"]);

    let (delete_status, _) = send(&app, "DELETE", &feeding_uri, None).await;
    let (second_delete_status, _) = send(&app, "DELETE", &feeding_uri, None).await;
    let (get_status, _) = send(&app, "GET", &feeding_uri, None).await;

    assert_eq!(delete_status, StatusCode::NO_CONTENT);
    assert_eq!(second_delete_status, StatusCode::NOT_FOUND);
    assert_eq!(get_status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn summary_totals_the_last_day_and_reports_the_latest_feeding() {
    let app = Household::new().await;

    let feedings = [
        json!({ "fed_at": minutes_ago(26 * 60), "amount_ml": 200, "kind": "formula" }),
        json!({ "fed_at": minutes_ago(6 * 60), "amount_ml": 90, "kind": "formula" }),
        json!({ "fed_at": minutes_ago(60), "amount_ml": 70, "kind": "breast_milk" }),
    ];

    for feeding in feedings {
        send(&app, "POST", "/feedings", Some(feeding)).await;
    }

    let (status, summary) = send(&app, "GET", "/summary", None).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(summary["total_ml"], 160);
    assert_eq!(summary["feeding_count"], 2);
    assert_eq!(summary["formula_ml"], 90);
    assert_eq!(summary["breast_milk_ml"], 70);
    assert_eq!(summary["last_feeding"]["amount_ml"], 70);
}

#[tokio::test]
async fn summary_on_an_empty_database_has_no_last_feeding() {
    let app = Household::new().await;

    let (status, summary) = send(&app, "GET", "/summary", None).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(summary["total_ml"], 0);
    assert!(summary["last_feeding"].is_null());
}

#[tokio::test]
async fn daily_statistics_cover_the_requested_days() {
    let app = Household::new().await;

    send(
        &app,
        "POST",
        "/feedings",
        Some(json!({ "fed_at": minutes_ago(1), "amount_ml": 50, "kind": "formula" })),
    )
    .await;

    let (status, statistics) = send(
        &app,
        "GET",
        "/stats/daily?days=3&timezone=Europe/Prague",
        None,
    )
    .await;
    let (bad_timezone_status, _) =
        send(&app, "GET", "/stats/daily?timezone=Mars/Olympus", None).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(statistics["timezone"], "Europe/Prague");
    assert_eq!(statistics["days"].as_array().unwrap().len(), 3);
    assert_eq!(statistics["total_ml"], 50);
    assert_eq!(bad_timezone_status, StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn reusing_a_feeding_id_from_another_household_conflicts_without_leaking_it() {
    let app = TestApp::new().await;
    let mom = app.sign_in("mom").await;
    let neighbour = app.sign_in("neighbour").await;
    let our_household = app.create_household(&mom).await;
    let their_household = app.create_household(&neighbour).await;

    let shared_id = Uuid::now_v7();
    let feeding = json!({ "id": shared_id, "fed_at": minutes_ago(5), "amount_ml": 80, "kind": "formula" });

    let ours = app.post(&format!("/v1/households/{our_household}/feedings"), &mom, feeding.clone()).await;
    let theirs = app.post(&format!("/v1/households/{their_household}/feedings"), &neighbour, feeding).await;
    let their_lookup = app.get(&format!("/v1/households/{their_household}/feedings/{shared_id}"), &neighbour).await;

    assert_eq!(ours.status, StatusCode::CREATED);
    assert_eq!(theirs.status, StatusCode::CONFLICT);
    assert_eq!(their_lookup.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn nursing_is_logged_with_duration_and_side_instead_of_amount() {
    let household = Household::new().await;

    let nursing = json!({ "fed_at": minutes_ago(20), "kind": "nursing", "duration_minutes": 15, "breast_side": "left" });
    let nursing_with_amount =
        json!({ "fed_at": minutes_ago(20), "kind": "nursing", "duration_minutes": 15, "breast_side": "left", "amount_ml": 60 });
    let nursing_without_side = json!({ "fed_at": minutes_ago(20), "kind": "nursing", "duration_minutes": 15 });
    let bottle_without_amount = json!({ "fed_at": minutes_ago(20), "kind": "formula" });

    let (created_status, created) = send(&household, "POST", "/feedings", Some(nursing)).await;
    let (with_amount_status, _) = send(&household, "POST", "/feedings", Some(nursing_with_amount)).await;
    let (without_side_status, _) = send(&household, "POST", "/feedings", Some(nursing_without_side)).await;
    let (bottle_status, _) = send(&household, "POST", "/feedings", Some(bottle_without_amount)).await;

    assert_eq!(created_status, StatusCode::CREATED);
    assert_eq!(created["kind"], "nursing");
    assert_eq!(created["duration_minutes"], 15);
    assert_eq!(created["breast_side"], "left");
    assert!(created["amount_ml"].is_null());
    assert_eq!(with_amount_status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(without_side_status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(bottle_status, StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn changing_the_kind_clears_the_previous_kinds_fields() {
    let household = Household::new().await;
    let nursing = json!({ "fed_at": minutes_ago(20), "kind": "nursing", "duration_minutes": 15, "breast_side": "left" });

    let (_, created) = send(&household, "POST", "/feedings", Some(nursing)).await;
    let feeding_path = format!("/feedings/{}", created["id"].as_str().unwrap());

    let (side_status, side_changed) = send(&household, "PATCH", &feeding_path, Some(json!({ "breast_side": "right" }))).await;
    let (incomplete_status, _) = send(&household, "PATCH", &feeding_path, Some(json!({ "kind": "formula" }))).await;
    let (bottle_status, bottle) =
        send(&household, "PATCH", &feeding_path, Some(json!({ "kind": "breast_milk", "amount_ml": 70 }))).await;

    assert_eq!(side_status, StatusCode::OK);
    assert_eq!(side_changed["duration_minutes"], 15);
    assert_eq!(side_changed["breast_side"], "right");
    assert_eq!(incomplete_status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(bottle_status, StatusCode::OK);
    assert_eq!(bottle["kind"], "breast_milk");
    assert_eq!(bottle["amount_ml"], 70);
    assert!(bottle["duration_minutes"].is_null());
    assert!(bottle["breast_side"].is_null());
}

#[tokio::test]
async fn idempotent_retry_compares_nursing_fields_too() {
    let household = Household::new().await;
    let id = Uuid::now_v7();
    let fed_at = minutes_ago(20);
    let left = json!({ "id": id, "fed_at": fed_at, "kind": "nursing", "duration_minutes": 15, "breast_side": "left" });
    let right = json!({ "id": id, "fed_at": fed_at, "kind": "nursing", "duration_minutes": 15, "breast_side": "right" });

    send(&household, "POST", "/feedings", Some(left.clone())).await;
    let (retry_status, _) = send(&household, "POST", "/feedings", Some(left)).await;
    let (other_side_status, _) = send(&household, "POST", "/feedings", Some(right)).await;

    assert_eq!(retry_status, StatusCode::OK);
    assert_eq!(other_side_status, StatusCode::CONFLICT);
}

#[tokio::test]
async fn summary_reports_nursing_minutes_and_the_next_feeding() {
    let household = Household::new().await;

    let feedings = [
        json!({ "fed_at": minutes_ago(300), "kind": "formula", "amount_ml": 90 }),
        json!({ "fed_at": minutes_ago(120), "kind": "nursing", "duration_minutes": 20, "breast_side": "right" }),
    ];

    for feeding in feedings {
        send(&household, "POST", "/feedings", Some(feeding)).await;
    }

    let (status, summary) = send(&household, "GET", "/summary", None).await;

    let last_fed_at: chrono::DateTime<Utc> = summary["last_feeding"]["fed_at"].as_str().unwrap().parse().unwrap();
    let next_feeding_at: chrono::DateTime<Utc> = summary["next_feeding_at"].as_str().unwrap().parse().unwrap();

    assert_eq!(status, StatusCode::OK);
    assert_eq!(summary["feeding_count"], 2);
    assert_eq!(summary["total_ml"], 90);
    assert_eq!(summary["nursing_minutes"], 20);
    assert_eq!(summary["nursing_right_minutes"], 20);
    assert_eq!(summary["feeding_interval_minutes"], 180);
    assert_eq!(next_feeding_at - last_fed_at, Duration::minutes(180));
}
