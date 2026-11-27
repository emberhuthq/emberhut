//! One-time secret links: the page (`GET /s/{token}`) and its form (`POST /s/{token}`), made
//! by the secret link service (the `secret_link` tool's logic).
#![expect(
    clippy::unwrap_used,
    reason = "test helpers: a failure here fails the test that called them"
)]

use axum::body::Body;
use axum::http::{Method, Request};
use emberhut_api::paths;
use emberhut_api::state::State;
use emberhut_server::services::secret_links::SecretLinkError;
use jiff::SignedDuration;

use crate::common::{Answer, Call, TestApp, check_against_document, documented_path, text};

/// The path of the link at `url`.
fn path_of(url: &str) -> String {
    let at = url.find("/s/").unwrap();
    url[at..].to_owned()
}

/// Sends the form `body` to `path`, as a browser does.
async fn post_form(app: &TestApp, path: &str, body: &str) -> Answer {
    let request = Request::builder()
        .method(Method::POST)
        .uri(path)
        .header("content-type", "application/x-www-form-urlencoded")
        .body(Body::from(body.to_owned()))
        .unwrap();
    let answer = Answer::read(app.send(request).await).await;
    check_against_document(&Method::POST, &documented_path(path), &answer);
    answer
}

#[tokio::test]
pub(crate) async fn a_link_shows_its_form_once_and_saves_what_is_typed() {
    let app = TestApp::new().await;
    let link = app
        .state
        .secret_links()
        .create(
            &["slack-token", "other-key"],
            "To read <your> channels & DMs",
        )
        .await
        .unwrap();
    assert!(link.url.starts_with("http://127.0.0.1:18990/s/"));
    assert_eq!(
        link.expires_at,
        app.clock_now() + SignedDuration::from_hours(24)
    );
    let path = path_of(&link.url);

    let page = app.call(Call::get(&path)).await;
    assert_eq!(page.status, 200);
    assert_eq!(page.header("content-type"), "text/html; charset=utf-8");
    assert_eq!(page.header("cache-control"), "no-store");
    assert_eq!(page.header("referrer-policy"), "no-referrer");
    assert!(
        page.text
            .contains("To read &lt;your&gt; channels &amp; DMs")
    );
    assert!(
        page.text
            .contains(r#"<input name="slack-token" type=password"#)
    );
    assert!(
        page.text
            .contains(r#"<input name="other-key" type=password"#)
    );
    assert!(page.text.contains("<form method=post>"));

    let saved = post_form(
        &app,
        &path,
        "slack-token=+xoxp-123%0A&other-key=&extra=ignored",
    )
    .await;
    assert_eq!(saved.status, 200);
    assert!(
        saved
            .text
            .contains("Saved slack-token. You can close this page.")
    );
    let file = app.state.secret_links().secrets_dir().join("slack-token");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "xoxp-123");
    assert!(
        !app.state
            .secret_links()
            .secrets_dir()
            .join("other-key")
            .exists()
    );
    assert!(
        !app.state
            .secret_links()
            .secrets_dir()
            .join("extra")
            .exists()
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = std::fs::metadata(&file).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "the server's user only");
        let dir = std::fs::metadata(app.state.secret_links().secrets_dir()).unwrap();
        assert_eq!(dir.permissions().mode() & 0o777, 0o700);
    }

    // Used up: the page and the form are gone.
    let again = app.call(Call::get(&path)).await;
    assert_eq!(again.status, 404);
    assert!(again.text.contains("expired or was already used"));
    let twice = post_form(&app, &path, "slack-token=other").await;
    assert_eq!(twice.status, 404);
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "xoxp-123");

    // Main notes which secrets were saved, never their values.
    let session = app.sign_in().await;
    let state: State = app
        .call(Call::get(paths::STATE).token(&session))
        .await
        .read_as();
    let note = state.chat.last().unwrap();
    assert!(note.event);
    assert_eq!(note.text, "Saved slack-token on the private page.");
    assert!(!state.chat.iter().any(|m| m.text.contains("xoxp")));
}

