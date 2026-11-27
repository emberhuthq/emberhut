//! The app's updates as the views show them: this copy's version, when the feed was last read
//! and how old it was, the build downloaded and ready (the bar at the top of the chat, until
//! the user puts it away for that build), and what a check is doing or why it failed.
//!
//! The checks run on a background thread through [`crate::updates::Engine`]: at launch, then
//! whenever the last one is 4 hours old (a minute's tick asks, so a Mac waking from a long
//! sleep checks within a minute). A ready build installs on Restart to Update, from the menu's
//! Check for Updates…, or by itself once the Mac has had no input for 10 minutes and no
//! message is being written. Off the Mac, and in a development build, nothing is checked.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime};

use emberhut_api::releases::Build;
use gpui_kit::{App, AppContext as _, Context, EntityId, Task, WeakEntity};

use super::{Store, update_or_log};
use crate::updates::installer::Verdict;
use crate::updates::platform::{self, PowerOffWatch};
use crate::updates::record::Record;
use crate::updates::version::Running;
use crate::updates::{CheckError, Earlier, Engine, Found, Ready, Unavailable, cadence};

/// How long the Mac must go without input before a ready build installs by itself.
pub(crate) const IDLE_FOR: Duration = Duration::from_mins(10);

/// The line Settings shows under the rows: how updates work.
pub(crate) const HOW_IT_WORKS: &str = "Updates come from your Emberhut server; Emberhut checks every 4 hours and installs by itself when the Mac is idle.";

/// What the updater is doing.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) enum Phase {
    /// Nothing yet.
    #[default]
    Idle,
    /// Reading the feed.
    Checking,
    /// Downloading and checking this version's payload.
    Downloading(String),
    /// The last check found nothing newer for this Mac (why the newest isn't offered, if one
    /// isn't).
    UpToDate(Option<String>),
    /// The last check failed, worded for the user.
    Failed(String),
    /// The ready build is being swapped in.
    Installing,
}

/// The answer to Check for Updates…, for its dialog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Answer {
    /// Nothing newer.
    Latest {
        /// This copy, `0.2.0, build 12`.
        running: String,
        /// How old the feed is (`published 3 days ago`), and why a newer build doesn't fit.
        detail: Option<String>,
    },
    /// A newer build is downloaded and verified.
    Ready {
        /// `Emberhut 0.3.0 (build 15) is ready`.
        title: String,
        /// Its notes, and the unverified notes of the releases it skips.
        notes: String,
    },
    /// The build already downloaded installs now.
    Installing,
    /// The check failed.
    Failed(String),
    /// This copy doesn't update itself.
    Unavailable {
        /// This copy, `0.1.0, development build`.
        running: String,
        /// Why.
        why: String,
    },
    /// A check is already running.
    Busy,
}

/// The updater's state in the store.
pub(crate) struct Updates {
    running: Running,
    /// The updater, once made: none in a copy that doesn't update itself.
    engine: Option<Arc<Engine>>,
    /// Why there is none, once known.
    unavailable: Option<Unavailable>,
    phase: Phase,
    /// The build downloaded, verified and staged.
    ready: Option<Ready>,
    record: Record,
    /// The chat views whose composer holds words.
    writing: HashSet<EntityId>,
    /// Tells the watch to stop (a logout or power-off while it watches).
    stop: Arc<AtomicBool>,
    /// The logout notice's registration, while the watch runs.
    power_off: Option<PowerOffWatch>,
    /// What runs once the watch has ended (the app quits; the tests note it).
    after_watch: fn(&mut App),
}

impl Default for Updates {
    fn default() -> Self {
        Self {
            running: Running::this(),
            engine: None,
            unavailable: None,
            phase: Phase::Idle,
            ready: None,
            record: Record::default(),
            writing: HashSet::new(),
            stop: Arc::default(),
            power_off: None,
            after_watch: quit,
        }
    }
}

impl std::fmt::Debug for Updates {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Updates")
            .field("running", &self.running)
            .field("phase", &self.phase)
            .field("ready", &self.ready)
            .finish_non_exhaustive()
    }
}

/// The update bar: a build ready that the user hasn't put away.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Bar {
    /// `Emberhut 0.3.0 is ready`, or `Build 15 is ready` when the version doesn't change.
    pub(crate) title: String,
    /// The build it offers.
    pub(crate) build: Build,
}

