//! The messaging stores (the `slack`, `mail`, `voice_notes`, `wa_chats`, `wa_msgs` and
//! `wa_sent` tables) against a real SQLite file.

use emberhut_core::RoomId;
use emberhut_core::messaging::mail::MailMessage;
use emberhut_core::messaging::slack::SlackMessage;
use emberhut_core::messaging::voice::{SeenNote, VoiceSource, VoiceStatus};
use emberhut_core::messaging::whatsapp::{ChatKind, ChatStatus, WaChat, WaMessage};
use emberhut_db::messaging::{ChatChange, NewWaSent, Outcome};

use crate::common::{at, database, reopen};

fn slack(id: &str, who: &str, minutes: i64, from_me: bool) -> SlackMessage {
    SlackMessage {
        id: id.to_owned(),
        workspace: "Acme".to_owned(),
        who: who.to_owned(),
        body: format!("message {id}"),
        at: at(minutes),
        from_me,
        sender: Some(if from_me { "Owner" } else { who }.to_owned()),
        channel: Some("D1".to_owned()),
    }
}

#[tokio::test]
async fn slack_keeps_messages_by_conversation_and_brings_bodies_up_to_date() {
    let (dir, db) = database().await;
    let store = db.slack();
    store.keep(&slack("a", "Ann", 1, false)).await.unwrap();
    store.keep(&slack("b", "Ann", 2, true)).await.unwrap();
    store
        .keep(&slack("c", "#releases", 3, false))
        .await
        .unwrap();
    let mut edited = slack("a", "Ann", 1, false);
    edited.body = "message a, edited".to_owned();
    store.keep(&edited).await.unwrap();
    let mut notification = slack("n1", "Bo", 4, false);
    notification.channel = None;
    assert!(store.add(&notification).await.unwrap());
    assert!(!store.add(&notification).await.unwrap(), "kept once");

    let store = reopen(&dir).await.slack();
    let ann = store.messages("Acme", "Ann", 10).await.unwrap();
    assert_eq!(
        ann.iter().map(|m| m.body.as_str()).collect::<Vec<_>>(),
        ["message a, edited", "message b"],
        "oldest first"
    );
    assert_eq!(store.messages("Acme", "Ann", 1).await.unwrap()[0].id, "b");
    assert_eq!(store.last_mine("Acme").await.unwrap(), Some(at(2)));
    assert_eq!(store.last_notification().await.unwrap(), Some(at(4)));
    assert_eq!(store.channels("Acme", "Ann").await.unwrap(), ["D1"]);

    let conversations = store.conversations(at(0)).await.unwrap();
    assert_eq!(conversations[0].who, "Bo", "the newest first");
    let ann = conversations.iter().find(|c| c.who == "Ann").unwrap();
    assert_eq!((ann.messages, ann.mine, ann.last_at), (2, 1, at(2)));
    assert_eq!(store.conversations(at(3)).await.unwrap().len(), 1);

    let found = store.search("RELEASES", at(0), 10).await.unwrap();
    assert_eq!(found.len(), 1, "by the conversation's name, in any case");
    assert_eq!(store.search("message", at(2), 10).await.unwrap().len(), 2);

    assert!(store.replace_in_body("c", "message", "note").await.unwrap());
    assert_eq!(store.get("c").await.unwrap().unwrap().body, "note c");
    assert!(!store.replace_in_body("nope", "a", "b").await.unwrap());
}

fn mail(id: &str, chat: &str, minutes: i64, got: i64, loud: bool) -> MailMessage {
    MailMessage {
        id: format!("home:{id}"),
        account: "home".to_owned(),
        chat: chat.to_owned(),
        who: "Ann".to_owned(),
        sender: "Ann <ann@example.com>".to_owned(),
        from_me: false,
        subject: format!("Subject {id}"),
        body: "Lunch on Friday?".to_owned(),
        at: at(minutes),
        got: at(got),
        loud,
        thread: Some("t1".to_owned()),
    }
}