#[tokio::test]
pub(crate) async fn an_unknown_or_expired_link_is_404() {
    let app = TestApp::new().await;
    let unknown = app
        .call(Call::get("/s/abcdefghijklmnopqrstuvwxyz012345"))
        .await;
    assert_eq!(unknown.status, 404);
    let short = app.call(Call::get("/s/short")).await;
    assert_eq!(short.status, 404);
    assert_eq!(short.header("content-type"), "text/html; charset=utf-8");

    let link = app
        .state
        .secret_links()
        .create(&["key"], "Why")
        .await
        .unwrap();
    let path = path_of(&link.url);
    app.clock.advance(SignedDuration::from_mins(24 * 60 - 1));
    assert_eq!(app.call(Call::get(&path)).await.status, 200, "for a day");
    app.clock.advance(SignedDuration::from_mins(2));
    assert_eq!(app.call(Call::get(&path)).await.status, 404);
    let late = post_form(&app, &path, "key=value").await;
    assert_eq!(late.status, 404);
    assert!(!app.state.secret_links().secrets_dir().join("key").exists());
}

#[tokio::test]
async fn a_form_sent_empty_uses_the_link_up_and_saves_nothing() {
    let app = TestApp::new().await;
    let link = app
        .state
        .secret_links()
        .create(&["key"], "Why")
        .await
        .unwrap();
    let path = path_of(&link.url);
    let empty = post_form(&app, &path, "key=+++").await;
    assert_eq!(empty.status, 200);
    assert!(empty.text.contains("Nothing was entered."));
    assert_eq!(app.call(Call::get(&path)).await.status, 404);
}

#[tokio::test]
async fn a_link_asks_for_one_to_six_plain_names() {
    let app = TestApp::new().await;
    let links = app.state.secret_links();
    for names in [
        vec![],
        vec!["a"; 7],
        vec!["Upper"],
        vec!["../escape"],
        vec!["api-key"],
    ] {
        assert!(
            matches!(
                links.create(&names, "why").await,
                Err(SecretLinkError::Invalid(_))
            ),
            "{names:?}"
        );
    }
    assert!(links.create(&["a"; 6], "why").await.is_ok());
}

#[tokio::test]
async fn links_point_at_the_public_url() {
    let app = TestApp::new().await;
    let one = app.state.secret_links().create(&["a"], "x").await.unwrap();
    let two = app.state.secret_links().create(&["a"], "x").await.unwrap();
    assert_ne!(one.url, two.url, "each link is its own");
    let token = one.url.rsplit('/').next().unwrap();
    assert_eq!(token.len(), 32);
}

