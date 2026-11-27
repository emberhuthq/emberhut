//! The CLI's stream-json protocol, as far as the server speaks it: the lines it writes (one
//! JSON message each) read into [`Said`], and the lines the server writes to it (a user
//! message, the `initialize` and `interrupt` control requests, the answer to a control request
//! it does not serve).

use jiff::Timestamp;
use serde_json::{Value, json};

/// What one line of the CLI's output says, as far as a turn cares.
#[derive(Clone, Debug, PartialEq)]
pub enum Said {
    /// The session started: its id, for resuming it later.
    Init {
        /// The session's id.
        session_id: String,
    },
    /// The conversation was compacted (summarised to make room).
    Compacted,
    /// The assistant's message: its words and the tool it calls last, if any.
    Assistant {
        /// Its text blocks, joined.
        text: String,
        /// The last tool call: its name and input.
        tool: Option<(String, Value)>,
        /// The service refused it for the account's usage.
        rate_limited: bool,
    },
    /// The account's usage, as the service sees it; `rejected` when it is out.
    RateLimit {
        /// It refused the turn.
        rejected: bool,
        /// Which window (`five_hour`, `seven_day`).
        kind: Option<String>,
        /// When it takes work again.
        resets_at: Option<Timestamp>,
    },
    /// The CLI asks the server something (a permission, a hook): its request's id and kind.
    ControlRequest {
        /// The request's id.
        id: String,
        /// Its kind.
        subtype: String,
    },
    /// The turn ended.
    Result {
        /// `success`, or how it failed (`error_max_turns`, `error_during_execution`).
        subtype: String,
        /// It is an error.
        is_error: bool,
        /// The answer's text (on success), or the errors (otherwise).
        text: String,
    },
    /// A background task (a background agent or command) started: its id.
    TaskStarted {
        /// The task's id.
        id: String,
    },
    /// The session's background tasks running now (its own, not ambient ones or sub-tasks):
    /// each one's id, description and type.
    BackgroundTasks {
        /// The running tasks.
        tasks: Vec<(String, String, String)>,
    },
    /// A background task ended: its id, how (`completed`, `failed`, `stopped`) and its summary.
    TaskEnded {
        /// The task's id.
        id: String,
        /// How it ended.
        status: String,
        /// What it said it did.
        summary: String,
    },
    /// Anything else (control answers, progress, keep-alives).
    Other,
}

impl Said {
    /// Reads one line; a line that isn't a JSON object says nothing.
    #[must_use]
    pub fn read(line: &str) -> Self {
        let Ok(Value::Object(message)) = serde_json::from_str::<Value>(line) else {
            return Self::Other;
        };
        let text = |key: &str| message.get(key).and_then(Value::as_str).unwrap_or_default();
        match (text("type"), text("subtype")) {
            ("system", "init") => Self::Init {
                session_id: text("session_id").to_owned(),
            },
            ("system", "compact_boundary") => Self::Compacted,
            ("system", "task_started") => Self::TaskStarted {
                id: text("task_id").to_owned(),
            },
            ("system", "background_tasks_changed") => background_tasks(&message),
            ("system", "task_notification") => Self::TaskEnded {
                id: text("task_id").to_owned(),
                status: text("status").to_owned(),
                summary: text("summary").to_owned(),
            },
            ("assistant", _) => {
                let blocks = message
                    .get("message")
                    .and_then(|m| m.get("content"))
                    .and_then(Value::as_array)
                    .map_or(&[][..], Vec::as_slice);
                let words: Vec<&str> = blocks
                    .iter()
                    .filter(|block| block.get("type").and_then(Value::as_str) == Some("text"))
                    .filter_map(|block| block.get("text").and_then(Value::as_str))
                    .collect();
                let tool = blocks
                    .iter()
                    .rev()
                    .find(|block| block.get("type").and_then(Value::as_str) == Some("tool_use"))
                    .map(|block| {
                        (
                            block
                                .get("name")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .to_owned(),
                            block.get("input").cloned().unwrap_or(Value::Null),
                        )
                    });
                Self::Assistant {
                    text: words.join(" ").trim().to_owned(),
                    tool,
                    rate_limited: text("error") == "rate_limit",
                }
            }
            ("rate_limit_event", _) => {
                let info = message.get("rate_limit_info");
                let field = |key: &str| info.and_then(|info| info.get(key));
                let rejected = field("status").and_then(Value::as_str) == Some("rejected")
                    && field("overageStatus").and_then(Value::as_str) != Some("allowed");
                Self::RateLimit {
                    rejected,
                    kind: field("rateLimitType")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                    resets_at: field("resetsAt").and_then(seconds),
                }
            }
            ("control_request", _) => Self::ControlRequest {
                id: text("request_id").to_owned(),
                subtype: message
                    .get("request")
                    .and_then(|request| request.get("subtype"))
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
            },
            ("result", subtype) => {
                let is_error = message
                    .get("is_error")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let text = if subtype == "success" {
                    text("result").to_owned()
                } else {
                    message.get("errors").and_then(Value::as_array).map_or_else(
                        String::new,
                        |errors| {
                            errors
                                .iter()
                                .filter_map(Value::as_str)
                                .map(str::trim)
                                .filter(|error| !error.is_empty())
                                .collect::<Vec<_>>()
                                .join("\n")
                        },
                    )
                };
                Self::Result {
                    subtype: subtype.to_owned(),
                    is_error,
                    text,
                }
            }
            _ => Self::Other,
        }
    }
}

