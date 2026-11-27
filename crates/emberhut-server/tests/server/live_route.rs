//! `GET /api/live`: the room as it is, then each change, as server-sent events of `LiveEvent`.
#![expect(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "a test helper reads the stream: a failure there fails the test"
)]

use std::time::Duration;

use axum::body::Body;
use emberhut_api::error::ErrorBody;
use emberhut_api::live::LiveEvent;
use emberhut_core::RoomId;
use http_body_util::BodyExt as _;

use crate::common::{Call, TestApp};

/// Reads a live stream's events as they come.
struct Events {
    body: Body,
    buffer: String,
}

impl Events {
    /// The next event's data, read as a `LiveEvent` (every event the server sends must be one).
    async fn next(&mut self) -> LiveEvent {
        loop {
            if let Some(end) = self.buffer.find("\n\n") {
                let block: String = self.buffer.drain(..end + 2).collect();
                let data: Vec<&str> = block
                    .lines()
                    .filter_map(|line| line.strip_prefix("data: "))
                    .collect();
                if data.is_empty() {
                    continue; // a comment: the keep-alive ping
                }
                let json = data.join("\n");
                return serde_json::from_str(&json)
                    .unwrap_or_else(|error| panic!("not a LiveEvent ({error}): {json}"));
            }
            let frame = tokio::time::timeout(Duration::from_secs(5), self.body.frame())
                .await
                .expect("an event within 5 s")
                .expect("the stream is open")
                .expect("a frame");
            if let Ok(chunk) = frame.into_data() {
                self.buffer.push_str(std::str::from_utf8(&chunk).unwrap());
            }
        }
    }
}

async fn open(app: &TestApp, call: Call) -> Events {
    let response = app.send(call.request()).await;
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["content-type"], "text/event-stream");
    assert_eq!(response.headers()["cache-control"], "no-cache");
    assert_eq!(response.headers()["x-accel-buffering"], "no");
    Events {
        body: response.into_body(),
        buffer: String::new(),
    }
}

#[tokio::test]
pub(crate) async fn the_stream_starts_with_the_room_now_then_follows_a_turn_to_its_end() {
    let app = TestApp::new().await;
    let session = app.sign_in().await;
    let mut events = open(&app, Call::get("/api/live").token(&session)).await;
    assert_eq!(events.next().await, LiveEvent::Idle, "nothing runs yet");

    app.assistant
        .answers(&["Looking at the example", "Writing"], Some("Done."));
    app.assistant.hold();
    app.state
        .talk()
        .talk(
            RoomId::MAIN,
            "Can you look at the example?".to_owned(),
            None,
        )
        .await
        .unwrap();
    // Reading, or already its first step: a watcher sees the latest value.
    assert!(matches!(events.next().await, LiveEvent::Working(_)));
    let mut seen = Vec::new();
    app.assistant.release();
    loop {
        let event = events.next().await;
        if event == LiveEvent::Idle {
            break;
        }
        seen.push(event.doing().unwrap_or_default().to_owned());
    }
    assert!(
        seen.iter().all(|step| [
            "Reading your new messages",
            "Looking at the example",
            "Writing"
        ]
        .contains(&step.as_str())),
        "{seen:?}"
    );
    let reply = app
        .state
        .state()
        .state(RoomId::MAIN)
        .await
        .unwrap()
        .chat
        .pop()
        .unwrap();
    assert_eq!(reply.text, "Done.");
    assert!(!reply.mine);
}

#[tokio::test]
async fn a_stream_opened_during_a_turn_starts_with_what_is_being_done() {
    let app = TestApp::new().await;
    let session = app.sign_in().await;
    app.assistant.answers(&["Thinking"], None);
    app.assistant.hold();
    app.state
        .talk()
        .talk(RoomId::MAIN, "hello".to_owned(), None)
        .await
        .unwrap();
    let mut events = open(&app, Call::get("/api/live").token(&session)).await;
    assert!(matches!(events.next().await, LiveEvent::Working(_)));
    app.assistant.release();
    while events.next().await != LiveEvent::Idle {}
}

#[tokio::test]
async fn the_stream_ends_when_the_server_stops() {
    let app = TestApp::new().await;
    let session = app.sign_in().await;
    let mut events = open(&app, Call::get("/api/live").token(&session)).await;
    assert_eq!(events.next().await, LiveEvent::Idle);
    app.state.stopping().cancel();
    let end = tokio::time::timeout(Duration::from_secs(5), events.body.frame())
        .await
        .expect("the stream ends at once");
    assert!(end.is_none(), "no more frames");
}

