//! A worker's session of the agent CLI: one task's run, started with its brief and read until
//! its result. A worker reaches the server's worker endpoint (`POST /mcp/worker`, its tools act
//! on its own task) with its task's token in its environment ([`MCP_TOKEN_VAR`]), never on its
//! command line; so is the model provider the owner saved, while no account is configured
//! ([`ProviderStore::env`]). Its permission mode is the configured one (`dontAsk` by default, its own
//! server's tools allowed). Unless the owner's words cover its outward steps, its push, merge
//! and release commands are refused outright and it gets the gate as a hook before each shell
//! command; an ultra run's lead gets the gate before each new sub-agent as well. The hook asks
//! the server (`POST /ultra/gate`) and refuses when the server doesn't answer.

use std::path::PathBuf;

use serde_json::json;

use emberhut_db::DbError;

use super::cli::{CliPipes, CliSpawner, Launch, SpawnError};
use super::stream::{self, MCP_TOKEN_VAR, Said};
use super::{AgentAccount, TOOL_TIMEOUT_MS};
use crate::services::agent_provider::{ProviderEnv, ProviderStore};

/// The shell commands a worker may never run unless the owner's words cover its outward steps:
/// matched by their first words only, so they back the gate up and never stand in for it.
pub const DENY: [&str; 10] = [
    "git push",
    "gh pr merge",
    "gh pr create",
    "gh pr comment",
    "gh pr review",
    "gh issue comment",
    "gh issue create",
    "gh release",
    "gh repo delete",
    "gh api",
];

/// How workers run the agent CLI.
#[derive(Clone, Debug)]
pub struct WorkerConfig {
    /// The CLI's program.
    pub command: PathBuf,
    /// Arguments added after the server's own.
    pub args: Vec<String>,
    /// The model, when the task names none.
    pub model: Option<String>,
    /// The CLI's permission mode; `dontAsk` when none.
    pub permission_mode: Option<String>,
    /// The accounts, in order (the first with room takes a worker).
    pub accounts: Vec<AgentAccount>,
    /// The worker endpoint, as the CLI reaches it.
    pub mcp_url: String,
    /// The ultra gate, as a hook reaches it.
    pub gate_url: String,
    /// Where workers run when their task names no folder.
    pub workdir: Option<PathBuf>,
    /// Where the model provider workers run on is saved; none: their account's or the CLI's
    /// own login.
    pub provider: Option<ProviderStore>,
}

/// One worker run to start.
#[derive(Clone, Debug)]
pub struct WorkerLaunch {
    /// The brief: the first message.
    pub brief: String,
    /// The session to carry on, when it is a resume.
    pub resume: Option<String>,
    /// It leads an ultra run: the gate is its hook before each new sub-agent.
    pub ultra: bool,
    /// Its outward commands are refused, and the gate is its hook before each shell command.
    pub deny_outward: bool,
    /// The task's model.
    pub model: Option<String>,
    /// The task's scratch folder (added to what it may edit).
    pub folder: PathBuf,
    /// The account it runs on.
    pub account: Option<String>,
    /// On a configured agent's task: the worker runs apart from the owner's setup.
    pub agent: Option<AgentWorker>,
}

/// How a worker on a configured agent's task runs: none of the owner's settings, memory, hooks
/// or extra arguments, `dontAsk`, in its task's folder, with only the file tools (and the shell
/// when its agent allows one), each behind the gate, which keeps the file tools to the agent's
/// own folders.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AgentWorker {
    /// Its agent lets its workers run shell commands, as the server's user.
    pub shell: bool,
}

/// The built-in tools a worker on an agent's task has, without a shell.
pub const AGENT_WORKER_TOOLS: &str = "Read,Glob,Grep,Write,Edit";

/// The tools the gate runs before for a worker on an agent's task: every built-in it has.
pub const GATED_AGENT: &str = "Read|Glob|Grep|Write|Edit|Bash";

/// The tools the gate runs before for a worker: its shell commands.
pub const GATED_SHELL: &str = "Bash";

/// The tools the gate runs before for an ultra run's lead: each new sub-agent and its shell
/// commands.
pub const GATED_LEAD: &str = "Agent|Task|Workflow|Bash";

/// The hook settings a worker runs with: the gate before each tool `matcher` names, the
/// server's answer passed on, a refusal when the server doesn't answer.
#[must_use]
pub fn gate_settings(gate_url: &str, matcher: &str) -> String {
    let deny = json!({ "hookSpecificOutput": {
        "hookEventName": "PreToolUse",
        "permissionDecision": "deny",
        "permissionDecisionReason": "The server didn't answer the gate; wait a minute and try again.",
    }})
    .to_string();
    let quote = |x: &str| format!("'{}'", x.replace('\'', r"'\''"));
    let command = format!(
        "curl -s -m 20 -f -X POST -H \"Authorization: Bearer ${MCP_TOKEN_VAR}\" -H 'Content-Type: application/json' --data-binary @- {} || printf '%s' {}",
        quote(gate_url),
        quote(&deny)
    );
    json!({ "hooks": { "PreToolUse": [{
        "matcher": matcher,
        "hooks": [{ "type": "command", "command": command, "timeout": 30 }],
    }]}})
    .to_string()
}

impl WorkerConfig {
    /// The provider a worker starting now runs on: the saved one while no account is
    /// configured.
    ///
    /// # Errors
    ///
    /// [`DbError`] when it can't be read.
    pub async fn session_provider(&self) -> Result<Option<ProviderEnv>, DbError> {
        match &self.provider {
            Some(store) => store.env(&self.accounts).await,
            None => Ok(None),
        }
    }

