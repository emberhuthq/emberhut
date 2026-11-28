//! The owner's Slack: mirrored from Slack's Web API and the Mac helper, read and searched with
//! the messages tools, a file fetched with `slack_file`, and `slack_send`'s guard: nothing goes
//! out until the owner's Send it for that exact text, and never twice.

use emberhut_api::mcp::Content;
use emberhut_core::RoomId;
use emberhut_server::services::messaging::guard::{CardAnswer, Draft};
use emberhut_server::services::messaging::hooks::{Ask, Tasks};
use jiff::SignedDuration;
use serde_json::{Value, json};

use crate::common::text;
use crate::messaging_fakes::Kit;

/// A Slack `ts` `minutes` before the kit's now.
fn ts(kit: &Kit, minutes: i64) -> String {
    format!(
        "{}.000100",
        (kit.now() - SignedDuration::from_mins(minutes)).as_second()
    )
}

/// A kit with workspace Acme connected: the owner (UME) wrote to Ann (UANN) in DM D1 with a
/// PDF and a voice clip attached.
async fn acme() -> Kit {
    connect(Kit::new().await).await
}

/// `kit` with workspace Acme connected, as [`acme`] has it.
pub(crate) async fn connect(kit: Kit) -> Kit {
    kit.secret("slack-acme-user", "xoxp-test");
    kit.slack.scopes("chat:write,search:read,files:read");
    kit.slack.answers(
        "auth.test",
        json!({ "ok": true, "team": "Acme", "user_id": "UME" }),
    );
    let message = json!({
        "ts": ts(&kit, 5),
        "channel": { "id": "D1" },
        "text": "Here it is <@UANN>",
        "files": [
            { "id": "F2", "name": "plan.pdf", "pretty_type": "PDF" },
            {
                "id": "F3", "name": "note.m4a", "mimetype": "audio/mp4",
                "url_private_download": "https://files.example/F3", "duration_ms": 42000
            }
        ]
    });
    kit.slack.answers(
        "search.messages",
        json!({ "ok": true, "messages": { "matches": [message.clone()] } }),
    );
    kit.slack.answers(
        "conversations.history",
        json!({ "ok": true, "messages": [message] }),
    );
    kit.slack.answers_for(
        "users.info",
        "UANN",
        json!({ "ok": true, "user": { "profile": { "display_name": "Ann" } } }),
    );
    kit.slack.answers_for(
        "users.info",
        "UME",
        json!({ "ok": true, "user": { "real_name": "Me Myself" } }),
    );
    kit.slack.answers_for(
        "conversations.info",
        "D1",
        json!({ "ok": true, "channel": { "is_im": true, "user": "UANN" } }),
    );
    kit.slack
        .file("https://files.example/F3", 200, "audio/mp4", b"AAAA");
    kit.slack.answers(
        "chat.postMessage",
        json!({ "ok": true, "ts": "1800000000.000200" }),
    );
    kit.transcriber.hears("see you at five");
    kit.mac.answers(
        "/notifications",
        &json!([{
            "id": "n1", "title": "Beta", "subtitle": "#general", "body": "standup moved",
            "at": (kit.now() - SignedDuration::from_mins(3)).to_string()
        }]),
    );
    kit.mac.answers("/slack-mine", &json!({
        "messages": [{
            "id": "own1", "workspace": "Beta", "who": "#general", "body": "thanks for the heads up",
            "at": (kit.now() - SignedDuration::from_mins(2)).to_string(), "channel": "C9"
        }],
        "saved": { "Beta": (kit.now() - SignedDuration::from_mins(1)).to_string() }
    }));
    kit.mac.answers(
        "/chats",
        &json!([{ "id": "wa:12", "name": "Bo", "unread": 1 }]),
    );
    kit.mac.answers("/search", &json!([]));
    kit.app.state.messaging().collect().await;
    kit
}

