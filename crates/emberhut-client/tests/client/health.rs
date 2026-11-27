//! `GET /health`: whether the server answers, and the dialect each client speaks.

use std::net::TcpListener;
use std::time::{Duration, Instant};

use emberhut_client::dialect::Dialect;
use emberhut_client::{Client, ClientConfig, Error, MemorySessionStore};
use serde_json::json;

use crate::support::{Reply, TestServer};

fn healthy() -> serde_json::Value {
    json!({"ok": true, "lag": {"ms": 1, "p95_1m": 2, "max_1m": 3, "p95_5m": 2, "max_5m": 4}})
}

#[test]
fn a_server_that_answers_is_healthy_without_a_session() {
    let server = TestServer::start(|_| Reply::Json(200, healthy()));
    let client = server.client_speaking(false);
    let health = client.health(Duration::from_secs(5)).unwrap();
    assert!(health.ok);
    assert_eq!(health.lag.max_5m, 4);
    let asked = server.requests_to("/health");
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0].method, "GET");
    assert_eq!(
        asked[0].header("authorization"),
        None,
        "the probe isn't signed"
    );
}

#[test]
fn a_failing_or_strange_answer_is_not_health() {
    let server = TestServer::start(|_| Reply::Json(503, json!({"error": "starting"})));
    let error = server.client_speaking(false).health(Duration::from_secs(5));
    assert!(
        matches!(error, Err(Error::Status { status: 503, .. })),
        "{error:?}"
    );

    let server = TestServer::start(|_| Reply::Text(200, "<html>a proxy's page</html>"));
    let error = server.client_speaking(false).health(Duration::from_secs(5));
    assert!(matches!(error, Err(Error::Decode(_))), "{error:?}");

    let server = TestServer::start(|_| Reply::Json(200, json!({"ok": false, "lag": {}})));
    let error = server.client_speaking(false).health(Duration::from_secs(5));
    assert!(error.is_err(), "{error:?}");
}

#[test]
fn a_server_that_is_gone_fails_the_check_at_once() {
    // A port nothing listens on: the connection is refused.
    let port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let config = ClientConfig::new(&format!("http://127.0.0.1:{port}")).unwrap();
    let client = Client::new(&config, MemorySessionStore::new()).unwrap();
    let started = Instant::now();
    let error = client.health(Duration::from_secs(5)).unwrap_err();
    assert!(matches!(error, Error::Network(_)), "{error}");
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[test]
fn each_client_speaks_its_own_servers_dialect() {
    let server = TestServer::start(|_| Reply::Json(200, healthy()));
    assert!(server.client_speaking(false).dialect().is_empty());
    assert_eq!(
        server.client_speaking(true).dialect().is_empty(),
        Dialect::built().is_empty(),
        "a server marked as speaking it gets the built dialect"
    );
    let default = ClientConfig::new("http://example.com").unwrap();
    assert_eq!(default.dialect().is_empty(), Dialect::built().is_empty());
    assert!(default.with_built_dialect(false).dialect().is_empty());
}
