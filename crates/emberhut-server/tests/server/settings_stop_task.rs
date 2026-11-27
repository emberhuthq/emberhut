//! `stopTask`: stopping one of the assistant's own background tasks from the app.

use emberhut_api::error::ErrorBody;
use emberhut_server::config::AuthMode;
use serde_json::json;

use crate::common::{Call, TestApp};

#[tokio::test]
pub(crate) async fn stop_task_stops_the_assistants_background_task() {
    let app = TestApp::new().await;
    app.workers.background("bg-7");
    let session = app.sign_in().await;
    for body in [
        json!({ "stopTask": "bg-7" }),
        json!({ "stopTask": "gone" }),
        json!({ "stopTask": 7 }),
    ] {
        let answer = app
            .call(Call::post("/api/settings").token(&session).json(&body))
            .await;
        assert_eq!(answer.status, 200, "{}", answer.text);
        assert_eq!(
            answer.json()["ok"],
            true,
            "an ended or unknown task changes nothing"
        );
    }
    assert_eq!(app.workers.background_stopped(), ["bg-7"]);
}

#[tokio::test]
async fn stop_task_needs_a_signed_in_device_even_in_report_mode() {
    let app = TestApp::with_mode(AuthMode::Report).await;
    app.workers.background("bg-7");
    let refused = app
        .call(Call::post("/api/settings").json(&json!({ "stopTask": "bg-7" })))
        .await;
    assert_eq!(refused.status, 401, "{}", refused.text);
    assert_eq!(
        refused.read_as::<ErrorBody>(),
        ErrorBody::sign_in_required()
    );
    assert_eq!(app.workers.background_stopped(), Vec::<String>::new());

    let session = app.sign_in().await;
    let stopped = app
        .call(
            Call::post("/api/settings")
                .token(&session)
                .json(&json!({ "stopTask": "bg-7" })),
        )
        .await;
    assert_eq!(stopped.status, 200, "{}", stopped.text);
    assert_eq!(app.workers.background_stopped(), ["bg-7"]);
}