/// Settings' rows about this copy and its updates.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Rows {
    /// `0.2.0 (build 12)`.
    pub(crate) version: String,
    /// `5 minutes ago`, `Not yet`; none where nothing is checked.
    pub(crate) last_checked: Option<String>,
    /// How old the feed was at the last check.
    pub(crate) published: Option<String>,
    /// `Build 15`, when one is ready.
    pub(crate) ready: Option<String>,
    /// What a check is doing, or why the last one failed or the copy doesn't update.
    pub(crate) status: Option<String>,
    /// Whether Check for updates can be pressed.
    pub(crate) can_check: bool,
}

impl Updates {
    /// What the updater is doing.
    #[cfg(test)]
    #[must_use]
    pub(crate) fn phase(&self) -> &Phase {
        &self.phase
    }

    /// The bar at the top of the chat, if it shows.
    #[must_use]
    pub(crate) fn bar(&self) -> Option<Bar> {
        let ready = self.ready.as_ref()?;
        let build = ready.staged.build;
        if self.record.dismissed.is_some_and(|away| away >= build)
            || self.phase == Phase::Installing
        {
            return None;
        }
        let title = if ready.staged.version == self.running.version {
            format!("Build {build} is ready")
        } else {
            format!("Emberhut {} is ready", ready.staged.version)
        };
        Some(Bar { title, build })
    }

    /// Settings' rows, at `now`.
    #[must_use]
    pub(crate) fn rows(&self, now: SystemTime) -> Rows {
        let checks = self.engine.is_some();
        let status = match (&self.unavailable, &self.phase) {
            (Some(why), _) => Some(why.to_string()),
            (None, Phase::Checking) => Some("Checking for updates…".to_owned()),
            (None, Phase::Downloading(version)) => Some(format!("Downloading Emberhut {version}…")),
            (None, Phase::Installing) => Some("Installing the update…".to_owned()),
            (None, Phase::Failed(why)) => Some(why.clone()),
            (None, Phase::UpToDate(unfit)) => unfit.clone(),
            (None, Phase::Idle) => None,
        };
        Rows {
            version: self.running.to_string(),
            last_checked: checks.then(|| {
                self.record
                    .last_checked()
                    .map_or_else(|| "Not yet".to_owned(), |at| ago(at, now))
            }),
            published: checks
                .then(|| {
                    self.record
                        .published
                        .as_deref()
                        .and_then(|at| published(at, now))
                })
                .flatten(),
            ready: self
                .ready
                .as_ref()
                .map(|ready| format!("Build {}", ready.staged.build)),
            status,
            can_check: checks && !self.busy(),
        }
    }

    fn busy(&self) -> bool {
        matches!(
            self.phase,
            Phase::Checking | Phase::Downloading(_) | Phase::Installing
        )
    }

    /// Whether a message is being written: words in a composer, or a draft kept for a room.
    fn writing(&self, drafts: bool) -> bool {
        drafts || !self.writing.is_empty()
    }
}

/// Whether a ready build installs by itself: the Mac has had no input for [`IDLE_FOR`] (`idle`,
/// as the system says), and no message is being written.
#[must_use]
pub(crate) fn installs_alone(idle: Option<Duration>, writing: bool) -> bool {
    !writing && idle.is_some_and(|idle| idle >= IDLE_FOR)
}

/// `0.2.0, build 12` or `0.1.0, development build`, for the dialog's parentheses.
fn short(running: Running) -> String {
    match running.build {
        Some(build) => format!("{}, build {build}", running.version),
        None => format!("{}, development build", running.version),
    }
}

/// How long ago `at` was at `now`: `just now`, `5 minutes ago`, `3 hours ago`, `2 days ago`.
#[must_use]
pub(crate) fn ago(at: SystemTime, now: SystemTime) -> String {
    let seconds = now.duration_since(at).map_or(0, |age| age.as_secs());
    let (count, unit) = match seconds {
        0..60 => return "just now".to_owned(),
        60..3600 => (seconds / 60, "minute"),
        3600..86_400 => (seconds / 3600, "hour"),
        _ => (seconds / 86_400, "day"),
    };
    let plural = if count == 1 { "" } else { "s" };
    format!("{count} {unit}{plural} ago")
}

