//! The runner sessions: a transcript, the live stream, a message, Stop, settings, a move to
//! another task, a new session and its sheet, and an external thread's steps.

use emberhut_api::ids::{MachineId, SessionId, TaskId, ToolCallId};
use emberhut_api::runner::actions::{Effort, MoveSession, SendToSession, SessionSettings};
use emberhut_api::runner::new_session::{BranchQuery, FolderQuery, OpenSession, Worktree};
use emberhut_api::runner::transcript::{SessionLive, StepsQuery};
use emberhut_api::threads::ActivityQuery;
use emberhut_client::{Error, LiveEnd};
use serde_json::json;

use crate::support::{Reply, TestServer, fixture, fresh};

const STREAM: [&str; 4] = [
    "data: {\"type\":\"delta\",\"turn\":\"r-1a:3\",\"start\":true,\"text\":\"\"}\n\n",
    ": ping\n\n",
    "data: {\"type\":\"a newer kind\"}\n\n",
    "data: {\"type\":\"tokens\",\"turn\":\"r-1a:3\",\"tokens\":12}\n\n",
];

fn session() -> SessionId {
    SessionId::from("r-1a")
}

#[test]
fn a_transcript_is_asked_for_from_a_step() {
    let server = TestServer::start(|_| Reply::Json(200, fixture("runner_transcript.json")));
    let client = server.client(fresh("r1"));
    let query = StepsQuery {
        after: 0,
        limit: Some(1000),
        agent: Some(ToolCallId::from("call-0")),
    };
    let transcript = client.session_transcript(&session(), &query).unwrap();
    assert_eq!(transcript.events.len(), 10);
    let sent = &server.requests_to("/api/runner/sessions/r-1a/transcript")[0];
    assert_eq!(sent.query, "after=0&limit=1000&agent=call-0");
}

#[test]
fn the_live_stream_skips_what_it_cant_read() {
    let server = TestServer::start(|_| Reply::Stream(STREAM.to_vec()));
    let client = server.client(fresh("r1"));
    let mut events = Vec::new();
    let query = StepsQuery {
        after: 16,
        ..StepsQuery::default()
    };
    let end = client
        .follow_session(&session(), &query, || false, |e| events.push(e))
        .unwrap();
    assert_eq!(end, LiveEnd::Closed);
    assert_eq!(events.len(), 2, "the unknown kind is skipped");
    assert!(matches!(events[1], SessionLive::Tokens { .. }));
    let sent = &server.requests_to("/api/runner/sessions/r-1a/live")[0];
    assert_eq!(sent.query, "after=16");
    assert_eq!(sent.header("accept"), Some("text/event-stream"));
}

#[test]
fn a_stopped_client_ends_a_sessions_stream_and_opens_no_other() {
    let server = TestServer::start(|_| Reply::Stream(STREAM.to_vec()));
    let client = server.client(fresh("r1"));
    let mut events = Vec::new();
    let end = client
        .follow_session(
            &session(),
            &StepsQuery::default(),
            || false,
            |event| {
                events.push(event);
                client.stop().unwrap();
            },
        )
        .unwrap();
    assert_eq!(end, LiveEnd::Stopped, "ended at the item after the stop");
    assert_eq!(events.len(), 1);
    assert!(matches!(
        client.follow_session(&session(), &StepsQuery::default(), || false, |_| {}),
        Err(Error::Stopped)
    ));
    assert_eq!(
        server.requests_to("/api/runner/sessions/r-1a/live").len(),
        1,
        "nothing sent after"
    );
}

#[test]
fn a_stream_the_runner_doesnt_answer_is_refused() {
    let server = TestServer::start(|_| {
        Reply::Json(
            502,
            json!({ "ok": false, "error": "the runner on runner didn't answer" }),
        )
    });
    let client = server.client(fresh("r1"));
    let error = client
        .follow_session(&session(), &StepsQuery::default(), || false, |_| {})
        .unwrap_err();
    assert!(
        matches!(&error, Error::Status { status: 502, message: Some(m) } if m.contains("didn't answer")),
        "{error:?}"
    );
}

