//! Claude Code, through its own command line: `claude -p` with stream-json in and out, one
//! process per open session, its account chosen by `CLAUDE_CONFIG_DIR`.
//!
//! The runner writes user messages and control requests (`interrupt`, `initialize`,
//! `get_usage`) as JSON lines on its standard input and reads every message, control answer and
//! permission question (`can_use_tool`, sent because of `--permission-prompt-tool=stdio`) from its
//! standard output.

use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;

use serde_json::Value;
use tokio::io::{AsyncBufReadExt as _, AsyncReadExt as _, BufReader};
use tokio::process::Command;
use tokio::sync::{Mutex, mpsc, oneshot};

use super::{Agent, AgentOutput, Engine, EngineError, Launch, Pending, SessionStart};

/// The most of a line the runner reads: a message carrying a whole file or picture can be large.
const MAX_LINE: usize = 64 * 1024 * 1024;
/// How much of the agent's standard error is kept for saying why it ended.
const STDERR_TAIL: usize = 2000;

/// Claude Code's command line.
#[derive(Debug, Clone)]
pub struct ClaudeCode {
    command: PathBuf,
}

impl ClaudeCode {
    /// The agent at `command` (`claude` on the `PATH`, or a path).
    #[must_use]
    pub fn new(command: PathBuf) -> Self {
        Self { command }
    }

