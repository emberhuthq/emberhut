//! The app's updates as the views show them: this copy's version, the channel and whether it
//! checks by itself, what a check is doing or why it failed, the build downloaded and ready
//! (the bar at the top of the chat, until the user puts it away for that build), and the notes
//! of what's new.
//!
//! The checks run on a background thread through [`crate::updates::Engine`]. With automatic
//! checks on (the default): at launch, then whenever the last one is a day old (a minute's tick
//! asks, so a computer waking from a long sleep checks within a minute); a ready build then
//! also installs by itself once the computer has had no input for 10 minutes and no message is
//! being written. With them off, only Check for updates (in Settings or the app menu) checks.
//! Switching the channel checks at once. Off macOS and Windows, and in a development build,
//! nothing is checked.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime};

use emberhut_api::releases::Build;
use gpui_kit::{App, AppContext as _, Context, EntityId, Task, WeakEntity};

use super::{Store, update_or_log};
use crate::prefs::Kept;
use crate::updates::channel::Channel;
use crate::updates::installer::Verdict;
use crate::updates::platform::{self, PowerOffWatch};
use crate::updates::record::Record;
use crate::updates::version::{self, Running};
use crate::updates::{CheckError, Earlier, Engine, Found, Ready, Unavailable, cadence};

/// How long the computer must go without input before a ready build installs by itself.
pub(crate) const IDLE_FOR: Duration = Duration::from_mins(10);

/// What the updater is doing.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) enum Phase {
    /// Nothing yet.
    #[default]
    Idle,
    /// Reading the releases.
    Checking,
    /// Downloading and checking this version's payload, this far (in percent, once known).
    Downloading(String, Option<u8>),
    /// The last check found nothing newer for this computer (why the newest isn't offered, or
    /// that the channel has no release yet, if so).
    UpToDate(Option<String>),
    /// The last check failed, worded for the user.
    Failed(String),
    /// The ready build is being installed.
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
    /// The releases followed.
    channel: Channel,
    /// Whether the app checks (and installs when idle) by itself.
    automatic: bool,
    phase: Phase,
    /// The build downloaded, verified and staged.
    ready: Option<Ready>,
    /// This copy's own release notes, once a check found them.
    own_notes: Option<String>,
    /// The channel changed during a check: another runs once it ends.
    recheck: bool,
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
        let running = Running::this();
        Self {
            running,
            engine: None,
            unavailable: None,
            channel: Channel::default_for(running),
            automatic: true,
            phase: Phase::Idle,
            ready: None,
            own_notes: None,
            recheck: false,
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
            .field("channel", &self.channel)
            .field("automatic", &self.automatic)
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

/// Settings' Updates page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Panel {
    /// This copy's version, `0.1.0-rc.1`.
    pub(crate) version: String,
    /// Its build, `Build 10001`, or `Development build`.
    pub(crate) build: String,
    /// Whether this copy updates itself: the channel, the check and the switch show.
    pub(crate) updates: bool,
    /// Why it doesn't, when it doesn't.
    pub(crate) unavailable: Option<String>,
    /// The releases followed.
    pub(crate) channel: Channel,
    /// Whether the app checks by itself.
    pub(crate) automatic: bool,
    /// What a check is doing, or what the last one found.
    pub(crate) status: Option<String>,
    /// Why the last check failed, instead.
    pub(crate) problem: Option<String>,
    /// Why a release candidate on Stable stays where it is, when it does.
    pub(crate) waiting: Option<String>,
    /// Whether Check for updates can be pressed.
    pub(crate) can_check: bool,
    /// The build ready to install, `0.2.0`, when one is.
    pub(crate) ready: Option<String>,
    /// What's new: in the build offered, or in this copy when nothing is.
    pub(crate) notes: Option<WhatsNew>,
}

