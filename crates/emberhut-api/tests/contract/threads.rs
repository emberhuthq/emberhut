//! Coding threads (the state's `threads` and `followed`), read from the wire, written back in
//! the same shape.

use emberhut_api::threads::{CodingThread, FollowedThread};
use serde_json::{Value, json};

use crate::support::{from_value, to_value};

/// A runner session as the wire lists it: every field it has.
fn session() -> Value {
    json!({
        "machine": "runner",
        "id": "r-1a2b3c4d",
        "title": "Fix the example build",
        "project": "Example",
        "status": "running",
        "updatedAt": "2026-11-24T21:57:53.410Z",
        "parent": null,
        "ask": "Fix the build",
        "lastAsk": "And run the tests",
        "last": "Building now.",
        "statusLine": "STATUS: building",
        "doing": "Running cargo test",
        "account": "work",
        "cwd": "/work/example",
        "branch": "fix/build",
        "since": "2026-11-24T21:50:00.000Z",
        "direct": true,
        "model": "a-model",
        "effort": "high",
        "question": "Which branch?"
    })
}

#[test]
fn a_thread_reads_and_writes_its_wire_shape() {
    let thread: CodingThread = from_value(session());
    assert_eq!(thread.machine.as_str(), "runner");
    assert_eq!(thread.parent, None);
    assert!(thread.direct);
    assert_eq!(
        thread.since.as_ref().unwrap().as_str(),
        "2026-11-24T21:50:00.000Z"
    );
    assert_eq!(to_value(&thread), session());
}

#[test]
fn a_thread_with_little_known_leaves_the_rest_out() {
    let bare = json!({
        "machine": "laptop",
        "id": "mcp:42",
        "title": "Look at the logs",
        "project": "",
        "status": "completed",
        "updatedAt": "2026-11-24T21:57:53.410Z",
        "parent": "mcp:41"
    });
    let thread: CodingThread = from_value(bare.clone());
    assert_eq!(thread.parent.as_deref(), Some("mcp:41"));
    assert!(thread.ask.is_none() && !thread.direct);
    assert_eq!(to_value(&thread), bare);

    // A field set to null reads as unknown, and is left out.
    let mut nulls = bare.clone();
    nulls["doing"] = Value::Null;
    nulls["since"] = Value::Null;
    let thread: CodingThread = from_value(nulls);
    assert_eq!(to_value(&thread), bare);
}

#[test]
fn a_followed_thread_carries_its_latest_look_or_null() {
    let seen = json!({
        "id": "r-1a2b3c4d",
        "machine": "runner",
        "why": "Fix the example build",
        "at": "2026-11-24T21:40:00.000Z",
        "thread": session()
    });
    let followed: FollowedThread = from_value(seen.clone());
    assert_eq!(followed.thread.as_ref().unwrap().id, followed.id);
    assert_eq!(to_value(&followed), seen);

    let unseen = json!({
        "id": "r-5e6f7a8b",
        "machine": "runner",
        "why": "Review the fix",
        "at": "2026-11-24T21:41:00.000Z",
        "thread": null
    });
    let followed: FollowedThread = from_value(unseen.clone());
    assert!(followed.thread.is_none());
    assert_eq!(to_value(&followed), unseen);
}
