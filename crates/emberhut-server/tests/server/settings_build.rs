//! `POST /api/settings` with `{ build }`: the release step reports a build a store has ready,
//! and the state's `builds` carries the newest per platform.

use emberhut_api::builds::LatestBuilds;
use emberhut_api::error::ErrorBody;
use emberhut_api::paths;
use emberhut_api::settings::SettingsReply;
use emberhut_api::state::State;
use serde_json::{Value, json};

use crate::common::{Call, TestApp};

/// What the server says to a build that is not one.
const NOT_A_BUILD: &str = "build: { platform: \"ios\" or \"mac\", number: a whole build number }";

async fn report(app: &TestApp, session: &emberhut_core::session::Session, body: &Value) -> Value {
    let answer = app
        .call(Call::post(paths::SETTINGS).token(session).json(body))
        .await;
    assert_eq!(answer.status, 200, "{}", answer.text);
    let reply: SettingsReply = answer.read_as();
    assert!(reply.ok);
    answer.json()
}

#[tokio::test]
pub(crate) async fn a_reported_build_is_in_the_state_and_only_moves_up() {
    let app = TestApp::new().await;
    let session = app.sign_in().await;
    let before: State = app
        .call(Call::get(paths::STATE).token(&session))
        .await
        .read_as();
    assert_eq!(before.builds, Some(LatestBuilds::default()));

    let reply = report(
        &app,
        &session,
        &json!({ "build": { "platform": "ios", "number": 12 } }),
    )
    .await;
    assert_eq!(reply["builds"], json!({ "ios": 12, "mac": null }));
    assert_eq!(reply["room"], 1, "the state of Main is spread in");

    report(
        &app,
        &session,
        &json!({ "build": { "platform": "ios", "number": 9 } }),
    )
    .await;
    let reply = report(
        &app,
        &session,
        &json!({ "build": { "platform": "mac", "number": "4" } }),
    )
    .await;
    assert_eq!(reply["builds"], json!({ "ios": 12, "mac": 4 }));

    let after: State = app
        .call(Call::get(paths::STATE).token(&session))
        .await
        .read_as();
    assert_eq!(
        after.builds,
        Some(LatestBuilds {
            ios: Some(12),
            mac: Some(4)
        })
    );
}

#[tokio::test]
pub(crate) async fn a_build_that_is_not_one_is_400_and_changes_nothing() {
    let app = TestApp::new().await;
    let session = app.sign_in().await;
    for build in [
        json!({ "platform": "android", "number": 3 }),
        json!({ "platform": "ios", "number": 0 }),
        json!({ "platform": "ios", "number": 2.5 }),
        json!({ "platform": "ios" }),
        json!({}),
    ] {
        let refused = app
            .call(
                Call::post(paths::SETTINGS)
                    .token(&session)
                    .json(&json!({ "build": build })),
            )
            .await;
        assert_eq!(refused.status, 400, "{build}");
        assert_eq!(
            refused.json(),
            json!({ "ok": false, "error": NOT_A_BUILD }),
            "{build}"
        );
    }
    let not_json = app
        .call(Call::post(paths::SETTINGS).token(&session).raw("{build"))
        .await;
    assert_eq!(not_json.status, 400);

    // A `build` that is not an object is not a report.
    let ignored = report(&app, &session, &json!({ "build": 12, "room": 99 })).await;
    assert_eq!(ignored["builds"], json!({ "ios": null, "mac": null }));
    assert_eq!(ignored["room"], 1, "an unknown room is Main");
}

#[tokio::test]
pub(crate) async fn reporting_a_build_needs_a_session_and_a_small_body() {
    let app = TestApp::new().await;
    let refused = app
        .call(
            Call::post(paths::SETTINGS)
                .json(&json!({ "build": { "platform": "mac", "number": 2 } })),
        )
        .await;
    assert_eq!(refused.status, 401);
    assert_eq!(refused.json()["signIn"], true);

    let session = app.sign_in().await;
    let huge =
        json!({ "build": { "platform": "mac", "number": 2 }, "pad": "x".repeat(3 * 1024 * 1024) });
    let too_big = app
        .call(
            Call::post(paths::SETTINGS)
                .token(&session)
                .raw(&huge.to_string()),
        )
        .await;
    assert_eq!(too_big.status, 413);
    assert_eq!(
        too_big.read_as::<ErrorBody>(),
        ErrorBody::new("The request is too large.")
    );
    let state: State = app
        .call(Call::get(paths::STATE).token(&session))
        .await
        .read_as();
    assert_eq!(state.builds, Some(LatestBuilds::default()));
}
