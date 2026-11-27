//! `GET /health` and `GET /api/loop`: the server answers, with its runtime's lag and the work
//! that held it.

use std::time::Duration;

use emberhut_api::error::ErrorBody;
use emberhut_api::health::{Health, LoopReport};

use crate::common::{Call, TestApp, assert_keys_within, fixture};

#[tokio::test]
pub(crate) async fn health_needs_no_sign_in_and_says_the_lag() {
    let app = TestApp::new().await;
    let answer = app.call(Call::get("/health")).await;
    assert_eq!(answer.status, 200);
    assert_keys_within(&answer.json(), &fixture("health.json"), "health");
    let health: Health = answer.read_as();
    assert!(health.ok);
    assert_eq!(health.lag.max_5m, 0, "nothing sampled yet");
}

#[tokio::test]
pub(crate) async fn the_loop_report_names_the_work_that_held_the_runtime() {
    let app = TestApp::new().await;
    let session = app.sign_in().await;
    app.state
        .lag()
        .note_slow(Duration::from_millis(120), "GET /api/state");
    app.state
        .lag()
        .note_slow(Duration::from_millis(60), "GET /api/state");
    app.state
        .lag()
        .note_slow(Duration::from_millis(70), "POST /api/tasks/:x/message");
    app.state
        .lag()
        .note_slow(Duration::from_millis(10), "GET /api/quick");
    let answer = app.call(Call::get("/api/loop").token(&session)).await;
    assert_eq!(answer.status, 200);
    assert_keys_within(&answer.json(), &fixture("loop.json"), "loop");
    let report: LoopReport = answer.read_as();
    let slow: Vec<(&str, u32, u64, u64)> = report
        .slow
        .iter()
        .map(|work| (work.what.as_str(), work.n, work.ms, work.max))
        .collect();
    assert_eq!(
        slow,
        [
            ("GET /api/state", 2, 180, 120),
            ("POST /api/tasks/:x/message", 1, 70, 70)
        ],
        "the longest in all first; under 50 ms is not noted"
    );
}

#[tokio::test]
pub(crate) async fn the_loop_report_needs_a_session() {
    let app = TestApp::new().await;
    let refused = app.call(Call::get("/api/loop")).await;
    assert_eq!(refused.status, 401);
    assert_eq!(
        refused.read_as::<ErrorBody>(),
        ErrorBody::sign_in_required()
    );
}

#[tokio::test]
async fn the_running_server_samples_its_lag() {
    let app = TestApp::new().await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let server = tokio::spawn(emberhut_server::run(
        listener,
        app.state.clone(),
        Duration::from_secs(5),
        Duration::from_secs(5),
        async {
            stopped.await.ok();
        },
    ));
    let text = crate::common::http_client()
        .get(format!("http://{address}/health"))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    let health: Health = serde_json::from_str(&text).unwrap();
    assert!(health.ok);
    tokio::time::timeout(Duration::from_secs(5), async {
        while app.state.lag().samples() == 0 {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the probe samples within a second of the start");
    stop.send(()).unwrap();
    server.await.unwrap().unwrap();
}
