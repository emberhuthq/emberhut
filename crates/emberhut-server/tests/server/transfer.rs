//! Files between the server and its runners' machines: the person's files put into a session's
//! inbox with their reply, and a file brought from a runner's machine into the uploads folder
//! (once, while it is unchanged), served from its own folder.

use emberhut_db::{Database, RemoteFile};
use serde_json::json;

#[cfg(unix)]
use emberhut_server::app::Dependencies;
#[cfg(unix)]
use emberhut_server::config::AuthMode;
use emberhut_server::services::runner_api::by;
use emberhut_server::services::runners::SessionId;

use crate::common::{Call, PRIVATE_FILES, TestApp};

const BUILD: &str = "token-of-build";

async fn app() -> TestApp {
    let app = TestApp::new().await;
    app.runners.on(&[("build", "Build box", BUILD)]);
    let event = json!({
        "machine": "build", "kind": "activity", "at": "2026-11-25T10:00:00.000Z",
        "session": { "id": "r-1a2b", "title": "Session", "project": "", "status": "idle" },
    });
    let answer = app
        .call(
            Call::post("/api/runner/events")
                .header("authorization", &format!("Bearer {BUILD}"))
                .json(&event),
        )
        .await;
    assert_eq!(answer.status, 200);
    app
}

#[tokio::test]
pub(crate) async fn a_reply_puts_its_files_in_the_sessions_inbox_first() {
    let app = app().await;
    let session = app.sign_in().await;
    let stored = app
        .call(
            Call::post("/api/uploads")
                .query("name=plan.txt")
                .token(&session)
                .raw("the plan"),
        )
        .await;
    assert_eq!(stored.status, 200, "{}", stored.text);
    let file = stored.json()["file"].clone();
    let answer = app
        .call(
            Call::post("/api/runner/sessions/r-1a2b/send")
                .token(&session)
                .json(&json!({ "text": "", "files": [file] })),
        )
        .await;
    assert_eq!(answer.status, 200, "{}", answer.text);
    let uploads = app.runners.uploads();
    assert_eq!(uploads.len(), 1);
    let (machine, path, bytes) = &uploads[0];
    assert_eq!(machine, "build");
    assert!(
        path.starts_with(&format!(
            "/sessions/r-1a2b/files?name=plan.txt&by={}&size=8",
            by::PERSON
        )),
        "{path}"
    );
    assert_eq!(bytes, b"the plan");
    let sent = &app.runners.requests_to("/sessions/r-1a2b/send")[0];
    let body = sent.body.as_ref().unwrap();
    assert_eq!(body["prompt"], "(see the files)");
    assert_eq!(body["files"][0]["name"], "plan.txt");
}

#[tokio::test]
pub(crate) async fn a_file_outside_the_allowed_folders_is_never_sent() {
    let app = app().await;
    let secrets = app.dir().join("secrets");
    std::fs::create_dir_all(&secrets).unwrap();
    std::fs::write(secrets.join("key"), "secret").unwrap();
    let transfer = app.state.transfer();
    let session = SessionId::parse("r-1a2b").unwrap();
    let refused = transfer
        .push(
            "build",
            &session,
            &secrets.join("key").to_string_lossy(),
            by::ASSISTANT,
        )
        .await
        .unwrap_err();
    assert!(
        refused.to_string().contains("isn't one to send"),
        "{refused}"
    );
    let home = app.dir().join("home");
    std::fs::create_dir_all(home.join(".ssh")).unwrap();
    std::fs::write(home.join(".ssh/id"), "key").unwrap();
    let refused = transfer
        .push("build", &session, "~/.ssh/id", by::ASSISTANT)
        .await
        .unwrap_err();
    assert!(
        refused.to_string().contains("isn't one to send"),
        "{refused}"
    );
    let refused = transfer
        .push("build", &session, "~/../x", by::ASSISTANT)
        .await
        .unwrap_err();
    assert_eq!(refused.to_string(), "no .. in paths");
    std::fs::write(home.join("notes.md"), "notes").unwrap();
    let pushed = transfer
        .push("build", &session, "~/notes.md", by::ASSISTANT)
        .await
        .unwrap();
    assert_eq!(pushed.path, "/work/inbox/landed");
    assert!(
        app.runners
            .uploads()
            .iter()
            .all(|(_, _, bytes)| bytes != b"secret")
    );
}