/// The notes Settings shows under What's new.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct WhatsNew {
    /// Whose notes: `In 0.2.0`.
    pub(crate) title: String,
    /// The notes, plain text.
    pub(crate) text: String,
    /// The releases skipped on the way, newest first: notes not verified, shown folded.
    pub(crate) earlier: Vec<Earlier>,
}

impl Updates {
    /// What the updater is doing.
    #[cfg(test)]
    #[must_use]
    pub(crate) fn phase(&self) -> &Phase {
        &self.phase
    }

    /// The releases followed.
    #[cfg(test)]
    #[must_use]
    pub(crate) fn channel(&self) -> Channel {
        self.channel
    }

    /// This copy, `0.2.0 (build 12)`.
    #[must_use]
    pub(crate) fn version_line(&self) -> String {
        self.running.to_string()
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

    /// Settings' Updates page, at `now`.
    #[must_use]
    pub(crate) fn panel(&self, now: SystemTime) -> Panel {
        let updates = self.engine.is_some();
        let checked = self.record.last_checked().map(|at| ago(at, now));
        let problem = match &self.phase {
            Phase::Failed(why) if updates => Some(why.clone()),
            _ => None,
        };
        let status = match &self.phase {
            _ if !updates => None,
            Phase::Checking => Some("Checking…".to_owned()),
            Phase::Downloading(version, Some(percent)) => {
                Some(format!("Downloading {version} ({percent}%)…"))
            }
            Phase::Downloading(version, None) => Some(format!("Downloading {version}…")),
            Phase::Installing => Some("Installing…".to_owned()),
            Phase::Failed(_) => None,
            _ if self.ready.is_some() => Some("Ready to install".to_owned()),
            Phase::UpToDate(Some(why)) => Some(why.clone()),
            Phase::UpToDate(None) => Some(checked.as_ref().map_or_else(
                || "You're up to date".to_owned(),
                |ago| format!("You're up to date · checked {ago}"),
            )),
            Phase::Idle => checked.map(|ago| format!("Last checked {ago}")),
        };
        let waiting = (updates
            && self.channel == Channel::Stable
            && self.running.is_candidate()
            && self.ready.is_none())
        .then(|| {
            format!(
                "You're on a release candidate. Emberhut never goes back to an older build: it \
                 moves to Stable with the first stable release after {}.",
                self.running.version
            )
        });
        let notes = match (&self.ready, &self.own_notes) {
            (Some(ready), _) => Some(WhatsNew {
                title: format!("In {}", ready.staged.version),
                text: ready.notes.trim().to_owned(),
                earlier: ready.earlier.clone(),
            }),
            (None, Some(own)) => Some(WhatsNew {
                title: format!("In {}", self.running.version),
                text: own.trim().to_owned(),
                earlier: Vec::new(),
            }),
            (None, None) => None,
        }
        .filter(|notes| !notes.text.is_empty() || !notes.earlier.is_empty());
        Panel {
            version: self.running.version.to_owned(),
            build: self.running.build.map_or_else(
                || "Development build".to_owned(),
                |build| format!("Build {build}"),
            ),
            updates,
            unavailable: self
                .unavailable
                .clone()
                .or_else(|| {
                    (!updates && self.running.build.is_none())
                        .then_some(Unavailable::DevelopmentBuild)
                })
                .map(|why| why.to_string()),
            channel: self.channel,
            automatic: self.automatic,
            status,
            problem,
            waiting,
            can_check: updates && !self.busy(),
            ready: self
                .ready
                .as_ref()
                .filter(|_| self.phase != Phase::Installing)
                .map(|ready| ready.staged.version.clone()),
            notes,
        }
    }

    fn busy(&self) -> bool {
        matches!(
            self.phase,
            Phase::Checking | Phase::Downloading(..) | Phase::Installing
        )
    }

    /// Whether a message is being written: words in a composer, or a draft kept for a room.
    fn writing(&self, drafts: bool) -> bool {
        drafts || !self.writing.is_empty()
    }
}

/// Whether a ready build installs by itself: the computer has had no input for [`IDLE_FOR`]
/// (`idle`, as the system says), and no message is being written.
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

/// `published 3 days ago`, for a feed published at `at`.
fn published(at: &str, now: SystemTime) -> Option<String> {
    let at: jiff::Timestamp = at
        .parse()
        .inspect_err(|error| log::debug!("the feed's time doesn't read ({at}): {error}"))
        .ok()?;
    let at = SystemTime::UNIX_EPOCH + Duration::from_secs(u64::try_from(at.as_second()).ok()?);
    Some(format!("published {}", ago(at, now)))
}

/// What Settings says when the channel has no release for this system yet.
const NOTHING_ON_CHANNEL: &str = "No release on this channel yet";

impl Store {
    /// The updater's state.
    #[must_use]
    pub(crate) fn updates(&self) -> &Updates {
        &self.updates
    }