#[tokio::test]
async fn mail_keeps_each_message_once_and_groups_by_correspondent() {
    let (_dir, db) = database().await;
    let store = db.mail();
    let ann = "email:home:ann@example.com";
    assert!(store.add(&mail("1", ann, 1, 1, true)).await.unwrap());
    assert!(!store.add(&mail("1", ann, 1, 1, true)).await.unwrap());
    store.add(&mail("2", ann, 2, 10, true)).await.unwrap();
    store
        .add(&mail("3", "email:home:news@example.com", 3, 3, false))
        .await
        .unwrap();
    assert!(store.has("home:2").await.unwrap());
    assert_eq!(store.count("home").await.unwrap(), 3);

    let conversations = store.conversations(at(0)).await.unwrap();
    assert_eq!(conversations.len(), 1, "quiet mail is kept for search only");
    assert_eq!(conversations[0].messages, 2);
    let changes = store.changes(at(5)).await.unwrap();
    assert_eq!(changes.len(), 1, "got after 5: message 2 only");
    assert_eq!(changes[0].messages, 1);

    let messages = store.messages(ann, 10).await.unwrap();
    assert_eq!(messages[0].id, "home:1");
    assert_eq!(store.new_since(ann, at(5), 10).await.unwrap().len(), 1);
    assert_eq!(
        store.search("home", "subject 3", at(0), 10).await.unwrap()[0].id,
        "home:3"
    );
    assert_eq!(
        store
            .search("away", "lunch", at(0), 10)
            .await
            .unwrap()
            .len(),
        0
    );
}

fn seen(id: &str) -> SeenNote {
    SeenNote {
        id: id.to_owned(),
        source: VoiceSource::of_mac_id(id),
        reference: id.to_owned(),
        name: Some("voice.m4a".to_owned()),
        seconds: None,
        chat: Some("wa:12".to_owned()),
        chat_name: None,
        sender: Some("Ann".to_owned()),
        msg_at: Some(at(1)),
        msg: None,
    }
}

#[tokio::test]
async fn voice_notes_are_kept_once_and_record_each_try() {
    let (_dir, db) = database().await;
    let store = db.voice_notes();
    let first = store.seen(&seen("wa:m1"), at(2)).await.unwrap();
    assert_eq!(first.status, VoiceStatus::New);
    assert_eq!(first.source, VoiceSource::WhatsApp);
    let mut again = seen("wa:m1");
    again.chat_name = Some("Ann Lee".to_owned());
    again.seconds = Some(12.0);
    again.sender = Some("someone else".to_owned());
    let kept = store.seen(&again, at(3)).await.unwrap();
    assert_eq!(kept.chat_name.as_deref(), Some("Ann Lee"), "filled in");
    assert_eq!(kept.sender.as_deref(), Some("Ann"), "kept as first seen");
    assert_eq!(kept.at, at(2));

    assert_eq!(
        store.due(at(10), at(1)).await.unwrap(),
        Vec::<String>::new()
    );
    assert_eq!(
        store.due(at(10), at(5)).await.unwrap(),
        ["wa:m1"],
        "a stale new one"
    );

    store
        .finish(
            "wa:m1",
            &Outcome::Failed {
                error: "fetch failed".to_owned(),
                tries: 1,
                next_at: Some(at(20)),
            },
            at(10),
        )
        .await
        .unwrap();
    assert_eq!(store.due(at(19), at(0)).await.unwrap().len(), 0);
    assert_eq!(store.due(at(20), at(0)).await.unwrap(), ["wa:m1"]);
    store
        .finish(
            "wa:m1",
            &Outcome::Done {
                text: "see you at five".to_owned(),
                seconds: None,
            },
            at(21),
        )
        .await
        .unwrap();
    let done = store.get("wa:m1").await.unwrap().unwrap();
    assert_eq!(done.status, VoiceStatus::Done);
    assert_eq!(done.seconds, Some(12.0), "its length stays");
    assert_eq!(done.error, None);
    assert_eq!(done.done_at, Some(at(21)));

    let found = store.search("FIVE", at(0), 5).await.unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(store.search("five", at(1), 5).await.unwrap().len(), 0);

    let slack = SeenNote {
        source: VoiceSource::Slack,
        reference: "acme|https://files.example/F1".to_owned(),
        ..seen("slack:F1")
    };
    store.seen(&slack, at(1)).await.unwrap();
    store
        .finish(
            "slack:F1",
            &Outcome::Failed {
                error: "needs files:read".to_owned(),
                tries: 4,
                next_at: None,
            },
            at(2),
        )
        .await
        .unwrap();
    assert_eq!(
        store.get("slack:F1").await.unwrap().unwrap().status,
        VoiceStatus::GaveUp
    );
    assert_eq!(store.retry_slack(at(30)).await.unwrap(), 1);
    let retried = store.get("slack:F1").await.unwrap().unwrap();
    assert_eq!((retried.status, retried.tries), (VoiceStatus::Failed, 0));
    store.set_shown("slack:F1", "[voice] hi").await.unwrap();
    assert_eq!(
        store
            .get("slack:F1")
            .await
            .unwrap()
            .unwrap()
            .shown
            .as_deref(),
        Some("[voice] hi")
    );
}

