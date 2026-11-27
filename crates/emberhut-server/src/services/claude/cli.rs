//! Starting the agent CLI: what to run ([`Launch`]), and the [`CliSpawner`] that runs it,
//! handing back its standard input and output as lines ([`CliPipes`]).
//!
//! The real spawner ([`ProcessSpawner`]) runs the configured command as a child process in a
//! process group of its own: stopping it ends its input and sends `SIGTERM` to the whole group
//! (the CLI and whatever it started), then `SIGKILL` after [`STOP_GRACE`]. The server's tests use
//! a scripted spawner that answers like the CLI does, so no test runs a real CLI.

use std::fmt;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

/// How long a stopped CLI's process group gets, after `SIGTERM`, before it is killed.
pub const STOP_GRACE: Duration = Duration::from_secs(5);

/// Characters of a line the CLI writes on its error output that reach the log.
const STDERR_LINE: usize = 300;

/// What to run: the program, its arguments, the environment changes and the directory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Launch {
    /// The program.
    pub program: PathBuf,
    /// Its arguments, in order.
    pub args: Vec<String>,
    /// Variables set for it, over the server's own environment.
    pub env: Vec<(String, String)>,
    /// Variables of the server's environment it does not get.
    pub env_remove: Vec<String>,
    /// Where it runs; the server's directory when none.
    pub cwd: Option<PathBuf>,
}

impl Launch {
    /// The value of `--flag=value` or `--flag value` among the arguments, if it is there.
    #[must_use]
    pub fn flag(&self, flag: &str) -> Option<&str> {
        let joined = format!("{flag}=");
        let mut args = self.args.iter();
        while let Some(arg) = args.next() {
            if let Some(value) = arg.strip_prefix(&joined) {
                return Some(value);
            }
            if arg == flag {
                return args.next().map(String::as_str);
            }
        }
        None
    }

    /// The value the launch sets `name` to, if it sets it.
    #[must_use]
    pub fn var(&self, name: &str) -> Option<&str> {
        self.env
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }
}

/// A running CLI's pipes: lines to write to its input, lines it writes to its output (the
/// channel closes when it exits). Dropping this ends its input and stops it.
#[derive(Debug)]
pub struct CliPipes {
    /// Lines for its standard input (each without its newline).
    pub input: mpsc::UnboundedSender<String>,
    /// Lines from its standard output, as they come.
    pub output: mpsc::UnboundedReceiver<String>,
    /// Cancelled to stop it.
    pub stop: CancellationToken,
    /// Cancelled once it has exited and been waited for (after a stop, once it is gone).
    pub exited: CancellationToken,
}

impl CliPipes {
    /// Writes `line` to its input; `false` when it is gone.
    #[must_use]
    pub fn send(&self, line: String) -> bool {
        self.input.send(line).is_ok()
    }

    /// Stops it, and resolves once it has exited.
    pub async fn close(self) {
        let exited = self.exited.clone();
        drop(self);
        exited.cancelled().await;
    }
}

impl Drop for CliPipes {
    fn drop(&mut self) {
        self.stop.cancel();
    }
}

/// Why the CLI could not start.
#[derive(Debug, thiserror::Error)]
#[error("the agent CLI {program} could not start")]
pub struct SpawnError {
    /// The program it tried.
    pub program: String,
    /// Why.
    #[source]
    pub source: std::io::Error,
}

/// Starts the agent CLI.
pub trait CliSpawner: fmt::Debug + Send + Sync {
    /// Runs `launch`, its pipes ready.
    ///
    /// # Errors
    ///
    /// [`SpawnError`] when it could not start.
    fn spawn(&self, launch: &Launch) -> Result<CliPipes, SpawnError>;
}

/// Runs the CLI as a child process, in a process group of its own.
#[derive(Clone, Copy, Debug)]
pub struct ProcessSpawner {
    grace: Duration,
}

impl Default for ProcessSpawner {
    fn default() -> Self {
        Self::with_grace(STOP_GRACE)
    }
}

impl ProcessSpawner {
    /// A spawner whose stopped CLIs get `grace` after `SIGTERM` before they are killed
    /// ([`STOP_GRACE`] by default).
    #[must_use]
    pub const fn with_grace(grace: Duration) -> Self {
        Self { grace }
    }
}

