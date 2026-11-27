//! The coding-thread app's answers, read into typed values: the JSON-RPC answer around a tool's
//! result, and each tool's result. A field the app may leave out is optional here; one it must
//! send is required, so an answer that renames it is an error, not an empty value.

use emberhut_core::thread::ActivityItem;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use super::{CodingAppError, ListedThread, Sent, ThreadPage};

/// A JSON-RPC answer to a tool call.
#[derive(Debug, Deserialize)]
struct RpcAnswer {
    /// The call failed before the tool ran (an unknown method, bad arguments).
    #[serde(default)]
    error: Option<RpcError>,
    /// The tool's result.
    #[serde(default)]
    result: Option<CallResult>,
}

/// A JSON-RPC error.
#[derive(Debug, Deserialize)]
struct RpcError {
    #[serde(default)]
    message: Option<String>,
}

/// A tool's result: its content, and whether the tool failed.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CallResult {
    #[serde(default)]
    content: Vec<Content>,
    #[serde(default)]
    is_error: Option<bool>,
}

/// One part of a result's content; the app's tools answer with one text part holding JSON.
#[derive(Debug, Deserialize)]
struct Content {
    #[serde(default)]
    text: Option<String>,
}

/// A tool call's result from the answer's `body` (plain JSON, or the first `data:` line of an
/// event stream), read as a `T` from the JSON of its first text content. A tool that failed
/// answers with `isError` and its reason as that text (often `{ code, message }`).
pub(super) fn tool_result<T: DeserializeOwned>(
    machine: &str,
    body: &str,
) -> Result<T, CodingAppError> {
    let unreadable = |reason: String| CodingAppError::Unreachable {
        machine: machine.to_owned(),
        reason,
    };
    let json = if body.trim_start().starts_with('{') {
        body
    } else {
        body.lines()
            .find_map(|line| line.strip_prefix("data:"))
            .ok_or_else(|| unreadable("an answer the server can't read".to_owned()))?
    };
    let answer: RpcAnswer = serde_json::from_str(json)
        .map_err(|error| unreadable(format!("an answer the server can't read: {error}")))?;
    if let Some(error) = answer.error {
        return Err(CodingAppError::Refused {
            machine: machine.to_owned(),
            reason: error.message.unwrap_or_else(|| "an error".to_owned()),
        });
    }
    let result = answer.result.unwrap_or_default();
    let said = result
        .content
        .into_iter()
        .next()
        .and_then(|first| first.text)
        .unwrap_or_default();
    if result.is_error == Some(true) {
        return Err(CodingAppError::Refused {
            machine: machine.to_owned(),
            reason: refusal(&said),
        });
    }
    serde_json::from_str(&said)
        .map_err(|error| unreadable(format!("a result the server can't read: {error}")))
}

/// A failed tool's reason from its text: `code: message` when it is JSON with them, else the
/// text, else a fixed sentence.
fn refusal(said: &str) -> String {
    #[derive(Deserialize)]
    struct Failure {
        #[serde(default)]
        code: Option<String>,
        #[serde(default)]
        message: Option<String>,
    }
    let joined = serde_json::from_str::<Failure>(said)
        .ok()
        .map_or_default(|failure| {
            [failure.code, failure.message]
                .into_iter()
                .flatten()
                .filter(|part| !part.is_empty())
                .collect::<Vec<_>>()
                .join(": ")
        });
    if !joined.is_empty() {
        joined
    } else if !said.is_empty() {
        said.to_owned()
    } else {
        "the coding-thread app refused it".to_owned()
    }
}

/// What the project list tool answers.
#[derive(Debug, Deserialize)]
pub(super) struct ProjectList {
    /// Every project.
    pub(super) projects: Vec<Project>,
}

/// A project, as listed.
#[derive(Debug, Deserialize)]
pub(super) struct Project {
    /// Its id, as the app gave it (sent back as is to list its threads).
    pub(super) id: Value,
    /// Its title.
    #[serde(default)]
    pub(super) title: Option<String>,
}