    /// The arguments a launch becomes.
    #[must_use]
    pub fn args(launch: &Launch) -> Vec<String> {
        let mut a: Vec<String> = [
            "-p",
            "--output-format",
            "stream-json",
            "--verbose",
            "--input-format",
            "stream-json",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect();
        // Every session runs with full access in its folder, as a terminal session with the flag
        // would; since `-p` without a mode starts in the asking mode, the mode is always given.
        a.extend([
            "--permission-mode".to_owned(),
            "bypassPermissions".to_owned(),
            "--allow-dangerously-skip-permissions".to_owned(),
        ]);
        if launch.partial_messages {
            a.push("--include-partial-messages".to_owned());
            a.extend([
                "--thinking=adaptive".to_owned(),
                "--thinking-display=summarized".to_owned(),
            ]);
        }
        if launch.ask_runner {
            a.push("--permission-prompt-tool=stdio".to_owned());
        }
        match &launch.session {
            SessionStart::Fresh => {}
            SessionStart::New(id) => a.extend(["--session-id".to_owned(), id.clone()]),
            SessionStart::Resume(id) => a.extend(["--resume".to_owned(), id.clone()]),
        }
        if let Some(m) = &launch.model {
            a.extend(["--model".to_owned(), m.clone()]);
        }
        if let Some(e) = &launch.effort {
            a.extend(["--effort".to_owned(), e.clone()]);
        }
        if !launch.disallowed_tools.is_empty() {
            a.push("--disallowedTools".to_owned());
            a.extend(launch.disallowed_tools.iter().cloned());
        }
        if let Some(s) = &launch.settings {
            a.extend(["--settings".to_owned(), s.to_string()]);
        }
        if let Some(p) = &launch.append_system_prompt {
            a.extend(["--append-system-prompt".to_owned(), p.clone()]);
        }
        if let Some(m) = &launch.mcp_servers {
            a.extend([
                "--mcp-config".to_owned(),
                serde_json::json!({ "mcpServers": m }).to_string(),
            ]);
        }
        if launch.strict_mcp {
            a.push("--strict-mcp-config".to_owned());
        }
        for d in &launch.add_dirs {
            a.extend(["--add-dir".to_owned(), d.display().to_string()]);
        }
        a
    }
}

impl Engine for ClaudeCode {
    fn start(
        &self,
        launch: &Launch,
    ) -> Result<(Agent, mpsc::UnboundedReceiver<AgentOutput>), EngineError> {
        let mut cmd = Command::new(&self.command);
        cmd.args(Self::args(launch))
            .current_dir(&launch.cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        for k in &launch.remove_env {
            cmd.env_remove(k);
        }
        for (k, v) in &launch.set_env {
            cmd.env(k, v);
        }
        let mut child = cmd.spawn().map_err(|source| EngineError::Start {
            command: self.command.display().to_string(),
            source,
        })?;
        let (Some(stdin), Some(stdout), Some(stderr)) =
            (child.stdin.take(), child.stdout.take(), child.stderr.take())
        else {
            return Err(EngineError::Start {
                command: self.command.display().to_string(),
                source: std::io::Error::other("no pipes"),
            });
        };
        let pending: Pending = Arc::default();
        let (out_tx, out_rx) = mpsc::unbounded_channel();
        let (kill_tx, kill_rx) = oneshot::channel::<()>();
        let agent = Agent::new(Box::new(stdin), Arc::clone(&pending), kill_tx);

        let tail = Arc::new(Mutex::new(String::new()));
        let tail_w = Arc::clone(&tail);
        tokio::spawn(async move {
            let mut buf = Vec::new();
            let mut stderr = stderr;
            let mut chunk = [0_u8; 4096];
            while let Ok(n) = stderr.read(&mut chunk).await {
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&chunk[..n]);
                if buf.len() > STDERR_TAIL * 4 {
                    buf.drain(..buf.len() - STDERR_TAIL);
                }
            }
            let text = String::from_utf8_lossy(&buf);
            let start = text.len().saturating_sub(STDERR_TAIL);
            *tail_w.lock().await = text.get(start..).unwrap_or_default().trim().to_owned();
        });

        let reading = tokio::spawn(read_lines(stdout, out_tx.clone(), Arc::clone(&pending)));

        tokio::spawn(async move {
            tokio::select! {
                status = child.wait() => {
                    drop(reading.await);
                    let why = match status {
                        Ok(s) => s.code().map_or_else(|| "ended by a signal".to_owned(), |c| format!("exited with status {c}")),
                        Err(e) => format!("couldn't be waited on: {e}"),
                    };
                    let tail = tail.lock().await.clone();
                    let why = if tail.is_empty() { why } else { format!("{why}: {tail}") };
                    pending.lock().await.clear();
                    drop(out_tx.send(AgentOutput::Exited { why }));
                }
                _ = kill_rx => {
                    drop(child.start_kill());
                    drop(child.wait().await);
                    reading.abort();
                    pending.lock().await.clear();
                }
            }
        });
        Ok((agent, out_rx))
    }
}

/// Reads the agent's output, one JSON line at a time, until it ends.
async fn read_lines<R: tokio::io::AsyncRead + Unpin>(
    output: R,
    out: mpsc::UnboundedSender<AgentOutput>,
    pending: Pending,
) {
    let mut lines = BufReader::with_capacity(1 << 16, output);
    let mut line = Vec::new();
    loop {
        line.clear();
        match lines.read_until(b'\n', &mut line).await {
            Ok(0) | Err(_) => break,
            Ok(_) if line.len() > MAX_LINE => continue,
            Ok(_) => {}
        }
        let Ok(v) = serde_json::from_slice::<Value>(&line) else {
            continue;
        };
        route(v, &out, &pending).await;
    }
}

/// An agent over any pair of pipes, speaking Claude Code's stream-json: the runner writes to
/// `input` and reads `output`; the agent ends when its output does. For a stand-in agent (the
/// tests'), or one reached some other way than a child process.
pub fn piped<W, R>(input: W, output: R) -> (Agent, mpsc::UnboundedReceiver<AgentOutput>)
where
    W: tokio::io::AsyncWrite + Send + Unpin + 'static,
    R: tokio::io::AsyncRead + Send + Unpin + 'static,
{
    let pending: Pending = Arc::default();
    let (out_tx, out_rx) = mpsc::unbounded_channel();
    let (kill_tx, kill_rx) = oneshot::channel::<()>();
    let agent = Agent::new(Box::new(input), Arc::clone(&pending), kill_tx);
    let reading = tokio::spawn(read_lines(output, out_tx.clone(), Arc::clone(&pending)));
    tokio::spawn(async move {
        tokio::select! {
            _ = reading => {
                pending.lock().await.clear();
                drop(out_tx.send(AgentOutput::Exited { why: "its output ended".to_owned() }));
            }
            _ = kill_rx => pending.lock().await.clear(),
        }
    });
    (agent, out_rx)
}

/// One line from the agent: a control answer resolves its request, a permission question and every
/// other message go to the session.
async fn route(v: Value, out: &mpsc::UnboundedSender<AgentOutput>, pending: &Pending) {
    match v.get("type").and_then(Value::as_str) {
        Some("control_response") => {
            let r = v.get("response").cloned().unwrap_or(Value::Null);
            let Some(id) = r.get("request_id").and_then(Value::as_str) else {
                return;
            };
            if let Some(tx) = pending.lock().await.remove(id) {
                let answer = if r.get("subtype").and_then(Value::as_str) == Some("error") {
                    Err(r
                        .get("error")
                        .and_then(Value::as_str)
                        .unwrap_or("refused")
                        .to_owned())
                } else {
                    Ok(r.get("response").cloned().unwrap_or(Value::Null))
                };
                drop(tx.send(answer));
            }
        }
        Some("control_request") => {
            let id = v
                .get("request_id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            let r = v.get("request").cloned().unwrap_or(Value::Null);
            if r.get("subtype").and_then(Value::as_str) == Some("can_use_tool") {
                drop(
                    out.send(AgentOutput::Permission {
                        request_id: id,
                        tool: r
                            .get("tool_name")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_owned(),
                        input: r.get("input").cloned().unwrap_or(Value::Null),
                        tool_use_id: r
                            .get("tool_use_id")
                            .and_then(Value::as_str)
                            .map(str::to_owned),
                    }),
                );
            }
        }
        Some("keep_alive" | "control_cancel_request") => {}
        _ => drop(out.send(AgentOutput::Message(v))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_launch_becomes_the_agents_arguments() {
        let launch = Launch {
            model: Some("opus".to_owned()),
            effort: Some("high".to_owned()),
            session: SessionStart::Resume("0000-1".to_owned()),
            disallowed_tools: vec!["AskUserQuestion".to_owned()],
            partial_messages: true,
            ..Launch::default()
        };
        let a = ClaudeCode::args(&launch).join(" ");
        assert!(a.starts_with("-p --output-format stream-json --verbose --input-format stream-json --permission-mode bypassPermissions"));
        assert!(
            a.contains("--resume 0000-1")
                && a.contains("--model opus")
                && a.contains("--effort high")
        );
        assert!(
            a.contains("--include-partial-messages")
                && a.contains("--disallowedTools AskUserQuestion")
        );
        assert!(!a.contains("--permission-prompt-tool"));
    }
}
