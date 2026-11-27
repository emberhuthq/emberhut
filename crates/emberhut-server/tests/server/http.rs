//! What every request goes through: unknown paths, request ids.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::json;

use crate::common::{TestApp, get, json};

#[tokio::test]
async fn an_unknown_path_is_404_with_an_error_body() {
    let app = TestApp::new().await;
    let response = app.send(get("/nowhere")).await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        response.headers()["content-type"],
        "application/json",
        "errors are JSON"
    );
    assert_eq!(json(response).await, json!({ "error": "not found" }));
}

#[tokio::test]
async fn every_response_carries_a_request_id() {
    let app = TestApp::new().await;
    let response = app.send(get("/nowhere")).await;
    let id = response.headers()["x-request-id"].to_str().unwrap();
    assert_eq!(id.len(), 36, "a new UUID: {id}");

    let response = app
        .send(
            Request::get("/nowhere")
                .header("x-request-id", "from-the-client")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(response.headers()["x-request-id"], "from-the-client");
}

#[tokio::test]
async fn a_client_request_id_that_is_too_long_or_not_plain_is_replaced() {
    let app = TestApp::new().await;
    for unusable in ["x".repeat(65), "has space".to_owned(), String::new()] {
        let response = app
            .send(
                Request::get("/nowhere")
                    .header("x-request-id", &unusable)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
        let id = response.headers()["x-request-id"].to_str().unwrap();
        assert_eq!(
            id.len(),
            36,
            "{unusable:?} was replaced by a new UUID, not {id:?}"
        );
    }
    let longest = "x".repeat(64);
    let response = app
        .send(
            Request::get("/nowhere")
                .header("x-request-id", &longest)
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(response.headers()["x-request-id"], longest.as_str());
}
