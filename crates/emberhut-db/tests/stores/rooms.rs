//! Rooms and messages against a real SQLite database.

use crate::common::{at, database};
use emberhut_core::RoomId;
use emberhut_core::message::{Author, HistoryQuery, NewMessage, PAGE_SIZE};

fn said(text: &str) -> NewMessage {
    NewMessage {
        room: RoomId::MAIN,
        author: Author::User,
        text: text.to_owned(),
        reply_to: None,
    }
}

#[tokio::test]
async fn every_database_starts_with_main() {
    let (_dir, db) = database().await;
    let rooms = db.rooms().list().await.unwrap();
    assert_eq!(rooms.len(), 1);
    assert_eq!(rooms[0].id, RoomId::MAIN);
    assert_eq!(rooms[0].name, "Main");
    assert_eq!(rooms[0].seen_at, None);
    assert_eq!(rooms[0].seen_through, None);
}

#[tokio::test]
async fn marking_a_room_seen() {
    let (_dir, db) = database().await;
    let rooms = db.rooms();
    assert!(rooms.mark_seen(RoomId::MAIN, at(1)).await.unwrap());
    assert_eq!(
        rooms.get(RoomId::MAIN).await.unwrap().unwrap().seen_at,
        Some(at(1))
    );
    assert!(!rooms.mark_seen(RoomId::new(99), at(1)).await.unwrap());
    assert_eq!(rooms.get(RoomId::new(99)).await.unwrap(), None);
}

#[tokio::test]
async fn a_message_is_stored_as_said() {
    let (_dir, db) = database().await;
    let messages = db.messages();
    let first = messages.append(said("hello"), at(0)).await.unwrap();
    let reply = messages
        .append(
            NewMessage {
                author: Author::Assistant,
                reply_to: Some(first.id),
                ..said("hi")
            },
            at(1),
        )
        .await
        .unwrap();
    assert_eq!(reply.room, RoomId::MAIN);
    assert_eq!(reply.author, Author::Assistant);
    assert_eq!(reply.reply_to, Some(first.id));
    assert_eq!(reply.created_at, at(1));
    assert!(reply.id > first.id);
}

#[tokio::test]
async fn a_message_needs_its_room() {
    let (_dir, db) = database().await;
    let nowhere = NewMessage {
        room: RoomId::new(99),
        ..said("hello")
    };
    assert!(db.messages().append(nowhere, at(0)).await.is_err());
}

