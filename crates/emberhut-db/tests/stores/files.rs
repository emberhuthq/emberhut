//! The chat's files (`files`, `message_files`) against a real SQLite database.

use crate::common::{at, database};
use emberhut_core::message::{Author, HistoryQuery, NewMessage};
use emberhut_core::upload::{FileKind, NewFile};
use emberhut_core::{FileId, MessageId, RoomId};

fn file(stored_name: &str, name: &str) -> NewFile {
    NewFile {
        stored_name: stored_name.to_owned(),
        name: name.to_owned(),
        kind: FileKind::of(name),
        size: 42,
    }
}

fn said(text: &str, reply_to: Option<MessageId>) -> NewMessage {
    NewMessage {
        room: RoomId::MAIN,
        author: Author::User,
        text: text.to_owned(),
        reply_to,
    }
}

#[tokio::test]
async fn a_file_is_noted_and_found_by_its_stored_name() {
    let (_dir, db) = database().await;
    let files = db.files();
    let stored = files
        .add(file("0a1b2c3d-photo.png", "photo.png"), at(1))
        .await
        .unwrap();
    assert_eq!(stored.kind, FileKind::Image);
    assert_eq!(stored.size, 42);
    assert_eq!(stored.created_at, at(1));
    assert_eq!(
        files.by_stored_name("0a1b2c3d-photo.png").await.unwrap(),
        Some(stored)
    );
    assert_eq!(files.by_stored_name("photo.png").await.unwrap(), None);
    assert!(
        files
            .add(file("0a1b2c3d-photo.png", "again.png"), at(2))
            .await
            .is_err(),
        "a stored name is noted once"
    );
}

#[tokio::test]
async fn a_message_keeps_its_files_in_order() {
    let (_dir, db) = database().await;
    let files = db.files();
    let clip = files
        .add(file("1-clip.mov", "clip.mov"), at(0))
        .await
        .unwrap();
    let notes = files
        .add(file("2-notes.pdf", "notes.pdf"), at(0))
        .await
        .unwrap();
    let messages = db.messages();
    let with = messages
        .append_with_files(said("", None), &[notes.id, clip.id], at(1))
        .await
        .unwrap();
    let without = messages.append(said("plain", None), at(2)).await.unwrap();
    let found = files.of_messages(&[with.id, without.id]).await.unwrap();
    assert_eq!(found.len(), 1, "a message without files has no entry");
    assert_eq!(found[&with.id], [notes, clip]);
}

#[tokio::test]
async fn a_message_with_a_file_that_is_not_there_is_not_stored() {
    let (_dir, db) = database().await;
    let messages = db.messages();
    assert!(
        messages
            .append_with_files(said("hello", None), &[FileId::new(99)], at(1))
            .await
            .is_err()
    );
    let page = messages
        .page(RoomId::MAIN, HistoryQuery::default())
        .await
        .unwrap();
    assert!(page.is_empty(), "nothing is stored");
}

#[tokio::test]
async fn a_history_page_brings_its_quotes_and_their_files() {
    let (_dir, db) = database().await;
    let photo = db
        .files()
        .add(file("1-photo.png", "photo.png"), at(0))
        .await
        .unwrap();
    let messages = db.messages();
    let quoted = messages
        .append_with_files(said("look", None), &[photo.id], at(0))
        .await
        .unwrap();
    for n in 0..3 {
        messages
            .append(said(&format!("message {n}"), None), at(1))
            .await
            .unwrap();
    }
    let reply = messages
        .append(said("nice", Some(quoted.id)), at(2))
        .await
        .unwrap();
    let page = db
        .snapshots()
        .history(
            RoomId::MAIN,
            HistoryQuery {
                before: None,
                after: Some(MessageId::new(quoted.id.get() + 1)),
            },
        )
        .await
        .unwrap();
    assert_eq!(page.chat.last().map(|m| m.id), Some(reply.id));
    assert!(page.chat.iter().all(|m| m.id != quoted.id));
    assert_eq!(page.quoted, std::slice::from_ref(&quoted));
    assert_eq!(page.files[&quoted.id], std::slice::from_ref(&photo));

    let snapshot = db.snapshots().read(RoomId::MAIN).await.unwrap();
    assert_eq!(snapshot.files[&quoted.id], [photo]);
}