    /// What the CLI runs with for `run`, its token given as `token`, on `provider` when there
    /// is one.
    #[must_use]
    pub fn launch(
        &self,
        run: &WorkerLaunch,
        token: &str,
        provider: Option<&ProviderEnv>,
    ) -> Launch {
        let mut args: Vec<String> = [
            "--output-format",
            "stream-json",
            "--verbose",
            "--input-format",
            "stream-json",
        ]
        .map(str::to_owned)
        .to_vec();
        if let Some(model) = run.model.as_ref().or(self.model.as_ref()) {
            args.push(format!("--model={model}"));
        }
        let mode = match run.agent {
            Some(_) => "dontAsk",
            None => self.permission_mode.as_deref().unwrap_or("dontAsk"),
        };
        args.push(format!("--permission-mode={mode}"));
        if let Some(session) = &run.resume {
            args.push(format!("--resume={session}"));
        }
        args.push("--mcp-config".to_owned());
        args.push(stream::mcp_config(
            emberhut_api::mcp::SERVER_NAME,
            &self.mcp_url,
        ));
        args.push("--strict-mcp-config".to_owned());
        let own = format!("mcp__{}__*", emberhut_api::mcp::SERVER_NAME);
        match run.agent {
            Some(agent) => {
                let tools = if agent.shell {
                    format!("{AGENT_WORKER_TOOLS},Bash")
                } else {
                    AGENT_WORKER_TOOLS.to_owned()
                };
                args.push("--setting-sources=".to_owned());
                args.push(format!("--tools={tools}"));
                args.push(format!("--allowedTools={own},{tools}"));
            }
            None => args.push(format!("--allowedTools={own}")),
        }
        args.push(format!("--add-dir={}", run.folder.display()));
        if run.deny_outward {
            args.push("--disallowedTools".to_owned());
            args.extend(DENY.iter().map(|c| format!("Bash({c}:*)")));
        }
        let gated = if run.agent.is_some() {
            Some(GATED_AGENT)
        } else if run.ultra {
            Some(GATED_LEAD)
        } else {
            run.deny_outward.then_some(GATED_SHELL)
        };
        if let Some(matcher) = gated {
            args.push("--settings".to_owned());
            args.push(gate_settings(&self.gate_url, matcher));
        }
        if run.agent.is_none() {
            args.extend(self.args.iter().cloned());
        }
        let mut env = vec![
            ("MCP_TOOL_TIMEOUT".to_owned(), TOOL_TIMEOUT_MS.to_owned()),
            (MCP_TOKEN_VAR.to_owned(), token.to_owned()),
        ];
        let mut env_remove = vec!["CLAUDECODE".to_owned()];
        let dir = run.account.as_deref().and_then(|id| {
            self.accounts
                .iter()
                .find(|known| known.id == id)
                .and_then(|known| known.config_dir.clone())
        });
        match dir {
            Some(dir) => env.push(("CLAUDE_CONFIG_DIR".to_owned(), dir.display().to_string())),
            None if run.account.is_some() => env_remove.push("CLAUDE_CONFIG_DIR".to_owned()),
            None => {}
        }
        let mut launch = Launch {
            program: self.command.clone(),
            args,
            env,
            env_remove,
            env_remove_prefixed: Vec::new(),
            cwd: if run.agent.is_some() {
                Some(run.folder.clone())
            } else {
                self.workdir.clone()
            },
            bridge: None,
        };
        if let Some(provider) = provider {
            provider.apply(&mut launch);
        }
        launch
    }
}

/// What a worker is told about where it runs, after the CLI's own system prompt.
pub const WORKER_RULES: &str = "You are one of the owner's assistant's workers: you own exactly \
one task and work on it alone, through your MCP server's task tools (task, task_note, \
task_findings, task_ask, task_runner, task_done). Messages are the server's: your brief, the \
owner's answers and replies, a changed brief.";

/// A worker's running session: its lines as they come.
#[derive(Debug)]
pub struct WorkerSession {
    pipes: CliPipes,
}

impl WorkerSession {
    /// Starts `launch` with `spawner`, and gives it its brief.
    ///
    /// # Errors
    ///
    /// [`SpawnError`] when the CLI could not start.
    pub fn start(
        spawner: &dyn CliSpawner,
        launch: &Launch,
        brief: &str,
    ) -> Result<Self, SpawnError> {
        let pipes = spawner.spawn(launch)?;
        if !pipes.send(stream::initialize("init-1", WORKER_RULES)) {
            tracing::debug!("the worker ended before it was set up");
        }
        if !pipes.send(stream::user_message(brief)) {
            tracing::debug!("the worker ended before its brief");
        }
        Ok(Self { pipes })
    }

    /// The next line it writes, or none once it has exited. A control request the server does
    /// not serve is refused as it passes.
    pub async fn line(&mut self) -> Option<String> {
        let line = self.pipes.output.recv().await?;
        if let Said::ControlRequest { id, subtype } = Said::read(&line)
            && !self.pipes.send(stream::unsupported(&id, &subtype))
        {
            tracing::debug!("the worker ended before its request was answered");
        }
        Some(line)
    }

    /// Stops it, and resolves once it has exited.
    pub async fn close(self) {
        self.pipes.close().await;
    }
}
