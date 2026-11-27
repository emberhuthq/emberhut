//! What installing a verified payload takes, whoever does it: staging the new bundle beside the
//! installed one with its checks, the swap, and the watch over the new copy's start, ending in
//! a [`Verdict`]. The Mac's own installer is in `install/`; the tests script one.

use std::io;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use emberhut_api::releases::{Build, Release};

use super::version::Running;
use crate::backend::Cause;

/// How long the new copy has to draw its first frame.
pub(crate) const WATCH_FOR: Duration = Duration::from_secs(120);

/// A new bundle, staged beside the installed one and checked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Staged {
    /// Its build.
    pub(crate) build: Build,
    /// Its version.
    pub(crate) version: String,
}

/// Why a new bundle couldn't be staged or swapped in.
#[derive(Debug, thiserror::Error)]
pub(crate) enum InstallError {
    /// The disk image holds no `Emberhut.app`.
    #[error("the update holds no Emberhut.app")]
    NoBundle,
    /// The disk image couldn't be opened or copied from.
    #[error("couldn't open the update")]
    Image(#[source] Cause),
    /// The new bundle's signature, team or notarization didn't check out.
    #[error("the new bundle's signature doesn't check out")]
    Signature(#[source] Cause),
    /// The new bundle isn't the release's (another app, version or build).
    #[error("the new bundle isn't the release")]
    Bundle(#[source] Cause),
    /// A file step failed (staging, the swap, the marker).
    #[error("couldn't put the new bundle in place")]
    Local(#[source] io::Error),
    /// Nothing is staged for that build.
    #[error("the update isn't staged any more")]
    NotStaged,
}

impl InstallError {
    /// Whether the payload itself failed a check (rather than this Mac failing a step).
    #[must_use]
    pub(crate) fn is_verification(&self) -> bool {
        matches!(self, Self::NoBundle | Self::Signature(_) | Self::Bundle(_))
    }
}

/// What installs: the real one on a Mac, a scripted one in the tests.
pub(crate) trait Installer: Send + Sync + std::fmt::Debug {
    /// Takes the bundle out of `payload` (verified) and checks it against `release`, staged
    /// beside the installed app.
    ///
    /// # Errors
    ///
    /// The step or check that failed; nothing stays staged then.
    fn stage(&self, payload: &Path, release: &Release) -> Result<Staged, InstallError>;

    /// Swaps `staged` in for the running copy `running`, and gives the watch to run while the
    /// new copy starts.
    ///
    /// # Errors
    ///
    /// The swap failed; the running app is as it was.
    fn swap(&self, staged: &Staged, running: Running) -> Result<Box<dyn Watch>, InstallError>;
}

/// Watching the new copy start, in the old one; run on a background thread.
pub(crate) trait Watch: Send {
    /// Starts the new copy and watches it until it is healthy, fails, or `stop` is set (a quit
    /// or a logout: the new build then stays, unwatched).
    fn run(self: Box<Self>, stop: &AtomicBool) -> Verdict;
}

/// What the watch saw first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Seen {
    /// The new copy said it is healthy.
    Healthy,
    /// The new copy exited first.
    Exited,
    /// Two minutes passed.
    TimedOut,
    /// The watch was told to stop (a quit or a logout).
    Stopped,
}

/// How the update ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Verdict {
    /// The new build stays.
    Kept,
    /// The new build stays, and was started again (the watch had ended it).
    KeptRestarted,
    /// The old build is back, for this reason.
    RolledBack(String),
    /// The watch stopped; the new build stays, unwatched.
    Stopped,
    /// The new build didn't start and the old one couldn't be put back, for this reason: the
    /// new build is still in place, and nothing is started.
    Stuck(String),
}

/// The verdict once the watch saw `seen`, the marker then read `healthy_after` (the new copy
/// may have turned healthy as it was ended), the watch having ended the new copy itself or not.
#[must_use]
pub(crate) fn decide(seen: Seen, healthy_after: bool, ended_by_us: bool) -> Verdict {
    match seen {
        Seen::Healthy => Verdict::Kept,
        Seen::Stopped => Verdict::Stopped,
        Seen::Exited | Seen::TimedOut if healthy_after => {
            if ended_by_us {
                Verdict::KeptRestarted
            } else {
                Verdict::Kept
            }
        }
        Seen::Exited => Verdict::RolledBack("it quit before it opened its window".to_owned()),
        Seen::TimedOut => Verdict::RolledBack(format!(
            "it didn't open its window in {} seconds",
            WATCH_FOR.as_secs()
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_healthy_copy_is_kept_and_one_that_never_drew_is_rolled_back() {
        assert_eq!(decide(Seen::Healthy, false, false), Verdict::Kept);
        assert!(matches!(
            decide(Seen::Exited, false, false),
            Verdict::RolledBack(why) if why.contains("quit")
        ));
        assert!(matches!(
            decide(Seen::TimedOut, false, true),
            Verdict::RolledBack(why) if why.contains("120 seconds")
        ));
    }

    #[test]
    fn a_copy_that_turned_healthy_as_it_was_ended_is_kept_and_started_again() {
        assert_eq!(decide(Seen::TimedOut, true, true), Verdict::KeptRestarted);
        assert_eq!(
            decide(Seen::Exited, true, false),
            Verdict::Kept,
            "the user quit it after its first frame: not started again"
        );
    }

    #[test]
    fn a_stopped_watch_leaves_the_new_build() {
        assert_eq!(decide(Seen::Stopped, false, false), Verdict::Stopped);
    }

    #[test]
    fn only_the_payloads_own_failures_count_against_it() {
        assert!(InstallError::NoBundle.is_verification());
        assert!(InstallError::Signature("no".into()).is_verification());
        assert!(!InstallError::Image("busy".into()).is_verification());
        assert!(!InstallError::NotStaged.is_verification());
    }
}