impl CliSpawner for ProcessSpawner {
    fn spawn(&self, launch: &Launch) -> Result<CliPipes, SpawnError> {
        let failed = |source| SpawnError {
            program: launch.program.display().to_string(),
            source,
        };
        let mut command = tokio::process::Command::new(&launch.program);
        command
            .args(&launch.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        for name in &launch.env_remove {
            command.env_remove(name);
        }
        command.envs(launch.env.iter().map(|(key, value)| (key, value)));
        if let Some(cwd) = &launch.cwd {
            command.current_dir(cwd);
        }
        // A group of its own: a stop reaches what it started too, and a signal meant for the
        // server (a terminal's Ctrl-C) does not reach it.
        #[cfg(unix)]
        command.process_group(0);
        let mut child = command.spawn().map_err(failed)?;
        let group = child.id();
        let missing = || failed(std::io::Error::other("a pipe was not opened"));
        let mut stdin = child.stdin.take().ok_or_else(missing)?;
        let stdout = child.stdout.take().ok_or_else(missing)?;
        let stderr = child.stderr.take().ok_or_else(missing)?;
        let (input, mut lines_in) = mpsc::unbounded_channel::<String>();
        let (lines_out, output) = mpsc::unbounded_channel::<String>();
        let stop = CancellationToken::new();

        tokio::spawn(async move {
            while let Some(line) = lines_in.recv().await {
                let written = async {
                    stdin.write_all(line.as_bytes()).await?;
                    stdin.write_all(b"\n").await?;
                    stdin.flush().await
                };
                if let Err(error) = written.await {
                    tracing::debug!(%error, "the agent CLI's input is closed");
                    break;
                }
            }
            // Dropping stdin here ends the CLI's input.
        });
        tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if lines_out.send(line).is_err() {
                    break;
                }
            }
        });
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if line.to_lowercase().contains("error") {
                    let line: String = line.trim().chars().take(STDERR_LINE).collect();
                    tracing::warn!(line, "the agent CLI says");
                }
            }
        });
        let stopping = stop.clone();
        let exited = CancellationToken::new();
        let gone = exited.clone();
        let grace = self.grace;
        tokio::spawn(async move {
            tokio::select! {
                status = child.wait() => {
                    match status {
                        Ok(status) if !status.success() => {
                            tracing::warn!(%status, "the agent CLI exited");
                        }
                        Ok(_) => tracing::debug!("the agent CLI exited"),
                        Err(error) => tracing::warn!(%error, "the agent CLI could not be waited for"),
                    }
                }
                () = stopping.cancelled() => stop_child(&mut child, group, grace).await,
            }
            gone.cancel();
        });
        Ok(CliPipes {
            input,
            output,
            stop,
            exited,
        })
    }
}

/// Stops `child` (the leader of process group `group`): `SIGTERM` to the group, and `SIGKILL`
/// to it when the child has not exited after `grace`. Its input has ended already (the pipes
/// were dropped).
async fn stop_child(child: &mut tokio::process::Child, group: Option<u32>, grace: Duration) {
    signal_group(child, group, Stop::Term);
    if tokio::time::timeout(grace, child.wait()).await.is_ok() {
        return;
    }
    tracing::info!(?grace, "the agent CLI did not stop: killing it");
    signal_group(child, group, Stop::Kill);
    if let Err(error) = child.wait().await {
        tracing::debug!(%error, "the agent CLI could not be waited for");
    }
}

/// The signals a stop sends.
#[derive(Clone, Copy, Debug)]
enum Stop {
    Term,
    Kill,
}

/// Sends `stop` to `group`, the process group `child` leads. The child has not been waited for
/// yet, so the group's id is still its own.
#[cfg(unix)]
fn signal_group(child: &mut tokio::process::Child, group: Option<u32>, stop: Stop) {
    use rustix::process::{Pid, Signal, kill_process_group};

    let signal = match stop {
        Stop::Term => Signal::TERM,
        Stop::Kill => Signal::KILL,
    };
    let pid = group
        .and_then(|id| i32::try_from(id).ok())
        .and_then(Pid::from_raw);
    let sent = pid.is_some_and(|pid| match kill_process_group(pid, signal) {
        Ok(()) => true,
        Err(error) => {
            tracing::debug!(%error, "the agent CLI's process group could not be signalled");
            false
        }
    });
    // Without its group, the child itself is still killed.
    if !sent
        && let Stop::Kill = stop
        && let Err(error) = child.start_kill()
    {
        tracing::debug!(%error, "the agent CLI was gone already");
    }
}

