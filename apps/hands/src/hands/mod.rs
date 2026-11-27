//! The helper's requests: one app in the background, the whole screen, the lock and the
//! recorder, each a path the server's Mac tools `POST` to with a JSON body. [`Hands::handle`]
//! answers one, over any [`Desktop`].

mod apps;
mod elements;
pub mod screen;
pub mod session;
mod snapshot;
mod steps;

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use self::screen::{Ended, Screen};
use self::session::Session;
use crate::desktop::{Bar, Desktop};
use crate::fail::{Fail, Reply, field};
use crate::record::Recorder;
use crate::rules::{Use, refusal};

/// Every path the helper answers with a JSON body (`POST`).
pub const PATHS: &[&str] = &[
    "/health",
    "/apps",
    "/which",
    "/describe",
    "/open",
    "/snapshot",
    "/press",
    "/set",
    "/keys",
    "/menu",
    "/release",
    "/screen/start",
    "/screen/shot",
    "/screen/act",
    "/screen/stop",
    "/record/start",
    "/record/stop",
    "/record/status",
    "/lock",
];

/// What the agent is told without the Accessibility grant.
const NO_ACCESSIBILITY: &str = "Emberhut Hands has no Accessibility permission yet: the owner turns \
                                it on in System Settings → Privacy & Security → Accessibility.";

/// The time now, swapped in tests.
pub trait Clock: Send + Sync {
    /// Now.
    fn now(&self) -> Instant;
}

/// The system's monotonic clock.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

/// A request's answer, as the HTTP layer sends it.
pub trait Handler: Send + Sync {
    /// Answers `path` with `body`.
    ///
    /// # Errors
    ///
    /// [`Fail`]: the status and words of a refusal.
    fn handle(&self, path: &str, body: &Reply) -> Result<Reply, Fail>;
}

/// The helper's state over a desktop: the app held, the screen session and the recorder.
pub struct Hands<D: Desktop> {
    desktop: Arc<D>,
    clock: Arc<dyn Clock>,
    recorder: Recorder,
    session: Mutex<Option<Session<D::Element>>>,
    screen: Mutex<Screen>,
    bar: Arc<Bar>,
    /// One request at a time, as the reference helper's queue runs them.
    work: Mutex<()>,
}

impl<D: Desktop> std::fmt::Debug for Hands<D> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Hands").finish_non_exhaustive()
    }
}

/// An app found for a request: running, allowed, and its accessibility root.
struct Target<E> {
    pid: i32,
    name: String,
    hidden: bool,
    root: E,
}

