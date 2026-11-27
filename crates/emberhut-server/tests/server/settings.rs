//! `POST /api/settings`: its actions (`ui` so far), its answer with the state, and its gate.

use emberhut_api::error::ErrorBody;
use emberhut_api::paths;
use emberhut_api::settings::SettingsReply;
use emberhut_api::state::{DisplayUi, State};
use emberhut_db::Database;
use serde_json::json;

use crate::common::{Call, TestApp};

#[tokio::test]
pub(crate) async fn ui_changes_what_the_app_hides_and_answers_with_the_state() {
    let app = TestApp::new().await;
    let session = app.sign_in().await;
    let before: State = app
        .call(Call::get(paths::STATE).token(&session))
        .await
        .read_as();
    assert_eq!(
        before.ui,
        Some(DisplayUi {
            hide_goals: true,
            hide_projects: true
        }),
        "goals and projects start hidden"
    );

    let answer = app
        .call(
            Call::post("/api/settings")
                .token(&session)
                .json(&json!({ "ui": { "hideGoals": false } })),
        )
        .await;
    assert_eq!(answer.status, 200);
    let reply: SettingsReply = answer.read_as();
    assert!(reply.ok);
    assert!(reply.state.room.is_main());
    let shown_goals = DisplayUi {
        hide_goals: false,
        hide_projects: true,
    };
    assert_eq!(reply.state.ui, Some(shown_goals), "the other switch stays");

    // A switch that isn't true or false, and a `ui` that isn't an object, change nothing.
    for body in [
        json!({ "ui": { "hideGoals": "yes", "hideProjects": 1 } }),
        json!({ "ui": true }),
        json!({ "ui": null }),
    ] {
        let reply: SettingsReply = app
            .call(Call::post(paths::SETTINGS).token(&session).json(&body))
            .await
            .read_as();
        assert_eq!(reply.state.ui, Some(shown_goals), "{body}");
    }

    let reply: SettingsReply = app
        .call(
            Call::post(paths::SETTINGS)
                .token(&session)
                .json(&json!({ "ui": { "hideProjects": false, "hideGoals": true } })),
        )
        .await
        .read_as();
    let after = DisplayUi {
        hide_goals: true,
        hide_projects: false,
    };
    assert_eq!(reply.state.ui, Some(after));
    let state: State = app
        .call(Call::get(paths::STATE).token(&session))
        .await
        .read_as();
    assert_eq!(state.ui, Some(after), "kept: the state says so from now on");

    let db = Database::connect(&app.url).await.unwrap();
    let log: Vec<String> = db
        .log()
        .recent(10)
        .await
        .unwrap()
        .into_iter()
        .map(|line| line.text)
        .collect();
    assert!(
        log.iter()
            .any(|line| line == "The app now hides goals and shows projects"),
        "{log:?}"
    );
}

#[tokio::test]
pub(crate) async fn fields_no_action_takes_are_left_alone_and_the_room_is_read_leniently() {
    let app = TestApp::new().await;
    let session = app.sign_in().await;
    for body in [
        json!({}),
        json!({ "notAnAction": 1 }),
        json!({ "room": 99 }),
        json!({ "room": "1" }),
        json!({ "room": -2 }),
        json!([1, 2, 3]),
    ] {
        let answer = app
            .call(Call::post(paths::SETTINGS).token(&session).json(&body))
            .await;
        assert_eq!(answer.status, 200, "{body}");
        let reply: SettingsReply = answer.read_as();
        assert!(reply.ok);
        assert!(reply.state.room.is_main(), "{body}: no such room is Main");
    }
    // An empty body reads as `{}`.
    let empty = app.call(Call::post(paths::SETTINGS).token(&session)).await;
    assert_eq!(empty.status, 200);
}

