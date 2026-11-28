//! The coding agent a session runs, behind [`Engine`]: Claude Code today ([`claude::ClaudeCode`],
//! through its own command line), others later.
//!
//! An engine starts an [`Agent`]: one process that takes the session's turns as user messages,
//! streams back what it does as JSON messages, answers control requests (interrupt, its account,
//! its use) and asks for permission when a tool needs someone to answer. The messages are
//! Claude Code's stream-json; another engine would translate its own into the same shapes.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::AsyncWriteExt as _;
use tokio::sync::{Mutex, mpsc, oneshot};

pub mod claude;

/// What an agent process is started with.
#[derive(Debug, Clone, Default)]
pub struct Launch {
    /// The folder it works in.
    pub cwd: PathBuf,
    /// Variables to set, on top of the runner's own environment.
    pub set_env: Vec<(String, String)>,
    /// Variables to remove from the runner's environment.
    pub remove_env: Vec<String>,
    /// The model, as the agent takes it.
    pub model: Option<String>,
    /// The effort (`low` … `max`).
    pub effort: Option<String>,
    /// Resume this agent session (its transcript exists), or start one with this id.
    pub session: SessionStart,
    /// Text appended to the agent's own system prompt.
    pub append_system_prompt: Option<String>,
    /// Settings on top of the account's own, as JSON.
    pub settings: Option<Value>,
    /// MCP servers it gets, as an `mcpServers` object.
    pub mcp_servers: Option<Value>,
    /// Folders it may reach besides its own.
    pub add_dirs: Vec<PathBuf>,
    /// Tools taken out of its reach.
    pub disallowed_tools: Vec<String>,
    /// Whether permission questions come to the runner (`can_use_tool`), so a question for the
    /// owner can go to their app.
    pub ask_runner: bool,
    /// Whether its words and thinking stream as they're written.
    pub partial_messages: bool,
    /// Only the MCP servers given here, none of the folder's or the account's (a probe).
    pub strict_mcp: bool,
}

/// How an agent session starts.
#[derive(Debug, Clone, Default)]
pub enum SessionStart {
    /// A new session the agent picks the id of (a probe).
    #[default]
    Fresh,
    /// A new session with this id.
    New(String),
    /// The session with this id, from its transcript.
    Resume(String),
}

/// What an agent process sends the runner.
#[derive(Debug)]
pub enum AgentOutput {
    /// A stream-json message (system, assistant, user, result, `stream_event`, rate limits…).
    Message(Value),
    /// A tool needs permission: answer it with [`Agent::answer`].
    Permission {
        /// The request's id.
        request_id: String,
        /// The tool.
        tool: String,
        /// Its input.
        input: Value,
        /// Its call's id, when the agent gives it.
        tool_use_id: Option<String>,
    },
    /// The process ended.
    Exited {
        /// Why, in words (its exit status and the end of what it wrote to stderr).
        why: String,
    },
}

/// An engine failure.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum EngineError {
    /// The agent's command didn't start.
    #[error("cannot start {command}: {source}")]
    Start {
        /// The command.
        command: String,
        /// Why.
        source: std::io::Error,
    },
    /// Writing to the agent failed (it has ended).
    #[error("the agent isn't taking input: {0}")]
    Write(std::io::Error),
    /// A control request got no answer in time.
    #[error("the agent didn't answer {0} in time")]
    Timeout(String),
    /// A control request was refused.
    #[error("the agent refused {subtype}: {why}")]
    Refused {
        /// The request.
        subtype: String,
        /// What it said.
        why: String,
    },
    /// The agent ended before answering.
    #[error("the agent ended")]
    Ended,
}

/// A coding agent the runner can start sessions on.
pub trait Engine: Send + Sync + std::fmt::Debug {
    /// Starts an agent process; its output comes on the returned receiver.
    ///
    /// # Errors
    /// The process can't be started.
    fn start(
        &self,
        launch: &Launch,
    ) -> Result<(Agent, mpsc::UnboundedReceiver<AgentOutput>), EngineError>;
}

