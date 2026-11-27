//! An agent CLI the tests script: it reads the lines the server writes and answers each with
//! the lines a script gives, as the real CLI's stream-json would, and keeps every launch and
//! line. No process runs.
#![expect(
    clippy::unwrap_used,
    reason = "test helpers: a failure here fails the test that called them"
)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use emberhut_core::ManualClock;
use emberhut_db::Database;
use emberhut_server::services::claude::{
    AgentAccount, ClaudeAssistant, ClaudeConfig, CliPipes, CliSpawner, Launch, SpawnError,
};
use emberhut_server::services::talk::Assistant;
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

/// What a script answers to end the session there: the CLI exits.
pub(crate) const EXIT: &str = "EXIT";

/// What the script answers one line with.
type Script = dyn Fn(&Launch, &Value) -> Vec<String> + Send + Sync;

/// The scripted CLI.
pub(crate) struct FakeCli {
    script: Arc<Script>,
    launches: Mutex<Vec<Launch>>,
    lines: Arc<Mutex<Vec<(usize, Value)>>>,
    stops: Mutex<Vec<CancellationToken>>,
}

impl std::fmt::Debug for FakeCli {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FakeCli").finish_non_exhaustive()
    }
}

impl FakeCli {
    /// A CLI answering each line with what `script` gives for it.
    pub(crate) fn new(
        script: impl Fn(&Launch, &Value) -> Vec<String> + Send + Sync + 'static,
    ) -> Arc<Self> {
        Arc::new(Self {
            script: Arc::new(script),
            launches: Mutex::default(),
            lines: Arc::default(),
            stops: Mutex::default(),
        })
    }

    /// Which launches (by index) have been stopped.
    pub(crate) fn stopped(&self) -> Vec<usize> {
        let stops = self.stops.lock().unwrap();
        (0..stops.len())
            .filter(|index| stops[*index].is_cancelled())
            .collect()
    }

    /// Every launch so far.
    pub(crate) fn launches(&self) -> Vec<Launch> {
        self.launches.lock().unwrap().clone()
    }

    /// Every line the server wrote, with the launch (its index) it went to.
    pub(crate) fn lines(&self) -> Vec<(usize, Value)> {
        self.lines.lock().unwrap().clone()
    }

    /// The texts of the user messages the server wrote, in order.
    pub(crate) fn said(&self) -> Vec<String> {
        self.lines()
            .into_iter()
            .filter(|(_, line)| line["type"] == "user")
            .map(|(_, line)| line["message"]["content"].as_str().unwrap().to_owned())
            .collect()
    }
}

impl CliSpawner for FakeCli {
    fn spawn(&self, launch: &Launch) -> Result<CliPipes, SpawnError> {
        let index = {
            let mut launches = self.launches.lock().unwrap();
            launches.push(launch.clone());
            launches.len() - 1
        };
        let (input, mut written) = mpsc::unbounded_channel::<String>();
        let (answer, output) = mpsc::unbounded_channel::<String>();
        let script = Arc::clone(&self.script);
        let lines = Arc::clone(&self.lines);
        let launch = launch.clone();
        tokio::spawn(async move {
            while let Some(line) = written.recv().await {
                let line: Value = serde_json::from_str(&line).unwrap();
                lines.lock().unwrap().push((index, line.clone()));
                for out in script(&launch, &line) {
                    if out == EXIT {
                        return;
                    }
                    if answer.send(out).is_err() {
                        return;
                    }
                }
            }
        });
        let stop = CancellationToken::new();
        self.stops.lock().unwrap().push(stop.clone());
        Ok(CliPipes {
            input,
            output,
            // It is gone as soon as it is stopped.
            exited: stop.child_token(),
            stop,
        })
    }
}