/// Locks `mutex`, going on with its data if a panicking request left it poisoned.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl<D: Desktop> Hands<D> {
    /// The helper over `desktop`, timed by `clock`, recording with `recorder`.
    #[must_use]
    pub fn new(desktop: Arc<D>, clock: Arc<dyn Clock>, recorder: Recorder) -> Self {
        Self {
            desktop,
            clock,
            recorder,
            session: Mutex::new(None),
            screen: Mutex::new(Screen::default()),
            bar: Arc::new(Bar::default()),
            work: Mutex::new(()),
        }
    }

    /// The recorder.
    #[must_use]
    pub fn recorder(&self) -> &Recorder {
        &self.recorder
    }

    /// The owner pressed Stop on the bar: the screen session and any recording end at once.
    pub fn owner_stopped(&self) {
        let now = self.clock.now();
        lock(&self.screen).end(Ended::Stop, now);
        if self.recorder.current().is_some() {
            let said = self.recorder.stop("the owner pressed Stop");
            tracing::info!(said = %field(&said, "text"), "recording stopped from the bar");
        }
        tracing::info!("the owner pressed Stop");
        self.show_bar();
    }

    /// Checks the screen session against the clock and the owner's touch (every quarter of a
    /// second): it ends when its time is up, and when the owner moves the mouse or types.
    pub fn watch(&self) {
        let now = self.clock.now();
        let ended = {
            let mut screen = lock(&self.screen);
            if !screen.driving() {
                return;
            }
            screen.watch(now, self.desktop.idle_seconds(), self.desktop.input_count())
        };
        if let Some(cause) = ended {
            tracing::info!(?cause, "the screen session ended");
            self.show_bar();
        }
    }

    /// Asks for the bar that says what runs now (the screen session, a recording, both or
    /// none), worked out under the bar's lock so the last ask carries the latest state: the
    /// ask's number, to wait for.
    fn show_bar(&self) -> u64 {
        let asked = self.bar.ask(|| {
            let driving = lock(&self.screen).reason();
            let recording = self.recorder.current().map(|(_, title)| title);
            screen::bar_text(driving.as_deref(), recording.as_deref())
        });
        self.desktop.show_bar(&self.bar);
        asked
    }

    /// The recording's capture ended by itself (macOS stopped it): the recording stops, its
    /// tracks go to the server, and the bar says so.
    /// `number` is the lost recording's: a newer recording goes on.
    pub fn capture_lost(&self, number: u64) {
        if let Some(said) = self.recorder.stop_lost(number) {
            tracing::warn!(said = %field(&said, "text"), "the recording's capture was lost");
        }
        self.show_bar();
    }

    /// `/record/start`: starts recording, and keeps it only once the bar with Stop says so.
    fn record_start(&self, body: &Reply) -> Result<Reply, Fail> {
        let started = self.recorder.start(body)?;
        let asked = self.show_bar();
        if !self.bar.wait(asked, screen::BAR_UP) {
            let said = self.recorder.stop("the bar with Stop could not be shown");
            tracing::warn!(said = %field(&said, "text"), "a recording without its bar was stopped");
            self.show_bar();
            return Err(Fail::broke(
                "The bar with Stop could not be shown on the Mac, so the call is not recorded.",
            ));
        }
        Ok(started)
    }

    /// `/health`: the grants, what runs, how idle and whether locked.
    fn health(&self) -> Reply {
        let now = self.clock.now();
        let (driving, stopped) = {
            let screen = lock(&self.screen);
            (screen.who(), screen.stopped_for(now))
        };
        let idle = (self.desktop.idle_seconds() * 10.0).round() / 10.0;
        let reply = json!({
            "ok": true,
            "accessibility": self.desktop.accessibility(),
            "screen": self.desktop.screen_recording(),
            "driving": driving,
            "recording": self.recorder.current().map(|(id, _)| id),
            "idle": idle,
            "locked": self.desktop.locked(),
            "stopped": stopped.map(|s| s.as_secs_f64().round()),
        });
        object(reply)
    }

    /// `/lock`: locks the screen. Nothing here ever unlocks it.
    fn lock_screen(&self) -> Result<Reply, Fail> {
        self.desktop.lock().map_err(Fail::bad)?;
        tracing::info!("locked the screen");
        Ok(object(json!({ "text": "Locked.", "locked": true })))
    }

    /// `/release`: lets go of the app held, so another can be used at once.
    fn release(&self) -> Reply {
        let held = lock(&self.session).take();
        crate::fail::said(match held {
            Some(held) => format!("Let go of {}.", held.name),
            None => "No app was held.".to_owned(),
        })
    }

    /// The app `body.app` names (a name, a bundle id or a pid), running and allowed for
    /// `wanted`, with the held app's rule kept; it becomes the app held.
    fn target(&self, body: &Reply, wanted: Use) -> Result<Target<D::Element>, Fail> {
        if !self.desktop.accessibility() {
            return Err(Fail::never(NO_ACCESSIBILITY));
        }
        let want = field(body, "app").trim().to_owned();
        if want.is_empty() {
            return Err(Fail::bad("Which app? (app: its name, like Calculator)"));
        }
        let running = self.desktop.running();
        let mut found = apps::matching(&running, &want);
        if found.is_empty()
            && let Some(installed) = self.desktop.installed(&want)
            && !installed.bundle.is_empty()
        {
            found = self.desktop.running_with_bundle(&installed.bundle);
        }
        let app = match found.as_slice() {
            [] => {
                let close: Vec<&str> = running
                    .iter()
                    .filter(|app| app.name.to_lowercase().contains(&want.to_lowercase()))
                    .map(|app| app.name.as_str())
                    .collect();
                let hint = if close.is_empty() {
                    String::new()
                } else {
                    format!(" (did you mean {}?)", close.join(", "))
                };
                return Err(Fail::missing(format!(
                    "{want} isn't running{hint}: mac_app_open opens it in the background."
                )));
            }
            [one] => one.clone(),
            many => {
                let pids: Vec<String> = many.iter().map(|app| app.pid.to_string()).collect();
                return Err(Fail::bad(format!(
                    "More than one {want} is running (pids {}): say which by pid.",
                    pids.join(", ")
                )));
            }
        };
        if let Some(refused) = refusal(&app.name, &app.bundle, wanted) {
            return Err(refused);
        }
        self.take(app.pid, &app.name)?;
        Ok(Target {
            pid: app.pid,
            name: app.name,
            hidden: app.hidden,
            root: self.desktop.application(app.pid),
        })
    }

    /// Makes the app `pid` the one held, when the rule lets it (see [`session::admit`]).
    fn take(&self, pid: i32, name: &str) -> Result<(), Fail> {
        let now = self.clock.now();
        let mut session = lock(&self.session);
        let held_runs = session
            .as_ref()
            .is_some_and(|held| self.desktop.app(held.pid).is_some());
        session::admit(session.as_ref(), Some(pid), name, held_runs, now)?;
        match session.as_mut() {
            Some(held) if held.pid == pid => held.used = now,
            _ => *session = Some(Session::new(pid, name, now)),
        }
        Ok(())
    }

    /// Whether `name` (the app `pid`, when it runs) may be used now, without taking it.
    fn may_use(&self, pid: Option<i32>, name: &str) -> Result<(), Fail> {
        let session = lock(&self.session);
        let held_runs = session
            .as_ref()
            .is_some_and(|held| self.desktop.app(held.pid).is_some());
        session::admit(session.as_ref(), pid, name, held_runs, self.clock.now())
    }

    /// Runs `act`; if it brought the target to the front, the app the owner had in front goes
    /// back there, and the answer's words say so.
    fn keep_front<T>(
        &self,
        target: &Target<D::Element>,
        act: impl FnOnce() -> Result<T, Fail>,
    ) -> Result<(T, String), Fail> {
        let before = self.desktop.front();
        let done = act()?;
        self.desktop.pause(Duration::from_millis(300));
        let after = self.desktop.front();
        if let Some(before) = before
            && before != target.pid
            && after == Some(target.pid)
        {
            self.desktop.activate(before);
            let name = self
                .desktop
                .running()
                .into_iter()
                .find(|app| app.pid == before)
                .map_or_else(|| "the owner's app".to_owned(), |app| app.name);
            return Ok((
                done,
                format!(
                    " ({} came to the front; {name} was put back in front)",
                    target.name
                ),
            ));
        }
        Ok((done, String::new()))
    }

    /// The app's state as the answers say it: hidden or not, in front or in the background.
    fn state(&self, pid: i32, hidden: bool) -> String {
        let place = if self.desktop.front() == Some(pid) {
            "in front"
        } else {
            "in the background"
        };
        if hidden {
            format!("hidden, {place}")
        } else {
            place.to_owned()
        }
    }
}

