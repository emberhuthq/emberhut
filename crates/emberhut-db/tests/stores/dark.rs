//! The off-the-record room's own database: its chat, and a clear that leaves nothing behind.
#![expect(
    clippy::unwrap_used,
    reason = "a test helper reads the file: a failure fails the test"
)]

use emberhut_core::dark::DarkRole;
use emberhut_db::{DARK_FILE, DarkStore};
use sea_orm::{ConnectionTrait as _, Statement};

use crate::common::{at, database};

/// The tables a SQLite file at `path` has.
async fn tables(path: &std::path::Path) -> Vec<String> {
    let conn = sea_orm::Database::connect(format!(
        "{}?mode=ro",
        emberhut_db::DatabaseUrl::from_path(path).unwrap()
    ))
    .await
    .unwrap();
    let rows = conn
        .query_all_raw(Statement::from_string(
            sea_orm::DatabaseBackend::Sqlite,
            "SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name",
        ))
        .await
        .unwrap();
    rows.iter()
        .map(|row| row.try_get::<String>("", "name").unwrap())
        .collect()
}

#[tokio::test]
async fn dark_messages_live_in_the_room_s_file_and_not_in_the_server_s() {
    let (dir, _db) = database().await;
    let path = dir.path().join(DARK_FILE);
    let store = DarkStore::open(&path).await.unwrap();
    store.add(DarkRole::User, "hi", at(0)).await.unwrap();
    let theirs = tables(&path).await;
    assert!(theirs.contains(&"dark_messages".to_owned()), "{theirs:?}");
    assert!(
        !theirs.contains(&"messages".to_owned()),
        "no other table: {theirs:?}"
    );
    let servers = tables(&dir.path().join("emberhut.db")).await;
    assert!(
        !servers.contains(&"dark_messages".to_owned()),
        "{servers:?}"
    );
}

#[tokio::test]
async fn the_room_keeps_its_chat_in_its_own_file_and_a_clear_leaves_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(DARK_FILE);
    let store = DarkStore::open(&path).await.unwrap();
    store
        .add(DarkRole::User, "a very private word", at(0))
        .await
        .unwrap();
    store
        .add(DarkRole::Assistant, "noted", at(1))
        .await
        .unwrap();
    let lines = store.lines().await.unwrap();
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[1].role, DarkRole::Assistant);
    assert_eq!(lines[0].at, at(0));
    assert!(
        !dir.path().join("dark.db-wal").exists(),
        "no write-ahead log beside it"
    );

    store.clear().await.unwrap();
    assert_eq!(store.lines().await.unwrap(), Vec::new());
    let bytes = std::fs::read(&path).unwrap();
    let word = b"a very private word";
    assert!(
        !bytes.windows(word.len()).any(|window| window == word),
        "the cleared words are gone from the file"
    );
    let again = DarkStore::open(&path).await.unwrap();
    assert_eq!(again.lines().await.unwrap(), Vec::new());
}
