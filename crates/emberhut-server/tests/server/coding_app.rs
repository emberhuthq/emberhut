//! The real coding-thread app client against a stand-in MCP endpoint on this machine: the
//! session it opens, the tools it calls, the token it sends, a restarted app, and refusals.
#![expect(
    clippy::unwrap_used,
    reason = "test helpers: a stand-in that can't start fails the test"
)]

use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use emberhut_server::config::CodingAppMachine;
use emberhut_server::services::coding_app::{
    CodingAppError, CodingThreadApp, McpCodingApp, ReadAsk, tool,
};
use serde_json::{Value, json};
use tokio::net::TcpListener;

/// What the stand-in app saw, and the session it hands out.
#[derive(Debug, Default)]
struct Seen {
    session: Option<String>,
    opened: usize,
    calls: Vec<(String, Value)>,
    tokens: Vec<Option<String>>,
}

type Shared = Arc<Mutex<Seen>>;

/// The tool's answer, as the app puts it in a result's text.
fn answer(name: &str, arguments: &Value) -> Result<Value, &'static str> {
    match name {
        tool::PROJECT_LIST => Ok(json!({ "projects": [{ "id": "p1", "title": "Example" }] })),
        tool::THREAD_LIST => Ok(json!({ "threads": [
            { "threadId": "mcp:a", "title": "Fix it", "status": "running",
              "updatedAt": "2026-11-24T21:00:00.000Z", "parentThreadId": null },
            { "threadId": "mcp:b", "title": "Lane", "status": "completed",
              "updatedAt": "2026-11-24T21:01:00.000Z", "parentThreadId": "mcp:a" },
        ] })),
        tool::THREAD_READ if arguments["threadId"] == "mcp:a" => Ok(json!({
            "thread": { "title": "Fix it", "status": "running", "updatedAt": "2026-11-24T21:00:00.000Z", "itemCount": 2 },
            "items": [
                { "type": "user_message", "text": "Fix the build", "updatedAt": "2026-11-24T20:00:00.000Z" },
                { "type": "assistant_message", "text": "Done", "title": "Answer" },
            ],
        })),
        tool::THREAD_SEND if arguments["threadId"] == "mcp:a" => {
            Ok(json!({ "run": { "runId": "run-9" }, "mode": "queue" }))
        }
        _ => Err(r#"{"code":"thread_not_found","message":"No such thread"}"#),
    }
}

/// The stand-in MCP endpoint: `initialize` opens a session, a call with another session is
/// 404 (as a restarted app answers), and tool results come as an event stream.
async fn endpoint(State(seen): State<Shared>, headers: HeaderMap, body: Bytes) -> Response {
    let message: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    let mut seen = seen
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    seen.tokens.push(
        headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned),
    );
    let session = headers.get("mcp-session-id").and_then(|v| v.to_str().ok());
    match message["method"].as_str().unwrap_or_default() {
        "initialize" => {
            seen.opened += 1;
            let id = format!("s-{}", seen.opened);
            seen.session = Some(id.clone());
            (StatusCode::OK, [("mcp-session-id", id)], "{}").into_response()
        }
        "notifications/initialized" => StatusCode::ACCEPTED.into_response(),
        "tools/call" if session.is_some() && session == seen.session.as_deref() => {
            let name = message["params"]["name"].as_str().unwrap_or_default();
            let arguments = message["params"]["arguments"].clone();
            seen.calls.push((name.to_owned(), arguments.clone()));
            let result = match answer(name, &arguments) {
                Ok(value) => json!({ "content": [{ "type": "text", "text": value.to_string() }] }),
                Err(why) => {
                    json!({ "isError": true, "content": [{ "type": "text", "text": why }] })
                }
            };
            let reply = json!({ "jsonrpc": "2.0", "id": 1, "result": result });
            (
                [("content-type", "text/event-stream")],
                format!("event: message\ndata: {reply}\n\n"),
            )
                .into_response()
        }
        _ => StatusCode::NOT_FOUND.into_response(),
    }
}

