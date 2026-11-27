//! The assistant's turns: blank messages, a failing or panicking assistant, the per-room
//! bound, and shutdown.

use std::time::Duration;

use emberhut_api::error::ErrorBody;
use emberhut_api::state::State;
use emberhut_core::RoomId;
use emberhut_server::services::talk::{MAX_WAITING, TalkError};
use serde_json::json;

use crate::common::{Call, TestApp};

#[tokio::test]
async fn files_the_server_did_not_store_are_dropped_and_a_blank_message_is_refused() {
    let app = TestApp::new().await;
    let session = app.sign_in().await;
    let file = json!({ "name": "notes.txt", "url": "/uploads/notes.txt", "kind": "file",
        "path": "/data/uploads/notes.txt" });
    for body in [
        json!({ "text": "", "files": [file] }),
        json!({ "text": "  \n ", "files": [file] }),
    ] {
        let refused = app
            .call(Call::post("/api/talk").token(&session).json(&body))
            .await;
        assert_eq!(refused.status, 400, "{body}");
        assert_eq!(
            refused.read_as::<ErrorBody>(),
            ErrorBody::new("Say something.")
        );
    }
    let blank = app
        .state
        .talk()
        .talk(RoomId::MAIN, " ".to_owned(), None)
        .await;
    assert!(matches!(blank, Err(TalkError::Blank)), "{blank:?}");
    let state: State = app
        .call(Call::get("/api/state").token(&session))
        .await
        .read_as();
    assert_eq!(state.chat, []);
    assert!(app.assistant.turns().is_empty(), "no turn on nothing");

    let said = app
        .call(
            Call::post("/api/talk")
                .token(&session)
                .json(&json!({ "text": "See the notes", "files": [file] })),
        )
        .await;
    assert_eq!(said.status, 200);
    app.settle().await;
    let state: State = app
        .call(Call::get("/api/state").token(&session))
        .await
        .read_as();
    assert_eq!(state.chat.len(), 1);
    assert!(
        state.chat[0].files.is_empty(),
        "a file not stored here is dropped"
    );
    assert_eq!(
        app.assistant.turns()[0].files,
        [] as [emberhut_server::services::uploads::Attachment; 0]
    );
}

#[tokio::test]
async fn a_panicking_assistant_ends_its_turn_with_a_note() {
    let app = TestApp::new().await;
    let session = app.sign_in().await;
    app.assistant.answers(&["About to fail"], None);
    app.assistant.panics();
    app.state
        .talk()
        .talk(RoomId::MAIN, "Hello".to_owned(), None)
        .await
        .unwrap();
    app.settle().await;

    let state: State = app
        .call(Call::get("/api/state").token(&session))
        .await
        .read_as();
    assert!(
        !state.busy && state.live.is_none(),
        "the room is not left busy"
    );
    assert!(!state.rooms[0].busy);
    let note = state.chat.last().unwrap();
    assert!(note.event);
    assert_eq!(
        note.text,
        "Something went wrong while answering. Try again."
    );

    // The room takes turns again.
    app.assistant.answers(&[], Some("Back."));
    app.state
        .talk()
        .talk(RoomId::MAIN, "Again".to_owned(), None)
        .await
        .unwrap();
    app.settle().await;
    let state = app.state.state().state(RoomId::MAIN).await.unwrap();
    assert_eq!(state.chat.last().unwrap().text, "Back.");
}

#[tokio::test]
pub(crate) async fn a_room_takes_eight_turns_at_once_and_refuses_the_ninth() {
    let app = TestApp::new().await;
    let session = app.sign_in().await;
    app.assistant.answers(&["Thinking"], None);
    app.assistant.hold();
    for n in 0..MAX_WAITING {
        let said = app
            .call(
                Call::post("/api/talk")
                    .token(&session)
                    .json(&json!({ "text": format!("message {n}") })),
            )
            .await;
        assert_eq!(said.status, 200, "message {n}");
    }
    let refused = app
        .call(
            Call::post("/api/talk")
                .token(&session)
                .json(&json!({ "text": "one too many" })),
        )
        .await;
    assert_eq!(refused.status, 429);
    assert_eq!(
        refused.read_as::<ErrorBody>(),
        ErrorBody::new(
            "Too many messages are waiting for an answer in this room. Try again in a moment."
        )
    );
    // The first turn is held; let it go, and the rest follow.
    app.assistant.release();
    app.settle().await;
    assert_eq!(app.assistant.turns().len(), MAX_WAITING);
}

#[tokio::test]
pub(crate) async fn shutdown_lets_the_running_turn_finish_and_drops_the_queued_ones() {
    let app = TestApp::new().await;
    let talk = app.state.talk();
    app.assistant.answers(&["Thinking"], Some("Finished."));
    app.assistant.hold();
    for text in ["first", "second", "third"] {
        talk.talk(RoomId::MAIN, text.to_owned(), None)
            .await
            .unwrap();
    }
    // The first turn is running (held); the other two wait for it.
    while app.assistant.turns().is_empty() {
        tokio::task::yield_now().await;
    }
    app.state.stopping().cancel();
    assert!(
        app.assistant.turns()[0].stopping.is_cancelled(),
        "the running turn is told the server is stopping"
    );
    let refused = talk.talk(RoomId::MAIN, "late".to_owned(), None).await;
    assert!(
        matches!(refused, Err(TalkError::ShuttingDown)),
        "{refused:?}"
    );
    let session = app.sign_in().await;
    let over_http = app
        .call(
            Call::post("/api/talk")
                .token(&session)
                .json(&json!({ "text": "late" })),
        )
        .await;
    assert_eq!(over_http.status, 503);
    assert_eq!(
        over_http.read_as::<ErrorBody>(),
        ErrorBody::new("The server is restarting. Try again in a moment.")
    );

    app.assistant.release();
    assert!(
        talk.finish(Duration::from_secs(5)).await,
        "the running turn ended"
    );
    assert_eq!(app.assistant.turns().len(), 1, "the queued turns never ran");
    let chat = app.state.state().state(RoomId::MAIN).await.unwrap().chat;
    let texts: Vec<&str> = chat.iter().map(|m| m.text.as_str()).collect();
    assert_eq!(texts, ["first", "second", "third", "Finished."]);
}

#[tokio::test]
async fn shutdown_gives_up_on_a_turn_that_outlasts_the_grace() {
    let app = TestApp::new().await;
    app.assistant.answers(&["Thinking"], None);
    app.assistant.hold();
    app.state
        .talk()
        .talk(RoomId::MAIN, "hello".to_owned(), None)
        .await
        .unwrap();
    while app.assistant.turns().is_empty() {
        tokio::task::yield_now().await;
    }
    app.state.stopping().cancel();
    assert!(
        !app.state.talk().finish(Duration::from_millis(50)).await,
        "a held turn is still running when the grace runs out"
    );
    app.assistant.release();
}
