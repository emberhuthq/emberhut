//! `POST /api/settings`'s assistant actions: `on` (pause and resume), `interrupt`,
//! `testLimit`, `cancelCheckin` and `stopWatch`, each answered with the state.

use emberhut_api::paths;
use emberhut_api::settings::SettingsReply;
use emberhut_core::RoomId;
use serde_json::json;

use crate::common::{Call, TestApp};

async fn settings(app: &TestApp, body: serde_json::Value) -> SettingsReply {
    let session = app.sign_in().await;
    let answer = app
        .call(Call::post(paths::SETTINGS).token(&session).json(&body))
        .await;
    assert_eq!(answer.status, 200, "{}", answer.text);
    answer.read_as()
}

#[tokio::test]
async fn on_pauses_and_resumes_the_assistant() {
    let app = TestApp::new().await;
    assert!(settings(&app, json!({})).await.state.on, "on until paused");
    let paused = settings(&app, json!({ "on": false })).await;
    assert!(!paused.state.on);
    let ignored = settings(&app, json!({ "on": "yes" })).await;
    assert!(
        !ignored.state.on,
        "a value that isn't true or false changes nothing"
    );
    let resumed = settings(&app, json!({ "on": true })).await;
    assert!(resumed.state.on);
    let did: Vec<String> = resumed.state.did.into_iter().map(|d| d.text).collect();
    assert_eq!(
        did,
        [
            "The owner turned the assistant on",
            "The owner paused the assistant"
        ]
    );
}

#[tokio::test]
async fn pausing_stops_the_turn_running_and_interrupt_stops_one_room_s() {
    let app = TestApp::new().await;
    app.assistant.answers(&["Thinking"], Some("Late answer"));
    app.assistant.hold();
    app.state
        .talk()
        .talk(RoomId::MAIN, "Go".to_owned(), None)
        .await
        .unwrap();
    while app.assistant.turns().is_empty() {
        tokio::task::yield_now().await;
    }
    let turn = app.assistant.turns().remove(0);
    assert!(!turn.interrupt.is_cancelled());
    settings(&app, json!({ "on": false })).await;
    assert!(
        turn.interrupt.is_cancelled(),
        "pausing stops the running turn"
    );
    app.assistant.release();
    app.settle().await;

    settings(&app, json!({ "on": true })).await;
    app.assistant.hold();
    app.state
        .talk()
        .talk(RoomId::MAIN, "Again".to_owned(), None)
        .await
        .unwrap();
    while app.assistant.turns().len() < 2 {
        tokio::task::yield_now().await;
    }
    let turn = app.assistant.turns().remove(1);
    let reply = settings(&app, json!({ "interrupt": true, "room": 1 })).await;
    assert!(turn.interrupt.is_cancelled());
    assert_eq!(
        reply
            .state
            .chat
            .last()
            .map(|line| (line.event, line.text.as_str())),
        Some((true, "Stopped."))
    );
    app.assistant.release();
    app.settle().await;

    // `interrupt` other than true does nothing.
    let before = settings(&app, json!({})).await.state.chat.len();
    let after = settings(&app, json!({ "interrupt": false }))
        .await
        .state
        .chat
        .len();
    assert_eq!(before, after);
}

#[tokio::test]
async fn test_limit_reaches_the_assistant_of_the_body_s_room() {
    // The scripted assistant has no accounts: the action is taken and changes nothing else.
    let app = TestApp::new().await;
    let reply = settings(&app, json!({ "testLimit": true })).await;
    assert!(reply.ok);
    let other = settings(&app, json!({ "testLimit": "yes" })).await;
    assert!(other.ok);
}

#[tokio::test]
async fn cancel_checkin_and_stop_watch_are_the_app_s_cancel_buttons() {
    let app = TestApp::new().await;
    let wake = app.state.wake();
    let at = wake.wake_me_at(20, "the call", RoomId::MAIN).await.unwrap();
    wake.wake_me_at(40, "the other", RoomId::MAIN)
        .await
        .unwrap();
    let id = wake
        .add_watch(
            emberhut_server::services::wake::NewWatch {
                title: "Build".to_owned(),
                thread: None,
                what: "the build".to_owned(),
                every_minutes: 10.0,
                hours: 1.0,
                note: String::new(),
            },
            RoomId::MAIN,
        )
        .await
        .unwrap();
    let reply = settings(&app, json!({ "cancelCheckin": at.as_str() })).await;
    assert_eq!(reply.state.checkins.len(), 1);
    assert_eq!(reply.state.checkins[0].why, "the other");
    assert_eq!(reply.state.watches.len(), 1);
    let ignored = settings(&app, json!({ "stopWatch": "1" })).await;
    assert_eq!(ignored.state.watches.len(), 1, "a watch's number, not text");
    let reply = settings(&app, json!({ "stopWatch": id })).await;
    assert!(reply.state.watches.is_empty(), "none");
}