/// `published 3 days ago`, for a feed published at the server's time `at`.
fn published(at: &str, now: SystemTime) -> Option<String> {
    let at: jiff::Timestamp = at
        .parse()
        .inspect_err(|error| log::debug!("the feed's time doesn't read ({at}): {error}"))
        .ok()?;
    let at = SystemTime::UNIX_EPOCH + Duration::from_secs(u64::try_from(at.as_second()).ok()?);
    Some(format!("published {}", ago(at, now)))
}

impl Store {
    /// The updater's state.
    #[must_use]
    pub(crate) fn updates(&self) -> &Updates {
        &self.updates
    }

    /// Makes the updater (reading this copy's signature takes a moment, so off the UI thread),
    /// checks once, and starts the minute's tick.
    pub(super) fn start_updates(&mut self, cx: &mut Context<'_, Self>) {
        let files = self.files.clone();
        cx.spawn(async move |this, cx| {
            let made = cx
                .background_spawn(async move {
                    let engine = Engine::for_this_app(files.as_ref())?;
                    let record = Record::read(&engine.record_path());
                    Ok::<_, Unavailable>((engine, record))
                })
                .await;
            let started = update_or_log(&this, cx, |store, cx| match made {
                Ok((engine, record)) => {
                    store.use_updater(Arc::new(engine), record, cx);
                    true
                }
                Err(why) => {
                    log::info!("no updates: {why}");
                    store.updates.unavailable = Some(why);
                    cx.notify();
                    false
                }
            });
            if started != Some(true) {
                return;
            }
            loop {
                cx.background_executor().timer(cadence::TICK).await;
                if update_or_log(&this, cx, Store::tick).is_none() {
                    return;
                }
            }
        })
        .detach();
    }

    /// Uses `engine` with the `record` kept, and checks now (a launch).
    fn use_updater(&mut self, engine: Arc<Engine>, record: Record, cx: &mut Context<'_, Self>) {
        if let Some(notice) = engine.rolled_back() {
            self.updates.phase = Phase::Failed(notice);
        }
        self.updates.running = engine.running();
        self.updates.engine = Some(engine);
        self.updates.record = record;
        self.check_updates(cx).detach();
    }

    /// A minute's tick: a check when the last is 4 hours old, an install when the Mac is idle.
    fn tick(&mut self, cx: &mut Context<'_, Self>) {
        if self.updates.busy() {
            return;
        }
        let writing = self.updates.writing(self.composer.has_drafts());
        if self.updates.ready.is_some() && installs_alone(platform::idle(), writing) {
            log::info!("the Mac is idle: installing the update");
            self.restart_to_update(cx);
            return;
        }
        if cadence::is_due(self.updates.record.last_checked(), SystemTime::now()) {
            self.check_updates(cx).detach();
        }
    }

    /// The chat view `view`'s composer holds words (or doesn't any more), so a ready build
    /// doesn't install by itself under them.
    pub(crate) fn note_writing(&mut self, view: EntityId, writing: bool) {
        if writing {
            self.updates.writing.insert(view);
        } else {
            self.updates.writing.remove(&view);
        }
    }

    /// Check for Updates…: a check now, answered for its dialog; a build already downloaded
    /// installs at once.
    pub(crate) fn check_for_updates(&mut self, cx: &mut Context<'_, Self>) -> Task<Answer> {
        if self.updates.ready.is_some() && !self.updates.busy() {
            self.restart_to_update(cx);
            return Task::ready(Answer::Installing);
        }
        self.check_updates(cx)
    }

    /// Reads the feed and takes what it offers, on a background thread.
    fn check_updates(&mut self, cx: &mut Context<'_, Self>) -> Task<Answer> {
        let Some(engine) = self.updates.engine.clone() else {
            let why = self.updates.unavailable.as_ref().map_or_else(
                || "Emberhut is still starting".to_owned(),
                ToString::to_string,
            );
            return Task::ready(Answer::Unavailable {
                running: short(self.updates.running),
                why,
            });
        };
        if self.updates.busy() {
            return Task::ready(Answer::Busy);
        }
        self.updates.phase = Phase::Checking;
        cx.notify();
        let backend = Arc::clone(&self.backend);
        let record = self.updates.record.clone();
        let staged = self.updates.ready.as_ref().map(|ready| ready.staged.build);
        let (downloading, told) = smol::channel::bounded::<String>(1);
        cx.spawn(async move |this, cx| {
            while let Ok(version) = told.recv().await {
                let shown = update_or_log(&this, cx, |store, cx| {
                    store.updates.phase = Phase::Downloading(version);
                    cx.notify();
                });
                if shown.is_none() {
                    return;
                }
            }
        })
        .detach();
        cx.spawn(async move |this, cx| {
            let checked = cx
                .background_spawn(async move {
                    let on_download = |version: &str| {
                        if downloading.try_send(version.to_owned()).is_err() {
                            log::debug!("the download's start wasn't shown");
                        }
                    };
                    engine.check(&*backend, record, staged, SystemTime::now(), &on_download)
                })
                .await;
            update_or_log(&this, cx, |store, cx| store.checked(checked, cx)).unwrap_or(Answer::Busy)
        })
    }

