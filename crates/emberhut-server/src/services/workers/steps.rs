//! A worker's steps as the app shows them, read back from its own stream: each of its runs is a
//! turn file (`turn-<ms>.json`, the CLI's stream-json, one message a line, in start order),
//! and each message gives steps: what it said, each tool call and what it returned, an end
//! per turn; a sub-agent's steps sit under the call that started it (`parent`). So a worker is
//! watched as the session `w-<task>`, like a runner's session.
//!
//! A step's `seq` is its file's place (from 1) × 10¹⁰, plus its line × 100, plus its place in
//! the message: it stays put as files grow, and a new run's steps come after.

use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};

/// A file's share of the step numbers.
const PER_FILE: u64 = 10_000_000_000;

/// `text` cut to `n` characters, with an ellipsis when it was longer.
fn clip(text: &str, n: usize) -> String {
    if text.chars().count() > n {
        format!("{}…", text.chars().take(n).collect::<String>())
    } else {
        text.to_owned()
    }
}

/// A JSON value as the reference reads it as text (`String(x ?? "")`).
fn text_of(value: Option<&Value>) -> String {
    match value {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(items)) => items
            .iter()
            .map(|v| text_of(Some(v)))
            .collect::<Vec<_>>()
            .join(","),
        Some(Value::Object(_)) => "[object Object]".to_owned(),
        Some(other) => other.to_string(),
    }
}

fn truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Number(n)) => n.as_f64().is_some_and(|n| n != 0.0),
        Some(_) => true,
    }
}

/// What a tool call is doing, in a few words for the task's card ("Running: npm test").
#[must_use]
pub fn doing(name: &str, input: &Value) -> String {
    let base = |v: Option<&Value>| text_of(v).rsplit('/').next().unwrap_or_default().to_owned();
    match name {
        "Bash" => {
            let said = if truthy(input.get("description")) {
                input.get("description")
            } else {
                input.get("command")
            };
            format!("Running: {}", clip(&text_of(said), 70))
        }
        "Read" => format!("Reading {}", base(input.get("file_path"))),
        "Edit" | "Write" | "MultiEdit" | "NotebookEdit" => format!(
            "Editing {}",
            base(
                input
                    .get("file_path")
                    .or_else(|| input.get("notebook_path"))
            )
        ),
        "Grep" | "Glob" => "Searching".to_owned(),
        "Agent" | "Task" => format!(
            "Helper agent: {}",
            clip(&text_of(input.get("description")), 60)
        ),
        "WebSearch" => "Searching the web".to_owned(),
        "WebFetch" => "Reading a web page".to_owned(),
        other => other.rsplit("__").next().unwrap_or(other).replace('_', " "),
    }
}

fn summary(name: &str, input: &Value) -> String {
    let first = if name == "Bash" {
        input.get("command").cloned()
    } else {
        ["file_path", "pattern", "url", "query", "description"]
            .iter()
            .find_map(|k| input.get(*k).filter(|v| !v.is_null()).cloned())
    };
    let text = match first {
        Some(v) => text_of(Some(&v)),
        None => input.to_string(),
    };
    clip(&text, 400)
}