/// The stand-in app on a free port, and the client configured for it as `laptop`.
async fn stand_in(tokens: Option<std::path::PathBuf>) -> (Shared, McpCodingApp) {
    let seen = Shared::default();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/mcp", listener.local_addr().unwrap());
    let router = Router::new()
        .fallback(endpoint)
        .with_state(Arc::clone(&seen));
    tokio::spawn(async move { axum::serve(listener, router).await });
    let machine = CodingAppMachine {
        name: "laptop".to_owned(),
        url,
    };
    (seen, McpCodingApp::new(vec![machine], tokens).unwrap())
}

#[tokio::test(flavor = "multi_thread")]
async fn the_client_lists_reads_and_sends_over_one_session() {
    let dir = tempfile::tempdir().unwrap();
    let tokens = dir.path().join("tokens");
    std::fs::write(&tokens, "# one per machine\nlaptop=secret-1\n").unwrap();
    let (seen, app) = stand_in(Some(tokens.clone())).await;
    assert_eq!(app.machines(), ["laptop"]);

    let listed = app.list("laptop").await.unwrap();
    assert_eq!(listed.len(), 2);
    assert_eq!(
        (listed[0].id.as_str(), listed[0].project.as_str()),
        ("mcp:a", "Example")
    );
    assert_eq!(listed[1].parent.as_deref(), Some("mcp:a"));

    let ask = ReadAsk {
        limit: 40,
        after: Some(3),
        activity: true,
        max_chars: 1200,
    };
    let page = app.read("laptop", "mcp:a", ask).await.unwrap();
    assert_eq!(page.item_count, 2);
    assert_eq!(page.items[1].title.as_deref(), Some("Answer"));

    let sent = app.send("laptop", "mcp:a", "Run the tests").await.unwrap();
    assert_eq!(sent.run_id.as_deref(), Some("run-9"));
    assert!(sent.queued);

    let seen_now = seen.lock().unwrap();
    assert_eq!(seen_now.opened, 1, "one session for every call");
    let names: Vec<&str> = seen_now.calls.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(
        names,
        [
            tool::PROJECT_LIST,
            tool::THREAD_LIST,
            tool::THREAD_READ,
            tool::THREAD_SEND
        ]
    );
    assert_eq!(seen_now.calls[1].1["projectId"], "p1");
    assert_eq!(seen_now.calls[1].1["settled"], false);
    let read = &seen_now.calls[2].1;
    assert_eq!(
        (&read["afterPosition"], &read["view"], &read["limit"]),
        (&json!(3), &json!("activity"), &json!(40))
    );
    assert_eq!(seen_now.calls[3].1["mode"], "queue");
    assert!(
        seen_now
            .tokens
            .iter()
            .all(|t| t.as_deref() == Some("Bearer secret-1"))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_restarted_app_gets_a_new_session_and_refusals_keep_their_reason() {
    let (seen, app) = stand_in(None).await;
    app.list("laptop").await.unwrap();
    // The app restarts: the session it handed out is gone.
    seen.lock().unwrap().session = None;
    app.list("laptop").await.unwrap();
    assert_eq!(seen.lock().unwrap().opened, 2);
    assert!(seen.lock().unwrap().tokens.iter().all(Option::is_none));

    let refused = app.send("laptop", "mcp:zz", "Hello").await.unwrap_err();
    assert!(matches!(refused, CodingAppError::Refused { .. }));
    assert_eq!(
        refused.to_string(),
        "laptop: thread_not_found: No such thread"
    );
    assert!(matches!(
        app.list("desk").await.unwrap_err(),
        CodingAppError::UnknownMachine
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn an_app_that_cannot_be_reached_is_unreachable() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/mcp", listener.local_addr().unwrap());
    drop(listener);
    let machine = CodingAppMachine {
        name: "laptop".to_owned(),
        url,
    };
    let app = McpCodingApp::new(vec![machine], None).unwrap();
    assert!(matches!(
        app.list("laptop").await.unwrap_err(),
        CodingAppError::Unreachable { .. }
    ));
}
