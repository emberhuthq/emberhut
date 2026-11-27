//! `POST /api/talk` and `POST /api/seen`.

use emberhut_api::error::{ErrorBody, OkReply};
use emberhut_api::state::State;
use emberhut_api::talk::TalkReply;
use serde_json::json;

use crate::common::{Call, TestApp};

#[tokio::test]
pub(crate) async fn what_is_said_is_in_the_chat_and_the_reply_comes_after() {
    let app = TestApp::new().await;
    let session = app.sign_in().await;
    app.assistant.answers(&["Thinking"], Some("Hello back."));
    let said = app
        .call(
            Call::post("/api/talk")
                .token(&session)
                .json(&json!({ "text": "Hello", "room": 1 })),
        )
        .await;
    assert_eq!(said.status, 200);
    assert_eq!(
        said.read_as::<TalkReply>(),
        TalkReply {
            ok: true,
            started: true,
            id: None
        }
    );
    app.settle().await;

    let state: State = app
        .call(Call::get("/api/state").token(&session))
        .await
        .read_as();
    let chat: Vec<(bool, &str)> = state
        .chat
        .iter()
        .map(|m| (m.mine, m.text.as_str()))
        .collect();
    assert_eq!(chat, [(true, "Hello"), (false, "Hello back.")]);
    assert!(state.unread, "the reply is new");
    assert_eq!(state.rooms[0].last.as_deref(), Some("Hello back."));
    assert!(!state.rooms[0].last_mine);

    let turns = app.assistant.turns();
    assert_eq!(turns.len(), 1);
    assert_eq!(turns[0].said.text, "Hello");
    assert_eq!(turns[0].history.len(), 1);
}

#[tokio::test]
async fn a_failed_turn_leaves_a_note_in_the_chat() {
    let app = TestApp::new().await;
    let session = app.sign_in().await;
    app.assistant.fails();
    app.call(
        Call::post("/api/talk")
            .token(&session)
            .json(&json!({ "text": "Hello" })),
    )
    .await;
    app.settle().await;
    let state: State = app
        .call(Call::get("/api/state").token(&session))
        .await
        .read_as();
    let note = state.chat.last().unwrap();
    assert!(note.event && !note.mine);
    assert!(
        !note.text.contains("scripted"),
        "the cause stays in the log"
    );
}

#[tokio::test]
async fn a_reply_to_a_message_that_does_not_exist_is_a_plain_message() {
    let app = TestApp::new().await;
    let session = app.sign_in().await;
    let said = app
        .call(
            Call::post("/api/talk")
                .token(&session)
                .json(&json!({ "text": "Hello", "replyTo": 99 })),
        )
        .await;
    assert_eq!(said.status, 200);
    let state: State = app
        .call(Call::get("/api/state").token(&session))
        .await
        .read_as();
    assert!(state.chat[0].reply.is_none());
}

#[tokio::test]
pub(crate) async fn a_body_the_route_refuses_is_400_with_the_reason() {
    let app = TestApp::new().await;
    let session = app.sign_in().await;
    for body in [
        json!({ "text": "   " }),
        json!({ "text": "Hello", "room": 0 }),
        json!({ "text": 42 }),
    ] {
        let refused = app
            .call(Call::post("/api/talk").token(&session).json(&body))
            .await;
        assert_eq!(refused.status, 400, "{body}");
        let error: ErrorBody = refused.read_as();
        assert_ne!(error.error, "");
    }
    let not_json = app
        .call(Call::post("/api/talk").token(&session).raw("{text"))
        .await;
    assert_eq!(not_json.status, 400);
}

#[tokio::test]
pub(crate) async fn talking_needs_a_session() {
    let app = TestApp::new().await;
    let refused = app
        .call(Call::post("/api/talk").json(&json!({ "text": "Hello" })))
        .await;
    assert_eq!(refused.status, 401);
    assert_eq!(
        refused.read_as::<ErrorBody>(),
        ErrorBody::sign_in_required()
    );
    let seen = app
        .call(Call::post("/api/seen").json(&json!({ "room": 1 })))
        .await;
    assert_eq!(seen.status, 401);
}

#[tokio::test]
pub(crate) async fn seeing_a_room_clears_its_unread() {
    let app = TestApp::new().await;
    let session = app.sign_in().await;
    app.assistant.answers(&[], Some("Hello back."));
    app.call(
        Call::post("/api/talk")
            .token(&session)
            .json(&json!({ "text": "Hello" })),
    )
    .await;
    app.settle().await;
    app.advance_minutes(1);

    let seen = app
        .call(
            Call::post("/api/seen")
                .token(&session)
                .json(&json!({ "room": 1 })),
        )
        .await;
    assert_eq!(seen.status, 200);
    assert_eq!(seen.read_as::<OkReply>(), OkReply::OK);
    let state: State = app
        .call(Call::get("/api/state").token(&session))
        .await
        .read_as();
    assert!(!state.unread);
    assert_eq!(state.rooms[0].unread, 0);

    // An empty body is `{}`: Main.
    let empty = app.call(Call::post("/api/seen").token(&session)).await;
    assert_eq!(empty.status, 200);
    let unknown = app
        .call(
            Call::post("/api/seen")
                .token(&session)
                .json(&json!({ "room": 77 })),
        )
        .await;
    assert_eq!(unknown.status, 200, "an unknown room is Main");
}

#[tokio::test]
pub(crate) async fn a_body_over_the_limit_is_413() {
    let app = TestApp::new().await;
    let session = app.sign_in().await;
    let huge = json!({ "text": "x".repeat(3 * 1024 * 1024) }).to_string();
    for path in ["/api/talk", "/api/seen"] {
        let answer = app.call(Call::post(path).token(&session).raw(&huge)).await;
        assert_eq!(answer.status, 413, "{path}");
        assert_eq!(
            answer.read_as::<ErrorBody>(),
            ErrorBody::new("The request is too large.")
        );
    }
}

#[tokio::test]
pub(crate) async fn seeing_a_room_that_is_not_a_positive_number_is_400() {
    let app = TestApp::new().await;
    let session = app.sign_in().await;
    for body in [json!({ "room": 0 }), json!({ "room": "two" })] {
        let refused = app
            .call(Call::post("/api/seen").token(&session).json(&body))
            .await;
        assert_eq!(refused.status, 400, "{body}");
    }
}