#[tokio::test]
async fn slack_is_mirrored_read_and_searched_with_files_and_voice_clips_shown() {
    let kit = acme().await;
    let read = kit
        .app
        .call_tool("messages_read", json!({ "chat": "slack:Acme:Ann" }))
        .await;
    let read: Value = serde_json::from_str(text(&read)).unwrap();
    assert_eq!(read["conversation"], "Ann (Slack Acme)");
    assert!(
        read["note"]
            .as_str()
            .unwrap()
            .starts_with("The whole conversation")
    );
    let line = read["messages"][0].as_str().unwrap();
    assert!(line.contains("Owner: Here it is @Ann"), "{line}");
    assert!(
        line.contains("[file: plan.pdf, PDF; slack_file F2]"),
        "{line}"
    );
    assert!(line.contains("[voice, 0:42] see you at five"), "{line}");
    let heard = kit.transcriber.heard();
    assert_eq!(heard[0].0, "note.m4a");
    assert!(
        heard[0].2.contains("Names that may come up: Owner."),
        "{:?}",
        heard[0]
    );

    let chats = kit
        .app
        .call_tool("messages_chats", json!({ "days": 7 }))
        .await;
    let chats = text(&chats);
    assert!(
        chats.starts_with(r#"[{"id":"wa:12""#),
        "the Mac's chats first: {chats}"
    );
    assert!(chats.contains(r#""id":"slack:Acme:Ann""#), "{chats}");
    assert!(
        chats.contains(r#""id":"slack:Beta:#general""#),
        "the Mac's notifications: {chats}"
    );

    let found = kit
        .app
        .call_tool("messages_search", json!({ "q": "heads up" }))
        .await;
    let found = text(&found);
    assert!(found.contains("thanks for the heads up"), "{found}");
    assert!(
        found.contains(r#""sender":"Owner""#),
        "the owner's own, from the Mac: {found}"
    );

    let beta = kit
        .app
        .call_tool("messages_read", json!({ "chat": "slack:Beta:#general" }))
        .await;
    let beta: Value = serde_json::from_str(text(&beta)).unwrap();
    assert!(
        beta["note"]
            .as_str()
            .unwrap()
            .contains("as of Slack's last save at"),
        "{beta}"
    );
    assert_eq!(beta["messages"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn replied_says_whether_the_owner_already_wrote_and_when_it_can_t_tell() {
    let kit = acme().await;
    let since = (kit.now() - SignedDuration::from_hours(1)).to_string();
    let replied = kit
        .app
        .call_tool(
            "replied",
            json!({ "chat": "slack:Acme:Ann", "since": since }),
        )
        .await;
    let replied: Value = serde_json::from_str(text(&replied)).unwrap();
    assert_eq!(replied["answered"], true);
    assert!(
        replied["his"][0]
            .as_str()
            .unwrap()
            .contains("Here it is @Ann")
    );
    assert!(replied.get("warning").is_none());

    let later = kit.now().to_string();
    let not_yet = kit
        .app
        .call_tool(
            "replied",
            json!({ "chat": "slack:Beta:#general", "since": later }),
        )
        .await;
    let not_yet: Value = serde_json::from_str(text(&not_yet)).unwrap();
    assert_eq!(not_yet["answered"], false);
    assert!(
        not_yet["warning"].as_str().unwrap().contains("last saved"),
        "a workspace read from the Mac only: {not_yet}"
    );
}

#[tokio::test]
async fn slack_file_fetches_a_file_and_hands_a_picture_back() {
    let kit = acme().await;
    kit.slack.answers_for(
        "files.info",
        "F7",
        json!({ "ok": true, "file": {
            "name": "shot one.png", "mimetype": "image/png", "pretty_type": "PNG",
            "url_private": "https://files.example/F7", "size": 4
        } }),
    );
    kit.slack
        .file("https://files.example/F7", 200, "image/png", b"\x89PNG");
    let result = kit
        .app
        .call_tool("slack_file", json!({ "file": "F7" }))
        .await;
    assert_ne!(result.is_error, Some(true));
    let Content::Image { mime_type, .. } = &result.content[0] else {
        panic!("a picture first: {result:?}");
    };
    assert_eq!(mime_type, "image/png");
    let file: Value = serde_json::from_str(result.content[1].as_text().unwrap()).unwrap();
    assert_eq!(file["workspace"], "Acme");
    assert_eq!(file["type"], "PNG");
    let path = file["path"].as_str().unwrap();
    assert!(
        std::path::Path::new(path).ends_with("files/slack/F7-shot_one.png"),
        "{path}"
    );
    assert_eq!(std::fs::read(path).unwrap(), b"\x89PNG");

    let wrong = kit
        .app
        .call_tool("slack_file", json!({ "file": "../x" }))
        .await;
    assert!(text(&wrong).starts_with("Couldn't fetch it: ../x isn't a Slack file id"));
    kit.slack
        .file("https://files.example/F7", 200, "text/html", b"<html>");
    let refused = kit
        .app
        .call_tool("slack_file", json!({ "file": "F7" }))
        .await;
    assert!(text(&refused).contains("files:read?"), "{}", text(&refused));
}

#[tokio::test]
async fn slack_send_posts_only_after_the_owner_s_send_it_for_that_exact_text() {
    let kit = acme().await;
    let send = |text: &str, task: Option<i64>| {
        let mut args = json!({ "chat": "slack:Acme:Ann", "text": text });
        if let Some(task) = task {
            args["task_id"] = json!(task);
        }
        args
    };
    let call =
        |args: Value| async { text(&kit.app.call_tool("slack_send", args).await).to_owned() };

    assert!(call(send("hi", None)).await.contains("needs task_id"));
    assert!(
        call(send("it's *done*", Some(7)))
            .await
            .starts_with("Not plain text")
    );
    let first = call(send("See you at five <3", Some(7))).await;
    assert!(
        first.starts_with("Not sent. The owner was asked"),
        "{first}"
    );
    let ask = &kit.tasks.asks(7)[0];
    assert_eq!(ask.question, "Send this Slack message to Ann?");
    assert_eq!(ask.options, ["Send it", "Cancel"]);
    assert!(
        ask.brief
            .contains(&"Message: See you at five <3".to_owned())
    );
    assert_eq!(
        kit.slack.calls("chat.postMessage").len(),
        0,
        "nothing went out"
    );
    assert!(
        call(send("See you at five <3", Some(7)))
            .await
            .starts_with("Still waiting")
    );

    kit.later(60);
    kit.tasks.choose(7, "Send it", kit.now());
    let changed = call(send("See you at six", Some(7))).await;
    assert!(
        changed.starts_with("Not sent. The owner was asked"),
        "another text is a new ask: {changed}"
    );
    assert_eq!(kit.slack.calls("chat.postMessage").len(), 0);
    kit.later(60);
    kit.tasks.choose(7, "Send it", kit.now());
    let posted = call(send("See you at six", Some(7))).await;
    assert_eq!(
        posted,
        "Sent to Ann on Slack (Acme), as the owner. Logged on task 7."
    );
    let posts = kit.slack.calls("chat.postMessage");
    assert_eq!(posts.len(), 1);
    assert!(posts[0].contains(&("channel".to_owned(), "D1".to_owned())));
    assert!(posts[0].contains(&("mrkdwn".to_owned(), "false".to_owned())));
    assert!(
        kit.tasks
            .notes(7)
            .iter()
            .any(|n| n.starts_with("Sent on Slack to Ann"))
    );
    let again = call(send("See you at six", Some(7))).await;
    assert!(again.starts_with("Already sent to Ann"), "{again}");
    assert_eq!(
        kit.slack.calls("chat.postMessage").len(),
        1,
        "never a resend"
    );

    let html = call(send("1 < 2 & 3 > 2", Some(9))).await;
    assert!(html.starts_with("Not sent. The owner was asked"));
    kit.later(60);
    kit.tasks.choose(9, "Send it", kit.now());
    call(send("1 < 2 & 3 > 2", Some(9))).await;
    let posts = kit.slack.calls("chat.postMessage");
    assert!(posts[1].contains(&("text".to_owned(), "1 &lt; 2 &amp; 3 &gt; 2".to_owned())));
}

#[tokio::test]
async fn slack_send_stops_at_cancel_and_at_a_token_that_can_t_post() {
    let kit = acme().await;
    let args = json!({ "chat": "slack:Acme:Ann", "text": "Running late", "task_id": 8 });
    kit.app.call_tool("slack_send", args.clone()).await;
    kit.later(30);
    kit.tasks.choose(8, "Cancel", kit.now());
    let cancelled = kit.app.call_tool("slack_send", args.clone()).await;
    assert_eq!(
        text(&cancelled),
        "The owner said Cancel: not sent. Don't send it."
    );
    assert_eq!(kit.slack.calls("chat.postMessage").len(), 0);

    kit.slack.scopes("search:read");
    let unscoped = kit
        .app
        .call_tool(
            "slack_send",
            json!({ "chat": "slack:Acme:Ann", "text": "Hello", "task_id": 10 }),
        )
        .await;
    assert!(
        text(&unscoped).contains("no chat:write scope"),
        "{}",
        text(&unscoped)
    );
    assert_eq!(kit.tasks.asks(10).len(), 0, "nothing asked");

    let elsewhere = kit
        .app
        .call_tool(
            "slack_send",
            json!({ "chat": "slack:Gamma:Cy", "text": "Hello", "task_id": 11 }),
        )
        .await;
    assert!(
        text(&elsewhere).starts_with("No Slack token for Gamma"),
        "{}",
        text(&elsewhere)
    );
}

#[tokio::test]
async fn a_draft_kept_as_a_card_goes_out_on_the_owner_s_tap_or_a_standing_ok() {
    let kit = acme().await;
    let slack = kit.app.state.messaging().slack();
    let draft = Draft {
        chat: "slack:Acme:Ann".to_owned(),
        text: "Can we move to Monday?".to_owned(),
        question: "Ask Ann to move the call?".to_owned(),
        ask: kit.tasks.ask_card(20, "Ask Ann to move the call?"),
        send: "Send it".to_owned(),
        room: Some(RoomId::MAIN),
    };
    let prepared = slack.prepare(20, &draft).await.unwrap();
    assert_eq!(prepared.to, "Ann");
    assert_eq!(prepared.line, "Ann on Slack (Acme, slack:Acme:Ann)");
    assert_eq!(prepared.free, None);
    let card = slack.card(20).await.unwrap().unwrap();
    assert_eq!((card.via, card.tool), ("Slack", "slack_send"));
    assert_eq!(card.question, "Ask Ann to move the call?");

    let untapped = slack.answer_card(20, "Send it", &card).await.unwrap();
    assert!(
        matches!(&untapped, CardAnswer::Failed(why) if why.starts_with("Still waiting")),
        "{untapped:?}"
    );
    assert_eq!(kit.slack.calls("chat.postMessage").len(), 0);
    kit.oks.tap(20, card.at);
    let tapped = slack.answer_card(20, "Send it", &card).await.unwrap();
    assert!(matches!(tapped, CardAnswer::Sent(_)), "{tapped:?}");
    assert_eq!(kit.slack.calls("chat.postMessage").len(), 1);
    assert_eq!(
        kit.oks.sent(),
        [(20, "Can we move to Monday?".to_owned(), "Slack".to_owned())]
    );
    let stale = slack.answer_card(20, "Send it", &card).await.unwrap();
    assert!(
        matches!(stale, CardAnswer::Sent(_)),
        "already out: {stale:?}"
    );

    let formatted = Draft {
        text: "**bold**".to_owned(),
        ..draft.clone()
    };
    assert!(
        slack
            .prepare(21, &formatted)
            .await
            .unwrap_err()
            .starts_with("Not plain text")
    );

    kit.oks.stand("slack:Acme:Ann");
    let free = kit
        .app
        .call_tool(
            "slack_send",
            json!({ "chat": "slack:Acme:Ann", "text": "Thanks!", "task_id": 22 }),
        )
        .await;
    assert_eq!(
        text(&free),
        "Sent to Ann on Slack (Acme), as the owner. Logged on task 22."
    );
    assert_eq!(
        kit.tasks.asks(22).len(),
        0,
        "no card for a chat on the standing list"
    );
    assert!(
        kit.tasks
            .notes(22)
            .iter()
            .any(|n| n.contains("the owner's standing OK"))
    );
}

/// `slack_send`'s arguments: `text` to Ann on `task`.
fn to_ann(text: &str, task: i64) -> Value {
    json!({ "chat": "slack:Acme:Ann", "text": text, "task_id": task })
}

/// What `slack_send` answers for `text` to Ann on `task`.
pub(crate) async fn send_ann(kit: &Kit, text: &str, task: i64) -> String {
    crate::common::text(&kit.app.call_tool("slack_send", to_ann(text, task)).await).to_owned()
}

#[tokio::test]
async fn with_the_default_guard_nothing_goes_out_as_the_owner_and_nothing_waits() {
    let kit = connect(Kit::with_default_guard().await).await;
    for _ in 0..2 {
        assert_eq!(
            send_ann(&kit, "See you at five", 7).await,
            "Not sent, and the owner couldn't be asked: tasks isn't set up on this server."
        );
    }
    assert_eq!(kit.slack.calls("chat.postMessage").len(), 0);
    let messaging = kit.app.state.messaging();
    assert_eq!(messaging.slack().card(7).await.unwrap(), None);

    kit.mac.answers(
        "/wa-contact",
        &json!({ "chat": "wa:12", "name": "Bo Park", "kind": "person", "phone": "15550100500", "me": false }),
    );
    for _ in 0..2 {
        let answer = kit
            .app
            .call_tool(
                "whatsapp_send",
                json!({ "chat": "wa:12", "text": "On my way", "task_id": 8 }),
            )
            .await;
        assert!(
            text(&answer).starts_with("WhatsApp Web isn't available on this server"),
            "{}",
            text(&answer)
        );
    }
    assert_eq!(messaging.whatsapp_send().card(8).await.unwrap(), None);
}

#[tokio::test]
async fn a_go_in_the_chat_or_an_ok_of_the_exact_text_sends_once_and_closes_the_card() {
    let kit = acme().await;
    let went = "Sent to Ann on Slack (Acme), as the owner.";

    kit.oks.go_in_chat("Running late", "ok1");
    let first = send_ann(&kit, "Running late", 12).await;
    assert_eq!(first, format!("{went} Logged on task 12."));
    assert_eq!(kit.oks.spent(), ["ok1"]);
    assert!(kit.tasks.asks(12).is_empty(), "no card");
    let notes = kit.tasks.notes(12);
    assert!(notes.contains(&"The owner said send it in chat (ok1)".to_owned()));
    assert!(
        notes
            .iter()
            .any(|n| n.starts_with("Sent (you said send it in chat) on Slack to Ann")),
        "{notes:?}"
    );
    assert!(
        send_ann(&kit, "Running late", 12)
            .await
            .starts_with("Already sent to Ann")
    );
    assert_eq!(kit.slack.calls("chat.postMessage").len(), 1);

    kit.oks.go_in_chat("Running later", "ok2");
    kit.oks.spend_elsewhere("ok2");
    let spent = send_ann(&kit, "Running later", 13).await;
    assert!(
        spent.starts_with("Not sent. The owner was asked"),
        "a go spent on another message is no OK: {spent}"
    );
    assert_eq!(kit.slack.calls("chat.postMessage").len(), 1);

    assert!(
        send_ann(&kit, "Lunch at one?", 14)
            .await
            .starts_with("Not sent. The owner was asked")
    );
    kit.oks.go_in_chat("Lunch at one?", "ok3");
    assert_eq!(
        send_ann(&kit, "Lunch at one?", 14).await,
        format!("{went} Logged on task 14.")
    );
    let slack = kit.app.state.messaging().slack();
    assert_eq!(slack.card(14).await.unwrap(), None);

    assert!(
        send_ann(&kit, "Thanks a lot", 15)
            .await
            .starts_with("Not sent. The owner was asked")
    );
    kit.oks.ok_text(15, "Thanks a lot", "on the brief's card");
    assert_eq!(
        send_ann(&kit, "Thanks a lot", 15).await,
        format!("{went} Logged on task 15.")
    );
    assert!(
        kit.tasks.notes(15).contains(
            &"Slack to Ann: the exact text the owner OK'd (on the brief's card), no new card"
                .to_owned()
        ),
        "{:?}",
        kit.tasks.notes(15)
    );
    assert_eq!(
        kit.oks.closed(),
        [
            (14, "Send this Slack message to Ann?".to_owned()),
            (15, "Send this Slack message to Ann?".to_owned())
        ],
        "an OK given another way closes the card it answers"
    );
    assert_eq!(kit.slack.calls("chat.postMessage").len(), 3);
    assert_eq!(
        kit.oks
            .sent()
            .iter()
            .map(|(task, _, via)| (*task, via.as_str()))
            .collect::<Vec<_>>(),
        [(12, "Slack"), (14, "Slack"), (15, "Slack")]
    );
}

#[tokio::test]
async fn a_tap_counts_only_for_its_card_and_an_answer_to_another_card_sends_nothing() {
    let kit = acme().await;
    let slack = kit.app.state.messaging().slack();
    let draft = Draft {
        chat: "slack:Acme:Ann".to_owned(),
        text: "Can we move to Monday?".to_owned(),
        question: "Ask Ann to move the call?".to_owned(),
        ask: kit.tasks.ask_card(20, "Ask Ann to move the call?"),
        send: "Send it".to_owned(),
        room: Some(RoomId::MAIN),
    };
    slack.prepare(20, &draft).await.unwrap();
    let first = slack.card(20).await.unwrap().unwrap();
    kit.later(60);
    let draft = Draft {
        ask: kit.tasks.ask_card(20, "Ask Ann to move the call?"),
        ..draft
    };
    slack.prepare(20, &draft).await.unwrap();
    let second = slack.card(20).await.unwrap().unwrap();
    assert_ne!(first.at, second.at);
    kit.oks.tap(20, first.at);
    let untapped = slack.answer_card(20, "Send it", &second).await.unwrap();
    assert!(
        matches!(&untapped, CardAnswer::Failed(why) if why.starts_with("Still waiting")),
        "a tap on the card asked before is no tap on this one: {untapped:?}"
    );
    assert_eq!(
        slack.answer_card(20, "Send it", &first).await.unwrap(),
        CardAnswer::Failed(
            "this message was asked again or changed since; nothing was sent.".to_owned()
        )
    );
    assert_eq!(kit.slack.calls("chat.postMessage").len(), 0);
    kit.oks.tap(20, second.at);
    let tapped = slack.answer_card(20, "Send it", &second).await.unwrap();
    assert!(matches!(tapped, CardAnswer::Sent(_)), "{tapped:?}");
    assert_eq!(kit.slack.calls("chat.postMessage").len(), 1);

    assert!(
        send_ann(&kit, "Room 4 at ten", 16)
            .await
            .starts_with("Not sent. The owner was asked")
    );
    kit.later(30);
    kit.tasks
        .ask(
            16,
            Ask {
                question: "Book the room too?".to_owned(),
                options: vec!["Send it".to_owned(), "Cancel".to_owned()],
                title: "Book the room".to_owned(),
                brief: Vec::new(),
                approve: false,
            },
        )
        .await
        .unwrap();
    kit.later(30);
    kit.tasks.choose(16, "Send it", kit.now());
    let other = send_ann(&kit, "Room 4 at ten", 16).await;
    assert!(
        other.starts_with("Not sent. The owner was asked"),
        "a Send it on another card is no OK for this one, and that card replaced it: {other}"
    );
    assert_eq!(kit.slack.calls("chat.postMessage").len(), 1);
}

#[tokio::test]
async fn a_post_that_slack_refuses_still_spends_the_owner_s_ok() {
    let kit = acme().await;
    assert!(
        send_ann(&kit, "Hello there", 17)
            .await
            .starts_with("Not sent. The owner was asked")
    );
    kit.later(60);
    kit.tasks.choose(17, "Send it", kit.now());
    kit.slack.answers(
        "chat.postMessage",
        json!({ "ok": false, "error": "channel_not_found" }),
    );
    assert_eq!(
        send_ann(&kit, "Hello there", 17).await,
        "Not sent: Slack said channel_not_found. The owner's OK is spent; if they still want it, \
         ask again (call slack_send again)."
    );
    assert_eq!(
        kit.oks.sent(),
        [(17, "Hello there".to_owned(), "Slack (failed)".to_owned())]
    );
    assert!(
        kit.tasks
            .notes(17)
            .iter()
            .any(|n| n.ends_with("not sent: Slack said channel_not_found")),
        "{:?}",
        kit.tasks.notes(17)
    );
    let slack = kit.app.state.messaging().slack();
    assert_eq!(slack.card(17).await.unwrap(), None);
    kit.slack.answers(
        "chat.postMessage",
        json!({ "ok": true, "ts": "1800000000.000300" }),
    );
    let again = send_ann(&kit, "Hello there", 17).await;
    assert!(
        again.starts_with("Not sent. The owner was asked"),
        "the spent OK doesn't post it now; it is a new ask: {again}"
    );
    assert_eq!(kit.tasks.asks(17).len(), 2);
    assert_eq!(kit.slack.calls("chat.postMessage").len(), 1);

    let draft = Draft {
        chat: "slack:Acme:Ann".to_owned(),
        text: "See you there".to_owned(),
        question: "Tell Ann you'll come?".to_owned(),
        ask: kit.tasks.ask_card(18, "Tell Ann you'll come?"),
        send: "Send it".to_owned(),
        room: None,
    };
    slack.prepare(18, &draft).await.unwrap();
    let card = slack.card(18).await.unwrap().unwrap();
    kit.oks.tap(18, card.at);
    kit.slack.answers(
        "chat.postMessage",
        json!({ "ok": false, "error": "is_archived" }),
    );
    assert_eq!(
        slack.answer_card(18, "Send it", &card).await.unwrap(),
        CardAnswer::Failed("Slack said is_archived.".to_owned())
    );
    assert_eq!(slack.card(18).await.unwrap(), None, "the tap spent it");
}

#[tokio::test]
async fn a_tap_and_a_slack_send_at_once_post_once() {
    let kit = acme().await;
    let slack = kit.app.state.messaging().slack();
    assert!(
        send_ann(&kit, "On the way", 19)
            .await
            .starts_with("Not sent. The owner was asked")
    );
    let card = slack.card(19).await.unwrap().unwrap();
    kit.later(60);
    kit.tasks.choose(19, "Send it", kit.now());
    let (tap, call) = tokio::join!(
        slack.answer_card(19, "Send it", &card),
        send_ann(&kit, "On the way", 19)
    );
    assert!(matches!(tap.unwrap(), CardAnswer::Sent(_)));
    assert!(
        call.starts_with("Sent to Ann") || call.starts_with("Already sent to Ann"),
        "{call}"
    );
    assert_eq!(kit.slack.calls("chat.postMessage").len(), 1);
}

#[tokio::test]
async fn through_the_server_s_own_tasks_only_the_owner_s_tap_on_that_text_sends() {
    use emberhut_api::paths;
    use emberhut_core::task::{Status, Who};

    use crate::common::Call;

    let kit = connect(Kit::with(|deps| deps.tasks = None).await).await;
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
    let send =
        |text: &str| json!({ "chat": "slack:Acme:Ann", "text": text, "task_id": task.get() });
    let call =
        |args: Value| async { text(&kit.app.call_tool("slack_send", args).await).to_owned() };
    let session = kit.app.sign_in().await;
    let tap = |choice: &'static str| {
        let session = session.clone();
        let app = &kit.app;
        async move {
            app.call(
                Call::post(paths::SETTINGS)
                    .token(&session)
                    .json(&json!({ "taskDecide": task.get(), "choice": choice })),
            )
            .await
            .status
        }
    };

    let first = call(send("See you at five")).await;
    assert!(
        first.starts_with("Not sent. The owner was asked"),
        "{first}"
    );
    let tasks = kit.app.state.tasks();
    assert_eq!(
        tasks.status_of(task).await.unwrap(),
        Some(Status::Waiting),
        "the ask is on the task"
    );
    let issue = tasks.issue(task).await.unwrap().unwrap();
    let decision = issue.decision.unwrap();
    assert_eq!(decision.question, "Send this Slack message to Ann?");
    assert_eq!(decision.options, ["Send it", "Cancel"]);
    assert_eq!(kit.slack.calls("chat.postMessage").len(), 0);

    kit.later(60);
    assert_eq!(tap("Send it").await, 200);
    // The owner's Send it on the card sends its exact text there and then (the guard rails'
    // send-on-tap), through slack_send's own guard.
    for _ in 0..500 {
        if !kit.slack.calls("chat.postMessage").is_empty() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert_eq!(kit.slack.calls("chat.postMessage").len(), 1);
    let again = call(send("See you at five")).await;
    assert!(again.starts_with("Already sent to Ann"), "{again}");
    let changed = call(send("See you at six")).await;
    assert!(
        changed.starts_with("Not sent. The owner was asked"),
        "a tap is for that exact text only: {changed}"
    );
    assert_eq!(
        kit.slack.calls("chat.postMessage").len(),
        1,
        "never a resend, and nothing else without its own tap"
    );
    let history = tasks.history_of(task).await.unwrap();
    assert!(
        history
            .iter()
            .any(|line| line.text.starts_with("Sent on Slack to Ann")),
        "the send is logged on the task"
    );
}

#[tokio::test]
async fn through_the_server_s_own_tasks_a_tap_on_a_card_that_replaced_the_message_sends_nothing() {
    use emberhut_api::paths;
    use emberhut_core::task::Who;
    use emberhut_core::task::ask::OptionIn;
    use emberhut_server::services::tasks::AskInput;

    use crate::common::Call;

    let kit = connect(Kit::with(|deps| deps.tasks = None).await).await;
    let tasks = kit.app.state.tasks();
    let task = tasks
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
    let session = kit.app.sign_in().await;
    let tap = || async {
        let decide = json!({ "taskDecide": task.get(), "choice": "Send it" });
        let answer = kit
            .app
            .call(Call::post(paths::SETTINGS).token(&session).json(&decide))
            .await;
        assert_eq!(answer.status, 200);
        // Long enough for a send-on-tap to reach Slack, had it started one.
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    };
    let harmless = |question: &str, same: bool| AskInput {
        question: question.to_owned(),
        options: vec![
            OptionIn::Plain("Send it".to_owned()),
            OptionIn::Plain("Cancel".to_owned()),
        ],
        kind: Some("approve".to_owned()),
        brief: Some(vec![
            "To: Ann (Slack Acme, slack:Acme:Ann)".to_owned(),
            "Message: See you at lunch".to_owned(),
        ]),
        same,
        ..AskInput::default()
    };
    let wire = "Wire the deposit today";
    let asked = "Not sent. The owner was asked";

    assert!(send_ann(&kit, wire, task.get()).await.starts_with(asked));
    // A re-check of the card, with a harmless brief: the line it writes asks nothing, but it
    // is an ask of its own.
    kit.later(30);
    tasks
        .ask(task, harmless("Lunch with Ann?", true), Who::Assistant)
        .await
        .unwrap();
    kit.later(30);
    tap().await;
    assert_eq!(kit.slack.calls("chat.postMessage").len(), 0);
    let again = send_ann(&kit, wire, task.get()).await;
    assert!(again.starts_with(asked), "{again}");
    assert_eq!(kit.slack.calls("chat.postMessage").len(), 0);

    // A new card with the send card's very question and a harmless message.
    kit.later(30);
    tasks
        .ask(
            task,
            harmless("Send this Slack message to Ann?", false),
            Who::Assistant,
        )
        .await
        .unwrap();
    kit.later(30);
    tap().await;
    assert_eq!(kit.slack.calls("chat.postMessage").len(), 0);
    let again = send_ann(&kit, wire, task.get()).await;
    assert!(again.starts_with(asked), "{again}");
    assert_eq!(kit.slack.calls("chat.postMessage").len(), 0);

    // The harmless card's Send it is no OK for another text either.
    let other = send_ann(&kit, "See you at lunch", task.get()).await;
    assert!(other.starts_with(asked), "{other}");
    assert_eq!(kit.slack.calls("chat.postMessage").len(), 0);

    // Only the tap on the card that shows the text sends it.
    assert!(send_ann(&kit, wire, task.get()).await.starts_with(asked));
    kit.later(30);
    tap().await;
    for _ in 0..500 {
        if !kit.slack.calls("chat.postMessage").is_empty() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    let posts = kit.slack.calls("chat.postMessage");
    assert_eq!(posts.len(), 1);
    assert!(posts[0].contains(&("text".to_owned(), wire.to_owned())));
}