/// An app whose home and secrets are reached through a link and not made yet, as macOS's
/// temporary folders are (`/tmp` is `/private/tmp`, `/var/folders` is `/private/var/folders`);
/// its home's path through the link, and its real path.
#[cfg(unix)]
#[expect(
    clippy::unwrap_used,
    reason = "a test helper: a failure to make the folders fails the test"
)]
async fn linked_app() -> (TestApp, std::path::PathBuf, std::path::PathBuf) {
    let app = TestApp::with_dependencies(AuthMode::Enforce, |deps, dir| {
        std::fs::create_dir(dir.join("real")).unwrap();
        std::os::unix::fs::symlink(dir.join("real"), dir.join("link")).unwrap();
        Dependencies {
            home: Some(dir.join("link/home")),
            secrets_dir: dir.join("link/secrets"),
            ..deps
        }
    })
    .await;
    app.runners.on(&[("build", "Build box", BUILD)]);
    let event = json!({
        "machine": "build", "kind": "activity", "at": "2026-11-25T10:00:00.000Z",
        "session": { "id": "r-1a2b", "title": "Session", "project": "", "status": "idle" },
    });
    let answer = app
        .call(
            Call::post("/api/runner/events")
                .header("authorization", &format!("Bearer {BUILD}"))
                .json(&event),
        )
        .await;
    assert_eq!(answer.status, 200);
    let home = app.dir().join("link/home");
    let real = app.dir().canonicalize().unwrap().join("real/home");
    (app, home, real)
}

#[cfg(unix)]
#[tokio::test]
pub(crate) async fn a_private_folder_reached_through_a_link_and_made_later_is_never_sent() {
    let (app, home, real) = linked_app().await;
    let secrets = app.dir().join("link/secrets");
    std::fs::create_dir_all(home.join(".ssh")).unwrap();
    std::fs::create_dir_all(&secrets).unwrap();
    std::fs::write(home.join(".ssh/id"), "secret").unwrap();
    std::fs::write(secrets.join("key"), "secret").unwrap();
    std::fs::write(home.join("notes.md"), "notes").unwrap();
    assert_ne!(
        home.canonicalize().unwrap(),
        home,
        "the home is reached through a link"
    );

    for path in [
        home.join(".ssh/id"),
        real.join(".ssh/id"),
        secrets.join("key"),
        app.dir().join("real/secrets/key"),
    ] {
        assert!(refused(&app, &path).await, "{} was sent", path.display());
    }
    let transfer = app.state.transfer();
    let session = SessionId::parse("r-1a2b").unwrap();
    let answer = transfer
        .push("build", &session, "~/.ssh/id", by::ASSISTANT)
        .await;
    assert!(answer.is_err(), "~/.ssh/id was sent");
    for path in [home.join("notes.md"), real.join("notes.md")] {
        assert!(
            !refused(&app, &path).await,
            "{} was refused",
            path.display()
        );
    }
    assert!(
        app.runners
            .uploads()
            .iter()
            .all(|(_, _, bytes)| bytes != b"secret")
    );
}

/// Whether `transfer` refuses to send `path` as not one to send.
#[expect(
    clippy::unwrap_used,
    reason = "a test helper: the id is a session's, and a failure fails the test"
)]
async fn refused(app: &TestApp, path: &std::path::Path) -> bool {
    let session = SessionId::parse("r-1a2b").unwrap();
    let answer = app
        .state
        .transfer()
        .push("build", &session, &path.to_string_lossy(), by::ASSISTANT)
        .await;
    matches!(answer, Err(error) if error.to_string().contains("isn't one to send"))
}

#[tokio::test]
pub(crate) async fn the_servers_own_database_keys_and_tokens_are_never_sent() {
    let app = app().await;
    // The app's folder is in /tmp, where files may come from: only the private list keeps
    // these in.
    let dir = app.dir().to_path_buf();
    for name in ["emberhut.db-wal", "emberhut.db-shm", "machine.key"]
        .into_iter()
        .chain(PRIVATE_FILES)
    {
        std::fs::write(dir.join(name), "secret").unwrap();
    }
    for name in [
        "emberhut.db",
        "emberhut.db-wal",
        "emberhut.db-shm",
        "machine.key",
    ]
    .into_iter()
    .chain(PRIVATE_FILES)
    {
        assert!(refused(&app, &dir.join(name)).await, "{name} was sent");
    }
    std::fs::create_dir_all(dir.join("home")).unwrap();
    std::fs::write(dir.join("home/notes.md"), "notes").unwrap();
    assert!(!refused(&app, &dir.join("home/notes.md")).await);
    assert!(
        app.runners
            .uploads()
            .iter()
            .all(|(_, _, bytes)| bytes != b"secret")
    );
}

