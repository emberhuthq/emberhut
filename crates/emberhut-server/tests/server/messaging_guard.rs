//! The send guard's hold on an OK: the owner's answer counts only for the very ask that showed
//! the message (not another card with the same words), a card and a Send it grow old, two tasks
//! writing their messages at once lose none and post none twice, and while sign-in is only
//! reported nothing goes out as the owner on an OK a request without a session could give.

use emberhut_server::config::AuthMode;
use emberhut_server::services::messaging::hooks::{Ask, Tasks};
use futures_util::future::join_all;
use serde_json::json;

use crate::common::{Call, text};
use crate::messaging_fakes::Kit;
use crate::slack::{connect, send_ann};

/// What `slack_send` answers when it asked the owner and sent nothing.
const ASKED: &str = "Not sent. The owner was asked";

#[tokio::test]
async fn a_send_it_on_a_later_card_with_the_same_words_sends_nothing() {
    let kit = &connect(Kit::new().await).await;
    let first = send_ann(kit, "Wire the deposit today", 60).await;
    assert!(first.starts_with(ASKED), "{first}");
    kit.later(30);
    // Another card on the task, with the send card's own question and a harmless message.
    kit.tasks
        .ask(
            60,
            Ask {
                question: "Send this Slack message to Ann?".to_owned(),
                options: vec!["Send it".to_owned(), "Cancel".to_owned()],
                title: "Send Slack to Ann".to_owned(),
                brief: vec![
                    "To: Ann (Slack Acme, slack:Acme:Ann)".to_owned(),
                    "Message: See you at lunch".to_owned(),
                ],
                approve: true,
            },
        )
        .await
        .unwrap();
    kit.later(30);
    kit.tasks.choose(60, "Send it", kit.now());
    let again = send_ann(kit, "Wire the deposit today", 60).await;
    assert!(again.starts_with(ASKED), "{again}");
    assert!(
        again.ends_with("(Asked again: another question was asked on the task after its card.)"),
        "{again}"
    );
    assert_eq!(kit.slack.calls("chat.postMessage").len(), 0);
    assert_eq!(
        kit.tasks.asks(60).len(),
        3,
        "the message's card is shown again"
    );

    kit.later(30);
    kit.tasks.choose(60, "Send it", kit.now());
    assert_eq!(
        send_ann(kit, "Wire the deposit today", 60).await,
        "Sent to Ann on Slack (Acme), as the owner. Logged on task 60."
    );
    assert_eq!(kit.slack.calls("chat.postMessage").len(), 1);
}

#[tokio::test]
async fn a_send_it_on_a_harmless_card_is_no_ok_for_another_text() {
    let kit = &connect(Kit::new().await).await;
    assert!(
        send_ann(kit, "See you at lunch", 61)
            .await
            .starts_with(ASKED)
    );
    kit.later(30);
    kit.tasks.choose(61, "Send it", kit.now());
    let other = send_ann(kit, "Wire the deposit today", 61).await;
    assert!(other.starts_with(ASKED), "refused, and asked: {other}");
    assert_eq!(kit.slack.calls("chat.postMessage").len(), 0);
    let harmless = send_ann(kit, "See you at lunch", 61).await;
    assert!(
        harmless.starts_with(ASKED),
        "the new card replaced the one the owner answered: {harmless}"
    );
    assert_eq!(kit.slack.calls("chat.postMessage").len(), 0);
}

#[tokio::test]
async fn a_card_waits_a_day_and_a_send_it_counts_for_an_hour() {
    let kit = &connect(Kit::new().await).await;
    assert!(send_ann(kit, "Running late", 62).await.starts_with(ASKED));
    kit.later(24 * 3600 + 1);
    kit.tasks.choose(62, "Send it", kit.now());
    let old_card = send_ann(kit, "Running late", 62).await;
    assert!(
        old_card.starts_with(ASKED)
            && old_card.ends_with("(Asked again: its card waited more than 24 hours.)"),
        "{old_card}"
    );
    assert_eq!(kit.slack.calls("chat.postMessage").len(), 0);
    assert_eq!(kit.tasks.asks(62).len(), 2);

    kit.later(60);
    kit.tasks.choose(62, "Send it", kit.now());
    kit.later(3601);
    let old_ok = send_ann(kit, "Running late", 62).await;
    assert!(
        old_ok.starts_with(ASKED)
            && old_ok.ends_with("(Asked again: the owner's Send it is more than an hour old.)"),
        "{old_ok}"
    );
    assert_eq!(kit.slack.calls("chat.postMessage").len(), 0);
    assert_eq!(kit.tasks.asks(62).len(), 3);

    kit.later(60);
    kit.tasks.choose(62, "Send it", kit.now());
    kit.later(3599);
    assert_eq!(
        send_ann(kit, "Running late", 62).await,
        "Sent to Ann on Slack (Acme), as the owner. Logged on task 62."
    );
}