#[tokio::test]
pub(crate) async fn a_settings_body_that_is_not_json_is_400_and_one_too_large_413() {
    let app = TestApp::new().await;
    let session = app.sign_in().await;
    let refused = app
        .call(Call::post(paths::SETTINGS).token(&session).raw("{ui:"))
        .await;
    assert_eq!(refused.status, 400);
    assert!(
        refused
            .read_as::<ErrorBody>()
            .error
            .starts_with("Not a request this route takes")
    );

    let huge = json!({ "ui": { "x": "x".repeat(3 * 1024 * 1024) } }).to_string();
    let too_large = app
        .call(Call::post(paths::SETTINGS).token(&session).raw(&huge))
        .await;
    assert_eq!(too_large.status, 413);
}

#[tokio::test]
pub(crate) async fn settings_need_a_session() {
    let app = TestApp::new().await;
    let refused = app
        .call(Call::post(paths::SETTINGS).json(&json!({ "ui": { "hideGoals": false } })))
        .await;
    assert_eq!(refused.status, 401);
    assert_eq!(
        refused.read_as::<ErrorBody>(),
        ErrorBody::sign_in_required()
    );
    let session = app.sign_in().await;
    let state: State = app
        .call(Call::get(paths::STATE).token(&session))
        .await
        .read_as();
    assert_eq!(
        state.ui.map(|ui| ui.hide_goals),
        Some(true),
        "nothing changed"
    );
}

#[tokio::test]
async fn auth_revoke_signs_a_device_out_at_once() {
    let app = TestApp::new().await;
    let kept = app.sign_in().await;
    let gone = app
        .state
        .auth()
        .open_session(&crate::common::device("phone-2"))
        .await
        .unwrap();
    let devices = |reply: &SettingsReply| -> Vec<i64> {
        let auth = reply.state.auth.as_ref().unwrap();
        auth.devices.iter().map(|d| d.id.get()).collect()
    };

    // Not a number: nothing is asked for, so both stay in.
    let ignored: SettingsReply = app
        .call(
            Call::post(paths::SETTINGS)
                .token(&kept)
                .json(&json!({ "authRevoke": gone.device.get().to_string() })),
        )
        .await
        .read_as();
    assert_eq!(devices(&ignored).len(), 2);

    let answer = app
        .call(
            Call::post(paths::SETTINGS)
                .token(&kept)
                .json(&json!({ "authRevoke": gone.device.get() })),
        )
        .await;
    assert_eq!(answer.status, 200, "{}", answer.text);
    let reply: SettingsReply = answer.read_as();
    assert!(reply.ok);
    assert_eq!(devices(&reply), [kept.device.get()]);
    let refused = app.call(Call::get(paths::STATE).token(&gone)).await;
    assert_eq!(refused.status, 401, "its tokens stop working at once");

    // An id no device has changes nothing.
    let unknown = app
        .call(
            Call::post(paths::SETTINGS)
                .token(&kept)
                .json(&json!({ "authRevoke": 9999 })),
        )
        .await;
    assert_eq!(unknown.status, 200);
}

#[tokio::test]
async fn auth_revoke_needs_a_signed_in_device_even_in_report_mode() {
    let app = TestApp::with_mode(emberhut_server::config::AuthMode::Report).await;
    let phone = app.sign_in().await;

    // Report mode lets a request without a session through, but not to sign a device out.
    let refused = app
        .call(Call::post(paths::SETTINGS).json(&json!({ "authRevoke": phone.device.get() })))
        .await;
    assert_eq!(refused.status, 401, "{}", refused.text);
    assert_eq!(
        refused.read_as::<ErrorBody>(),
        ErrorBody::sign_in_required()
    );
    let still = app.call(Call::get(paths::STATE).token(&phone)).await;
    assert_eq!(still.status, 200, "the phone is still signed in");

    // Other actions still run without a session in report mode.
    let ui = app
        .call(Call::post(paths::SETTINGS).json(&json!({ "ui": { "hideGoals": false } })))
        .await;
    assert_eq!(ui.status, 200, "{}", ui.text);
}
