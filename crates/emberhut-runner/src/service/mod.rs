//! Running the runner as a service, so it starts by itself and comes back after a crash: a
//! systemd user unit on Linux, a launchd agent on macOS and a Windows service on Windows.
//!
//! A service runs under the account its manager starts it as. The systemd unit and the launchd
//! agent are the installing user's own, so they run as that user. The Windows service runs as the
//! account it is configured with: `LocalSystem` when installed, until another account is set in
//! its properties (Log On). The agent's logins, the macOS Keychain and the user's folders belong
//! to that account, so the agent has to be signed in as the account the service runs as.
//!
//! Every file written here starts with [`MARK`]; a file without it is neither replaced (unless
//! forced) nor removed, so a unit or agent someone wrote by hand is never lost. The Windows
//! service carries the same mark in its description. The texts come from pure functions
//! ([`systemd_unit`], [`launchd_plist`], [`windows_service`]) and the calls to `systemctl`,
//! `loginctl` and `launchctl` go through a small trait, so tests check both without a service
//! manager.

mod launchd;
mod scm;
mod systemd;
#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::config::Config;

pub use launchd::{LABEL, launchd_plist};
pub use scm::{WindowsService, run_as_service, windows_service};
pub use systemd::{UNIT, systemd_unit};

/// The line every file this module writes starts with, so it never replaces a file it didn't write.
pub const MARK: &str = "Written by emberhut-runner";
/// The Windows service's name.
pub const WINDOWS_SERVICE: &str = "emberhut-runner";
/// The runner's command that runs it in the foreground: what every service starts.
pub const RUN: &str = "run";
/// The flag after [`RUN`] that tells the runner the Windows service manager started it, so it
/// reports to the manager through [`run_as_service`].
pub const SERVICE_FLAG: &str = "--service";
/// The environment variable an installer sets to the binary's path that stays the same across
/// upgrades (through its `current` link), for [`ServiceCommand::current`].
pub const LAUNCHER_ENV: &str = "EMBERHUT_RUNNER_LAUNCHER";
/// The environment variable that tells a runner where its home is: every service sets it.
pub const HOME_ENV: &str = "EMBERHUT_RUNNER_HOME";
/// The runner's log file in its home, which the systemd unit and the launchd agent append to.
pub const LOG_FILE: &str = "runner.log";

/// What a service runs: the installed binary and its arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceCommand {
    /// The binary, at a path that stays the same across upgrades.
    pub program: PathBuf,
    /// Its arguments: [`RUN`], and [`SERVICE_FLAG`] on Windows.
    pub args: Vec<String>,
}

impl ServiceCommand {
    /// The command that runs `program` as this platform's service: `program run` (and
    /// `--service` on Windows).
    #[must_use]
    pub fn new(program: impl Into<PathBuf>) -> Self {
        let mut args = vec![RUN.to_owned()];
        if cfg!(windows) {
            args.push(SERVICE_FLAG.to_owned());
        }
        Self {
            program: program.into(),
            args,
        }
    }

    /// The command for this binary: the path in `EMBERHUT_RUNNER_LAUNCHER` when an installer set
    /// it, else the running executable.
    ///
    /// # Errors
    /// The running executable's path can't be found.
    pub fn current(env: &dyn Fn(&str) -> Option<String>) -> Result<Self, ServiceError> {
        match env(LAUNCHER_ENV).filter(|p| !p.is_empty()) {
            Some(launcher) => Ok(Self::new(launcher)),
            None => std::env::current_exe()
                .map(Self::new)
                .map_err(|source| ServiceError::Io {
                    path: PathBuf::from("the running executable"),
                    source,
                }),
        }
    }

    /// The program and its arguments, as words.
    fn words(&self) -> Vec<String> {
        std::iter::once(self.program.to_string_lossy().into_owned())
            .chain(self.args.iter().cloned())
            .collect()
    }
}

/// Why installing, removing or asking about the service failed.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ServiceError {
    /// A file (or Windows service) of the same name exists that the runner didn't write.
    #[error("{0} exists and wasn't written by emberhut-runner; move it away or force the install")]
    NotOurs(String),
    /// A file or folder can't be read or written.
    #[error("{path}: {source}")]
    Io {
        /// The file or folder.
        path: PathBuf,
        /// Why.
        source: std::io::Error,
    },
    /// A path the service would use can't be written into its definition.
    #[error("{0}: a service can't use a path that isn't UTF-8 or holds a line break")]
    Path(PathBuf),
    /// A command can't be started.
    #[error("cannot run {program}: {source}")]
    Spawn {
        /// The command.
        program: String,
        /// Why.
        source: std::io::Error,
    },
    /// A command ran and failed.
    #[error("{command} failed: {why}")]
    Failed {
        /// The command line.
        command: String,
        /// What it said.
        why: String,
    },
    /// The Windows service manager refused something.
    #[error("cannot {what}: {why}")]
    Windows {
        /// What was asked.
        what: String,
        /// What the system said.
        why: String,
    },
    /// No home folder to find the user's service folders from.
    #[error("no home folder: HOME is not set")]
    NoHome,
    /// This system has no service manager the runner knows.
    #[error("no service manager known for {0}: run `emberhut-runner run` under your own")]
    Unsupported(&'static str),
    /// The runner, run as a Windows service, stopped with an error.
    #[error("the runner stopped: {0}")]
    Runner(String),
}