/// What the thread list tool answers.
#[derive(Debug, Deserialize)]
pub(super) struct ThreadList {
    threads: Vec<ThreadEntry>,
}

/// A thread, as listed.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ThreadEntry {
    thread_id: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    updated_at: Option<String>,
    #[serde(default)]
    parent_thread_id: Option<String>,
}

impl ThreadList {
    /// The threads, in the project titled `project`.
    pub(super) fn into_listed(self, project: &str) -> impl Iterator<Item = ListedThread> {
        self.threads.into_iter().map(move |thread| ListedThread {
            id: thread.thread_id,
            title: thread.title.unwrap_or_default(),
            project: project.to_owned(),
            status: thread.status.unwrap_or_default(),
            updated_at: thread.updated_at.unwrap_or_default(),
            parent: thread.parent_thread_id,
        })
    }
}

/// What the thread read tool answers.
#[derive(Debug, Default, Deserialize)]
pub(super) struct ReadAnswer {
    #[serde(default)]
    thread: Option<ThreadHead>,
    #[serde(default)]
    items: Vec<Item>,
}

/// What a read says of the thread itself.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ThreadHead {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    updated_at: Option<String>,
    #[serde(default)]
    item_count: Option<f64>,
}

/// One of a thread's items.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Item {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    updated_at: Option<String>,
}

impl ReadAnswer {
    /// The answer as a page.
    pub(super) fn into_page(self) -> ThreadPage {
        let (title, status, updated_at, item_count) = match self.thread {
            Some(head) => (
                head.title,
                head.status,
                head.updated_at,
                head.item_count.map_or(0, whole_count),
            ),
            None => (None, None, None, 0),
        };
        ThreadPage {
            title,
            status,
            updated_at,
            item_count,
            items: self
                .items
                .into_iter()
                .map(|item| ActivityItem {
                    kind: item.kind,
                    title: item.title,
                    text: item.text.unwrap_or_default(),
                    at: item.updated_at,
                })
                .collect(),
        }
    }
}

/// A count the app wrote as a JSON number: a positive one as a whole number (a fraction rounds
/// down), anything else 0.
fn whole_count(count: f64) -> u64 {
    if count > 0.0 {
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "a positive count, whole in practice; a fraction rounds down"
        )]
        let whole = count as u64;
        whole
    } else {
        0
    }
}

/// What the thread send tool answers.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct SendAnswer {
    #[serde(default)]
    run_id: Option<Value>,
    #[serde(default)]
    run: Option<RunRef>,
    #[serde(default)]
    queued: Option<Value>,
    #[serde(default)]
    mode: Option<String>,
}

/// The run a message became, when the app answers it nested.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RunRef {
    #[serde(default)]
    run_id: Option<Value>,
}

impl SendAnswer {
    /// What the app said, as the server keeps it.
    pub(super) fn into_sent(self) -> Sent {
        let run_id = self
            .run_id
            .or_else(|| self.run.and_then(|run| run.run_id))
            .filter(|id| !id.is_null())
            .map(|id| id.as_str().map_or_else(|| id.to_string(), str::to_owned));
        let queued =
            self.queued.as_ref().is_some_and(truthy) || self.mode.as_deref() == Some("queue");
        Sent { run_id, queued }
    }
}