    /// A check's outcome, and the record as it left it.
    fn checked(
        &mut self,
        (record, found): (Record, Result<Found, CheckError>),
        cx: &mut Context<'_, Self>,
    ) -> Answer {
        self.updates.record = record;
        self.save_update_record(cx);
        let detail = |unfit: Option<String>, record: &Record| {
            let age = record
                .published
                .as_deref()
                .and_then(|at| published(at, SystemTime::now()));
            match (age, unfit) {
                (Some(age), Some(unfit)) => {
                    Some(format!("{}; {unfit}", crate::format::capitalized(&age)))
                }
                (Some(age), None) => Some(crate::format::capitalized(&age)),
                (None, unfit) => unfit,
            }
        };
        let answer = match found {
            Ok(Found::NothingPublished) => {
                self.updates.phase = Phase::UpToDate(None);
                Answer::Latest {
                    running: short(self.updates.running),
                    detail: Some("Your server has published no update yet".to_owned()),
                }
            }
            Ok(Found::Latest { unfit, .. }) => {
                self.updates.phase = Phase::UpToDate(unfit.clone());
                Answer::Latest {
                    running: short(self.updates.running),
                    detail: detail(unfit, &self.updates.record),
                }
            }
            Ok(Found::Ready(ready)) => {
                self.updates.ready = Some(ready);
                self.updates.phase = Phase::UpToDate(None);
                self.ready_answer()
            }
            Ok(Found::AlreadyReady) => {
                self.updates.phase = Phase::UpToDate(None);
                self.ready_answer()
            }
            Ok(Found::ByHand) => {
                let why =
                    "Update Emberhut by hand: your server publishes updates this copy can't read";
                self.updates.phase = Phase::Failed(why.to_owned());
                Answer::Failed(why.to_owned())
            }
            Err(error) => {
                log::warn!("the update check failed: {error}");
                self.updates.phase = Phase::Failed(error.to_string());
                Answer::Failed(error.to_string())
            }
        };
        cx.notify();
        answer
    }

    fn ready_answer(&self) -> Answer {
        let Some(ready) = &self.updates.ready else {
            return Answer::Busy;
        };
        let mut notes = ready.notes.trim().to_owned();
        if !ready.earlier.is_empty() {
            let earlier: Vec<String> = ready
                .earlier
                .iter()
                .map(|Earlier { version, notes }| format!("{version}: {}", notes.trim()))
                .collect();
            notes = format!(
                "{notes}\n\nEarlier releases (unverified):\n{}",
                earlier.join("\n")
            );
        }
        Answer::Ready {
            title: format!(
                "Emberhut {} (build {}) is ready",
                ready.staged.version, ready.staged.build
            ),
            notes,
        }
    }

    /// The X on the bar: it stays away until a newer build is ready.
    pub(crate) fn dismiss_update_bar(&mut self, cx: &mut Context<'_, Self>) {
        let Some(bar) = self.updates.bar() else {
            return;
        };
        self.updates.record.dismissed = Some(bar.build);
        self.save_update_record(cx);
        cx.notify();
    }

