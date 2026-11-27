//! The assistant's workers and background tasks, as the test scripts them.
#![expect(
    clippy::unwrap_used,
    reason = "test helpers: a poisoned lock fails the test"
)]

use std::collections::BTreeMap;
use std::sync::Mutex;

use emberhut_server::services::workers_view::{AssistantTasks, WorkerSessions};
use futures_util::future::BoxFuture;
use serde_json::{Map, Value, json};

#[derive(Debug, Default)]
struct Script {
    heads: BTreeMap<i64, Map<String, Value>>,
    steps: BTreeMap<i64, Vec<Value>>,
    messages: Vec<(i64, String)>,
    refuse: Option<String>,
    stopped: Vec<i64>,
    tasks: Vec<String>,
    background_stopped: Vec<String>,
    mcp: Option<Vec<Value>>,
    reconnected: Vec<String>,
}

/// The fake workers.
#[derive(Debug, Default)]
pub(crate) struct FakeWorkers {
    script: Mutex<Script>,
}

impl FakeWorkers {
    /// Task `task` has a worker titled `title`, with steps `1..=steps`.
    pub(crate) fn worker(&self, task: i64, title: &str, steps: u64) {
        let mut script = self.script.lock().unwrap();
        let head = json!({ "id": format!("w-{task}"), "title": title, "status": "running", "running": true });
        script.heads.insert(task, head.as_object().unwrap().clone());
        script.steps.insert(
            task,
            (1..=steps)
                .map(|seq| json!({ "seq": seq, "kind": "text", "text": format!("step {seq}") }))
                .collect(),
        );
    }

    /// The worker refuses replies, saying `why`.
    pub(crate) fn refuse(&self, why: &str) {
        self.script.lock().unwrap().refuse = Some(why.to_owned());
    }

    /// The background task `id` runs.
    pub(crate) fn background(&self, id: &str) {
        self.script.lock().unwrap().tasks.push(id.to_owned());
    }

    /// The replies sent so far.
    pub(crate) fn messages(&self) -> Vec<(i64, String)> {
        self.script.lock().unwrap().messages.clone()
    }

    /// The workers stopped so far.
    pub(crate) fn stopped(&self) -> Vec<i64> {
        self.script.lock().unwrap().stopped.clone()
    }

    /// The assistant's session is open, with these MCP servers.
    pub(crate) fn open_session(&self, servers: Vec<Value>) {
        self.script.lock().unwrap().mcp = Some(servers);
    }

    /// The servers reconnected so far.
    pub(crate) fn reconnected(&self) -> Vec<String> {
        self.script.lock().unwrap().reconnected.clone()
    }

    /// The background tasks stopped so far.
    pub(crate) fn background_stopped(&self) -> Vec<String> {
        self.script.lock().unwrap().background_stopped.clone()
    }
}

impl WorkerSessions for FakeWorkers {
    fn head(&self, task: i64) -> BoxFuture<'_, Option<Map<String, Value>>> {
        let head = self.script.lock().unwrap().heads.get(&task).cloned();
        Box::pin(async move { head })
    }

    fn steps<'a>(
        &'a self,
        task: i64,
        after: u64,
        _agent: Option<&'a str>,
    ) -> BoxFuture<'a, Vec<Value>> {
        let steps: Vec<Value> = self
            .script
            .lock()
            .unwrap()
            .steps
            .get(&task)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter(|step| step["seq"].as_u64().unwrap() > after)
            .collect();
        Box::pin(async move { steps })
    }

    fn message(
        &self,
        task: i64,
        text: String,
    ) -> BoxFuture<'_, Result<Map<String, Value>, String>> {
        let mut script = self.script.lock().unwrap();
        let answer = if let Some(why) = &script.refuse {
            Err(why.clone())
        } else {
            script.messages.push((task, text));
            Ok(json!({ "to": "worker" }).as_object().unwrap().clone())
        };
        Box::pin(async move { answer })
    }

    fn stop(&self, task: i64) -> BoxFuture<'_, ()> {
        self.script.lock().unwrap().stopped.push(task);
        Box::pin(async {})
    }
}

impl AssistantTasks for FakeWorkers {
    fn stop<'a>(&'a self, id: &'a str) -> BoxFuture<'a, bool> {
        let mut script = self.script.lock().unwrap();
        let running = script.tasks.iter().any(|task| task == id);
        if running {
            script.background_stopped.push(id.to_owned());
        }
        Box::pin(async move { running })
    }

    fn mcp_status(&self) -> BoxFuture<'_, Option<Vec<Value>>> {
        let servers = self.script.lock().unwrap().mcp.clone();
        Box::pin(async move { servers })
    }

    fn reconnect<'a>(&'a self, name: &'a str) -> BoxFuture<'a, Result<(), String>> {
        let mut script = self.script.lock().unwrap();
        let open = script.mcp.is_some();
        if open {
            script.reconnected.push(name.to_owned());
        }
        Box::pin(async move {
            if open {
                Ok(())
            } else {
                Err("the assistant's session isn't open right now".to_owned())
            }
        })
    }
}