/// Whether a JSON value counts as set (JavaScript's truthiness).
fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(set) => *set,
        Value::Number(n) => n.as_f64().is_some_and(|n| n != 0.0),
        Value::String(text) => !text.is_empty(),
        Value::Array(_) | Value::Object(_) => true,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn result_of(value: &Value) -> String {
        json!({ "result": { "content": [{ "type": "text", "text": value.to_string() }] } })
            .to_string()
    }

    #[test]
    fn a_result_is_read_from_json_or_an_event_stream() {
        let body = result_of(&json!({ "projects": [{ "id": "p1", "title": "Example" }] }));
        let listed: ProjectList = tool_result("m", &body).unwrap();
        assert_eq!(listed.projects[0].id, json!("p1"));
        let stream = format!("event: message\ndata: {body}\n\n");
        let listed: ProjectList = tool_result("m", &stream).unwrap();
        assert_eq!(listed.projects[0].title.as_deref(), Some("Example"));
        assert!(matches!(
            tool_result::<ProjectList>("m", "nonsense"),
            Err(CodingAppError::Unreachable { .. })
        ));
    }

    #[test]
    fn a_renamed_field_is_an_error_not_an_empty_value() {
        let renamed = result_of(&json!({ "threads": [{ "id": "mcp:a", "title": "Fix it" }] }));
        let error = tool_result::<ThreadList>("laptop", &renamed).unwrap_err();
        assert!(
            error.to_string().contains("missing field `threadId`"),
            "{error}"
        );
        let no_list = result_of(&json!({ "items": [] }));
        assert!(tool_result::<ProjectList>("laptop", &no_list).is_err());
        let untyped_item = result_of(&json!({ "items": [{ "kind": "user_message" }] }));
        assert!(tool_result::<ReadAnswer>("laptop", &untyped_item).is_err());
    }

    #[test]
    fn a_failed_tool_or_call_is_a_refusal_with_its_reason() {
        let failed = json!({ "result": { "isError": true, "content": [{ "text":
            "{\"code\":\"thread_not_found\",\"message\":\"No such thread\"}" }] } });
        let error = tool_result::<Value>("laptop", &failed.to_string()).unwrap_err();
        assert_eq!(
            error.to_string(),
            "laptop: thread_not_found: No such thread"
        );
        let plain = json!({ "result": { "isError": true, "content": [{ "text": "busy" }] } });
        assert_eq!(
            tool_result::<Value>("m", &plain.to_string())
                .unwrap_err()
                .to_string(),
            "m: busy"
        );
        let bare = json!({ "result": { "isError": true } });
        assert_eq!(
            tool_result::<Value>("m", &bare.to_string())
                .unwrap_err()
                .to_string(),
            "m: the coding-thread app refused it"
        );
        let rpc = json!({ "error": { "code": -32601, "message": "Method not found" } });
        assert_eq!(
            tool_result::<Value>("m", &rpc.to_string())
                .unwrap_err()
                .to_string(),
            "m: Method not found"
        );
    }

    #[test]
    fn a_page_reads_its_thread_and_items() {
        let answer: ReadAnswer = serde_json::from_value(json!({
            "thread": { "title": "Fix it", "status": "running", "updatedAt": "2026-11-24T21:00:00Z", "itemCount": 12 },
            "items": [{ "type": "user_message", "text": "Fix it", "updatedAt": "2026-11-24T20:00:00Z" }],
        }))
        .unwrap();
        let page = answer.into_page();
        assert_eq!(page.title.as_deref(), Some("Fix it"));
        assert_eq!(page.item_count, 12);
        assert_eq!(page.items[0].kind, "user_message");
        assert_eq!(page.items[0].at.as_deref(), Some("2026-11-24T20:00:00Z"));
        let empty: ReadAnswer = serde_json::from_value(json!({})).unwrap();
        assert_eq!(empty.into_page(), ThreadPage::default());
    }

    #[test]
    fn a_send_names_its_run_either_way() {
        let flat: SendAnswer =
            serde_json::from_value(json!({ "runId": "run-1", "queued": true })).unwrap();
        assert_eq!(
            flat.into_sent(),
            Sent {
                run_id: Some("run-1".to_owned()),
                queued: true
            }
        );
        let nested: SendAnswer =
            serde_json::from_value(json!({ "run": { "runId": 9 }, "mode": "queue" })).unwrap();
        assert_eq!(
            nested.into_sent(),
            Sent {
                run_id: Some("9".to_owned()),
                queued: true
            }
        );
        let bare: SendAnswer = serde_json::from_value(json!({})).unwrap();
        assert_eq!(bare.into_sent(), Sent::default());
    }
}