/// Installs the service for `command` (replacing one this runner wrote before) and starts it.
/// Returns what it did, in words.
///
/// # Errors
/// A file of the same name that the runner didn't write (unless `force`), a file that can't be
/// written, or a service manager that refuses.
pub fn install(
    config: &Config,
    command: &ServiceCommand,
    force: bool,
) -> Result<String, ServiceError> {
    if cfg!(target_os = "linux") {
        systemd::install(&System, config, command, force)
    } else if cfg!(target_os = "macos") {
        launchd::install(&System, config, command, force)
    } else if cfg!(windows) {
        scm::install(config, command, force)
    } else {
        Err(unsupported())
    }
}

/// Stops and removes the service. The runner's home (its settings, accounts and sessions) stays.
///
/// # Errors
/// A file that can't be removed, or a service manager that refuses.
pub fn uninstall(config: &Config) -> Result<String, ServiceError> {
    if cfg!(target_os = "linux") {
        systemd::uninstall(&System, config)
    } else if cfg!(target_os = "macos") {
        launchd::uninstall(&System, config)
    } else if cfg!(windows) {
        scm::uninstall(config)
    } else {
        Err(unsupported())
    }
}

/// Restarts the service, so it runs the binary its path now points at.
///
/// # Errors
/// The service manager refuses (the service isn't installed, for one).
pub fn restart() -> Result<String, ServiceError> {
    if cfg!(target_os = "linux") {
        systemd::restart(&System)
    } else if cfg!(target_os = "macos") {
        launchd::restart(&System)
    } else if cfg!(windows) {
        scm::restart()
    } else {
        Err(unsupported())
    }
}

/// What the service manager says of the service, in a few lines.
///
/// # Errors
/// The service manager can't be asked.
pub fn status() -> Result<String, ServiceError> {
    if cfg!(target_os = "linux") {
        systemd::status(&System)
    } else if cfg!(target_os = "macos") {
        launchd::status(&System)
    } else if cfg!(windows) {
        scm::status()
    } else {
        Err(unsupported())
    }
}

/// The runner's log file, which the services write to.
#[must_use]
pub fn log_file(config: &Config) -> PathBuf {
    config.home.join(LOG_FILE)
}

fn unsupported() -> ServiceError {
    ServiceError::Unsupported(std::env::consts::OS)
}

/// The `PATH` a service runs with: the installing shell's absolute entries, then the system's
/// usual folders, each once. Sessions then find the same tools (git, compilers, package managers)
/// as the shell the service was installed from.
fn service_path(installing: Option<&str>, system: &[&str]) -> String {
    let mut out: Vec<&str> = Vec::new();
    for dir in installing
        .unwrap_or_default()
        .split(':')
        .filter(|d| d.starts_with('/'))
        .chain(system.iter().copied())
    {
        if !out.contains(&dir) {
            out.push(dir);
        }
    }
    out.join(":")
}

/// A path as text a service definition can hold.
fn usable(path: &Path) -> Result<&str, ServiceError> {
    path.to_str()
        .filter(|s| !s.contains(['\n', '\r']))
        .ok_or_else(|| ServiceError::Path(path.to_path_buf()))
}

/// Checks the paths a service definition holds and makes the runner's home (owner-only on Unix).
fn prepare(config: &Config, command: &ServiceCommand) -> Result<(), ServiceError> {
    usable(&config.home)?;
    usable(&command.program)?;
    make_home(&config.home)
}

fn make_home(home: &Path) -> Result<(), ServiceError> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    builder.create(home).map_err(|source| ServiceError::Io {
        path: home.to_path_buf(),
        source,
    })
}

/// Whether a file's text is one this module wrote: the mark is on one of its first lines.
fn is_ours(bytes: &[u8]) -> bool {
    std::str::from_utf8(bytes).is_ok_and(|t| t.lines().take(3).any(|l| l.contains(MARK)))
}

/// Writes `text` to `path`, unless a file the runner didn't write is there (and `force` is off).
fn write_own(path: &Path, text: &str, force: bool) -> Result<(), ServiceError> {
    let io = |source| ServiceError::Io {
        path: path.to_path_buf(),
        source,
    };
    match std::fs::read(path) {
        Ok(bytes) if !force && !is_ours(&bytes) => {
            return Err(ServiceError::NotOurs(path.display().to_string()));
        }
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(io(e)),
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(io)?;
    }
    std::fs::write(path, text).map_err(io)
}

/// What removing a service file did.
#[derive(Debug, PartialEq, Eq)]
enum Removed {
    /// It was the runner's, and it is gone.
    Deleted,
    /// There was none.
    Missing,
    /// It wasn't the runner's, so it stays.
    NotOurs,
}