/// Without process groups, the child alone is killed once the grace is over.
#[cfg(not(unix))]
fn signal_group(child: &mut tokio::process::Child, _group: Option<u32>, stop: Stop) {
    if let Stop::Kill = stop
        && let Err(error) = child.start_kill()
    {
        tracing::debug!(%error, "the agent CLI was gone already");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_launch_finds_its_flags_either_way_they_are_written() {
        let launch = Launch {
            program: PathBuf::from("claude"),
            args: ["--model=opus", "--mcp-config", "{}", "--verbose"]
                .map(str::to_owned)
                .to_vec(),
            env: vec![("A".to_owned(), "1".to_owned())],
            env_remove: Vec::new(),
            cwd: None,
        };
        assert_eq!(launch.flag("--model"), Some("opus"));
        assert_eq!(launch.flag("--mcp-config"), Some("{}"));
        assert_eq!(launch.flag("--resume"), None);
        assert_eq!(launch.var("A"), Some("1"));
        assert_eq!(launch.var("B"), None);
    }

    #[tokio::test]
    async fn a_program_that_is_not_there_does_not_start() {
        let launch = Launch {
            program: PathBuf::from("/nonexistent/agent-cli"),
            args: Vec::new(),
            env: Vec::new(),
            env_remove: Vec::new(),
            cwd: None,
        };
        let error = ProcessSpawner::default().spawn(&launch).unwrap_err();
        assert_eq!(error.program, "/nonexistent/agent-cli");
    }

    /// A launch of `/bin/sh` running `script`, with `GREETING` set.
    #[cfg(unix)]
    fn sh(script: &str) -> Launch {
        Launch {
            program: PathBuf::from("/bin/sh"),
            args: vec!["-c".to_owned(), script.to_owned()],
            env: vec![("GREETING".to_owned(), "hello".to_owned())],
            env_remove: Vec::new(),
            cwd: None,
        }
    }

    /// The next line the child writes (within a few seconds).
    #[cfg(unix)]
    async fn next_line(pipes: &mut CliPipes) -> String {
        tokio::time::timeout(Duration::from_secs(5), pipes.output.recv())
            .await
            .expect("a line in time")
            .expect("a line before it exited")
    }

    /// Whether process `pid` is gone, polled for a few seconds (an orphan is reaped by
    /// whoever adopted it, a moment after it died).
    #[cfg(unix)]
    async fn gone(pid: &str) -> bool {
        use rustix::process::{Pid, test_kill_process};

        let pid = Pid::from_raw(pid.parse().unwrap()).unwrap();
        for _ in 0..500 {
            if test_kill_process(pid).is_err() {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        false
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn lines_go_in_and_come_out() {
        let script = r#"while read -r line; do echo "$GREETING $line"; done"#;
        let mut pipes = ProcessSpawner::default().spawn(&sh(script)).unwrap();
        assert!(pipes.send("one".to_owned()));
        assert!(pipes.send("two words".to_owned()));
        assert_eq!(next_line(&mut pipes).await, "hello one");
        assert_eq!(next_line(&mut pipes).await, "hello two words");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn dropping_the_pipes_stops_the_child_and_what_it_started() {
        let script = r"sleep 300 & echo $!; echo $$; while read -r line; do :; done";
        let mut pipes = ProcessSpawner::default().spawn(&sh(script)).unwrap();
        let started = next_line(&mut pipes).await;
        let child = next_line(&mut pipes).await;
        let pid = rustix::process::Pid::from_raw(child.parse().unwrap()).unwrap();
        assert!(rustix::process::test_kill_process(pid).is_ok(), "running");
        let exited = pipes.exited.clone();
        drop(pipes);
        tokio::time::timeout(Duration::from_secs(4), exited.cancelled())
            .await
            .expect("it exited well within the grace");
        assert!(gone(&child).await, "the child was waited for");
        assert!(gone(&started).await, "what it started was stopped too");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_child_ignoring_its_input_and_sigterm_is_killed_after_the_grace() {
        let script = r"trap '' TERM; echo $$; while :; do sleep 1; done";
        let spawner = ProcessSpawner::with_grace(Duration::from_millis(800));
        let mut pipes = spawner.spawn(&sh(script)).unwrap();
        let child = next_line(&mut pipes).await;
        let exited = pipes.exited.clone();
        let stopped = tokio::time::Instant::now();
        drop(pipes);
        tokio::time::timeout(Duration::from_secs(5), exited.cancelled())
            .await
            .expect("it was killed");
        assert!(
            stopped.elapsed() >= Duration::from_millis(800),
            "not before the grace"
        );
        assert!(gone(&child).await);
    }
}
