//! `/api/dark`: the off-the-record room. Its chat, a reply streamed from the local model or,
//! when that is off, the cloud one (saying so), one reply at a time, and clearing it.
#![expect(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "tests: a failure fails the test"
)]

use std::time::Duration;

use axum::body::Body;
use emberhut_api::dark::{DarkEvent, DarkRoom as DarkPage};
use emberhut_core::dark::{FALLBACK, FALLBACK_ERROR};
use emberhut_core::session::Session;
use http_body_util::BodyExt as _;
use serde_json::json;

use crate::common::{Call, TestApp};

/// Reads a reply's events to its last (`done` or `error`).
async fn events(body: Body) -> Vec<DarkEvent> {
    let mut body = body;
    let mut buffer = String::new();
    let mut events = Vec::new();
    loop {
        while let Some(end) = buffer.find("\n\n") {
            let block: String = buffer.drain(..end + 2).collect();
            if let Some(data) = block.strip_prefix("data: ") {
                let event: DarkEvent = serde_json::from_str(data.trim_end()).unwrap();
                let last = event.done.is_some() || event.error.is_some();
                events.push(event);
                if last {
                    return events;
                }
            }
        }
        // Generous: under coverage instrumentation on a busy machine an event can take seconds.
        let frame = tokio::time::timeout(Duration::from_secs(30), body.frame())
            .await
            .expect("an event within 30 s")
            .expect("the stream is open")
            .unwrap();
        if let Ok(chunk) = frame.into_data() {
            buffer.push_str(std::str::from_utf8(&chunk).unwrap());
        }
    }
}

async fn say(app: &TestApp, session: &Session, text: &str) -> Vec<DarkEvent> {
    let response = app
        .send(
            Call::post("/api/dark")
                .token(session)
                .json(&json!({ "text": text }))
                .request(),
        )
        .await;
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["content-type"], "text/event-stream");
    events(response.into_body()).await
}

async fn page(app: &TestApp, session: &Session) -> DarkPage {
    let got = app.call(Call::get("/api/dark").token(session)).await;
    assert_eq!(got.status, 200);
    got.read_as()
}

#[tokio::test]
pub(crate) async fn the_local_model_answers_and_the_chat_is_kept() {
    let app = TestApp::new().await;
    let session = app.sign_in().await;
    app.dark_local.replies(&[Ok("Hello there.")]);
    let got = say(&app, &session, "  hi  ").await;
    assert_eq!(got[0].delta.as_deref(), Some("Hello there."));
    let done = got.last().unwrap().done.clone().unwrap();
    assert_eq!(
        (done.role.as_str(), done.text.as_str()),
        ("assistant", "Hello there.")
    );

    let room = page(&app, &session).await;
    assert_eq!(room.model, "Local");
    assert_eq!(
        room.place.as_deref(),
        Some("Local on your own machine (Cloud when it's offline)")
    );
    assert!(!room.busy);
    let chat: Vec<(&str, &str)> = room
        .chat
        .iter()
        .map(|m| (m.role.as_str(), m.text.as_str()))
        .collect();
    assert_eq!(chat, vec![("user", "hi"), ("assistant", "Hello there.")]);
    let asked = app.dark_local.requests();
    assert!(asked[0].system.contains("off the record"));
    assert_eq!(asked[0].sampling["temperature"], 0.1);
    assert!(
        app.dark_cloud.requests().is_empty(),
        "the cloud model wasn't asked"
    );
}

#[tokio::test]
async fn when_the_local_model_is_off_the_cloud_one_answers_and_says_so() {
    let app = TestApp::new().await;
    let session = app.sign_in().await;
    app.dark_local.down(true);
    app.dark_cloud.replies(&[Ok("From the cloud.")]);
    let got = say(&app, &session, "hi").await;
    assert_eq!(got[0].delta.as_deref(), Some(FALLBACK));
    let done = got.last().unwrap().done.clone().unwrap();
    assert_eq!(done.text, format!("{FALLBACK}From the cloud."));

    // The mark stays in the chat, but the model is never sent it.
    app.dark_local.down(false);
    say(&app, &session, "again").await;
    let sent = &app.dark_local.requests()[0].lines;
    assert_eq!(sent[1].content, "From the cloud.");

    // Up, but failing before a word: the cloud one answers, saying that.
    app.dark_local.replies(&[Err("boom")]);
    let got = say(&app, &session, "and again").await;
    let done = got.last().unwrap().done.clone().unwrap();
    assert!(done.text.starts_with(FALLBACK_ERROR), "{}", done.text);
}

