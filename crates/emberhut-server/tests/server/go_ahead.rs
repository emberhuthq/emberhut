//! Go ahead cards for a job's final steps: the job's grant lets an ordinary step through, the
//! step's own card asks once and the owner's Go ahead clears it, a hard step asks every time,
//! and a planned job is asked once for its steps.

use std::sync::Arc;

use emberhut_core::RoomId;
use emberhut_server::services::guard::go_ahead::{GateCard, GateResult};
use emberhut_server::services::guard::{GitRemotes, GuardConfig, NoJudge};

use crate::guard_common::{guard_app, say, task};

fn card(question: &str) -> GateCard {
    GateCard {
        question: question.to_owned(),
        action: "OK the click".to_owned(),
        brief: Vec::new(),
    }
}

#[tokio::test]
async fn a_final_step_goes_through_on_the_job_s_grant_or_its_own_go_ahead() {
    let app = guard_app(
        Arc::new(NoJudge),
        Arc::new(GitRemotes),
        GuardConfig::default(),
    )
    .await;
    let guard = app.state.guard();
    let request = say(&app, RoomId::MAIN, true, "make the app record").await;
    let asked = task(&app, "the app record", Some(request)).await;
    let through = guard
        .step_gate(
            asked,
            "Create app",
            "https://example.com/apps",
            &card("Create the app?"),
            &[],
        )
        .await
        .unwrap();
    assert!(
        matches!(&through, GateResult::By(by) if by.starts_with("approved by your request")),
        "{through:?}"
    );

    let own = task(&app, "a setting", None).await;
    let held = guard
        .step_gate(
            own,
            "Save",
            "https://example.com/settings",
            &card("Save the setting?"),
            &[],
        )
        .await
        .unwrap();
    assert!(
        matches!(&held, GateResult::Text(t) if t.starts_with("Paused: \"Save\" is a final step")),
        "{held:?}"
    );
    let still = guard
        .step_gate(
            own,
            "Save",
            "https://example.com/settings",
            &card("Save the setting?"),
            &[],
        )
        .await
        .unwrap();
    assert!(
        matches!(&still, GateResult::Text(t) if t.starts_with("Still waiting")),
        "{still:?}"
    );
    app.advance_minutes(1);
    app.state.tasks().decide(own, "Go ahead").await.unwrap();
    let cleared = guard
        .step_gate(
            own,
            "Save",
            "https://example.com/settings",
            &card("Save the setting?"),
            &[],
        )
        .await
        .unwrap();
    assert_eq!(cleared, GateResult::By("their Go ahead".to_owned()));
    let next = guard
        .step_gate(
            own,
            "Save",
            "https://example.com/settings",
            &card("Save the setting?"),
            &[],
        )
        .await
        .unwrap();
    assert!(
        matches!(&next, GateResult::By(by) if by.starts_with("your Go ahead at ")),
        "their Go ahead is the job's grant from then on: {next:?}"
    );

    let pay = guard
        .step_gate(
            asked,
            "Pay now",
            "https://example.com/checkout",
            &card("Pay $5?"),
            &[],
        )
        .await
        .unwrap();
    assert!(
        matches!(&pay, GateResult::Text(t) if t.contains("money or account security")),
        "a hard step asks even on a granted job: {pay:?}"
    );
}

#[tokio::test]
async fn a_planned_job_is_asked_once_for_its_final_steps() {
    let app = guard_app(
        Arc::new(NoJudge),
        Arc::new(GitRemotes),
        GuardConfig::default(),
    )
    .await;
    let guard = app.state.guard();
    let own = task(&app, "the listing", None).await;
    let steps = [
        "Create listing".to_owned(),
        "Pay fee".to_owned(),
        "Post".to_owned(),
    ];
    let said = guard
        .plan_job(own, "publish the listing", &steps)
        .await
        .unwrap();
    assert!(
        said.contains("Asked the owner once for the whole job"),
        "{said}"
    );
    assert!(said.contains("\"Pay fee\" still asks"), "{said}");
    let detail = app.state.tasks().detail(own).await.unwrap().unwrap();
    assert!(
        detail
            .task
            .decision
            .unwrap()
            .question
            .contains("OK the whole job?")
    );
    app.state.tasks().decide(own, "Go ahead").await.unwrap();
    let through = guard
        .step_gate(
            own,
            "Create",
            "https://example.com",
            &card("Create it?"),
            &[],
        )
        .await
        .unwrap();
    assert!(matches!(through, GateResult::By(_)), "{through:?}");
}