#[cfg(unix)]
#[tokio::test]
pub(crate) async fn a_link_to_a_private_folder_is_never_followed_out() {
    use std::os::unix::fs::symlink;

    let app = app().await;
    let home = app.dir().join("home");
    let secrets = app.dir().join("secrets");
    std::fs::create_dir_all(home.join(".ssh")).unwrap();
    std::fs::create_dir_all(&secrets).unwrap();
    std::fs::write(home.join(".ssh/id"), "secret").unwrap();
    std::fs::write(secrets.join("key"), "secret").unwrap();
    // A link in /tmp to the home's keys, a link in home to the secrets folder, and a link in
    // home to the database.
    let in_tmp = tempfile::tempdir_in("/tmp").unwrap();
    symlink(home.join(".ssh"), in_tmp.path().join("keys")).unwrap();
    symlink(home.join(".ssh/id"), in_tmp.path().join("id")).unwrap();
    symlink(&secrets, home.join("kept")).unwrap();
    symlink(app.dir().join("emberhut.db"), home.join("db")).unwrap();
    for path in [
        in_tmp.path().join("keys/id"),
        in_tmp.path().join("id"),
        home.join("kept/key"),
        home.join("db"),
    ] {
        assert!(refused(&app, &path).await, "{} was sent", path.display());
    }
    let transfer = app.state.transfer();
    let session = SessionId::parse("r-1a2b").unwrap();
    for path in ["~/kept/key", "~/db"] {
        let answer = transfer.push("build", &session, path, by::ASSISTANT).await;
        assert!(answer.is_err(), "{path} was sent");
    }
    assert_eq!(app.runners.uploads().len(), 0, "nothing was sent");
}

#[tokio::test]
pub(crate) async fn a_file_is_brought_once_and_served_from_its_own_folder() {
    let app = app().await;
    app.runners.answer(
        "build",
        "/files?",
        json!({ "path": "/work/out/report.pdf", "size": 6, "mtime": "2026-11-25T09:00:00.000Z" }),
    );
    app.runners.file("/tmp/report.pdf", b"%PDF-1");
    let transfer = app.state.transfer();
    let pulled = transfer
        .pull(
            "build",
            "/tmp/report.pdf",
            by::ASSISTANT,
            "a look",
            Some("r-1a2b"),
        )
        .await
        .unwrap();
    assert_eq!(pulled.name, "report.pdf");
    assert_eq!(pulled.size, 6);
    assert!(pulled.local.starts_with("build-"), "{}", pulled.local);
    assert!(
        pulled.url().contains("%2F"),
        "one path segment: {}",
        pulled.url()
    );
    let again = transfer
        .pull("build", "/tmp/report.pdf", by::ASSISTANT, "a look", None)
        .await
        .unwrap();
    assert_eq!(
        again.local, pulled.local,
        "the same file, unchanged, comes once"
    );
    let downloads = app
        .runners
        .requests()
        .into_iter()
        .filter(|request| request.path.starts_with("/files?") && !request.path.contains("stat=1"))
        .count();
    assert_eq!(downloads, 1);

    let db = Database::connect(&app.url).await.unwrap();
    let remote = RemoteFile {
        machine: "build".to_owned(),
        path: "/work/out/report.pdf".to_owned(),
        size: 6,
        mtime: "2026-11-25T09:00:00.000Z".to_owned(),
    };
    assert_eq!(
        db.pulls().find(&remote).await.unwrap(),
        Some(pulled.local.clone())
    );

    let session = app.sign_in().await;
    let answer = app
        .send(Call::get(&pulled.url()).token(&session).request())
        .await;
    assert_eq!(answer.status(), 200);
    let answer = app
        .send(
            Call::get("/uploads/build-1a2b3c4d%2F..%2Femberhut.db")
                .token(&session)
                .request(),
        )
        .await;
    assert_eq!(answer.status(), 404);
}

#[cfg(unix)]
#[tokio::test]
pub(crate) async fn a_brought_files_folder_that_is_a_link_is_never_served() {
    use std::os::unix::fs::symlink;

    let app = app().await;
    let session = app.sign_in().await;
    // A real folder of the uploads folder is served; a link of the same shape is not, even to
    // a folder inside the uploads folder.
    let uploads = app.state.uploads().dir().to_path_buf();
    std::fs::create_dir_all(uploads.join("build-1a2b3c4d")).unwrap();
    std::fs::write(uploads.join("build-1a2b3c4d/report.pdf"), "%PDF-1").unwrap();
    let outside = app.dir().join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("report.pdf"), "secret").unwrap();
    symlink(&outside, uploads.join("build-0000aaaa")).unwrap();
    symlink(
        uploads.join("build-1a2b3c4d"),
        uploads.join("build-1111bbbb"),
    )
    .unwrap();
    let served = app
        .send(
            Call::get("/uploads/build-1a2b3c4d%2Freport.pdf")
                .token(&session)
                .request(),
        )
        .await;
    assert_eq!(served.status(), 200);
    for link in ["build-0000aaaa", "build-1111bbbb"] {
        let answer = app
            .send(
                Call::get(&format!("/uploads/{link}%2Freport.pdf"))
                    .token(&session)
                    .request(),
            )
            .await;
        assert_eq!(answer.status(), 404, "{link} was served");
    }
}
