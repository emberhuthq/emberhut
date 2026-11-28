//! Outward steps of a lead: held until the owner's words cover them (their request naming the
//! step, their OK relayed or tapped), or let through in a repository the installation always
//! lets through; the OK ends with the job.

use std::sync::Arc;

use emberhut_core::RoomId;
use emberhut_core::task::{Status, TaskChange, Who};
use emberhut_server::services::guard::{GuardConfig, NoJudge};

use crate::guard_common::{FakeJudge, FakeRemotes, guard_app, say, task};

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

#[tokio::test]
async fn a_command_the_rules_cannot_read_goes_to_the_assistant_before_the_owner() {
    let judge = Arc::new(FakeJudge::default());
    *judge.answer.lock().unwrap() =
        Some(r#"{"reads_only": true, "why": "it prints a report"}"#.to_owned());
    let app = guard_app(
        judge.clone(),
        Arc::new(FakeRemotes::default()),
        GuardConfig::default(),
    )
    .await;
    let guard = app.state.guard();
    let plain = say(&app, RoomId::MAIN, true, "look into the slow page").await;
    let study = task(&app, "the slow page", Some(plain)).await;
    let prompts = || judge.prompts.lock().unwrap().clone();

    // Read-only by the rules: no one is asked.
    let reads = r#"python3 -c "import json,sys; print(json.load(sys.stdin)['n'])""#;
    assert_eq!(guard.hold_outward(study, reads, "/").await.unwrap(), None);
    assert_eq!(prompts(), Vec::<String>::new());

    // A script the rules can't read: the assistant reads it as only reading, and it runs.
    let script = "python3 tools/report.py --since 2026-11-01";
    assert_eq!(
        guard.hold_outward(study, script, "/srv/app").await.unwrap(),
        None
    );
    assert_eq!(prompts().len(), 1);
    assert!(prompts()[0].contains(script), "{}", prompts()[0]);
    assert!(prompts()[0].contains("inline code or a script"));
    // Run again, it isn't read again.
    assert_eq!(
        guard.hold_outward(study, script, "/srv/app").await.unwrap(),
        None
    );
    assert_eq!(prompts().len(), 1);

    // A push the rules know: the owner's, never the assistant's.
    let held = guard.hold_outward(study, "git push", "/").await.unwrap();
    assert!(held.is_some_and(|w| w.starts_with("Outward actions")));
    let held = guard
        .hold_outward(study, "python3 -c 'print(1)' && npm publish", "/")
        .await
        .unwrap();
    assert!(held.is_some());
    assert_eq!(prompts().len(), 1, "no judge for a known outward step");

    // The assistant says no, or doesn't answer: the owner is asked.
    *judge.answer.lock().unwrap() =
        Some(r#"{"reads_only": false, "why": "it uploads"}"#.to_owned());
    assert!(
        guard
            .hold_outward(study, "./sync.sh", "/srv/app")
            .await
            .unwrap()
            .is_some()
    );
    *judge.answer.lock().unwrap() = None;
    assert!(
        guard
            .hold_outward(study, "python3 other.py", "/")
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        guard
            .hold_outward(study, "python3 other.py", "/")
            .await
            .unwrap()
            .is_some(),
        "no answer isn't kept as a verdict"
    );
    assert_eq!(prompts().len(), 4);
}

#[tokio::test]
async fn with_no_judge_a_command_the_rules_cannot_read_asks_the_owner() {
    let app = guard_app(
        Arc::new(NoJudge),
        Arc::new(FakeRemotes::default()),
        GuardConfig::default(),
    )
    .await;
    let guard = app.state.guard();
    let plain = say(&app, RoomId::MAIN, true, "look into the slow page").await;
    let study = task(&app, "the slow page", Some(plain)).await;
    assert!(
        guard
            .hold_outward(study, "./sync.sh", "/")
            .await
            .unwrap()
            .is_some()
    );
    assert_eq!(
        guard
            .hold_outward(study, "grep -rn TODO src | head", "/")
            .await
            .unwrap(),
        None
    );
}