#[tokio::test]
async fn history_pages_newest_first_and_reads_oldest_first() {
    let (_dir, db) = database().await;
    let messages = db.messages();
    let count = PAGE_SIZE + 5;
    let mut ids = Vec::new();
    for n in 0..count {
        let minute = i64::try_from(n).unwrap();
        ids.push(
            messages
                .append(said(&n.to_string()), at(minute))
                .await
                .unwrap()
                .id,
        );
    }

    let latest = messages
        .page(RoomId::MAIN, HistoryQuery::default())
        .await
        .unwrap();
    assert_eq!(latest.len(), 60);
    assert_eq!(latest.first().unwrap().id, ids[5]);
    assert_eq!(latest.last().unwrap().id, *ids.last().unwrap());
    assert!(messages.has_before(RoomId::MAIN, ids[5]).await.unwrap());
    assert!(!messages.has_before(RoomId::MAIN, ids[0]).await.unwrap());

    let earlier = messages
        .page(
            RoomId::MAIN,
            HistoryQuery {
                before: Some(ids[5]),
                after: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(
        earlier.iter().map(|m| m.id).collect::<Vec<_>>(),
        ids[..5].to_vec()
    );

    let since = messages
        .page(
            RoomId::MAIN,
            HistoryQuery {
                before: None,
                after: Some(ids[62]),
            },
        )
        .await
        .unwrap();
    assert_eq!(
        since.iter().map(|m| m.text.as_str()).collect::<Vec<_>>(),
        ["63", "64"]
    );
}

#[tokio::test]
async fn messages_are_read_back_by_id_and_the_last_said_skips_events() {
    let (_dir, db) = database().await;
    let messages = db.messages();
    let first = messages.append(said("hello"), at(0)).await.unwrap();
    let answer = messages
        .append(
            NewMessage {
                author: Author::Assistant,
                ..said("hi")
            },
            at(1),
        )
        .await
        .unwrap();
    messages
        .append(
            NewMessage {
                author: Author::Event,
                ..said("a note")
            },
            at(2),
        )
        .await
        .unwrap();

    assert_eq!(messages.get(first.id).await.unwrap(), Some(first.clone()));
    assert_eq!(
        messages
            .get(emberhut_core::MessageId::new(99))
            .await
            .unwrap(),
        None
    );
    let both = messages
        .get_many(&[answer.id, emberhut_core::MessageId::new(99), first.id])
        .await
        .unwrap();
    assert_eq!(both, [first.clone(), answer.clone()]);
    assert_eq!(messages.get_many(&[]).await.unwrap(), []);
    assert_eq!(
        messages.last_said(RoomId::MAIN).await.unwrap(),
        Some(answer)
    );
}

fn answer(text: &str) -> NewMessage {
    NewMessage {
        author: Author::Assistant,
        ..said(text)
    }
}

#[tokio::test]
async fn unread_counts_the_assistant_s_messages_after_the_newest_one_seen() {
    let (_dir, db) = database().await;
    let messages = db.messages();
    messages.append(said("hello"), at(0)).await.unwrap();
    let first = messages.append(answer("hi"), at(1)).await.unwrap();
    messages.append(answer("and"), at(3)).await.unwrap();
    assert_eq!(messages.count_unread(RoomId::MAIN, None).await.unwrap(), 2);
    assert_eq!(
        messages
            .count_unread(RoomId::MAIN, Some(first.id))
            .await
            .unwrap(),
        1
    );
}

#[tokio::test]
async fn seeing_a_room_sees_every_message_in_it_so_far() {
    let (_dir, db) = database().await;
    let messages = db.messages();
    let rooms = db.rooms();
    messages.append(answer("hi"), at(0)).await.unwrap();
    assert!(rooms.mark_seen(RoomId::MAIN, at(0)).await.unwrap());
    let main = rooms.get(RoomId::MAIN).await.unwrap().unwrap();
    assert_eq!(
        messages
            .count_unread(RoomId::MAIN, main.seen_through)
            .await
            .unwrap(),
        0
    );

    // A reply stored in the same millisecond as the `seen` is still unread: ids, not times.
    let same_moment = messages.append(answer("again"), at(0)).await.unwrap();
    assert_eq!(
        messages
            .count_unread(RoomId::MAIN, main.seen_through)
            .await
            .unwrap(),
        1
    );
    assert!(main.seen_through < Some(same_moment.id));
}

#[tokio::test]
async fn a_snapshot_reads_every_part_of_the_state() {
    let (_dir, db) = database().await;
    let messages = db.messages();
    let first = messages.append(said("hello"), at(0)).await.unwrap();
    messages
        .append(
            NewMessage {
                reply_to: Some(first.id),
                ..answer("hi")
            },
            at(1),
        )
        .await
        .unwrap();
    let snapshot = db.snapshots().read(RoomId::MAIN).await.unwrap();
    assert_eq!(snapshot.rooms.len(), 1);
    assert_eq!(snapshot.rooms[0].unread, 1);
    assert_eq!(snapshot.rooms[0].last.as_ref().unwrap().text, "hi");
    assert_eq!(snapshot.chat.len(), 2);
    assert!(!snapshot.earlier);
    assert_eq!(snapshot.quoted, [first]);
}

#[tokio::test]
async fn a_room_s_conversation_since_a_time_has_its_events_too() {
    let (_dir, db) = database().await;
    let messages = db.messages();
    messages.append(said("before"), at(0)).await.unwrap();
    let event = NewMessage {
        author: Author::Event,
        ..said("[Slack] Example person: hello")
    };
    messages.append(event, at(2)).await.unwrap();
    let answer = NewMessage {
        author: Author::Assistant,
        ..said("hi")
    };
    messages.append(answer, at(3)).await.unwrap();
    messages.append(said(""), at(4)).await.unwrap();
    let texts = |found: Vec<emberhut_core::message::Message>| -> Vec<String> {
        found.into_iter().map(|message| message.text).collect()
    };
    assert_eq!(
        texts(
            messages
                .conversation_since(RoomId::MAIN, at(1), 10)
                .await
                .unwrap()
        ),
        ["[Slack] Example person: hello", "hi"]
    );
    assert_eq!(
        texts(
            messages
                .conversation_since(RoomId::MAIN, at(1), 1)
                .await
                .unwrap()
        ),
        ["hi"]
    );
    assert_eq!(
        texts(messages.said_since(RoomId::MAIN, at(1), 10).await.unwrap()),
        ["hi"],
        "said_since leaves the events out"
    );
}