/// The settings the tests run the CLI with: `accounts` (`ID=DIR`), an MCP URL of its own.
pub(crate) fn config(accounts: &[(&str, &str)]) -> ClaudeConfig {
    ClaudeConfig {
        command: "agent-cli".into(),
        args: vec!["--add-dir=/work".to_owned()],
        model: Some("a-model".to_owned()),
        workdir: Some("/work".into()),
        permission_mode: Some("dontAsk".to_owned()),
        accounts: accounts
            .iter()
            .map(|(id, dir)| AgentAccount {
                id: (*id).to_owned(),
                config_dir: Some((*dir).into()),
            })
            .collect(),
        mcp_url: format!("http://127.0.0.1:18990{}", emberhut_api::paths::MCP_ROOM),
        prompt: Some("Answer in one line.".to_owned()),
        max_sessions: 4,
    }
}

/// The runtime over `cli`, for [`crate::common::TestApp::with_agent`].
pub(crate) fn agent(
    cli: &Arc<FakeCli>,
    config: ClaudeConfig,
) -> impl FnOnce(&Database, Arc<ManualClock>) -> Arc<dyn Assistant> {
    agent_kept(cli, config, Arc::default())
}

/// [`agent`], keeping the runtime it makes in `kept` too (to reach its background tasks).
pub(crate) fn agent_kept(
    cli: &Arc<FakeCli>,
    config: ClaudeConfig,
    kept: Arc<Mutex<Option<Arc<ClaudeAssistant>>>>,
) -> impl FnOnce(&Database, Arc<ManualClock>) -> Arc<dyn Assistant> {
    let cli = Arc::clone(cli);
    move |db, clock| {
        let plan = emberhut_server::services::plan::PlanService::new(
            db.settings(),
            db.log(),
            Arc::clone(&clock) as _,
        );
        let rules = emberhut_server::services::learning::LearningService::new(
            db.corrections(),
            db.settings(),
            db.conversation(),
            Arc::clone(&clock) as _,
        );
        let runtime = Arc::new(
            ClaudeAssistant::new(config, cli, db.settings(), db.rooms(), clock)
                .with_jobs(plan)
                .with_rules(rules)
                .with_accounts(db.prefs()),
        );
        *kept.lock().unwrap() = Some(Arc::clone(&runtime));
        runtime as Arc<dyn Assistant>
    }
}

/// `system/init` with this session id.
pub(crate) fn init(session: &str) -> String {
    json!({ "type": "system", "subtype": "init", "session_id": session }).to_string()
}

/// An assistant message with a note and a tool call.
pub(crate) fn step(note: &str, tool: &str) -> String {
    json!({ "type": "assistant", "parent_tool_use_id": null, "message": { "content": [
        { "type": "text", "text": note },
        { "type": "tool_use", "name": tool, "input": { "command": "ls" } },
    ] } })
    .to_string()
}

/// A successful result with this answer.
pub(crate) fn result(text: &str) -> String {
    json!({ "type": "result", "subtype": "success", "is_error": false, "result": text,
        "num_turns": 1 })
    .to_string()
}

/// An error result with this text, as the CLI says an account ran out.
pub(crate) fn error_result(text: &str) -> String {
    json!({ "type": "result", "subtype": "success", "is_error": true, "result": text,
        "num_turns": 1 })
    .to_string()
}

/// A rejected rate-limit notice for the five-hour window, out until `resets_at` (seconds).
pub(crate) fn rejected(resets_at: i64) -> String {
    json!({ "type": "rate_limit_event", "rate_limit_info": { "status": "rejected",
        "rateLimitType": "five_hour", "resetsAt": resets_at } })
    .to_string()
}

/// Whether `line` is a user message.
pub(crate) fn is_user(line: &Value) -> bool {
    line["type"] == "user"
}

/// Whether `line` is the control request `subtype`.
pub(crate) fn is_request(line: &Value, subtype: &str) -> bool {
    line["type"] == "control_request" && line["request"]["subtype"] == subtype
}

/// Waits (a little, polling) until `done` holds.
pub(crate) async fn until(mut done: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while !done() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
}