#[tokio::test]
pub(crate) async fn without_a_session_the_stream_is_refused() {
    let app = TestApp::new().await;
    let refused = app.call(Call::get("/api/live")).await;
    assert_eq!(refused.status, 401);
    assert_eq!(
        refused.read_as::<ErrorBody>(),
        ErrorBody::sign_in_required()
    );
}

#[tokio::test]
async fn the_document_lists_the_stream_as_live_events() {
    let document = crate::common::document();
    let ok = &document["paths"]["/api/live"]["get"]["responses"]["200"];
    let description = ok["description"].as_str().unwrap();
    assert!(description.contains("LiveEvent"), "{description}");
    assert!(ok["content"]["text/event-stream"].is_object());
}

/// The next raw block of the stream (an event or a comment), waiting as long as it takes: under
/// a paused clock, that is until the next timer.
async fn next_block(body: &mut Body, buffer: &mut String) -> String {
    loop {
        if let Some(end) = buffer.find("\n\n") {
            return buffer.drain(..end + 2).collect();
        }
        let frame = body.frame().await.unwrap().unwrap();
        if let Ok(chunk) = frame.into_data() {
            buffer.push_str(std::str::from_utf8(&chunk).unwrap());
        }
    }
}

/// A report-mode app, so the stream needs no session (and the request no database), with the
/// clock paused from here on.
async fn paused_app() -> TestApp {
    let app = TestApp::with_mode(emberhut_server::config::AuthMode::Report).await;
    tokio::time::pause();
    app
}

#[tokio::test]
async fn a_quiet_stream_gets_a_ping_every_15_seconds() {
    let app = paused_app().await;
    let response = app.send(Call::get("/api/live").request()).await;
    let mut body = response.into_body();
    let mut buffer = String::new();
    assert_eq!(next_block(&mut body, &mut buffer).await, "data: null\n\n");
    for _ in 0..2 {
        let before = tokio::time::Instant::now();
        assert_eq!(next_block(&mut body, &mut buffer).await, ": ping\n\n");
        // The paused clock moves in whole timer ticks: 15 s, to the millisecond.
        let waited = before.elapsed();
        assert!(
            (Duration::from_secs(15)..=Duration::from_millis(15_010)).contains(&waited),
            "{waited:?}"
        );
    }
}

#[tokio::test]
async fn the_request_timeout_does_not_cut_a_stream() {
    let app = paused_app().await;
    let router = emberhut_server::app::router(app.state.clone(), Duration::from_millis(100));
    let response = tower::ServiceExt::oneshot(router, Call::get("/api/live").request())
        .await
        .unwrap();
    let mut body = response.into_body();
    let mut buffer = String::new();
    assert_eq!(next_block(&mut body, &mut buffer).await, "data: null\n\n");
    tokio::time::advance(Duration::from_secs(1)).await;
    app.state.live().set(
        RoomId::MAIN,
        Some(emberhut_api::live::Live {
            text: String::new(),
            doing: Some("Still here".to_owned()),
        }),
    );
    assert_eq!(
        next_block(&mut body, &mut buffer).await,
        "data: {\"text\":\"\",\"doing\":\"Still here\"}\n\n",
        "ten timeouts later, the stream still carries events"
    );
}

#[tokio::test]
async fn the_stream_is_never_gzipped() {
    let app = TestApp::new().await;
    let session = app.sign_in().await;
    let response = app
        .send(
            Call::get("/api/live")
                .token(&session)
                .header("accept-encoding", "gzip")
                .request(),
        )
        .await;
    assert_eq!(response.status(), 200);
    assert!(response.headers().get("content-encoding").is_none());
}

#[tokio::test]
async fn a_live_sink_can_show_the_answer_as_it_is_written() {
    let app = TestApp::new().await;
    let session = app.sign_in().await;
    let mut events = open(&app, Call::get("/api/live").token(&session)).await;
    assert_eq!(events.next().await, LiveEvent::Idle);
    app.state
        .live()
        .update(RoomId::MAIN, |live| live.text = "Half an".to_owned());
    let event = events.next().await;
    assert_eq!(event.live().unwrap().text, "Half an");
}