/// Where the runner writes to an agent.
pub(crate) type Input = Box<dyn tokio::io::AsyncWrite + Send + Unpin>;

pub(crate) type Pending = Arc<Mutex<HashMap<String, oneshot::Sender<Result<Value, String>>>>>;

/// A running agent process: its input, its pending control requests and its stop handle.
#[derive(Clone)]
pub struct Agent {
    stdin: Arc<Mutex<Option<Input>>>,
    pending: Pending,
    next_id: Arc<AtomicU64>,
    kill: Arc<Mutex<Option<oneshot::Sender<()>>>>,
}

impl std::fmt::Debug for Agent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Agent").finish_non_exhaustive()
    }
}

impl Agent {
    pub(crate) fn new(stdin: Input, pending: Pending, kill: oneshot::Sender<()>) -> Self {
        Self {
            stdin: Arc::new(Mutex::new(Some(stdin))),
            pending,
            next_id: Arc::new(AtomicU64::new(1)),
            kill: Arc::new(Mutex::new(Some(kill))),
        }
    }

    async fn write(&self, line: &Value) -> Result<(), EngineError> {
        let mut guard = self.stdin.lock().await;
        let stdin = guard.as_mut().ok_or(EngineError::Ended)?;
        let mut bytes = line.to_string().into_bytes();
        bytes.push(b'\n');
        stdin.write_all(&bytes).await.map_err(EngineError::Write)?;
        stdin.flush().await.map_err(EngineError::Write)
    }

    /// Sends a user message: the agent takes it as its next turn (queued behind a running one).
    ///
    /// # Errors
    /// The agent has ended.
    pub async fn say(&self, text: &str) -> Result<(), EngineError> {
        self.write(&json!({ "type": "user", "message": { "role": "user", "content": text }, "parent_tool_use_id": null, "session_id": "" }))
            .await
    }

    /// Sends a control request and waits (up to `within`) for its answer.
    ///
    /// # Errors
    /// The agent has ended, refused, or didn't answer in time.
    pub async fn request(&self, request: Value, within: Duration) -> Result<Value, EngineError> {
        let subtype = request
            .get("subtype")
            .and_then(Value::as_str)
            .unwrap_or("request")
            .to_owned();
        let id = format!("runner-{}", self.next_id.fetch_add(1, Ordering::Relaxed));
        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(id.clone(), tx);
        self.write(&json!({ "type": "control_request", "request_id": id, "request": request }))
            .await?;
        match tokio::time::timeout(within, rx).await {
            Ok(Ok(Ok(v))) => Ok(v),
            Ok(Ok(Err(why))) => Err(EngineError::Refused { subtype, why }),
            Ok(Err(_)) => Err(EngineError::Ended),
            Err(_) => {
                self.pending.lock().await.remove(&id);
                Err(EngineError::Timeout(subtype))
            }
        }
    }

    /// Ends the running turn (queued messages stay).
    ///
    /// # Errors
    /// The agent has ended or didn't take it.
    pub async fn interrupt(&self) -> Result<(), EngineError> {
        self.request(json!({ "subtype": "interrupt" }), Duration::from_secs(15))
            .await
            .map(drop)
    }

    /// Answers a permission question: allowed as asked, or denied with words the agent reads.
    ///
    /// # Errors
    /// The agent has ended.
    pub async fn answer(
        &self,
        request_id: &str,
        allow: Result<Value, String>,
    ) -> Result<(), EngineError> {
        let response = match allow {
            Ok(input) => json!({ "behavior": "allow", "updatedInput": input }),
            Err(message) => json!({ "behavior": "deny", "message": message }),
        };
        self.write(&json!({ "type": "control_response", "response": { "subtype": "success", "request_id": request_id, "response": response } }))
            .await
    }

    /// Stops the process now (a running turn is cut off).
    pub async fn close(&self) {
        self.stdin.lock().await.take();
        if let Some(kill) = self.kill.lock().await.take() {
            // The process task may have ended already; nothing then to stop.
            kill.send(()).unwrap_or_default();
        }
    }
}
