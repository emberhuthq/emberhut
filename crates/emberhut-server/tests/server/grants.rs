//! One job, one OK: the owner's request is the job's grant, their tap on a card grants it (or
//! their Stop takes it back), an exact text's OK is used once in any channel, and a standing
//! OK lets a 1:1 chat be messaged without asking.

use std::sync::Arc;

use emberhut_core::RoomId;
use emberhut_core::task::Who;
use emberhut_core::task::ask::OptionIn;
use emberhut_server::services::guard::{GitRemotes, GuardConfig, NoJudge};
use emberhut_server::services::tasks::AskInput;
use serde_json::json;

use crate::guard_common::{guard_app, say, task};

fn ask(question: &str, draft: Option<&str>) -> AskInput {
    AskInput {
        question: question.to_owned(),
        options: vec![
            OptionIn::Plain("Go ahead".to_owned()),
            OptionIn::Plain("Stop".to_owned()),
        ],
        action: Some("OK it".to_owned()),
        draft: draft.map(str::to_owned),
        ..AskInput::default()
    }
}

#[tokio::test]
async fn the_owners_request_and_their_tap_grant_the_job_and_their_stop_takes_it_back() {
    let app = guard_app(
        Arc::new(NoJudge),
        Arc::new(GitRemotes),
        GuardConfig::default(),
    )
    .await;
    let guard = app.state.guard();
    let request = say(
        &app,
        RoomId::MAIN,
        true,
        "set up the store page, do it end to end",
    )
    .await;
    let asked_for = task(&app, "the store page", Some(request)).await;
    let grant = guard.job_grant(asked_for).await.unwrap().unwrap();
    assert!(grant.starts_with("approved by your request at "), "{grant}");

    let own = task(&app, "a chore of the assistant's", None).await;
    assert_eq!(guard.job_grant(own).await.unwrap(), None);
    app.state
        .tasks()
        .ask(own, ask("OK the job?", None), Who::Assistant)
        .await
        .unwrap();
    app.state.tasks().decide(own, "Go ahead").await.unwrap();
    let grant = guard.job_grant(own).await.unwrap().unwrap();
    assert!(grant.starts_with("your Go ahead at "), "{grant}");
    let history = app.state.tasks().history_of(own).await.unwrap();
    assert!(
        history
            .iter()
            .any(|h| h.text.starts_with("Job OK'd (your Go ahead at "))
    );

    app.advance_minutes(1);
    app.state
        .tasks()
        .ask(own, ask("And the next one?", None), Who::Assistant)
        .await
        .unwrap();
    app.state.tasks().decide(own, "Stop").await.unwrap();
    assert_eq!(
        guard.job_grant(own).await.unwrap(),
        None,
        "the Stop takes it back"
    );
}

#[tokio::test]
async fn an_exact_text_s_ok_goes_out_once_in_any_channel() {
    let dir = tempfile::tempdir().unwrap();
    let list = dir.path().join("send-without-ok.json");
    std::fs::write(&list, json!({ "Sam": ["slack:acme:D123"] }).to_string()).unwrap();
    let config = GuardConfig {
        send_without_ok: Some(list),
        ..GuardConfig::default()
    };
    let app = guard_app(Arc::new(NoJudge), Arc::new(GitRemotes), config).await;
    let guard = app.state.guard();
    let id = task(&app, "a reply to Sam", None).await;
    app.state
        .tasks()
        .ask(
            id,
            ask("Send the reply?", Some("hi Sam, it’s live")),
            Who::Assistant,
        )
        .await
        .unwrap();
    app.state.tasks().decide(id, "Go ahead").await.unwrap();
    let ok = guard
        .text_ok(id, "hi  Sam, it's live")
        .await
        .unwrap()
        .unwrap();
    assert!(ok.how.starts_with("your Go ahead at "), "{ok:?}");

    let by = guard
        .message_ok(id, "Send", &["hi Sam, it's live".to_owned()])
        .await
        .unwrap()
        .unwrap();
    assert!(
        by.0.starts_with("the exact text you OK'd (your Go ahead at "),
        "{by:?}"
    );
    assert!(guard.was_sent(id, "hi Sam, it's live").await.unwrap());
    assert_eq!(
        guard
            .message_ok(id, "Send", &["hi Sam, it's live".to_owned()])
            .await
            .unwrap(),
        None,
        "never a resend"
    );
    assert!(
        guard
            .message_ok(id, "Reply all", &[])
            .await
            .unwrap()
            .is_some(),
        "a button that only opens a box sends nothing"
    );
    assert!(
        guard
            .standing_ok(&["slack:acme:D123".to_owned()], true)
            .is_some()
    );
    assert_eq!(
        guard.standing_ok(&["slack:acme:D123".to_owned()], false),
        None,
        "a group asks"
    );
    assert_eq!(guard.standing_ok(&["wa:9".to_owned()], true), None);
}