fn chat(jid: &str, name: &str, kind: ChatKind) -> WaChat {
    WaChat {
        jid: jid.to_owned(),
        kind,
        name: Some(name.to_owned()),
        status: ChatStatus::Pending,
        via: None,
        task: None,
        room: None,
        topics: None,
        by: None,
        added: at(0),
        updated: at(0),
    }
}

fn wa(chat: &str, id: &str, minutes: i64, asked: bool) -> WaMessage {
    WaMessage {
        chat: chat.to_owned(),
        id: id.to_owned(),
        sender: Some("15550100100@s.whatsapp.net".to_owned()),
        who: Some("Ann".to_owned()),
        text: Some(format!("text {id}")),
        at: at(minutes),
        from_me: false,
        asked,
        answered: false,
    }
}

#[tokio::test]
async fn wa_chats_wa_msgs_and_wa_sent_keep_the_allow_list_the_context_and_the_log() {
    let (_dir, db) = database().await;
    let store = db.whatsapp();
    let group = "1203630001@g.us";
    assert!(
        store
            .add_chat(&chat(group, "Book Club", ChatKind::Group))
            .await
            .unwrap()
    );
    assert!(
        !store
            .add_chat(&chat(group, "Other", ChatKind::Group))
            .await
            .unwrap()
    );
    store
        .add_chat(&chat("15550100100@s.whatsapp.net", "Ann", ChatKind::Dm))
        .await
        .unwrap();
    store
        .change(
            group,
            ChatChange {
                status: Some(ChatStatus::Allowed),
                task: Some(7),
                room: Some(Some(RoomId::MAIN)),
                topics: Some(Some("books".to_owned())),
                ..ChatChange::default()
            },
            at(5),
        )
        .await
        .unwrap();
    let kept = store.chat(group).await.unwrap().unwrap();
    assert_eq!(kept.status, ChatStatus::Allowed);
    assert_eq!(kept.room, Some(RoomId::MAIN));
    assert_eq!(kept.topics.as_deref(), Some("books"));
    assert_eq!(kept.updated, at(5));
    assert_eq!(store.chats().await.unwrap()[0].jid, group, "allowed first");
    assert_eq!(store.chats_named("book club").await.unwrap().len(), 1);
    assert!(
        store.pending_on(7).await.unwrap().is_none(),
        "allowed already"
    );
    store
        .change(
            "15550100100@s.whatsapp.net",
            ChatChange {
                via: Some("9988@lid".to_owned()),
                task: Some(8),
                ..ChatChange::default()
            },
            at(6),
        )
        .await
        .unwrap();
    assert!(store.chat_via("9988@lid").await.unwrap().is_some());
    assert!(store.pending_on(8).await.unwrap().is_some());

    for n in 0..5 {
        store
            .keep(&wa(group, &format!("m{n}"), n, n % 2 == 0), 3)
            .await
            .unwrap();
    }
    let context = store.messages(group, 10).await.unwrap();
    assert_eq!(
        context.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
        ["m2", "m3", "m4"],
        "only the newest 3 stay"
    );
    assert_eq!(store.asks_since(at(0)).await.unwrap(), 2);
    let open = store.open_asks(group, at(0)).await.unwrap();
    assert_eq!(open[0].id, "m4", "the newest first");
    assert!(store.set_answered(group, "m4", true).await.unwrap());
    assert!(
        !store.set_answered(group, "m4", true).await.unwrap(),
        "claimed once"
    );
    assert!(store.is_answered(group, "m4").await.unwrap());
    assert!(store.set_answered(group, "m4", false).await.unwrap());

    let sent = store
        .log_sent(&NewWaSent {
            chat: group.to_owned(),
            name: Some("Book Club".to_owned()),
            at: at(10),
            text: "Friday works".to_owned(),
            msg: Some("out1".to_owned()),
            reply_to: Some("m4".to_owned()),
            room: Some(RoomId::MAIN),
        })
        .await
        .unwrap();
    assert_eq!(sent.text, "Friday works");
    assert_eq!(store.sent_count().await.unwrap(), 1);
    assert_eq!(store.sent_since(at(9)).await.unwrap(), 1);
    assert_eq!(store.sent_since(at(10)).await.unwrap(), 0);
    assert_eq!(store.last_sent_in(group).await.unwrap(), Some(at(10)));
    assert_eq!(store.last_sent_in("120363000099@g.us").await.unwrap(), None);
    assert_eq!(store.recent_sent(10).await.unwrap()[0].id, sent.id);
}