#[test]
fn a_message_stop_and_settings_post_their_bodies() {
    let server = TestServer::start(|request| match request.path.as_str() {
        "/api/runner/sessions/r-1a/send" if request.json()["text"] == "" => {
            Reply::Json(400, json!({ "error": "nothing to send" }))
        }
        "/api/runner/sessions/r-1a/send" => Reply::Json(200, fixture("runner_send_reply.json")),
        "/api/runner/sessions/r-1a/settings" => {
            Reply::Json(200, json!({ "ok": true, "session": { "id": "r-1a" } }))
        }
        _ => Reply::Json(200, json!({ "ok": true, "running": true })),
    });
    let client = server.client(fresh("r1"));
    let message = SendToSession {
        text: "Carry on".into(),
        files: Vec::new(),
    };
    assert!(client.send_to_session(&session(), &message).unwrap().queued);
    let blank = SendToSession {
        text: String::new(),
        files: Vec::new(),
    };
    let error = client.send_to_session(&session(), &blank).unwrap_err();
    assert!(
        matches!(error, Error::Status { status: 400, .. }),
        "{error:?}"
    );
    assert_eq!(client.stop_session(&session()).unwrap().running, Some(true));
    assert!(client.interrupt_session(&session()).unwrap().ok);
    let settings = SessionSettings {
        effort: Some(Effort::Max),
        ..SessionSettings::default()
    };
    client.change_session(&session(), &settings).unwrap();
    let sent = server.requests_to("/api/runner/sessions/r-1a/settings");
    assert_eq!(sent[0].json(), json!({ "effort": "max" }));
    let stop = &server.requests_to("/api/runner/sessions/r-1a/stop")[0];
    assert_eq!((stop.method.as_str(), stop.json()), ("POST", json!({})));
}

#[test]
fn a_move_to_another_task_posts_the_task() {
    let server = TestServer::start(|request| match request.json()["task"].as_i64() {
        Some(12) => Reply::Json(200, fixture("runner_move_reply.json")),
        _ => Reply::Json(404, json!({ "ok": false, "error": "no such task" })),
    });
    let client = server.client(fresh("r1"));
    let to = MoveSession {
        task: TaskId::new(12),
    };
    let moved = client.move_session(&session(), &to).unwrap();
    assert!(moved.ok);
    assert_eq!(moved.task, TaskId::new(12));
    let sent = &server.requests_to("/api/runner/sessions/r-1a/task")[0];
    assert_eq!(
        (sent.method.as_str(), sent.json()),
        ("POST", fixture("runner_move_request.json"))
    );
    let error = client
        .move_session(
            &session(),
            &MoveSession {
                task: TaskId::new(99),
            },
        )
        .unwrap_err();
    assert!(
        matches!(&error, Error::Status { status: 404, message: Some(m) } if m == "no such task"),
        "{error:?}"
    );
}

#[test]
fn a_new_session_and_its_sheet() {
    let server = TestServer::start(|request| match request.path.as_str() {
        "/api/runner/sessions" => Reply::Json(200, fixture("runner_open_reply.json")),
        "/api/runner/folders" => Reply::Json(200, fixture("runner_folders.json")),
        "/api/runner/branches" => Reply::Json(200, fixture("runner_branches.json")),
        _ => Reply::Json(200, fixture("t3_activity.json")),
    });
    let client = server.client(fresh("r1"));
    let open = OpenSession {
        machine: MachineId::from("runner"),
        cwd: "/work".into(),
        text: "Go".into(),
        files: Vec::new(),
        account: None,
        model: Some("example-model".into()),
        effort: Some(Effort::High),
        worktree: Some(Worktree::default()),
        title: None,
    };
    assert_eq!(
        client.open_session(&open).unwrap().id,
        SessionId::from("r-3c")
    );
    assert_eq!(
        server.requests_to("/api/runner/sessions")[0].json(),
        json!({ "machine": "runner", "cwd": "/work", "text": "Go", "files": [], "model": "example-model",
            "effort": "high", "worktree": {} })
    );
    let roots = FolderQuery {
        machine: MachineId::from("runner"),
        path: None,
    };
    assert_eq!(client.runner_folders(&roots).unwrap().recent.len(), 1);
    let branches = BranchQuery {
        machine: MachineId::from("runner"),
        cwd: "/work".into(),
    };
    assert_eq!(client.runner_branches(&branches).unwrap().branches.len(), 2);
    let thread = ActivityQuery {
        machine: "t3-mac".into(),
        thread: "t-1".into(),
    };
    assert_eq!(client.t3_activity(&thread).unwrap().items.len(), 2);
    assert_eq!(
        server.requests_to("/api/t3/activity")[0].query,
        "machine=t3-mac&thread=t-1"
    );
}
