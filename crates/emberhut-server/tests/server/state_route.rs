//! `GET /api/state`: the state, its `ETag` and 304, and the session gate.

use emberhut_api::error::ErrorBody;
use emberhut_api::headers;
use emberhut_api::state::State;
use emberhut_core::RoomId;
use emberhut_server::config::AuthMode;
use serde_json::json;

use crate::common::{Call, TestApp};

#[tokio::test]
pub(crate) async fn the_state_is_main_s_with_an_etag_and_unchanged_is_304() {
    let app = TestApp::new().await;
    let session = app.sign_in().await;
    let first = app.call(Call::get("/api/state").token(&session)).await;
    assert_eq!(first.status, 200);
    let state: State = first.read_as();
    assert!(state.room.is_main());
    assert_eq!(state.rooms.len(), 1);
    assert_eq!(state.rooms[0].name, "Main");
    assert_eq!(state.chat, []);
    assert!(!state.earlier && !state.busy && !state.unread);
    let auth = state.auth.unwrap();
    assert_eq!(auth.devices.len(), 1);
    assert_eq!(auth.devices[0].id.get(), session.device.get());

    let tag = first.header("etag").to_owned();
    assert!(tag.starts_with("W/\""), "{tag}");
    let unchanged = app
        .call(
            Call::get("/api/state")
                .token(&session)
                .header("if-none-match", &tag),
        )
        .await;
    assert_eq!(unchanged.status, 304);
    assert_eq!(unchanged.text, "");
    assert_eq!(unchanged.header("etag"), tag);

    app.state
        .talk()
        .talk(RoomId::MAIN, "hello".to_owned(), None)
        .await
        .unwrap();
    let changed = app
        .call(
            Call::get("/api/state")
                .token(&session)
                .header("if-none-match", &tag),
        )
        .await;
    assert_eq!(changed.status, 200);
    assert_ne!(changed.header("etag"), tag);
}

#[tokio::test]
pub(crate) async fn without_a_session_enforce_mode_says_sign_in() {
    let app = TestApp::new().await;
    for call in [
        Call::get("/api/state"),
        Call::get("/api/state").header("authorization", "Bearer expired-or-wrong"),
    ] {
        let refused = app.call(call).await;
        assert_eq!(refused.status, 401);
        assert_eq!(
            refused.read_as::<ErrorBody>(),
            ErrorBody::sign_in_required()
        );
    }
}

#[tokio::test]
async fn report_mode_lets_a_request_without_a_session_through() {
    let app = TestApp::with_mode(AuthMode::Report).await;
    let answer = app.call(Call::get("/api/state")).await;
    assert_eq!(answer.status, 200);
    let state: State = answer.read_as();
    assert_eq!(
        state.auth.unwrap().mode,
        emberhut_api::auth::AuthMode::Report
    );
}

#[tokio::test]
async fn a_media_player_may_send_the_token_in_the_query() {
    let app = TestApp::new().await;
    let session = app.sign_in().await;
    let query = format!("token={}", session.tokens.access.as_str());
    let answer = app.call(Call::get("/api/state").query(&query)).await;
    assert_eq!(answer.status, 200);
}

#[tokio::test]
async fn an_unknown_or_odd_room_is_main() {
    let app = TestApp::new().await;
    let session = app.sign_in().await;
    for room in ["room=99", "room=0", "room=main", "room=-1"] {
        let answer = app
            .call(Call::get("/api/state").query(room).token(&session))
            .await;
        assert_eq!(answer.status, 200, "{room}");
        assert!(answer.read_as::<State>().room.is_main(), "{room}");
    }
}

#[tokio::test]
async fn the_looking_header_marks_the_room_on_screen_and_changes_nothing_else() {
    let app = TestApp::new().await;
    let session = app.sign_in().await;
    let plain = app.call(Call::get("/api/state").token(&session)).await;
    let looking = app
        .call(
            Call::get("/api/state")
                .token(&session)
                .header(headers::LOOKING, "1"),
        )
        .await;
    assert_eq!(looking.status, 200);
    assert_eq!(looking.header("etag"), plain.header("etag"));
    assert!(app.state.presence().is_looked_at(RoomId::MAIN));
}

#[tokio::test]
async fn the_chat_shows_messages_oldest_first_with_replies_quoted() {
    let app = TestApp::new().await;
    let session = app.sign_in().await;
    let first = app
        .state
        .talk()
        .talk(RoomId::MAIN, "first".to_owned(), None)
        .await
        .unwrap();
    app.state
        .talk()
        .talk(RoomId::MAIN, "second".to_owned(), Some(first.id))
        .await
        .unwrap();
    let state: State = app
        .call(Call::get("/api/state").token(&session))
        .await
        .read_as();
    let texts: Vec<&str> = state.chat.iter().map(|m| m.text.as_str()).collect();
    assert_eq!(texts, ["first", "second"]);
    assert!(state.chat.iter().all(|m| m.mine && m.time == "now"));
    let quote = state.chat[1].reply.as_ref().unwrap();
    assert_eq!(quote.id.get(), first.id.get());
    assert_eq!(quote.text, "first");
    assert_eq!(
        serde_json::to_value(&state.chat[0]).unwrap()["files"],
        json!([])
    );
}

#[tokio::test]
async fn a_big_state_is_gzipped_for_a_client_that_takes_it() {
    let app = TestApp::new().await;
    let session = app.sign_in().await;
    let long = "A sentence long enough to fill the page quickly. ".repeat(8);
    for _ in 0..60 {
        app.state
            .talk()
            .talk(RoomId::MAIN, long.clone(), None)
            .await
            .unwrap();
        // One at a time: a room holds at most eight turns.
        app.settle().await;
    }
    app.settle().await;
    let plain = app.call(Call::get("/api/state").token(&session)).await;
    assert!(plain.text.len() > 16 * 1024);
    assert!(plain.headers.get("content-encoding").is_none());

    let request = Call::get("/api/state")
        .token(&session)
        .header("accept-encoding", "gzip")
        .request();
    let zipped = app.send(request).await;
    assert_eq!(zipped.status(), 200);
    assert_eq!(zipped.headers()["content-encoding"], "gzip");
    assert_eq!(
        zipped.headers()["etag"],
        plain.header("etag"),
        "the same weak tag"
    );
}