/// Removes `path` if the runner wrote it.
fn remove_own(path: &Path) -> Result<Removed, ServiceError> {
    let io = |source| ServiceError::Io {
        path: path.to_path_buf(),
        source,
    };
    match std::fs::read(path) {
        Ok(bytes) if is_ours(&bytes) => std::fs::remove_file(path)
            .map(|()| Removed::Deleted)
            .map_err(io),
        Ok(_) => Ok(Removed::NotOurs),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Removed::Missing),
        Err(e) => Err(io(e)),
    }
}

/// The few lines of a service manager's answer worth showing.
fn summary(ran: &Ran) -> String {
    let text = if ran.stdout.trim().is_empty() {
        ran.stderr.trim()
    } else {
        ran.stdout.trim()
    };
    if text.is_empty() {
        "not installed".to_owned()
    } else {
        text.lines().take(12).collect::<Vec<_>>().join("\n")
    }
}

/// What a command did.
#[derive(Debug, Clone, Default)]
struct Ran {
    /// Whether it exited with success.
    ok: bool,
    /// Its exit code, when it had one.
    code: Option<i32>,
    /// What it wrote to its output.
    stdout: String,
    /// What it wrote to its error output.
    stderr: String,
}

impl Ran {
    /// Why it failed, in its own words.
    fn why(&self) -> String {
        [self.stderr.trim(), self.stdout.trim()]
            .into_iter()
            .find(|s| !s.is_empty())
            .map_or_else(
                || match self.code {
                    Some(c) => format!("exit status {c}"),
                    None => "killed by a signal".to_owned(),
                },
                str::to_owned,
            )
    }
}

/// What the systemd and launchd parts need from the system: its environment, the user's id, the
/// service manager's commands and a pause. Tests put a recording stand-in in its place.
trait Host {
    /// An environment variable of the installing process.
    fn env(&self, key: &str) -> Option<String>;
    /// The user's numeric id.
    fn uid(&self) -> u32;
    /// Runs a command to its end, its output captured.
    fn run(&self, program: &str, args: &[&str]) -> Result<Ran, ServiceError>;
    /// Waits a while (between attempts).
    fn pause(&self, how_long: Duration);
}

/// Runs a command that has to succeed; returns its output.
fn must(host: &dyn Host, program: &str, args: &[&str]) -> Result<String, ServiceError> {
    let ran = host.run(program, args)?;
    if ran.ok {
        Ok(ran.stdout)
    } else {
        Err(ServiceError::Failed {
            command: std::iter::once(program)
                .chain(args.iter().copied())
                .collect::<Vec<_>>()
                .join(" "),
            why: ran.why(),
        })
    }
}

/// The user's home folder, from the host's environment.
fn user_home(host: &dyn Host) -> Result<PathBuf, ServiceError> {
    host.env("HOME")
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
        .ok_or(ServiceError::NoHome)
}

/// This system.
struct System;

impl Host for System {
    fn env(&self, key: &str) -> Option<String> {
        std::env::var(key).ok()
    }

    fn uid(&self) -> u32 {
        #[cfg(unix)]
        {
            rustix::process::getuid().as_raw()
        }
        #[cfg(not(unix))]
        {
            0
        }
    }

    fn run(&self, program: &str, args: &[&str]) -> Result<Ran, ServiceError> {
        let out = std::process::Command::new(program)
            .args(args)
            .stdin(std::process::Stdio::null())
            .output()
            .map_err(|source| ServiceError::Spawn {
                program: program.to_owned(),
                source,
            })?;
        Ok(Ran {
            ok: out.status.success(),
            code: out.status.code(),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        })
    }

    fn pause(&self, how_long: Duration) {
        std::thread::sleep(how_long);
    }
}

/// A request to stop, which the runner watches when a service manager runs it.
#[derive(Debug, Clone)]
pub struct Stop(tokio::sync::watch::Receiver<bool>);

/// The side of a [`Stop`] that asks for it.
#[derive(Debug)]
pub struct Stopper(tokio::sync::watch::Sender<bool>);

/// A new stop request, not asked yet: the runner watches the [`Stop`], and whatever learns of the
/// stop (the Windows service manager, a signal) asks through the [`Stopper`].
#[must_use]
pub fn stop_pair() -> (Stopper, Stop) {
    let (tx, rx) = tokio::sync::watch::channel(false);
    (Stopper(tx), Stop(rx))
}

impl Stopper {
    /// Asks for the stop.
    pub fn stop(&self) {
        self.0.send_replace(true);
    }
}

impl Stop {
    /// Whether the stop was asked for.
    #[must_use]
    pub fn requested(&self) -> bool {
        *self.0.borrow()
    }

    /// Waits until the stop is asked for (or its [`Stopper`] is gone, which ends the wait too).
    pub async fn wait(mut self) {
        // An error: the stopper is gone, so no stop can come; the wait ends then too.
        let _asked = self.0.wait_for(|stop| *stop).await.is_ok();
    }
}