/// The steps of one stream-json message.
#[must_use]
pub fn steps_of(m: &Value) -> Vec<Map<String, Value>> {
    let mut out = Vec::new();
    let parent = m
        .get("parent_tool_use_id")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let step = |mut s: Map<String, Value>| {
        if let Some(parent) = &parent {
            s.insert("parent".to_owned(), Value::String(parent.clone()));
        }
        s
    };
    let kind = m.get("type").and_then(Value::as_str).unwrap_or_default();
    let content = m.get("message").and_then(|msg| msg.get("content"));
    if kind == "assistant"
        && let Some(Value::Array(content)) = content
    {
        for c in content {
            match c.get("type").and_then(Value::as_str) {
                Some("text") => {
                    let text = c.get("text").and_then(Value::as_str).unwrap_or_default();
                    if !text.trim().is_empty() {
                        out.push(step(object(
                            json!({ "kind": "text", "text": clip(text, 12_000) }),
                        )));
                    }
                }
                Some("tool_use") => {
                    let name = c.get("name").and_then(Value::as_str).unwrap_or_default();
                    let input = c.get("input").cloned().unwrap_or_else(|| json!({}));
                    let pretty = serde_json::to_string_pretty(&input).unwrap_or_default();
                    out.push(step(object(json!({
                        "kind": "tool",
                        "id": c.get("id").cloned().unwrap_or(Value::Null),
                        "name": c.get("name").cloned().unwrap_or(Value::Null),
                        "summary": summary(name, &input),
                        "doing": doing(name, &input),
                        "input": clip(&pretty, 4000),
                    }))));
                }
                _ => {}
            }
        }
    }
    if kind == "user"
        && let Some(Value::Array(results)) = content
    {
        for c in results {
            if c.get("type").and_then(Value::as_str) != Some("tool_result") {
                continue;
            }
            let text = match c.get("content") {
                Some(Value::Array(parts)) => parts
                    .iter()
                    .filter(|p| p.get("type").and_then(Value::as_str) == Some("text"))
                    .map(|p| text_of(p.get("text")))
                    .collect::<Vec<_>>()
                    .join("\n"),
                other => text_of(other),
            };
            out.push(step(object(json!({
                "kind": "result",
                "id": c.get("tool_use_id").cloned().unwrap_or(Value::Null),
                "error": truthy(c.get("is_error")),
                "text": clip(&text, 4000),
            }))));
        }
    }
    if kind == "result" {
        let outcome = if truthy(m.get("is_error")) {
            "failed"
        } else {
            "completed"
        };
        out.push(object(json!({ "kind": "end", "outcome": outcome })));
    }
    out
}

fn object(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(map) => map,
        _ => Map::new(),
    }
}

/// A worker's turn files, oldest first.
#[must_use]
pub fn turn_files(dir: &Path) -> Vec<PathBuf> {
    let Ok(read) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<(u64, String, PathBuf)> = read
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let ms = name.strip_prefix("turn-")?.strip_suffix(".json")?;
            let ms: u64 = ms.parse().ok()?;
            Some((ms, name, entry.path()))
        })
        .collect();
    files.sort();
    files.into_iter().map(|(_, _, path)| path).collect()
}

/// The steps in `dir`'s turn files after the step `from` (0: all), each with its `seq`, its
/// turn (`w-<task>:<file>`) and its time; with `agent`, only that sub-agent call's steps, as if
/// they were the session's.
#[must_use]
pub fn more(task: i64, dir: &Path, from: u64, agent: Option<&str>) -> Vec<Value> {
    let files = turn_files(dir);
    let skip = usize::try_from((from / PER_FILE).saturating_sub(1)).unwrap_or(usize::MAX);
    let mut out = Vec::new();
    for (index, path) in files.iter().enumerate().skip(skip) {
        let Ok(bytes) = std::fs::read(path) else {
            continue;
        };
        let Some(end) = bytes.iter().rposition(|b| *b == b'\n') else {
            continue;
        };
        let text = String::from_utf8_lossy(&bytes[..end]);
        let file = u64::try_from(index + 1).unwrap_or(u64::MAX);
        let turn = format!("w-{task}:{file}");
        for (number, line) in text.split('\n').enumerate() {
            let line_no = u64::try_from(number + 1).unwrap_or(u64::MAX);
            let seq = file * PER_FILE + line_no * 100;
            if seq + 99 <= from || !line.starts_with('{') {
                continue;
            }
            let Ok(message) = serde_json::from_str::<Value>(line) else {
                continue;
            };
            for (i, step) in steps_of(&message).into_iter().take(100).enumerate() {
                let parent = step
                    .get("parent")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                let seq = seq + u64::try_from(i).unwrap_or(99);
                if agent.is_some_and(|agent| parent.as_deref() != Some(agent)) || seq <= from {
                    continue;
                }
                let mut o = Map::new();
                o.insert("seq".to_owned(), Value::from(seq));
                o.insert("turn".to_owned(), Value::String(turn.clone()));
                o.insert(
                    "at".to_owned(),
                    message.get("timestamp").cloned().unwrap_or(Value::Null),
                );
                for (k, v) in step {
                    if k != "parent" || agent.is_none() {
                        o.insert(k, v);
                    }
                }
                out.push(Value::Object(o));
            }
        }
    }
    out
}
