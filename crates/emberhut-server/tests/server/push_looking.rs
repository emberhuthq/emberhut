//! No push for the room that is on screen: an app that polls the state with the looking header
//! (`emberhut_api::headers::LOOKING`) holds that room's replies back from the phones for 10 s.

use emberhut_api::{headers, paths};
use emberhut_core::RoomId;
use emberhut_server::config::AuthMode;
use serde_json::json;

use crate::common::{Call, TestApp};
use crate::fake_push::{FakePush, Sent};

#[tokio::test]
async fn a_reply_in_the_room_on_screen_is_not_pushed_until_it_has_been_away_10_s() {
    let (apns, _fcm, senders) = FakePush::pair();
    let app = TestApp::with_push(AuthMode::Enforce, senders).await;
    let session = app.sign_in().await;
    let registered = app
        .call(
            Call::post(paths::DEVICES)
                .token(&session)
                .json(&json!({ "token": "a".repeat(64) })),
        )
        .await;
    assert_eq!(registered.status, 200);
    app.assistant.answers(&[], Some("Here it is."));

    let looked = app
        .call(
            Call::get(paths::STATE)
                .token(&session)
                .header(headers::LOOKING, "1"),
        )
        .await;
    assert_eq!(looked.status, 200);
    let talk = || {
        Call::post(paths::TALK)
            .token(&session)
            .json(&json!({ "text": "Show me" }))
    };
    app.call(talk()).await;
    app.settle().await;
    assert_eq!(apns.sent(), Vec::<Sent>::new(), "the room is being read");

    app.clock.advance(jiff::SignedDuration::from_secs(10));
    app.call(talk()).await;
    app.settle().await;
    assert_eq!(apns.sent().len(), 1, "away for 10 s: pushed");

    // Polling without the header doesn't mark the room.
    app.call(Call::get(paths::STATE).token(&session)).await;
    app.call(talk()).await;
    app.settle().await;
    assert_eq!(apns.sent().len(), 2);
}

#[tokio::test]
async fn looking_at_one_room_holds_back_only_that_room() {
    let (apns, _fcm, senders) = FakePush::pair();
    let app = TestApp::with_push(AuthMode::Enforce, senders).await;
    let session = app.sign_in().await;
    app.call(
        Call::post(paths::DEVICES)
            .token(&session)
            .json(&json!({ "token": "a".repeat(64) })),
    )
    .await;
    app.state.presence().looking(RoomId::MAIN);
    assert_eq!(app.state.push().reply(RoomId::MAIN, "x").await.delivered, 0);
    let other = app.state.push().reply(RoomId::new(2), "x").await;
    assert_eq!(other.delivered, 1);
    let sent = apns.sent();
    assert_eq!(sent[0].1.room(), Some(RoomId::new(2)));
    // A reply with nothing to show after its marks are gone is not pushed.
    app.state.presence().looking(RoomId::new(2));
    app.clock.advance(jiff::SignedDuration::from_secs(10));
    assert_eq!(
        app.state.push().reply(RoomId::new(2), "**").await.delivered,
        0
    );
}