#[tokio::test]
async fn a_reply_that_fails_is_an_error_and_the_message_stays() {
    let app = TestApp::new().await;
    let session = app.sign_in().await;
    app.dark_local.down(true);
    app.dark_cloud.replies(&[Err("Workers AI 500: down")]);
    let got = say(&app, &session, "hi").await;
    assert_eq!(
        got.last().unwrap().error.as_deref(),
        Some("Workers AI 500: down")
    );
    let room = page(&app, &session).await;
    assert_eq!(room.chat.len(), 1);
    assert_eq!(room.chat[0].role.as_str(), "user");
}

#[tokio::test]
pub(crate) async fn one_reply_at_a_time_and_a_clear_empties_the_room() {
    let app = TestApp::new().await;
    let session = app.sign_in().await;
    let blank = app
        .call(
            Call::post("/api/dark")
                .token(&session)
                .json(&json!({ "text": " " })),
        )
        .await;
    assert_eq!(blank.status, 400);
    assert_eq!(blank.json(), json!({ "error": "Say something." }));
    let not_json = app
        .call(Call::post("/api/dark").token(&session).raw("not json"))
        .await;
    assert_eq!(not_json.status, 400);
    let big = format!("{{\"text\":\"{}\"}}", "x".repeat(3_000_000));
    let too_big = app
        .call(Call::post("/api/dark").token(&session).raw(&big))
        .await;
    assert_eq!(too_big.status, 413);

    app.dark_local.hold(true);
    let response = app
        .send(
            Call::post("/api/dark")
                .token(&session)
                .json(&json!({ "text": "think hard" }))
                .request(),
        )
        .await;
    assert_eq!(response.status(), 200);
    tokio::time::timeout(Duration::from_secs(5), app.dark_local.speaking.notified())
        .await
        .expect("the reply starts");
    assert!(page(&app, &session).await.busy);
    let second = app
        .call(
            Call::post("/api/dark")
                .token(&session)
                .json(&json!({ "text": "and?" })),
        )
        .await;
    assert_eq!(second.status, 409);
    assert_eq!(
        second.json(),
        json!({ "error": "A reply is still being written." })
    );
    let clear = app.call(Call::delete("/api/dark").token(&session)).await;
    assert_eq!(clear.status, 409);

    app.dark_local.release();
    let got = events(response.into_body()).await;
    assert_eq!(
        got.last().unwrap().done.as_ref().unwrap().text,
        "Half a tho"
    );
    let clear = app.call(Call::delete("/api/dark").token(&session)).await;
    assert_eq!(clear.status, 200);
    assert_eq!(clear.json(), json!({ "ok": true }));
    assert_eq!(page(&app, &session).await.chat, Vec::new());
}

#[tokio::test]
pub(crate) async fn without_a_session_the_room_is_refused() {
    let app = TestApp::new().await;
    for call in [
        Call::get("/api/dark"),
        Call::post("/api/dark").json(&json!({ "text": "hi" })),
        Call::delete("/api/dark"),
    ] {
        assert_eq!(app.call(call).await.status, 401);
    }
    // Report mode lets a request without a session read the room, never write in it or clear
    // it.
    let report = TestApp::with_mode(emberhut_server::config::AuthMode::Report).await;
    for call in [
        Call::post("/api/dark").json(&json!({ "text": "hi" })),
        Call::delete("/api/dark"),
    ] {
        let refused = report.call(call).await;
        assert_eq!(refused.status, 401);
        assert_eq!(refused.json()["signIn"], true);
    }
}

#[tokio::test]
async fn the_room_is_kept_in_its_own_file_beside_the_database() {
    let app = TestApp::new().await;
    let session = app.sign_in().await;
    app.dark_local.replies(&[Ok("ok")]);
    say(&app, &session, "a private line").await;
    let folder = app.url.path().parent().unwrap();
    assert!(folder.join(emberhut_db::DARK_FILE).is_file());
    // Nothing of it reached the server's own database.
    let words = b"a private line";
    for file in ["emberhut.db", "emberhut.db-wal"] {
        let bytes = std::fs::read(folder.join(file)).unwrap_or_default();
        assert!(
            !bytes.windows(words.len()).any(|window| window == words),
            "{file}"
        );
    }
}