    /// Restart to Update: the ready build swaps in, this copy steps aside and watches it start,
    /// then quits (putting the old build back first if the new one doesn't start).
    pub(crate) fn restart_to_update(&mut self, cx: &mut Context<'_, Self>) {
        let (Some(engine), Some(ready)) = (self.updates.engine.clone(), self.updates.ready.clone())
        else {
            return;
        };
        if self.updates.busy() {
            return;
        }
        self.updates.phase = Phase::Installing;
        cx.notify();
        let stop = Arc::clone(&self.updates.stop);
        cx.spawn(async move |this, cx| {
            let swap = cx
                .background_spawn({
                    let engine = Arc::clone(&engine);
                    let staged = ready.staged.clone();
                    async move { engine.swap(&staged) }
                })
                .await;
            let watch = match swap {
                Ok(watch) => watch,
                Err(error) => {
                    log::error!("the update couldn't be swapped in: {error}");
                    update_or_log(&this, cx, |store, cx| {
                        store.updates.ready = None;
                        store.updates.phase =
                            Phase::Failed(format!("Couldn't install the update: {error}"));
                        cx.notify();
                    });
                    return;
                }
            };
            let after = update_or_log(&this, cx, |store, _| {
                let stopping = Arc::clone(&store.updates.stop);
                store.updates.power_off = Some(platform::watch_power_off(move || {
                    stopping.store(true, Ordering::Relaxed);
                }));
                store.updates.after_watch
            });
            hand_over(&this, cx).await;
            step_aside(cx);
            let verdict = cx.background_spawn(async move { watch.run(&stop) }).await;
            log::info!("the update ended: {verdict:?}");
            match &verdict {
                Verdict::RolledBack(why) => log::error!("the update was rolled back: {why}"),
                Verdict::Stuck(why) => log::error!("the update failed: {why}"),
                Verdict::Kept | Verdict::KeptRestarted | Verdict::Stopped => {}
            }
            if let Some(after) = after {
                cx.update(after);
            }
        })
        .detach();
    }

    /// Keeps the record for the next launch, off the UI thread.
    fn save_update_record(&self, cx: &mut Context<'_, Self>) {
        let Some(engine) = &self.updates.engine else {
            return;
        };
        let (record, path): (Record, PathBuf) = (self.updates.record.clone(), engine.record_path());
        cx.background_spawn(async move {
            if let Err(error) = record.write(&path) {
                log::warn!("couldn't keep the update record: {error}");
            }
        })
        .detach();
    }

    /// A build `version` (`build`) downloaded and ready, for the views' tests.
    #[cfg(test)]
    pub(crate) fn put_ready_update_for_tests(&mut self, version: &str, build: u64) {
        self.updates.ready = Some(Ready {
            staged: crate::updates::installer::Staged {
                build: Build::new(build),
                version: version.to_owned(),
            },
            notes: String::new(),
            earlier: Vec::new(),
            published: emberhut_api::time::Timestamp::new("2026-12-08T10:00:00.000Z"),
        });
    }

    /// The updater over `engine`, for the tests, with nothing running; `after_watch` runs once
    /// a watch ends.
    #[cfg(test)]
    pub(crate) fn use_test_updater(&mut self, engine: Arc<Engine>, after_watch: fn(&mut App)) {
        self.updates.running = engine.running();
        self.updates.engine = Some(engine);
        self.updates.after_watch = after_watch;
    }
}

/// Before the new copy starts, this one stops talking to the server, so only one process ever
/// renews the session: its live stream ends, its backend waits for a renewal in flight, saves
/// the session for the new copy to read, and sends and renews nothing more (the polls and
/// streams still running fail at once from then on).
async fn hand_over(this: &WeakEntity<Store>, cx: &mut gpui_kit::AsyncApp) {
    let Some(backend) = update_or_log(this, cx, |store, _| {
        store.live_feed.halt();
        Arc::clone(&store.backend)
    }) else {
        return;
    };
    if let Err(error) = cx.background_spawn(async move { backend.stop() }).await {
        log::error!("the session couldn't be saved for the update: {error}");
    }
}

/// This copy closes its windows and leaves the Dock and the menu bar while it watches the new
/// copy start, so the user sees one Emberhut.
fn step_aside(cx: &mut gpui_kit::AsyncApp) {
    cx.update(|cx| {
        for handle in cx.windows() {
            if let Err(error) = handle.update(cx, |_, window, _| window.remove_window()) {
                log::debug!("a window closed before the update: {error}");
            }
        }
        platform::step_aside();
    });
}

/// Quits the app: the watch is over.
fn quit(cx: &mut App) {
    cx.quit();
}

#[cfg(test)]
#[path = "updates_tests.rs"]
mod tests;
