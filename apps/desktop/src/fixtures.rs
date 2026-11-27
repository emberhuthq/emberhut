//! Made-up server data for the tests, built from the contract's JSON so every required field is
//! there and the rest take the contract's defaults.

use emberhut_api::chat::ChatMessage;
use emberhut_api::rooms::RoomRow;
use emberhut_api::state::State;
use emberhut_api::tasks::Issue;
use emberhut_api::work::WorkRow;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::wire::ASSISTANT_OWNER;

/// `base` with `extra`'s fields on top, read as `T`.
fn merged<T: DeserializeOwned>(mut base: Value, extra: Value) -> T {
    if let (Value::Object(base), Value::Object(extra)) = (&mut base, extra) {
        base.extend(extra);
    }
    serde_json::from_value(base).expect("a fixture the contract reads")
}

/// A state for room 1 with `extra`'s fields.
pub(crate) fn state(extra: Value) -> State {
    merged(
        json!({"room": 1, "rooms": [], "on": true, "busy": false, "live": null, "unread": false,
               "pending": 0, "chat": [], "earlier": false}),
        extra,
    )
}

/// The assistant's message `id`, said at `at`, with `extra`'s fields.
pub(crate) fn message_at(id: i64, at: &str, extra: Value) -> ChatMessage {
    merged(
        json!({"id": id, "mine": false, "text": format!("m{id}"), "time": "", "at": at}),
        extra,
    )
}

/// The assistant's message `id`.
pub(crate) fn message(id: i64) -> ChatMessage {
    message_at(id, "2026-01-01T00:00:00Z", json!({}))
}

/// Room `id`, active, with `extra`'s fields.
pub(crate) fn room(id: i64, extra: Value) -> RoomRow {
    merged(
        json!({"id": id, "name": format!("Room {id}"), "project": if id == 1 { Value::Null } else { json!(id) },
               "goal": null, "active": true, "unread": 0, "needs": 0, "ask": null, "busy": false,
               "doing": null, "last": null, "lastAt": null}),
        extra,
    )
}

/// Task `id` with its status word and `extra`'s fields.
pub(crate) fn issue(id: i64, status: &str, extra: Value) -> Issue {
    merged(
        json!({"id": id, "key": format!("T-{id}"), "title": format!("Task {id}"), "owner": "you",
               "status": status, "goal": null, "project": null, "pid": null, "priority": 0,
               "due": null, "today": false, "decideBy": null, "decision": null, "action": null,
               "updated": "2026-01-01T00:00:00Z", "doneAt": null, "parent": null, "score": 0.0}),
        extra,
    )
}

/// A work row waiting on the user for task `task`, with `extra`'s fields.
pub(crate) fn waiting(task: i64, extra: Value) -> WorkRow {
    merged(
        json!({"id": format!("w{task}"), "kind": "task", "task": task, "title": format!("Task {task}"),
               "pid": null, "who": ASSISTANT_OWNER, "state": "waiting", "need": "choose", "asked": null,
               "checked": null, "decideBy": null, "hardBy": false}),
        extra,
    )
}

/// Task `id`'s page (a to-do task, nothing spread in) with `extra`'s fields.
pub(crate) fn task_page(id: i64, extra: Value) -> emberhut_api::task_page::TaskPage {
    merged(
        json!({"id": id, "key": format!("T-{id}"), "title": format!("Task {id}"), "owner": "you",
               "status": "todo", "goal": null, "project": null, "pid": null, "priority": 0,
               "due": null, "today": false, "decideBy": null, "decision": null, "action": null,
               "updated": "2026-01-01T00:00:00Z", "doneAt": null, "parent": null}),
        extra,
    )
}
