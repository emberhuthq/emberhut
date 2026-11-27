//! Outward steps of a lead: held until the owner's words cover them (their request naming the
//! step, their OK relayed or tapped), or let through in a repository the installation always
//! lets through; the OK ends with the job.

use std::sync::Arc;

use emberhut_core::RoomId;
use emberhut_core::task::{Status, TaskChange, Who};
use emberhut_server::services::guard::{GuardConfig, NoJudge};

use crate::guard_common::{FakeRemotes, guard_app, say, task};

#[tokio::test]
async fn the_owners_request_naming_it_covers_a_merge_and_their_ok_ends_with_the_job() {
    let remotes = FakeRemotes(vec![("/srv/app".to_owned(), "owner/app".to_owned())]);
    let config = GuardConfig {
        always_merge: vec!["owner/app".to_owned()],
        ..GuardConfig::default()
    };
    let app = guard_app(Arc::new(NoJudge), Arc::new(remotes), config).await;
    let guard = app.state.guard();

    let named = say(
        &app,
        RoomId::MAIN,
        true,
        "fix the importer, merge it and deploy",
    )
    .await;
    let fix = task(&app, "the importer fix", Some(named)).await;
    let covered = guard.covered_by(fix).await.unwrap().unwrap();
    assert!(
        covered.starts_with(&format!("their request for {} at ", fix.key())),
        "{covered}"
    );
    assert!(covered.ends_with("(\"merge\")"), "{covered}");
    assert_eq!(
        guard.hold_outward(fix, "git push", "/").await.unwrap(),
        None
    );

    let plain = say(&app, RoomId::MAIN, true, "look into the slow page").await;
    let study = task(&app, "the slow page", Some(plain)).await;
    assert_eq!(guard.covered_by(study).await.unwrap(), None);
    let held = guard
        .hold_outward(study, "cd /tmp && git push", "/")
        .await
        .unwrap();
    assert!(held.is_some_and(|w| w.starts_with("Outward actions")));
    assert_eq!(
        guard
            .hold_outward(study, "cd /srv/app && git push", "/")
            .await
            .unwrap(),
        None,
        "a repository always let through"
    );
    assert_eq!(
        guard.hold_outward(study, "git status", "/").await.unwrap(),
        None
    );

    guard
        .outward_ok(study, "their OK relayed at 10:00")
        .await
        .unwrap();
    assert_eq!(
        guard.covered_by(study).await.unwrap().as_deref(),
        Some("their OK relayed at 10:00")
    );
    app.state
        .tasks()
        .update(study, TaskChange::status(Status::Done), Who::Worker, None)
        .await
        .unwrap();
    guard.outward_ended(study).await.unwrap();
    assert_eq!(
        guard.covered_by(study).await.unwrap(),
        None,
        "a reopened job asks again"
    );
}
