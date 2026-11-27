//! The OKs messaging asks the guard rails for (`SendOks`): the owner's go in the chat on the
//! exact text, an exact text's OK given on another card, the standing OK for a 1:1 chat; what
//! went out is spent for every channel, and a card answered another way closes.

use std::sync::Arc;

use emberhut_core::RoomId;
use emberhut_core::task::ask::OptionIn;
use emberhut_core::task::{AskId, Status, Who};
use emberhut_server::services::guard::send_tap::GuardOks;
use emberhut_server::services::guard::{GitRemotes, GuardConfig, NoJudge};
use emberhut_server::services::messaging::hooks::{CardOnly, SendOks};
use emberhut_server::services::tasks::AskInput;

use crate::guard_common::{guard_app, say, task};

#[tokio::test]
async fn the_guard_rails_oks_are_the_ones_messaging_asks_for() {
    let app = guard_app(
        Arc::new(NoJudge),
        Arc::new(GitRemotes),
        GuardConfig::default(),
    )
    .await;
    let oks = GuardOks::new(app.state.guard().clone(), Arc::new(CardOnly));
    let id = task(&app, "a reply", None).await;

    say(
        &app,
        RoomId::MAIN,
        false,
        "To Sam: «see you at 5». Send it?",
    )
    .await;
    say(&app, RoomId::MAIN, true, "send it").await;
    let ok = oks
        .chat_ok(Some(RoomId::MAIN), "see you at 5", "Sam", false, None)
        .await
        .unwrap();
    assert!(
        ok.line.starts_with("OK from the owner in the chat"),
        "{ok:?}"
    );
    assert!(oks.spend(&ok).await);
    assert!(!oks.spend(&ok).await, "one message per OK");

    assert!(oks.text_ok(id.get(), "the launch note").await.is_none());
    app.state
        .guard()
        .approve_text(id, "the launch note", "your Send it at 10:00", None)
        .await
        .unwrap();
    assert_eq!(
        oks.text_ok(id.get(), "the launch note").await.unwrap().how,
        "your Send it at 10:00"
    );
    oks.sent(id.get(), "the launch note", "Slack").await;
    assert!(
        oks.text_ok(id.get(), "the launch note").await.is_none(),
        "spent for every channel"
    );
    assert!(!oks.tapped(id.get(), app.clock_now()).await);
    assert_eq!(
        oks.standing_ok(&["slack:acme:D1".to_owned()], true).await,
        None
    );

    let question = "Send this Slack message to Sam?";
    let input = AskInput {
        question: question.to_owned(),
        options: vec![
            OptionIn::Plain("Send it".to_owned()),
            OptionIn::Plain("Cancel".to_owned()),
        ],
        action: Some("Send Sam the note".to_owned()),
        ..AskInput::default()
    };
    app.state
        .tasks()
        .ask(id, input, Who::Assistant)
        .await
        .unwrap();
    assert_eq!(
        app.state.tasks().status_of(id).await.unwrap(),
        Some(Status::Waiting)
    );
    oks.close_card(id.get(), question).await;
    assert_ne!(
        app.state.tasks().status_of(id).await.unwrap(),
        Some(Status::Waiting)
    );

    assert!(
        !app.state
            .guard()
            .tap_send(id, "Send it", Some(AskId::new(999)))
            .await
            .unwrap()
    );
    assert_eq!(app.state.guard().send_result(id).await.unwrap(), None);
}
