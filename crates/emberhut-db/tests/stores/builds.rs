//! The newest builds, and the backup's snapshot of the database, against a real SQLite file.

use emberhut_core::RoomId;
use emberhut_core::builds::{BuildReport, LatestBuilds};
use emberhut_core::message::{Author, NewMessage};
use emberhut_db::{Database, DatabaseUrl};

use crate::common::{at, database, reopen};

#[expect(
    clippy::unwrap_used,
    reason = "a test helper: a failure here fails the test that called it"
)]
fn report(platform: &str, number: f64) -> BuildReport {
    BuildReport::new(platform, number).unwrap()
}

#[tokio::test]
async fn a_new_database_knows_no_builds() {
    let (_dir, db) = database().await;
    assert_eq!(db.builds().latest().await.unwrap(), LatestBuilds::default());
}

#[tokio::test]
async fn a_platform_s_build_only_moves_up_and_is_kept() {
    let (dir, db) = database().await;
    let builds = db.builds();
    let after = builds.report(report("ios", 12.0)).await.unwrap();
    assert_eq!(
        after,
        LatestBuilds {
            ios: Some(12),
            mac: None
        }
    );
    let older = builds.report(report("ios", 7.0)).await.unwrap();
    assert_eq!(older.ios, Some(12), "an older build changes nothing");
    builds.report(report("mac", 3.0)).await.unwrap();
    builds.report(report("ios", 13.0)).await.unwrap();

    let again = reopen(&dir).await;
    assert_eq!(
        again.builds().latest().await.unwrap(),
        LatestBuilds {
            ios: Some(13),
            mac: Some(3)
        }
    );
}

#[tokio::test]
async fn a_snapshot_is_a_whole_database_with_the_latest_commits() {
    let (dir, db) = database().await;
    db.messages()
        .append(
            NewMessage {
                room: RoomId::MAIN,
                author: Author::User,
                text: "kept in the backup".to_owned(),
                reply_to: None,
            },
            at(1),
        )
        .await
        .unwrap();
    db.builds().report(report("mac", 5.0)).await.unwrap();

    let backups = db.backups();
    assert_eq!(backups.file(), dir.path().join("emberhut.db"));
    let copy = dir.path().join("copy.db");
    backups.snapshot_into(&copy).await.unwrap();
    assert!(
        backups.snapshot_into(&copy).await.is_err(),
        "a snapshot never overwrites a file"
    );

    let url: DatabaseUrl = DatabaseUrl::from_path(&copy).unwrap();
    let restored = Database::open_existing(&url).await.unwrap();
    assert_eq!(restored.builds().latest().await.unwrap().mac, Some(5));
    let snapshot = restored.snapshots().read(RoomId::MAIN).await.unwrap();
    assert!(
        snapshot
            .chat
            .iter()
            .any(|message| message.text == "kept in the backup")
    );
}
