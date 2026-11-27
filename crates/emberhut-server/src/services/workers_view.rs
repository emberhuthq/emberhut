//! What the runner pages see of the assistant's own workers and background tasks, which belong
//! to the assistant's runtime: a worker watched as the session `w-<task>` (its head, its steps,
//! a reply to it, stopping it), and stopping one of the assistant's background tasks (the
//! settings action `stopTask`).
//!
//! The runtime implements these traits: the workers service the worker sessions, the agent
//! runtime the background tasks. Without them (no agent CLI is configured) the server runs with
//! [`NoWorkers`]: no task has a worker, so the `w-{id}` routes answer 404 `no such task`, and
//! `stopTask` stops nothing.

use std::fmt;

use futures_util::future::BoxFuture;
use serde_json::{Map, Value};

/// The assistant's workers, as the session `w-<task>` shows them.
pub trait WorkerSessions: fmt::Debug + Send + Sync {
    /// The worker session's head (`{ id, title, status, running, doing, since, place, tasks,
    /// project, cwd }`), or `None` when there is no such task.
    fn head(&self, task: i64) -> BoxFuture<'_, Option<Map<String, Value>>>;

    /// Its steps after the step `after` (0: all), oldest first, each with its `seq`; with
    /// `agent`, only that sub-agent call's.
    fn steps<'a>(
        &'a self,
        task: i64,
        after: u64,
        agent: Option<&'a str>,
    ) -> BoxFuture<'a, Vec<Value>>;

    /// The person's reply: the worker carries on with it. Its answer, or why not.
    fn message(&self, task: i64, text: String)
    -> BoxFuture<'_, Result<Map<String, Value>, String>>;

    /// Stops the worker.
    fn stop(&self, task: i64) -> BoxFuture<'_, ()>;
}

/// The assistant's own session: its background tasks (its background agents and commands)
/// and its MCP servers.
pub trait AssistantTasks: fmt::Debug + Send + Sync {
    /// Stops the background task `id`; `false` when none runs by that id.
    fn stop<'a>(&'a self, id: &'a str) -> BoxFuture<'a, bool>;

    /// The main room's MCP servers as its open session sees them now (`[{ name, status,
    /// error, scope, config }]`); `None` when it isn't open.
    fn mcp_status(&self) -> BoxFuture<'_, Option<Vec<Value>>> {
        Box::pin(async { None })
    }

    /// Reconnects the MCP server `name` in the open session.
    fn reconnect<'a>(&'a self, _name: &'a str) -> BoxFuture<'a, Result<(), String>> {
        Box::pin(async { Err("the assistant's session isn't open right now".to_owned()) })
    }

    /// The servers or skills turned on or off: the rooms restart with the new settings once
    /// their turn is over.
    fn reload<'a>(&'a self, _why: &'a str) -> BoxFuture<'a, ()> {
        Box::pin(async {})
    }
}

/// No runtime: no worker, no background task. What the server runs with when no agent CLI is
/// configured.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoWorkers;

impl WorkerSessions for NoWorkers {
    fn head(&self, _task: i64) -> BoxFuture<'_, Option<Map<String, Value>>> {
        Box::pin(async { None })
    }

    fn steps<'a>(
        &'a self,
        _task: i64,
        _after: u64,
        _agent: Option<&'a str>,
    ) -> BoxFuture<'a, Vec<Value>> {
        Box::pin(async { Vec::new() })
    }

    fn message(
        &self,
        _task: i64,
        _text: String,
    ) -> BoxFuture<'_, Result<Map<String, Value>, String>> {
        Box::pin(async { Err("no such task".to_owned()) })
    }

    fn stop(&self, _task: i64) -> BoxFuture<'_, ()> {
        Box::pin(async {})
    }
}

impl AssistantTasks for NoWorkers {
    fn stop<'a>(&'a self, _id: &'a str) -> BoxFuture<'a, bool> {
        Box::pin(async { false })
    }
}

/// The page of `all` a transcript shows: after a step, the first `limit` of what came since;
/// else the newest `limit`, and whether older ones exist.
#[must_use]
pub fn page(all: Vec<Value>, after: u64, limit: usize) -> (Vec<Value>, bool) {
    if after != 0 {
        return (all.into_iter().take(limit).collect(), false);
    }
    let earlier = all.len() > limit;
    let skip = all.len().saturating_sub(limit);
    (all.into_iter().skip(skip).collect(), earlier)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn a_page_is_the_newest_or_what_came_after() {
        let all: Vec<Value> = (1..=5).map(|n| json!({ "seq": n })).collect();
        let (newest, earlier) = page(all.clone(), 0, 2);
        assert_eq!(newest, [json!({ "seq": 4 }), json!({ "seq": 5 })]);
        assert!(earlier);
        let (after, earlier) = page(all[2..].to_vec(), 2, 2);
        assert_eq!(after, [json!({ "seq": 3 }), json!({ "seq": 4 })]);
        assert!(!earlier);
    }
}
