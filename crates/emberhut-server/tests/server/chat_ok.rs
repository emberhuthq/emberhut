//! The owner's go in the chat on the exact text they were shown, and their yes in the chat
//! about a task: each message an OK once.

use std::sync::Arc;

use emberhut_core::RoomId;
use emberhut_server::services::guard::{GitRemotes, GuardConfig, NoJudge};

use crate::guard_common::{guard_app, say, task};

#[tokio::test]
async fn a_go_after_the_exact_text_is_the_ok_for_it_once() {
    let app = guard_app(
        Arc::new(NoJudge),
        Arc::new(GitRemotes),
        GuardConfig::default(),
    )
    .await;
    let guard = app.state.guard();
    say(
        &app,
        RoomId::MAIN,
        false,
        "The reply to Sam:\n> see you at 5\nSend it?",
    )
    .await;
    let go = say(&app, RoomId::MAIN, true, "send it").await;
    let ok = guard
        .find_chat_ok(RoomId::MAIN, "see you at 5", "slack:acme:Sam", false, None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(ok.id, go.get());
    assert!(
        guard
            .find_chat_ok(
                RoomId::MAIN,
                "see you at 5",
                "slack:acme:Robin",
                false,
                None
            )
            .await
            .unwrap()
            .is_none(),
        "never for another person"
    );
    assert!(guard.spend(ok.id).await.unwrap());
    assert!(!guard.spend(ok.id).await.unwrap(), "one message per OK");
    assert!(
        guard
            .find_chat_ok(RoomId::MAIN, "see you at 5", "slack:acme:Sam", false, None)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn a_yes_to_the_question_about_a_task_is_its_job_s_ok() {
    let app = guard_app(
        Arc::new(NoJudge),
        Arc::new(GitRemotes),
        GuardConfig::default(),
    )
    .await;
    let guard = app.state.guard();
    let id = task(&app, "the store page", None).await;
    say(
        &app,
        RoomId::MAIN,
        false,
        &format!("Shall I publish {} now?", id.key()),
    )
    .await;
    say(&app, RoomId::MAIN, true, "yes").await;
    let ok = guard
        .task_chat_ok(id, &[RoomId::MAIN])
        .await
        .unwrap()
        .unwrap();
    assert_eq!(ok.said, "yes");
    let grant = guard.job_grant(id).await.unwrap().unwrap();
    assert!(
        grant.starts_with("OK from the owner in the chat: their message #"),
        "{grant}"
    );
    say(&app, RoomId::MAIN, true, "wait").await;
    assert!(
        guard
            .task_chat_ok(id, &[RoomId::MAIN])
            .await
            .unwrap()
            .is_none(),
        "taken back"
    );
}