#[tokio::test]
async fn tasks_asking_and_sending_at_once_keep_every_card_and_post_each_once() {
    let kit = &connect(Kit::new().await).await;
    let slack = kit.app.state.messaging().slack();
    let tasks: Vec<i64> = (70..80).collect();
    let asked = join_all(
        tasks
            .iter()
            .map(|&task| async move { send_ann(kit, &format!("Note {task}"), task).await }),
    )
    .await;
    assert!(
        asked.iter().all(|said| said.starts_with(ASKED)),
        "{asked:?}"
    );
    for &task in &tasks {
        assert!(
            slack.card(task).await.unwrap().is_some(),
            "task {task}'s card is kept"
        );
    }

    kit.later(60);
    let sending: Vec<i64> = tasks.iter().copied().filter(|t| t % 2 == 0).collect();
    for &task in &sending {
        kit.tasks.choose(task, "Send it", kit.now());
    }
    // The even tasks send while the odd ones ask again with a new text, all at once.
    let calls = join_all(tasks.iter().map(|&task| async move {
        let text = if task % 2 == 0 {
            format!("Note {task}")
        } else {
            format!("Note {task}, again")
        };
        (task, send_ann(kit, &text, task).await)
    }))
    .await;
    for (task, said) in &calls {
        if task % 2 == 0 {
            assert!(said.starts_with("Sent to Ann"), "{task}: {said}");
        } else {
            assert!(said.starts_with(ASKED), "{task}: {said}");
        }
    }
    assert_eq!(kit.slack.calls("chat.postMessage").len(), sending.len());
    for &task in &tasks {
        let card = slack.card(task).await.unwrap();
        if task % 2 == 0 {
            assert_eq!(card, None, "task {task}'s message went, and stays gone");
        } else {
            assert_eq!(
                card.map(|c| c.text),
                Some(format!("Note {task}, again")),
                "task {task}'s new card is kept"
            );
        }
    }

    let again = join_all(
        sending
            .iter()
            .map(|&task| async move { send_ann(kit, &format!("Note {task}"), task).await }),
    )
    .await;
    assert!(
        again
            .iter()
            .all(|said| said.starts_with("Already sent to Ann")),
        "{again:?}"
    );
    assert_eq!(
        kit.slack.calls("chat.postMessage").len(),
        sending.len(),
        "nothing went twice"
    );
}

#[tokio::test]
async fn the_same_text_to_the_same_chat_goes_once_whatever_ok_comes_later() {
    let kit = &connect(Kit::new().await).await;
    kit.oks.go_in_chat("On my way", "ok1");
    assert!(
        send_ann(kit, "On my way", 63)
            .await
            .starts_with("Sent to Ann")
    );
    kit.oks.go_in_chat("On my way", "ok2");
    kit.oks.ok_text(63, "On my way", "on the brief's card");
    let again = send_ann(kit, "On my way", 63).await;
    assert!(again.starts_with("Already sent to Ann"), "{again}");
    assert_eq!(kit.slack.calls("chat.postMessage").len(), 1);
    assert_eq!(
        kit.oks.spent(),
        ["ok1"],
        "the new OK isn't spent on a resend"
    );
}

#[tokio::test]
async fn while_sign_in_is_only_reported_an_ok_takes_a_signed_in_device() {
    use emberhut_api::paths;
    use emberhut_core::task::{Status, Who};

    let kit = &connect(Kit::with_auth(AuthMode::Report, |deps| deps.tasks = None).await).await;
    assert_eq!(kit.app.call(Call::get("/api/whatsapp")).await.status, 401);
    let unsigned_switch = kit
        .app
        .call(Call::post("/api/whatsapp").json(&json!({ "kind": "on" })))
        .await;
    assert_eq!(unsigned_switch.status, 401);
    let session = kit.app.sign_in().await;
    let state = kit
        .app
        .call(Call::get("/api/whatsapp").token(&session))
        .await;
    assert_eq!(state.status, 200);

    let task = kit
        .app
        .state
        .tasks()
        .create(
            emberhut_db::NewTask {
                title: "Write to Ann".to_owned(),
                ..emberhut_db::NewTask::default()
            },
            Who::Person,
        )
        .await
        .unwrap()
        .id;
    let send = |words: &'static str| async move {
        let args = json!({ "chat": "slack:Acme:Ann", "text": words, "task_id": task.get() });
        text(&kit.app.call_tool("slack_send", args).await).to_owned()
    };
    // A go in the chat could come from a request without a session: it doesn't count.
    kit.oks.go_in_chat("See you at five", "ok1");
    let first = send("See you at five").await;
    assert!(first.starts_with(ASKED), "{first}");
    assert_eq!(kit.oks.spent(), Vec::<String>::new());

    kit.later(60);
    let decide = json!({ "taskDecide": task.get(), "choice": "Send it" });
    let unsigned = kit
        .app
        .call(Call::post(paths::SETTINGS).json(&decide))
        .await;
    assert_eq!(unsigned.status, 401, "{}", unsigned.text);
    let tasks = kit.app.state.tasks();
    assert_eq!(tasks.status_of(task).await.unwrap(), Some(Status::Waiting));
    assert!(send("See you at five").await.starts_with("Still waiting"));
    assert_eq!(kit.slack.calls("chat.postMessage").len(), 0);

    let signed = kit
        .app
        .call(Call::post(paths::SETTINGS).token(&session).json(&decide))
        .await;
    assert_eq!(signed.status, 200);
    let posted = send("See you at five").await;
    assert!(
        posted.starts_with("Sent to Ann") || posted.starts_with("Already sent to Ann"),
        "{posted}"
    );
    for _ in 0..500 {
        if !kit.slack.calls("chat.postMessage").is_empty() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert_eq!(kit.slack.calls("chat.postMessage").len(), 1);
}
