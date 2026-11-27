//! A worker watched as the runner session `w-<task>`: its transcript, live steps, a reply to
//! it, and stopping it.

use emberhut_api::error::ErrorBody;
use serde_json::json;

use crate::common::{Call, TestApp};

#[tokio::test]
pub(crate) async fn a_workers_transcript_is_its_steps() {
    let app = TestApp::new().await;
    app.workers.worker(12, "Write the report", 5);
    let session = app.sign_in().await;
    let answer = app
        .call(
            Call::get("/api/runner/sessions/w-12/transcript")
                .query("limit=2")
                .token(&session),
        )
        .await;
    assert_eq!(answer.status, 200, "{}", answer.text);
    let page = answer.json();
    assert_eq!(page["session"]["title"], "Write the report");
    assert_eq!(
        page["events"],
        json!([
            { "seq": 4, "kind": "text", "text": "step 4" },
            { "seq": 5, "kind": "text", "text": "step 5" },
        ])
    );
    assert_eq!(page["earlier"], true);
    let after = app
        .call(
            Call::get("/api/runner/sessions/w-12/transcript")
                .query("after=3")
                .token(&session),
        )
        .await
        .json();
    assert_eq!(after["events"].as_array().unwrap().len(), 2);
    assert_eq!(after["earlier"], false);
    let answer = app
        .call(Call::get("/api/runner/sessions/w-99/transcript").token(&session))
        .await;
    assert_eq!(answer.status, 404);
    assert_eq!(answer.read_as::<ErrorBody>().error, "no such task");
}

#[tokio::test]
pub(crate) async fn a_workers_live_stream_sends_its_steps_and_head() {
    let app = TestApp::new().await;
    app.workers.worker(12, "Write the report", 2);
    let session = app.sign_in().await;
    let response = app
        .send(
            Call::get("/api/runner/sessions/w-12/live")
                .query("after=1")
                .token(&session)
                .request(),
        )
        .await;
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["content-type"], "text/event-stream");
    let mut body = response.into_body();
    let mut seen = String::new();
    while !seen.contains("\"head\"") {
        let frame = http_body_util::BodyExt::frame(&mut body)
            .await
            .unwrap()
            .unwrap();
        if let Ok(data) = frame.into_data() {
            seen.push_str(&String::from_utf8_lossy(&data));
        }
    }
    assert!(seen.contains("\"step 2\""), "{seen}");
    assert!(!seen.contains("\"step 1\""), "only what came after: {seen}");
    let answer = app
        .call(Call::get("/api/runner/sessions/w-99/live").token(&session))
        .await;
    assert_eq!(answer.status, 404);
}

#[tokio::test]
pub(crate) async fn the_person_replies_to_and_stops_a_worker() {
    let app = TestApp::new().await;
    app.workers.worker(12, "Write the report", 1);
    let session = app.sign_in().await;
    let answer = app
        .call(
            Call::post("/api/runner/sessions/w-12/send")
                .token(&session)
                .json(&json!({ "text": " Shorter " })),
        )
        .await;
    assert_eq!(answer.status, 200, "{}", answer.text);
    assert_eq!(answer.json(), json!({ "ok": true, "to": "worker" }));
    assert_eq!(app.workers.messages(), [(12, "Shorter".to_owned())]);
    let answer = app
        .call(
            Call::post("/api/runner/sessions/w-12/send")
                .token(&session)
                .json(&json!({ "text": "" })),
        )
        .await;
    assert_eq!(answer.status, 400);
    app.workers.refuse("its worker isn't running");
    let answer = app
        .call(
            Call::post("/api/runner/sessions/w-12/send")
                .token(&session)
                .json(&json!({ "text": "Hi" })),
        )
        .await;
    assert_eq!(answer.status, 409);
    assert_eq!(
        answer.json(),
        json!({ "ok": false, "error": "its worker isn't running" })
    );
    for what in ["stop", "interrupt"] {
        let answer = app
            .call(Call::post(&format!("/api/runner/sessions/w-12/{what}")).token(&session))
            .await;
        assert_eq!(answer.status, 200);
    }
    assert_eq!(app.workers.stopped(), [12, 12]);
    for what in ["send", "stop", "interrupt"] {
        let answer = app
            .call(
                Call::post(&format!("/api/runner/sessions/w-99/{what}"))
                    .token(&session)
                    .json(&json!({ "text": "Hi" })),
            )
            .await;
        assert_eq!(answer.status, 404, "{what}");
    }
    for (method, what) in [
        ("GET", "transcript"),
        ("GET", "live"),
        ("POST", "send"),
        ("POST", "stop"),
        ("POST", "interrupt"),
    ] {
        let path = format!("/api/runner/sessions/w-12/{what}");
        let call = if method == "GET" {
            Call::get(&path)
        } else {
            Call::post(&path)
        };
        assert_eq!(app.call(call).await.status, 401, "{what}");
    }
}
