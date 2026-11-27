//! `POST /api/uploads` and `GET /uploads/{path}`: storing a file for the chat, serving it whole
//! or in byte ranges, and sending it with a message.
#![expect(
    clippy::unwrap_used,
    reason = "test helpers send requests: a failure there fails the test"
)]

use axum::body::Body;
use axum::http::{Method, Request};
use emberhut_api::chat::{ChatFile, FileKind};
use emberhut_api::error::ErrorBody;
use emberhut_api::state::State;
use emberhut_api::uploads::UploadReply;
use emberhut_core::session::Session;
use emberhut_server::services::uploads::describe;
use serde_json::{Value, json};

use crate::common::{Answer, Call, TestApp, assert_keys_within, check_against_document, fixture};

/// The documented path of a stored file.
const FILE_ROUTE: &str = "/uploads/{path}";

/// `POST /api/uploads?<query>` with `body` as the raw file, read and checked against the
/// document.
async fn upload(app: &TestApp, session: Option<&Session>, query: &str, body: &str) -> Answer {
    let mut request = Request::post(format!("/api/uploads?{query}"))
        .header("content-type", "application/octet-stream")
        .header("content-length", body.len());
    if let Some(session) = session {
        request = request.header(
            "authorization",
            format!("Bearer {}", session.tokens.access.as_str()),
        );
    }
    let response = app
        .send(request.body(Body::from(body.to_owned())).unwrap())
        .await;
    let answer = Answer::read(response).await;
    check_against_document(&Method::POST, "/api/uploads", &answer);
    answer
}

/// `GET <url>` of a stored file, with these headers, checked against the document.
async fn fetch(app: &TestApp, session: &Session, url: &str, headers: &[(&str, &str)]) -> Answer {
    let mut request = Request::get(url).header(
        "authorization",
        format!("Bearer {}", session.tokens.access.as_str()),
    );
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    let response = app.send(request.body(Body::empty()).unwrap()).await;
    let answer = Answer::read(response).await;
    check_against_document(&Method::GET, FILE_ROUTE, &answer);
    answer
}

/// Stores `body` as `name`, and gives the stored file.
async fn stored(app: &TestApp, session: &Session, name: &str, body: &str) -> ChatFile {
    let query = emberhut_api::uploads::UploadQuery::new(name).to_query();
    let answer = upload(app, Some(session), &query, body).await;
    assert_eq!(answer.status, 200, "{}", answer.text);
    answer.read_as::<UploadReply>().file
}

#[tokio::test]
pub(crate) async fn a_file_is_stored_beside_the_database_and_answered_whole() {
    let app = TestApp::new().await;
    let session = app.sign_in().await;
    let answer = upload(
        &app,
        Some(&session),
        "name=Week%203%20%26%204.png",
        "PNG bytes",
    )
    .await;
    assert_eq!(answer.status, 200);
    assert_keys_within(&answer.json(), &fixture("upload_reply.json"), "upload");
    let reply: UploadReply = answer.read_as();
    assert!(reply.ok);
    let file = reply.file;
    assert_eq!(file.name, "Week 3 & 4.png", "the name as given");
    assert_eq!(file.kind, FileKind::Image);
    let stored_name = file.url.strip_prefix("/uploads/").unwrap();
    assert!(
        stored_name.ends_with("-Week%203%20_%204.png"),
        "{stored_name}"
    );
    let path = std::path::PathBuf::from(file.path.unwrap());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "PNG bytes");
    let database = app.url.path();
    assert_eq!(
        path.parent().unwrap(),
        std::path::absolute(database.parent().unwrap().join("uploads")).unwrap()
    );
}

#[tokio::test]
async fn a_file_without_a_name_is_called_file() {
    let app = TestApp::new().await;
    let session = app.sign_in().await;
    let file: ChatFile = upload(&app, Some(&session), "", "x")
        .await
        .read_as::<UploadReply>()
        .file;
    assert_eq!(file.name, "file");
    assert_eq!(file.kind, FileKind::File);
    assert!(file.url.ends_with("-file"), "{}", file.url);
}

