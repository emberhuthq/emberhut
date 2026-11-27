//! Why the hut couldn't do what was asked, in words the onboarding can show as they are.

use std::io;
use std::path::PathBuf;

/// Why the hut couldn't start, stop, pair or change how it stays lit.
#[derive(Debug, thiserror::Error)]
pub(crate) enum HutError {
    /// No server binary was found.
    #[error(
        "This copy of Emberhut has no hut: its emberhut-server is missing. Reinstall Emberhut, \
         or set EMBERHUT_SERVER_BIN to an emberhut-server program."
    )]
    NoServer,
    /// The server binary doesn't run (it can't say its version).
    #[error("The hut's program {path} doesn't run: {said}")]
    Broken {
        /// The binary.
        path: PathBuf,
        /// What went wrong.
        said: String,
    },
    /// A folder or file of the hut couldn't be made or written.
    #[error("Couldn't set up the hut's files at {path}: {source}")]
    Files {
        /// The folder or file.
        path: PathBuf,
        /// Why.
        source: io::Error,
    },
    /// The data folder's path isn't UTF-8, which the server's database address needs.
    #[error("The hut's folder {0} has a name the hut can't use (it isn't UTF-8).")]
    Path(PathBuf),
    /// The server couldn't be started.
    #[error("Couldn't start the hut: {0}")]
    Launch(io::Error),
    /// The server stopped while it was starting.
    #[error("The hut stopped while it was starting ({status}). Its log is {log}.")]
    Exited {
        /// How it ended.
        status: String,
        /// Its log.
        log: PathBuf,
    },
    /// The server didn't answer as this computer's hut in time.
    #[error("The hut didn't answer within {seconds} seconds. Its log is {log}.")]
    Timeout {
        /// How long the app waited.
        seconds: u64,
        /// Its log.
        log: PathBuf,
    },
    /// Every port the hut may use is taken by another program.
    #[error(
        "Other programs use every port from {first} to {last}, so the hut has nowhere to listen."
    )]
    NoPort {
        /// The first port tried.
        first: u16,
        /// The last.
        last: u16,
    },
    /// The server didn't stop in time.
    #[error("The hut didn't stop within {seconds} seconds.")]
    StillLit {
        /// How long the app waited.
        seconds: u64,
    },
    /// `emberhut-server pair` made no code.
    #[error("The hut didn't make a pairing code: {said}")]
    NoCode {
        /// What it said.
        said: String,
    },
    /// Keeping the hut lit without the app was asked on a system without a service manager
    /// the app knows.
    #[error(
        "The hut can't stay lit without the app on this system: that needs macOS or Linux \
         with systemd."
    )]
    NoService,
    /// `launchctl` or `systemctl` refused.
    #[error("Couldn't {what}: {said}")]
    Service {
        /// What was asked, in words ("load the hut's launch agent").
        what: &'static str,
        /// What the command said.
        said: String,
    },
}
