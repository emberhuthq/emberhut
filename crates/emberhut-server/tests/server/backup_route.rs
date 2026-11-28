//! `GET /api/backup`: the database's snapshot and the data folder's files, as one `.tar.gz`.
#![expect(
    clippy::unwrap_used,
    clippy::panic,
    reason = "a test helper unpacks the archive: a failure there fails the test"
)]

use std::collections::BTreeMap;
use std::io::Read as _;

use emberhut_api::paths;
use emberhut_core::RoomId;
use emberhut_db::{Database, DatabaseUrl};
use emberhut_server::config::AuthMode;
use flate2::read::GzDecoder;
use serde_json::json;

use crate::common::{Call, TestApp};

/// Every regular file in the archive `bytes`, by path, with its bytes; and the folders.
fn unpack(bytes: &[u8]) -> (BTreeMap<String, Vec<u8>>, Vec<String>) {
    let mut archive = tar::Archive::new(GzDecoder::new(bytes));
    let mut files = BTreeMap::new();
    let mut folders = Vec::new();
    for entry in archive.entries().unwrap() {
        let mut entry = entry.unwrap();
        let path = entry.path().unwrap().to_string_lossy().into_owned();
        match entry.header().entry_type() {
            tar::EntryType::Regular => {
                let mut body = Vec::new();
                entry.read_to_end(&mut body).unwrap();
                files.insert(path, body);
            }
            tar::EntryType::Directory => folders.push(path),
            other => panic!("{path} is a {other:?}"),
        }
    }
    (files, folders)
}

#[tokio::test]
pub(crate) async fn a_signed_in_device_gets_the_database_and_the_files() {
    let app = TestApp::new().await;
    let session = app.sign_in().await;
    app.assistant.answers(&[], Some("Noted."));
    let said = app
        .call(
            Call::post(paths::TALK)
                .token(&session)
                .json(&json!({ "text": "back this up" })),
        )
        .await;
    assert_eq!(said.status, 200);
    app.settle().await;

    let data = app.url.path().parent().unwrap().to_path_buf();
    std::fs::create_dir_all(data.join("uploads/2026")).unwrap();
    std::fs::write(data.join("uploads/2026/notes.txt"), b"some notes").unwrap();
    std::fs::create_dir_all(data.join("files")).unwrap();
    std::fs::write(data.join("files/empty.bin"), b"").unwrap();
    std::fs::write(data.join("not-kept.txt"), b"outside the kept entries").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink("/etc/hostname", data.join("uploads/link")).unwrap();

    let response = app
        .send(
            Call::get(paths::BACKUP)
                .token(&session)
                .header("accept-encoding", "gzip")
                .request(),
        )
        .await;
    assert_eq!(response.status(), 200);
    let headers = response.headers().clone();
    assert_eq!(headers["content-type"], "application/gzip");
    assert!(
        headers.get("content-encoding").is_none(),
        "a .tar.gz is not gzipped again"
    );
    let disposition = headers["content-disposition"].to_str().unwrap();
    assert!(
        disposition.starts_with("attachment; filename=\"emberhut-backup-")
            && disposition.ends_with(".tar.gz\""),
        "{disposition}"
    );
    let bytes = http_body_util::BodyExt::collect(response.into_body())
        .await
        .unwrap()
        .to_bytes();

    let (files, folders) = unpack(&bytes);
    assert_eq!(
        files.keys().map(String::as_str).collect::<Vec<_>>(),
        ["emberhut.db", "files/empty.bin", "uploads/2026/notes.txt"],
        "the snapshot and the kept files; never a link"
    );
    assert_eq!(files["uploads/2026/notes.txt"], b"some notes");
    assert!(folders.contains(&"uploads/2026".to_owned()));

    // The snapshot is a whole database: it opens as one and has what was said.
    let restored = tempfile::tempdir().unwrap();
    let file = restored.path().join("emberhut.db");
    std::fs::write(&file, &files["emberhut.db"]).unwrap();
    let url: DatabaseUrl = DatabaseUrl::from_path(&file).unwrap();
    let db = Database::open_existing(&url).await.unwrap();
    let chat = db.snapshots().read(RoomId::MAIN).await.unwrap().chat;
    let texts: Vec<&str> = chat.iter().map(|message| message.text.as_str()).collect();
    assert_eq!(texts, ["back this up", "Noted."]);
}