#[tokio::test]
pub(crate) async fn a_file_over_500_mb_is_refused_before_it_is_read() {
    let app = TestApp::new().await;
    let session = app.sign_in().await;
    let request = Request::post("/api/uploads?name=big.mov")
        .header(
            "authorization",
            format!("Bearer {}", session.tokens.access.as_str()),
        )
        .header("content-length", (500 * 1024 * 1024 + 1).to_string())
        .body(Body::from("only a little"))
        .unwrap();
    let answer = Answer::read(app.send(request).await).await;
    check_against_document(&Method::POST, "/api/uploads", &answer);
    assert_eq!(answer.status, 400);
    assert_eq!(
        answer.json(),
        json!({ "ok": false, "error": "file is larger than 500 MB" })
    );
    assert_keys_within(&answer.json(), &fixture("upload_refused.json"), "refused");
    let folder = app.url.path().parent().unwrap().join("uploads");
    let left = std::fs::read_dir(&folder).map_or(0, Iterator::count);
    assert_eq!(left, 0, "no part of it is kept");
}

#[tokio::test]
pub(crate) async fn uploading_and_fetching_need_a_session() {
    let app = TestApp::new().await;
    let refused = upload(&app, None, "name=a.txt", "x").await;
    assert_eq!(refused.status, 401);
    assert!(refused.read_as::<ErrorBody>().sign_in);
    let session = app.sign_in().await;
    let file = stored(&app, &session, "a.txt", "secret").await;
    let anonymous = app.send(crate::common::get(&file.url)).await;
    assert_eq!(anonymous.status(), 401);
    // A player that can't send a header puts the token in the query.
    let with_token = format!("{}?token={}", file.url, session.tokens.access.as_str());
    let played = Answer::read(app.send(crate::common::get(&with_token)).await).await;
    assert_eq!(played.status, 200);
    assert_eq!(played.text, "secret");
}

#[tokio::test]
pub(crate) async fn a_stored_file_is_served_whole_or_in_ranges() {
    let app = TestApp::new().await;
    let session = app.sign_in().await;
    let file = stored(&app, &session, "clip.mp4", "0123456789").await;

    let whole = fetch(&app, &session, &file.url, &[]).await;
    assert_eq!(whole.status, 200);
    assert_eq!(whole.text, "0123456789");
    assert_eq!(whole.header("content-type"), "video/mp4");
    assert_eq!(whole.header("accept-ranges"), "bytes");
    assert_eq!(whole.header("content-length"), "10");
    assert_eq!(whole.header("cache-control"), "private, max-age=3600");

    for (range, body, content_range) in [
        ("bytes=2-4", "234", "bytes 2-4/10"),
        ("bytes=7-", "789", "bytes 7-9/10"),
        ("bytes=-3", "789", "bytes 7-9/10"),
        ("bytes=8-99", "89", "bytes 8-9/10"),
    ] {
        let part = fetch(&app, &session, &file.url, &[("range", range)]).await;
        assert_eq!(part.status, 206, "{range}");
        assert_eq!(part.text, body, "{range}");
        assert_eq!(part.header("content-range"), content_range, "{range}");
        assert_eq!(part.header("content-length"), body.len().to_string());
        assert!(part.headers.get("cache-control").is_none());
    }
    for range in ["bytes=10-", "bytes=5-2"] {
        let past = fetch(&app, &session, &file.url, &[("range", range)]).await;
        assert_eq!(past.status, 416, "{range}");
        assert_eq!(past.header("content-range"), "bytes */10");
    }
    // Several ranges, or another unit: the whole file.
    let other = fetch(&app, &session, &file.url, &[("range", "bytes=0-1,4-5")]).await;
    assert_eq!(other.status, 200);
}

#[tokio::test]
async fn a_report_is_served_sandboxed() {
    let app = TestApp::new().await;
    let session = app.sign_in().await;
    let file = stored(&app, &session, "report.html", "<p>hi</p>").await;
    assert_eq!(
        file.kind,
        FileKind::File,
        "a report is a plain file by its kind"
    );
    let served = fetch(&app, &session, &file.url, &[]).await;
    assert_eq!(served.header("content-type"), "text/html; charset=utf-8");
    assert!(
        served
            .header("content-security-policy")
            .starts_with("sandbox allow-scripts")
    );
    assert_eq!(served.header("x-content-type-options"), "nosniff");
    assert_eq!(served.header("referrer-policy"), "no-referrer");
}

