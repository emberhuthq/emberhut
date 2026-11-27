//! What the server writes is the contract's shape: every answer reads as emberhut-api's type
//! (checked in each route's tests) and writes no key the contract's fixtures don't have.

use emberhut_core::RoomId;
use serde_json::json;

use crate::common::{Call, TestApp, assert_keys_within, fixture};

#[tokio::test]
async fn sign_in_answers_have_the_fixtures_shape() {
    let app = TestApp::new().await;
    let session = app.sign_in().await;
    let code = app
        .call(Call::post("/api/auth/pair/new").token(&session))
        .await
        .json();
    assert_keys_within(&code, &fixture("pair_code.json"), "pair/new");

    let body = json!({
        "code": code["code"],
        "install": "phone-1",
        "name": "Room 2 phone",
        "model": "Example 2",
    });
    let paired = app
        .call(Call::post("/api/auth/pair").json(&body))
        .await
        .json();
    assert_keys_within(&paired, &fixture("auth_accepted.json"), "pair");

    let refused = app
        .call(Call::post("/api/auth/pair").json(&body))
        .await
        .json();
    assert_keys_within(&refused, &fixture("auth_refused.json"), "pair refused");

    let renewed = app
        .call(Call::post("/api/auth/refresh").json(&json!({ "refresh": paired["refresh"] })))
        .await
        .json();
    assert_keys_within(&renewed, &fixture("auth_accepted.json"), "refresh");
}

#[tokio::test]
async fn the_state_has_the_fixture_s_shape() {
    let app = TestApp::new().await;
    let session = app.sign_in().await;
    app.assistant.answers(&[], Some("Hello back."));
    let first = app
        .state
        .talk()
        .talk(RoomId::MAIN, "Hello".to_owned(), None)
        .await
        .unwrap();
    app.state
        .talk()
        .talk(RoomId::MAIN, "And this".to_owned(), Some(first.id))
        .await
        .unwrap();
    let state = app
        .call(Call::get("/api/state").token(&session))
        .await
        .json();
    assert_keys_within(&state, &fixture("state.json"), "state");
}

#[tokio::test]
async fn small_answers_have_the_fixtures_shape() {
    let app = TestApp::new().await;
    let session = app.sign_in().await;
    let talked = app
        .call(
            Call::post("/api/talk")
                .token(&session)
                .json(&json!({ "text": "Hello" })),
        )
        .await
        .json();
    assert_eq!(talked, fixture("talk_reply.json"));
    let seen = app
        .call(Call::post("/api/seen").token(&session).json(&json!({})))
        .await
        .json();
    assert_eq!(seen, fixture("ok.json"));
    let sign_in = app.call(Call::get("/api/state")).await.json();
    assert_keys_within(&sign_in, &fixture("error_sign_in.json"), "sign in");
}
