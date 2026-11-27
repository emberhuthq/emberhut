//! The watchdog: after the swap, the old copy starts the new one and watches it for up to 2
//! minutes. The new copy is healthy once its window has drawn its first frame (it says so in
//! the marker); then the old bundle and the marker go. If the new copy exits or the time runs
//! out first, it is ended and, unless it turned healthy meanwhile, the old bundle is exchanged
//! back into place, the marker records the rollback, and the restored app starts again.
//!
//! The rescue runs in the old copy because it is the one known to work: a new build that
//! can't even load its libraries never runs code of its own.

use std::os::unix::process::CommandExt as _;
use std::path::PathBuf;
use std::process::{Child, Command};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::{Duration, Instant, SystemTime};

use rustix::process::{Pid, Signal};

use super::super::installer::{Seen, Verdict, WATCH_FOR, Watch, decide};
use super::super::marker::{Marker, State};
use super::{Installed, remove_bundle, swap, tools};

/// How long an ended copy has between `SIGTERM` and `SIGKILL`.
const KILL_AFTER: Duration = Duration::from_secs(5);
/// How often the watch looks.
const LOOK_EVERY: Duration = Duration::from_millis(250);

/// The new copy's process while it is watched (0: nothing is).
static WATCHED: AtomicU32 = AtomicU32::new(0);

/// The new copy's process, while this copy watches it (a Dock click goes to it).
#[must_use]
pub(crate) fn watched() -> Option<u32> {
    match WATCHED.load(Ordering::Relaxed) {
        0 => None,
        pid => Some(pid),
    }
}

/// Whether process `pid` still runs.
#[must_use]
pub(crate) fn runs(pid: u32) -> bool {
    i32::try_from(pid)
        .ok()
        .and_then(Pid::from_raw)
        .is_some_and(|pid| rustix::process::test_kill_process(pid).is_ok())
}

/// The watch over one swap.
#[derive(Debug)]
pub(crate) struct Watchdog {
    installed: Installed,
    marker_path: PathBuf,
    marker: Marker,
}

impl Watchdog {
    pub(super) fn new(installed: Installed, marker_path: PathBuf, marker: Marker) -> Self {
        Self {
            installed,
            marker_path,
            marker,
        }
    }

    fn healthy(&self) -> bool {
        Marker::read(&self.marker_path).is_some_and(|marker| marker.state == State::Healthy)
    }

    fn start(&mut self) -> std::io::Result<Child> {
        // A process group of its own, so it outlives this copy.
        let child = Command::new(self.installed.executable())
            .process_group(0)
            .spawn()?;
        self.marker.new_copy = Some(child.id());
        if let Err(error) = self.marker.write(&self.marker_path) {
            log::warn!("couldn't note the new copy in the update marker: {error}");
        }
        Ok(child)
    }

    fn watch(&self, child: &mut Child, stop: &AtomicBool) -> Seen {
        let started = Instant::now();
        loop {
            if stop.load(Ordering::Relaxed) {
                return Seen::Stopped;
            }
            if self.healthy() {
                return Seen::Healthy;
            }
            match child.try_wait() {
                Ok(Some(status)) => {
                    log::warn!("the new copy exited before it was healthy: {status}");
                    return Seen::Exited;
                }
                Ok(None) => {}
                Err(error) => {
                    log::warn!("can't watch the new copy: {error}");
                    return Seen::Exited;
                }
            }
            if started.elapsed() >= WATCH_FOR {
                return Seen::TimedOut;
            }
            std::thread::sleep(LOOK_EVERY);
        }
    }

    /// Puts the old bundle back, records why, and starts it. When the old bundle can't be put
    /// back, the marker isn't marked rolled back (the new build is still in place) and nothing
    /// is started.
    fn roll_back(mut self, reason: String) -> Verdict {
        log::error!("the update didn't start ({reason}); putting the old build back");
        let (app, old) = (&self.installed.app, self.installed.kept_old());
        if let Err(error) = swap::exchange(app, &old) {
            log::error!("couldn't put the old build back: {error}");
            return Verdict::Stuck(format!(
                "{reason}, and the old build couldn't be put back: {error}"
            ));
        }
        if let Err(error) = remove_bundle(&old) {
            log::warn!("couldn't remove the refused bundle: {error}");
        }
        self.marker.set(State::RolledBack, SystemTime::now());
        self.marker.reason = Some(reason.clone());
        if let Err(error) = self.marker.write(&self.marker_path) {
            log::error!("couldn't record the rollback: {error}");
        }
        open_again(app);
        Verdict::RolledBack(reason)
    }

    /// Keeps the new build: the old bundle and the marker go.
    fn keep(&self) {
        if let Err(error) = remove_bundle(&self.installed.kept_old()) {
            log::warn!("couldn't remove the old bundle: {error}");
        }
        if let Err(error) = std::fs::remove_file(&self.marker_path) {
            log::warn!("couldn't remove the update marker: {error}");
        }
    }
}