impl<D: Desktop> Handler for Hands<D> {
    fn handle(&self, path: &str, body: &Reply) -> Result<Reply, Fail> {
        if path == "/health" {
            return Ok(self.health());
        }
        let _one_at_a_time = lock(&self.work);
        match path {
            "/apps" => Ok(self.apps()),
            "/which" => self.which(body),
            "/describe" => self.describe(body),
            "/open" => self.open(body),
            "/snapshot" => self.snapshot(body),
            "/press" => self.press(body),
            "/set" => self.set(body),
            "/keys" => self.keys(body),
            "/menu" => self.menu(body),
            "/release" => Ok(self.release()),
            "/screen/start" => self.screen_start(body),
            "/screen/shot" => self.screen_shot(body),
            "/screen/act" => self.screen_act(body),
            "/screen/stop" => Ok(self.screen_stop(body)),
            "/record/start" => self.record_start(body),
            "/record/stop" => {
                let stopped = self.recorder.stop("the assistant stopped it");
                self.show_bar();
                Ok(stopped)
            }
            "/record/status" => Ok(self.recorder.status()),
            "/lock" => self.lock_screen(),
            other => Err(Fail::missing(format!("no {other}"))),
        }
    }
}

/// `value`, an object made with `json!`, as a reply.
fn object(value: Value) -> Reply {
    match value {
        Value::Object(map) => map,
        _ => Reply::new(),
    }
}

#[cfg(test)]
mod tests;