/// The session's background tasks running now, its own: not ambient ones or sub-tasks.
fn background_tasks(message: &serde_json::Map<String, Value>) -> Said {
    let tasks = message
        .get("tasks")
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice)
        .iter()
        .filter(|task| !truthy(task.get("ambient")) && !truthy(task.get("parent_task_id")))
        .map(|task| {
            let field = |key: &str| {
                task.get(key)
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned()
            };
            (field("task_id"), field("description"), field("task_type"))
        })
        .collect();
    Said::BackgroundTasks { tasks }
}

/// A JSON value that counts as set: not null, false, 0 or empty.
fn truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(set)) => *set,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|n| n != 0.0),
        Some(Value::String(s)) => !s.is_empty(),
        Some(_) => true,
    }
}

/// Seconds since 1970 as an instant; none for nothing, zero or not a number.
fn seconds(value: &Value) -> Option<Timestamp> {
    let seconds = value.as_f64().filter(|seconds| *seconds > 0.0)?;
    #[expect(
        clippy::cast_possible_truncation,
        reason = "a reset time in seconds, far inside i64's range"
    )]
    let millis = (seconds * 1000.0).round() as i64;
    Timestamp::from_millisecond(millis).ok()
}

/// A user message with `text`, as the agent SDK writes one.
#[must_use]
pub(super) fn user_message(text: &str) -> String {
    json!({
        "type": "user",
        "message": { "role": "user", "content": text },
        "parent_tool_use_id": null,
        "session_id": "",
    })
    .to_string()
}

/// The `initialize` control request, with `append` after the CLI's own system prompt.
#[must_use]
pub(super) fn initialize(id: &str, append: &str) -> String {
    json!({
        "type": "control_request",
        "request_id": id,
        "request": { "subtype": "initialize", "appendSystemPrompt": append },
    })
    .to_string()
}

/// The `interrupt` control request: the turn in flight stops (its result still comes).
#[must_use]
pub(super) fn interrupt(id: &str) -> String {
    json!({
        "type": "control_request",
        "request_id": id,
        "request": { "subtype": "interrupt" },
    })
    .to_string()
}

/// The `stop_task` control request: the session's background task `task` stops.
#[must_use]
pub(super) fn stop_task(id: &str, task: &str) -> String {
    json!({
        "type": "control_request",
        "request_id": id,
        "request": { "subtype": "stop_task", "task_id": task },
    })
    .to_string()
}

