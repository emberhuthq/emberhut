use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt as _;
use serde_json::{Value, json};
use tower::ServiceExt as _;

use super::*;
use crate::extension::ExtensionFolder;
use crate::network::Callers;

/// This Mac, calling.
fn local() -> ConnectInfo<SocketAddr> {
    ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 50_000)))
}

/// Answers `/which` with the body it got, and refuses `/press`.
struct Echo;

impl Handler for Echo {
    fn handle(&self, path: &str, body: &Reply) -> Result<Reply, Fail> {
        match path {
            "/which" => Ok(body.clone()),
            "/press" => Err(Fail::never("Never here.")),
            other => Err(Fail::missing(format!("no {other}"))),
        }
    }
}

fn app(dir: &std::path::Path) -> Router {
    router(AppState::new(
        Arc::new(Echo),
        Token::new("s3cret"),
        Callers::for_listen(Some("10.0.0.5".parse().unwrap())),
        ExtensionFolder::new(dir.join("Extension"), crate::extension::id::EXTENSION_ID),
    ))
}

async fn call(app: Router, request: Request<Body>) -> (StatusCode, Value) {
    let response = app.oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

fn post(path: &str, body: &str) -> Request<Body> {
    Request::post(path)
        .extension(local())
        .header("authorization", "Bearer s3cret")
        .header("content-type", "application/json")
        .body(Body::from(body.to_owned()))
        .unwrap()
}

#[tokio::test]
async fn without_the_token_nothing_is_answered() {
    let dir = tempfile::tempdir().unwrap();
    for request in [
        Request::post("/which")
            .extension(local())
            .body(Body::empty())
            .unwrap(),
        Request::post("/which")
            .extension(local())
            .header("authorization", "Bearer wrong")
            .body(Body::empty())
            .unwrap(),
        Request::get("/extension")
            .extension(local())
            .body(Body::empty())
            .unwrap(),
        Request::post("/nowhere")
            .extension(local())
            .body(Body::empty())
            .unwrap(),
    ] {
        let (status, body) = call(app(dir.path()), request).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(body, json!({ "error": "not authorized" }));
    }
}

#[tokio::test]
async fn only_this_mac_and_the_listen_address_s_range_are_answered() {
    let dir = tempfile::tempdir().unwrap();
    let from = |ip: [u8; 4]| {
        Request::post("/which")
            .extension(ConnectInfo(SocketAddr::from((ip, 50_000))))
            .header("authorization", "Bearer s3cret")
            .body(Body::from("{}"))
            .unwrap()
    };
    let (status, _) = call(app(dir.path()), from([10, 9, 8, 7])).await;
    assert_eq!(status, StatusCode::OK);
    for outside in [[8, 8, 8, 8], [192, 168, 1, 2]] {
        let (status, body) = call(app(dir.path()), from(outside)).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(
            body,
            json!({ "error": "not from an address this helper answers" })
        );
    }
    let unknown = Request::post("/which")
        .header("authorization", "Bearer s3cret")
        .body(Body::from("{}"))
        .unwrap();
    let (status, _) = call(app(dir.path()), unknown).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn a_path_is_answered_with_its_json_or_its_refusal() {
    let dir = tempfile::tempdir().unwrap();
    let (status, body) = call(app(dir.path()), post("/which", r#"{"app":"Notes"}"#)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!({ "app": "Notes" }));
    let (status, body) = call(app(dir.path()), post("/which", "not json")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!({}));
    let (status, body) = call(app(dir.path()), post("/press", "{}")).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body, json!({ "error": "Never here." }));
    let (status, body) = call(app(dir.path()), post("/chrome/open", "{}")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body, json!({ "error": "no /chrome/open" }));
}

#[tokio::test]
async fn every_server_path_is_routed() {
    // Each answers from the handler (Echo's 404 says the path), not the router's fallback.
    let dir = tempfile::tempdir().unwrap();
    for path in PATHS {
        let (_, body) = call(app(dir.path()), post(path, "{}")).await;
        assert_ne!(body, Value::Null, "{path}");
    }
}

#[tokio::test]
async fn the_extension_s_version_is_null_until_one_is_installed() {
    let dir = tempfile::tempdir().unwrap();
    let request = Request::get("/extension")
        .extension(local())
        .header("authorization", "Bearer s3cret")
        .body(Body::empty())
        .unwrap();
    let (status, body) = call(app(dir.path()), request).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!({ "version": null }));
}

#[tokio::test]
async fn an_install_needs_a_zip_body_within_the_limit() {
    let dir = tempfile::tempdir().unwrap();
    let wrong_type = post("/extension/install", "{}");
    let (status, body) = call(app(dir.path()), wrong_type).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["error"].as_str().unwrap().contains("application/zip"));
    let zip = |bytes: Vec<u8>| {
        Request::post("/extension/install")
            .extension(local())
            .header("authorization", "Bearer s3cret")
            .header("content-type", "application/zip")
            .body(Body::from(bytes))
            .unwrap()
    };
    let (status, body) = call(app(dir.path()), zip(vec![0; MAX_ZIP + 1])).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body, json!({ "error": "the zip is over 20 MB" }));
    let (status, body) = call(app(dir.path()), zip(b"not a zip".to_vec())).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["error"].as_str().unwrap().starts_with("not a zip"));
}