impl Watch for Watchdog {
    fn run(self: Box<Self>, stop: &AtomicBool) -> Verdict {
        let mut this = *self;
        let mut child = match this.start() {
            Ok(child) => child,
            Err(error) => return this.roll_back(format!("it couldn't be started: {error}")),
        };
        WATCHED.store(child.id(), Ordering::Relaxed);
        let seen = this.watch(&mut child, stop);
        WATCHED.store(0, Ordering::Relaxed);
        let ended_by_us = seen == Seen::TimedOut && end(&mut child);
        let healthy_after = matches!(seen, Seen::Exited | Seen::TimedOut) && this.healthy();
        let verdict = decide(seen, healthy_after, ended_by_us);
        match &verdict {
            Verdict::Kept => this.keep(),
            Verdict::KeptRestarted => {
                this.keep();
                open_again(&this.installed.app);
            }
            Verdict::RolledBack(reason) => return this.roll_back(reason.clone()),
            Verdict::Stopped => log::info!("stopped watching the update; the new build stays"),
            Verdict::Stuck(_) => {}
        }
        verdict
    }
}

/// Ends `child`: `SIGTERM`, then `SIGKILL` after 5 seconds. Whether it was running to end.
fn end(child: &mut Child) -> bool {
    if !matches!(child.try_wait(), Ok(None)) {
        return false;
    }
    let pid = Pid::from_child(child);
    if let Err(error) = rustix::process::kill_process(pid, Signal::TERM) {
        log::warn!("couldn't ask the new copy to quit: {error}");
    }
    let asked = Instant::now();
    while asked.elapsed() < KILL_AFTER {
        if !matches!(child.try_wait(), Ok(None)) {
            return true;
        }
        std::thread::sleep(LOOK_EVERY);
    }
    if let Err(error) = child.kill() {
        log::warn!("couldn't end the new copy: {error}");
    }
    if let Err(error) = child.wait() {
        log::warn!("the new copy wasn't reaped: {error}");
    }
    true
}

/// Starts a fresh process of the bundle at `app` (`open -n`), which reads its session from
/// disk.
fn open_again(app: &std::path::Path) {
    if let Err(error) = tools::run("/usr/bin/open", ["-n".as_ref(), app.as_os_str()]) {
        log::error!("couldn't start Emberhut again: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn this_process_runs_and_a_made_up_one_doesnt() {
        assert!(runs(std::process::id()));
        assert!(!runs(0));
        assert!(!runs(u32::MAX));
    }

    #[test]
    fn a_copy_that_wont_start_is_rolled_back_and_recorded() {
        let dir = tempfile::tempdir().unwrap();
        let app = dir.path().join("Emberhut.app");
        let old = dir.path().join(".Emberhut.app.old");
        // The swap has happened: the broken build is Emberhut.app, the old one waits beside it.
        std::fs::create_dir_all(app.join("Contents/MacOS")).unwrap();
        std::fs::write(app.join("build"), "15").unwrap();
        std::fs::create_dir_all(&old).unwrap();
        std::fs::write(old.join("build"), "12").unwrap();
        let installed = Installed::of(&app.join("Contents/MacOS/emberhut-desktop")).unwrap();
        let marker_path = dir.path().join("update.json");
        let marker = super::super::super::marker::tests::pending();
        marker.write(&marker_path).unwrap();
        let watch = Box::new(Watchdog::new(installed, marker_path.clone(), marker));
        let verdict = watch.run(&AtomicBool::new(false));
        let recorded = Marker::read(&marker_path).unwrap();
        if std::fs::read_to_string(app.join("build")).unwrap() == "12" {
            assert!(
                matches!(&verdict, Verdict::RolledBack(why) if why.contains("couldn't be started")),
                "{verdict:?}"
            );
            assert_eq!(recorded.state, State::RolledBack);
            assert!(!old.exists(), "the refused bundle is gone");
        } else {
            // A file system without an atomic exchange: the old build couldn't be put back.
            assert!(matches!(&verdict, Verdict::Stuck(_)), "{verdict:?}");
            assert_eq!(recorded.state, State::Pending);
        }
    }

    #[test]
    fn a_rollback_that_cant_put_the_old_build_back_says_so_and_records_no_rollback() {
        let dir = tempfile::tempdir().unwrap();
        let app = dir.path().join("Emberhut.app");
        // The broken build is in place, and the old one is gone.
        std::fs::create_dir_all(app.join("Contents/MacOS")).unwrap();
        let installed = Installed::of(&app.join("Contents/MacOS/emberhut-desktop")).unwrap();
        let marker_path = dir.path().join("update.json");
        let marker = super::super::super::marker::tests::pending();
        marker.write(&marker_path).unwrap();
        let watch = Box::new(Watchdog::new(installed, marker_path.clone(), marker));
        let verdict = watch.run(&AtomicBool::new(false));
        assert!(
            matches!(&verdict, Verdict::Stuck(why) if why.contains("couldn't be put back")),
            "{verdict:?}"
        );
        assert_eq!(Marker::read(&marker_path).unwrap().state, State::Pending);
        assert!(app.exists(), "the new build is still in place");
    }
}