/// The answer to a control request the server does not serve.
#[must_use]
pub(super) fn unsupported(id: &str, subtype: &str) -> String {
    json!({
        "type": "control_response",
        "response": {
            "subtype": "error",
            "request_id": id,
            "error": format!("Unsupported control request subtype: {subtype}"),
        },
    })
    .to_string()
}

/// The variable the room's MCP token is passed in: the CLI expands `${NAME}` in its
/// `--mcp-config`, so the token stays off the command line (which any local user can read).
pub const MCP_TOKEN_VAR: &str = "EMBERHUT_MCP_TOKEN";

/// The `--mcp-config` value naming the server's MCP endpoint at `url`, called with the token
/// in [`MCP_TOKEN_VAR`].
#[must_use]
pub(super) fn mcp_config(server: &str, url: &str) -> String {
    let mut servers = serde_json::Map::new();
    servers.insert(
        server.to_owned(),
        json!({
            "type": "http",
            "url": url,
            "headers": { "Authorization": format!("Bearer ${{{MCP_TOKEN_VAR}}}") },
        }),
    );
    json!({ "mcpServers": servers }).to_string()
}

#[cfg(test)]
mod tests {
    use emberhut_api::mcp::SERVER_NAME;
    use emberhut_api::paths::MCP_ROOM;

    use super::*;

    #[test]
    fn the_cli_s_messages_read_as_a_turn_needs_them() {
        assert_eq!(
            Said::read(r#"{"type":"system","subtype":"init","session_id":"s-1"}"#),
            Said::Init {
                session_id: "s-1".to_owned()
            }
        );
        let said = Said::read(
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Let me look."},
            {"type":"tool_use","name":"Bash","input":{"command":"ls"}}]}}"#,
        );
        assert_eq!(
            said,
            Said::Assistant {
                text: "Let me look.".to_owned(),
                tool: Some(("Bash".to_owned(), json!({ "command": "ls" }))),
                rate_limited: false,
            }
        );
        assert_eq!(
            Said::read(
                r#"{"type":"rate_limit_event","rate_limit_info":{"status":"rejected",
                "rateLimitType":"five_hour","resetsAt":1764151200}}"#
            ),
            Said::RateLimit {
                rejected: true,
                kind: Some("five_hour".to_owned()),
                resets_at: Timestamp::from_second(1_764_151_200).ok(),
            }
        );
        assert_eq!(
            Said::read(
                r#"{"type":"result","subtype":"error_during_execution","is_error":true,
                "errors":[" boom ",""]}"#
            ),
            Said::Result {
                subtype: "error_during_execution".to_owned(),
                is_error: true,
                text: "boom".to_owned(),
            }
        );
        assert_eq!(Said::read("not json"), Said::Other);
        assert_eq!(Said::read(r#"{"type":"keep_alive"}"#), Said::Other);
    }

    #[test]
    fn the_server_s_lines_are_the_sdk_s() {
        let user: Value = serde_json::from_str(&user_message("hi")).unwrap();
        assert_eq!(user["message"], json!({ "role": "user", "content": "hi" }));
        let url = format!("http://127.0.0.1:1{MCP_ROOM}");
        let config: Value = serde_json::from_str(&mcp_config(SERVER_NAME, &url)).unwrap();
        assert_eq!(config["mcpServers"][SERVER_NAME]["url"], url);
        assert_eq!(
            config["mcpServers"][SERVER_NAME]["headers"]["Authorization"],
            "Bearer ${EMBERHUT_MCP_TOKEN}"
        );
        let init: Value = serde_json::from_str(&initialize("i", "rules")).unwrap();
        assert_eq!(init["request"]["appendSystemPrompt"], "rules");
        let stop: Value = serde_json::from_str(&interrupt("x")).unwrap();
        assert_eq!(stop["request"]["subtype"], "interrupt");
        let no: Value = serde_json::from_str(&unsupported("r", "can_use_tool")).unwrap();
        assert_eq!(no["response"]["subtype"], "error");
    }
}