#[tokio::test]
async fn the_secret_link_tool_makes_a_link_that_opens_its_page() {
    let app = TestApp::new().await;
    let made = app
        .call_tool(
            "secret_link",
            serde_json::json!({ "names": ["slack-user-token"], "why": "To read the channels" }),
        )
        .await;
    assert_ne!(made.is_error, Some(true), "{made:?}");
    let url = text(&made);
    assert!(url.starts_with("http://127.0.0.1:18990/s/"), "{url}");
    let page = app.call(Call::get(&path_of(url))).await;
    assert_eq!(page.status, 200);
    assert!(
        page.text
            .contains(r#"<input name="slack-user-token" type=password"#)
    );
    assert!(page.text.contains("To read the channels"));

    // Names a link can't ask for are the arguments' error.
    for (names, says) in [
        (
            serde_json::json!(["api-key"]),
            "names.0: that name is reserved",
        ),
        (
            serde_json::json!(["ok", "Not-OK"]),
            "names.1: lowercase letters",
        ),
        (serde_json::json!([]), "names"),
        (
            serde_json::json!(["a", "b", "c", "d", "e", "f", "g"]),
            "names",
        ),
    ] {
        let refused = app
            .call_tool(
                "secret_link",
                serde_json::json!({ "names": names, "why": "x" }),
            )
            .await;
        assert_eq!(refused.is_error, Some(true), "{names}");
        let said = text(&refused);
        assert!(
            said.starts_with(
                "MCP error -32602: Input validation error: Invalid arguments for tool secret_link: "
            ) && said.contains(says),
            "{names}: {said}"
        );
    }
}

#[tokio::test]
async fn the_secret_link_tool_tells_the_agent_what_a_name_is() {
    let app = TestApp::new().await;
    let listed: emberhut_api::mcp::ToolsList = serde_json::from_value(
        app.rpc("tools/list", serde_json::json!({}))
            .await
            .read_as::<emberhut_api::mcp::RpcResponse>()
            .result
            .unwrap(),
    )
    .unwrap();
    let tool = listed
        .tools
        .iter()
        .find(|tool| tool.name == "secret_link")
        .unwrap();
    let schema = &tool.input_schema;
    assert_eq!(schema["required"], serde_json::json!(["names", "why"]));
    assert_eq!(schema["properties"]["names"]["maxItems"], 6);
    assert_eq!(schema["$defs"]["SecretNameArg"]["pattern"], "^[a-z0-9-]+$");
}

/// Log lines written while a test runs, kept in memory.
#[derive(Clone, Default)]
struct Captured(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

impl std::io::Write for Captured {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn a_link_token_never_reaches_the_logs() {
    let logs = Captured::default();
    let writer = logs.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_ansi(false)
        .with_writer(move || writer.clone())
        .finish();
    // The test runs on one thread, so the subscriber sees every request's lines.
    let _logging = tracing::subscriber::set_default(subscriber);

    let app = TestApp::new().await;
    let link = app
        .state
        .secret_links()
        .create(&["key"], "Why")
        .await
        .unwrap();
    let path = path_of(&link.url);
    let token = path.trim_start_matches("/s/").to_owned();
    assert_eq!(app.call(Call::get(&path)).await.status, 200);
    assert_eq!(post_form(&app, &path, "key=value").await.status, 200);
    assert_eq!(app.call(Call::get(&path)).await.status, 404);
    let unrouted = Request::get(format!("{path}/more"))
        .body(Body::empty())
        .unwrap();
    assert_eq!(app.send(unrouted).await.status(), 404);

    let written = String::from_utf8(logs.0.lock().unwrap().clone()).unwrap();
    assert!(written.contains("path=/s/{token}"), "{written}");
    assert!(written.contains("path=/s/:x/more"), "{written}");
    assert!(!written.contains(&token), "{written}");
}

#[cfg(unix)]
#[tokio::test]
async fn a_saved_secret_replaces_a_link_or_a_loose_file_and_is_the_users_only() {
    use std::os::unix::fs::PermissionsExt as _;

    let app = TestApp::new().await;
    let links = app.state.secret_links();
    let dir = links.secrets_dir().to_owned();
    std::fs::create_dir_all(&dir).unwrap();
    // A file anyone may read, and a link pointing out of the directory.
    let loose = dir.join("loose-key");
    std::fs::write(&loose, "old").unwrap();
    std::fs::set_permissions(&loose, std::fs::Permissions::from_mode(0o644)).unwrap();
    let outside = tempfile::tempdir().unwrap();
    let target = outside.path().join("elsewhere");
    std::fs::write(&target, "untouched").unwrap();
    std::os::unix::fs::symlink(&target, dir.join("linked-key")).unwrap();

    let link = links
        .create(&["loose-key", "linked-key"], "Why")
        .await
        .unwrap();
    let saved = post_form(
        &app,
        &path_of(&link.url),
        "loose-key=new-1&linked-key=new-2",
    )
    .await;
    assert_eq!(saved.status, 200, "{}", saved.text);

    assert_eq!(std::fs::read_to_string(&loose).unwrap(), "new-1");
    let mode = std::fs::metadata(&loose).unwrap().permissions().mode();
    assert_eq!(
        mode & 0o777,
        0o600,
        "the server's user only, whatever was there"
    );
    let linked = dir.join("linked-key");
    assert!(!linked.symlink_metadata().unwrap().file_type().is_symlink());
    assert_eq!(std::fs::read_to_string(&linked).unwrap(), "new-2");
    assert_eq!(
        std::fs::read_to_string(&target).unwrap(),
        "untouched",
        "the link was replaced, not written through"
    );
    let left: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(left.len(), 2, "no temporary file is left: {left:?}");
}