    /// Takes the channel and the switch kept on this computer, makes the updater (reading
    /// this copy's signature takes a moment, so off the UI thread), checks once when automatic
    /// checks are on, and starts the minute's tick.
    pub(super) fn start_updates(&mut self, cx: &mut Context<'_, Self>) {
        let kept = Kept::get(cx).updates;
        self.updates.channel = kept
            .channel
            .unwrap_or_else(|| Channel::default_for(self.updates.running));
        self.updates.automatic = kept.automatic;
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

    /// Uses `engine` with the `record` kept, and checks now (a launch) when automatic checks
    /// are on and GitHub isn't limiting requests.
    fn use_updater(&mut self, engine: Arc<Engine>, record: Record, cx: &mut Context<'_, Self>) {
        if let Some(notice) = engine.rolled_back() {
            self.updates.phase = Phase::Failed(notice);
        }
        self.updates.running = engine.running();
        self.updates.engine = Some(engine);
        let limited = record.limited_at(SystemTime::now()).is_some();
        self.updates.record = record;
        cx.notify();
        if self.updates.automatic && !limited {
            self.check_updates(cx).detach();
        }
    }

    /// A minute's tick, with automatic checks on: an install when the computer is idle, a
    /// check when the last is a day old.
    fn tick(&mut self, cx: &mut Context<'_, Self>) {
        if self.updates.busy() || !self.updates.automatic {
            return;
        }
        let writing = self.updates.writing(self.composer.has_drafts());
        if self.updates.ready.is_some() && installs_alone(platform::idle(), writing) {
            log::info!("the computer is idle: installing the update");
            self.restart_to_update(cx);
            return;
        }
        let now = SystemTime::now();
        let record = &self.updates.record;
        if record.limited_at(now).is_none()
            && cadence::is_due(record.last_checked(), record.last_attempt(), now)
        {
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

    /// Follows `channel`, kept on this computer, and checks at once. Leaving candidates for
    /// Stable drops a candidate already downloaded: the copy waits for a stable release above
    /// itself instead.
    pub(crate) fn set_update_channel(&mut self, channel: Channel, cx: &mut Context<'_, Self>) {
        if channel == self.updates.channel {
            return;
        }
        self.updates.channel = channel;
        Kept::set_updates(cx, |kept| kept.channel = Some(channel));
        if channel == Channel::Stable
            && self
                .updates
                .ready
                .as_ref()
                .is_some_and(|ready| version::is_candidate(&ready.staged.version))
        {
            self.updates.ready = None;
        }
        if self.updates.busy() {
            self.updates.recheck = true;
        } else if self.updates.engine.is_some() {
            self.check_updates(cx).detach();
        }
        cx.notify();
    }

    /// Turns automatic checks on or off, kept on this computer.
    pub(crate) fn set_update_automatic(&mut self, on: bool, cx: &mut Context<'_, Self>) {
        if on == self.updates.automatic {
            return;
        }
        self.updates.automatic = on;
        Kept::set_updates(cx, |kept| kept.automatic = on);
        cx.notify();
    }

    /// Check for Updates… (the app menu): a check now, answered for its dialog; a build
    /// already downloaded installs at once.
    pub(crate) fn check_for_updates(&mut self, cx: &mut Context<'_, Self>) -> Task<Answer> {
        if self.updates.ready.is_some() && !self.updates.busy() {
            self.restart_to_update(cx);
            return Task::ready(Answer::Installing);
        }
        self.check_updates(cx)
    }

    /// Settings' Check for updates: a check now, its outcome on the page.
    pub(crate) fn check_updates_now(&mut self, cx: &mut Context<'_, Self>) {
        self.check_updates(cx).detach();
    }

    /// Reads the releases and takes what they offer, on a background thread.
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
        self.updates.recheck = false;
        cx.notify();
        let record = self.updates.record.clone();
        let staged = self.updates.ready.as_ref().map(|ready| ready.staged.build);
        let channel = self.updates.channel;
        let (downloading, told) = smol::channel::bounded::<(String, Option<u8>)>(4);
        cx.spawn(async move |this, cx| {
            while let Ok((version, percent)) = told.recv().await {
                let shown = update_or_log(&this, cx, |store, cx| {
                    if store.updates.busy() {
                        store.updates.phase = Phase::Downloading(version, percent);
                        cx.notify();
                    }
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
                    let on_download = |version: &str, percent: Option<u8>| {
                        if downloading.try_send((version.to_owned(), percent)).is_err() {
                            log::debug!("the download's progress wasn't shown");
                        }
                    };
                    engine.check(record, staged, channel, SystemTime::now(), &on_download)
                })
                .await;
            update_or_log(&this, cx, |store, cx| store.checked(checked, channel, cx))
                .unwrap_or(Answer::Busy)
        })
    }

    /// A check's outcome on `channel`, and the record as it left it.
    fn checked(
        &mut self,
        (record, found): (Record, Result<Found, CheckError>),
        channel: Channel,
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
                self.updates.phase = Phase::UpToDate(Some(NOTHING_ON_CHANNEL.to_owned()));
                Answer::Latest {
                    running: short(self.updates.running),
                    detail: Some(NOTHING_ON_CHANNEL.to_owned()),
                }
            }
            Ok(Found::Latest {
                unfit,
                notes,
                published: _,
            }) => {
                if notes.is_some() {
                    self.updates.own_notes = notes;
                }
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
                    "Update Emberhut by hand: its releases are in a format this copy can't read";
                self.updates.phase = Phase::Failed(why.to_owned());
                Answer::Failed(why.to_owned())
            }
            Err(error) => {
                log::warn!("the update check failed: {error}");
                self.updates.phase = Phase::Failed(error.to_string());
                Answer::Failed(error.to_string())
            }
        };
        if self.updates.recheck || channel != self.updates.channel {
            // The channel changed while this check ran: check again on the new one.
            self.check_updates(cx).detach();
        }
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

    /// Install and restart: the ready build goes in and the app quits. On macOS it swaps in,
    /// this copy steps aside and watches it start (putting the old build back if it doesn't);
    /// on Windows the installer runs once this copy has quit and starts the app again.
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
                    log::error!("the update couldn't be installed: {error}");
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
            notes: "- Rooms open faster (#12)\n- The update bar says what's new (#14)".to_owned(),
            earlier: vec![Earlier {
                version: "0.1.0-rc.2".to_owned(),
                notes: "- A fix for pasted pictures (#9)".to_owned(),
            }],
            published: emberhut_api::time::Timestamp::new("2026-12-08T10:00:00.000Z"),
        });
    }

    /// The updater over `engine`, for the tests, with nothing running; `after_watch` runs once
    /// a watch ends.
    #[cfg(test)]
    pub(crate) fn use_test_updater(&mut self, engine: Arc<Engine>, after_watch: fn(&mut App)) {
        self.updates.running = engine.running();
        self.updates.channel = Channel::default_for(engine.running());
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