#[tokio::test]
pub(crate) async fn only_a_signed_in_device_gets_the_backup_even_in_report_mode() {
    for mode in [AuthMode::Enforce, AuthMode::Report] {
        let app = TestApp::with_mode(mode).await;
        let refused = app.call(Call::get(paths::BACKUP)).await;
        assert_eq!(refused.status, 401, "{mode:?}");
        assert_eq!(
            refused.json(),
            json!({ "error": "Sign in first.", "signIn": true }),
            "{mode:?}"
        );
    }
}

#[tokio::test]
pub(crate) async fn one_backup_is_made_at_a_time() {
    let app = TestApp::new().await;
    let session = app.sign_in().await;
    let first = app
        .send(Call::get(paths::BACKUP).token(&session).request())
        .await;
    assert_eq!(first.status(), 200);

    // While the first is being sent, another is refused rather than a second copy made.
    let second = app.call(Call::get(paths::BACKUP).token(&session)).await;
    assert_eq!(second.status, 503);
    assert_eq!(
        second.json(),
        json!({ "error": "A backup is being made already. Try again when it is done." })
    );
    http_body_util::BodyExt::collect(first.into_body())
        .await
        .unwrap();
    let third = app
        .send(Call::get(paths::BACKUP).token(&session).request())
        .await;
    assert_eq!(third.status(), 200, "the first is done");
}

#[tokio::test]
async fn a_snapshot_a_crash_left_behind_is_removed_by_the_next_backup() {
    let app = TestApp::new().await;
    let session = app.sign_in().await;
    let data = app.url.path().parent().unwrap().to_path_buf();
    let leftover = data.join(format!(
        "{}left",
        emberhut_server::services::backup::STAGING_PREFIX
    ));
    std::fs::create_dir_all(&leftover).unwrap();
    std::fs::write(leftover.join("emberhut.db"), b"an old copy").unwrap();
    let answer = app
        .send(Call::get(paths::BACKUP).token(&session).request())
        .await;
    assert_eq!(answer.status(), 200);
    assert!(
        !leftover.exists(),
        "removed before the new snapshot was made"
    );
}

#[tokio::test]
pub(crate) async fn the_off_the_record_room_is_never_in_the_backup() {
    let app = TestApp::new().await;
    let session = app.sign_in().await;
    app.dark_local.replies(&[Ok("Off the record.")]);
    let said = app
        .send(
            Call::post("/api/dark")
                .token(&session)
                .json(&json!({ "text": "just between us" }))
                .request(),
        )
        .await;
    assert_eq!(said.status(), 200);
    let streamed = http_body_util::BodyExt::collect(said.into_body())
        .await
        .unwrap()
        .to_bytes();
    assert!(String::from_utf8_lossy(&streamed).contains("Off the record."));
    let data = app.url.path().parent().unwrap().to_path_buf();
    assert!(
        data.join(emberhut_db::DARK_FILE).exists(),
        "the room keeps its chat"
    );

    let response = app
        .send(Call::get(paths::BACKUP).token(&session).request())
        .await;
    assert_eq!(response.status(), 200);
    let bytes = http_body_util::BodyExt::collect(response.into_body())
        .await
        .unwrap()
        .to_bytes();
    let (files, _) = unpack(&bytes);
    assert!(
        files.keys().all(|path| !path.contains("dark")),
        "{:?}",
        files.keys().collect::<Vec<_>>()
    );
    assert!(files.contains_key("emberhut.db"));
}