#[tokio::test]
pub(crate) async fn a_missing_file_or_one_outside_the_folder_is_404() {
    let app = TestApp::new().await;
    let session = app.sign_in().await;
    let data = app.url.path().parent().unwrap().to_owned();
    std::fs::write(data.join("secret.txt"), "outside the folder").unwrap();
    // The folder exists, so a path out of it would resolve if the name were not checked.
    stored(&app, &session, "photo.png", "PNG").await;
    assert!(data.join("uploads").is_dir());
    for url in [
        "/uploads/nothing-here.png",
        "/uploads/..%2Fsecret.txt",
        "/uploads/..%2F..%2Fsecret.txt",
        "/uploads/..%2Femberhut.db",
        "/uploads/%E0%A4%A",
    ] {
        let missing = fetch(&app, &session, url, &[]).await;
        assert_eq!(missing.status, 404, "{url}");
        assert_eq!(missing.text, "", "{url}");
        assert!(!missing.text.contains("outside the folder"), "{url}");
    }
    #[cfg(unix)]
    {
        let folder = app.url.path().parent().unwrap().join("uploads");
        std::fs::create_dir_all(&folder).unwrap();
        std::os::unix::fs::symlink(folder.join("../secret.txt"), folder.join("0000-link.txt"))
            .unwrap();
        let linked = fetch(&app, &session, "/uploads/0000-link.txt", &[]).await;
        assert_eq!(
            linked.status, 404,
            "a link is not followed out of the folder"
        );
    }
}

#[tokio::test]
async fn a_message_comes_with_the_files_the_server_stored() {
    let app = TestApp::new().await;
    let session = app.sign_in().await;
    let picture = stored(&app, &session, "photo.png", "PNG").await;
    let notes = stored(&app, &session, "notes.pdf", "PDF").await;
    let mut forged = notes.clone();
    forged.path = Some("/etc/passwd".to_owned());
    let said = app
        .call(Call::post("/api/talk").token(&session).json(&json!({
            "text": "",
            "files": [picture, notes, forged],
        })))
        .await;
    assert_eq!(said.status, 200, "files alone are a message");
    app.settle().await;

    let state: State = app
        .call(Call::get("/api/state").token(&session))
        .await
        .read_as();
    let message = &state.chat[0];
    let files: Vec<(&str, &str)> = message
        .files
        .iter()
        .map(|file| (file.name.as_str(), file.url.as_str()))
        .collect();
    assert_eq!(
        files,
        [
            ("photo.png", picture.url.as_str()),
            ("notes.pdf", notes.url.as_str())
        ],
        "in order, the forged one dropped"
    );
    assert!(message.files.iter().all(|file| file.path.is_none()));
    assert_eq!(state.rooms[0].last.as_deref(), Some("Sent a file"));

    let turn = &app.assistant.turns()[0];
    assert_eq!(turn.files.len(), 2);
    assert_eq!(
        turn.files[0].path.to_string_lossy(),
        picture.path.clone().unwrap()
    );
    let described = describe(&turn.files);
    assert!(described.contains("attached 2 files"), "{described}");
    assert!(described.contains(&format!("- image: {}", picture.path.unwrap())));

    // A reply quotes the message's picture.
    let reply_to = message.id.get();
    app.call(
        Call::post("/api/talk")
            .token(&session)
            .json(&json!({ "text": "About that", "replyTo": reply_to })),
    )
    .await;
    app.settle().await;
    let state: Value = app
        .call(Call::get("/api/state").token(&session))
        .await
        .json();
    let quote = &state["chat"][1]["reply"];
    assert_eq!(quote["id"], reply_to);
    assert_eq!(quote["mine"], true);
    assert_eq!(quote["file"]["name"], "photo.png");
    assert_eq!(quote["file"]["kind"], "image");
}

#[tokio::test]
async fn a_file_the_server_sends_is_kept_in_the_uploads_folder_and_served() {
    let app = TestApp::new().await;
    let session = app.sign_in().await;
    // Not hidden: a temporary folder's default name starts with a dot, and a hidden file is
    // never kept.
    let made = tempfile::Builder::new().prefix("made-").tempdir().unwrap();
    let source = made.path().join("Weekly chart.png");
    std::fs::write(&source, "chart bytes").unwrap();

    let kept = app.state.uploads().keep(&source).await.unwrap();
    assert_eq!(kept.name, "Weekly chart.png", "its own file name");
    assert_eq!(kept.kind, emberhut_core::upload::FileKind::Image);
    assert_eq!(kept.size, 11);
    assert!(source.exists(), "the source is copied, not moved");
    let served = fetch(
        &app,
        &session,
        &emberhut_core::upload::url_of(&kept.stored_name),
        &[],
    )
    .await;
    assert_eq!(served.status, 200);
    assert_eq!(served.text, "chart bytes");

    for not_a_file in [made.path().to_path_buf(), made.path().join("missing.png")] {
        let refused = app.state.uploads().keep(&not_a_file).await.unwrap_err();
        assert!(
            matches!(
                refused,
                emberhut_server::services::uploads::UploadError::Storage(_)
            ),
            "{not_a_file:?}: {refused:?}"
        );
    }
}
