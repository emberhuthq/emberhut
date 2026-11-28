//! The processes the hut needs: the server itself, its `pair`, `id` and `--version` commands,
//! and `launchctl` or `systemctl`. Behind [`Spawner`], so the tests run a fake one.

use std::ffi::{OsStr, OsString};
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

/// A program to run: what, with which arguments and variables, where, and where its output
/// goes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Run {
    /// The program.
    pub(crate) program: PathBuf,
    /// Its arguments.
    pub(crate) args: Vec<OsString>,
    /// Variables set for it, over the app's own.
    pub(crate) env: Vec<(String, OsString)>,
    /// The folder it runs in; the app's when `None`.
    pub(crate) dir: Option<PathBuf>,
    /// The file its output is appended to, for a program left running; dropped when `None`.
    pub(crate) log: Option<PathBuf>,
}

impl Run {
    /// `program`, with no arguments yet.
    #[must_use]
    pub(crate) fn new(program: impl Into<PathBuf>) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
            env: Vec::new(),
            dir: None,
            log: None,
        }
    }

    /// With `arg` added.
    #[must_use]
    pub(crate) fn arg(mut self, arg: impl AsRef<OsStr>) -> Self {
        self.args.push(arg.as_ref().to_owned());
        self
    }

    /// With `args` added.
    #[must_use]
    pub(crate) fn args<I, A>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = A>,
        A: AsRef<OsStr>,
    {
        self.args
            .extend(args.into_iter().map(|arg| arg.as_ref().to_owned()));
        self
    }

    /// With the variable `name` set to `value`.
    #[must_use]
    pub(crate) fn env(mut self, name: &str, value: impl Into<OsString>) -> Self {
        self.env.push((name.to_owned(), value.into()));
        self
    }

    /// Run in `dir`.
    #[must_use]
    pub(crate) fn dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.dir = Some(dir.into());
        self
    }

    /// With its output appended to `log`.
    #[must_use]
    pub(crate) fn log(mut self, log: impl Into<PathBuf>) -> Self {
        self.log = Some(log.into());
        self
    }

    /// The arguments as text, for the tests and the log.
    #[must_use]
    pub(crate) fn words(&self) -> Vec<String> {
        self.args
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect()
    }
}

/// How a program that ran to its end went.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Ran {
    /// Whether it exited with success.
    pub(crate) ok: bool,
    /// What it printed on its standard output.
    pub(crate) stdout: String,
    /// What it printed on its standard error.
    pub(crate) stderr: String,
}

impl Ran {
    /// What it said about a failure: its last non-empty line of standard error, else of
    /// standard output.
    #[must_use]
    pub(crate) fn said(&self) -> String {
        let last = |text: &str| {
            text.lines()
                .map(str::trim)
                .rfind(|line| !line.is_empty())
                .map(str::to_owned)
        };
        last(&self.stderr)
            .or_else(|| last(&self.stdout))
            .unwrap_or_else(|| "it said nothing".to_owned())
    }
}

/// A program left running.
pub(crate) trait Running: Send {
    /// Its process id.
    fn pid(&self) -> u32;

    /// How it ended, once it has (and it is then reaped); `None` while it runs.
    fn exited(&mut self) -> Option<String>;

    /// Ends it at once and reaps it.
    fn kill(&mut self);
}

/// Starts and runs programs. Blocks: call it off the UI thread.
pub(crate) trait Spawner: Send + Sync {
    /// Starts `run` and leaves it running, with no input and its output in its log.
    ///
    /// # Errors
    ///
    /// The program couldn't be started.
    fn spawn(&self, run: &Run) -> io::Result<Box<dyn Running>>;

    /// Runs `run` to its end and gives its output.
    ///
    /// # Errors
    ///
    /// The program couldn't be started or its output read.
    fn run(&self, run: &Run) -> io::Result<Ran>;

    /// Asks process `pid` to stop: `SIGTERM` on Unix, which the server shuts down cleanly on.
    /// Windows has no such signal for a program without a console of its own to receive it, so
    /// there the process and the ones it started (the agent CLIs) are ended (`taskkill /T
    /// /F`); the database's transactions keep it whole.
    ///
    /// # Errors
    ///
    /// There is no such process, or it isn't this user's.
    fn terminate(&self, pid: u32) -> io::Result<()>;
}

/// The real [`Spawner`]: `std::process`.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct SystemSpawner;

impl SystemSpawner {
    /// The command for `run`, without its input and output set.
    fn command(run: &Run) -> Command {
        let mut command = Command::new(&run.program);
        command.args(&run.args);
        for (name, value) in &run.env {
            command.env(name, value);
        }
        if let Some(dir) = &run.dir {
            command.current_dir(dir);
        }
        // The server and the system's tools are console programs: started from the app (a
        // windowed program), each would open a console window of its own without this.
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt as _;
            /// `CREATE_NO_WINDOW`, from the process creation flags.
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            command.creation_flags(CREATE_NO_WINDOW);
        }
        command
    }
}

/// `log` opened for appending, readable by the user only.
fn append(log: &Path) -> io::Result<std::fs::File> {
    let mut options = std::fs::File::options();
    options.create(true).append(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options.open(log)
}

impl Spawner for SystemSpawner {
    fn spawn(&self, run: &Run) -> io::Result<Box<dyn Running>> {
        let mut command = Self::command(run);
        command.stdin(Stdio::null());
        match &run.log {
            Some(log) => {
                let out = append(log)?;
                command.stderr(out.try_clone()?).stdout(out);
            }
            None => {
                command.stdout(Stdio::null()).stderr(Stdio::null());
            }
        }
        Ok(Box::new(SystemChild(command.spawn()?)))
    }

    fn run(&self, run: &Run) -> io::Result<Ran> {
        let output = Self::command(run).stdin(Stdio::null()).output()?;
        Ok(Ran {
            ok: output.status.success(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }

    #[cfg(unix)]
    fn terminate(&self, pid: u32) -> io::Result<()> {
        let pid = i32::try_from(pid)
            .ok()
            .and_then(rustix::process::Pid::from_raw)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "not a process id"))?;
        rustix::process::kill_process(pid, rustix::process::Signal::TERM).map_err(io::Error::from)
    }

    #[cfg(windows)]
    fn terminate(&self, pid: u32) -> io::Result<()> {
        // The system's own copy, not whichever `taskkill` the PATH finds first.
        let taskkill = std::env::var_os("SystemRoot").map_or_else(
            || PathBuf::from("taskkill.exe"),
            |root| PathBuf::from(root).join("System32").join("taskkill.exe"),
        );
        let ran = self.run(&Run::new(taskkill).args(["/PID", &pid.to_string(), "/T", "/F"]))?;
        if ran.ok {
            Ok(())
        } else {
            Err(io::Error::other(ran.said()))
        }
    }

    #[cfg(not(any(unix, windows)))]
    fn terminate(&self, _pid: u32) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "stopping a process by its id needs Unix or Windows",
        ))
    }
}

/// A [`Running`] child process.
#[derive(Debug)]
struct SystemChild(Child);

impl Running for SystemChild {
    fn pid(&self) -> u32 {
        self.0.id()
    }

    fn exited(&mut self) -> Option<String> {
        self.0
            .try_wait()
            .ok()
            .flatten()
            .map(|status| status.to_string())
    }

    fn kill(&mut self) {
        if let Err(error) = self.0.kill() {
            log::warn!("couldn't end the hut's process: {error}");
        }
        if let Err(error) = self.0.wait() {
            log::warn!("couldn't reap the hut's process: {error}");
        }
    }
}
