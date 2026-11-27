//! The wake rule, per room: the server notices (events, check-ins coming due, watches'
//! reminders, routines), and the assistant decides. A room's waiting events are batched into
//! one turn of at most ten ([`emberhut_core::wake`]); a room in a turn is left until it ends, so
//! one room's busy turn never holds back another's news. Paused (the `on` setting off),
//! nothing wakes.
//!
//! Check-ins (`wake_me_at`) and watches are kept as settings (`checkins`, `watches`,
//! `watchSeq`), so they survive a restart and import as they are.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use emberhut_api::wake::{Checkin, Watch};
use emberhut_core::event::NewEvent;
use emberhut_core::wake::{PER_WAKE, WakeBudget, event_line, wakes_now};
use emberhut_core::{Clock, RoomId};
use emberhut_db::{DbError, EventStore, RoomStore, SettingKey, SettingStore};
use jiff::{SignedDuration, Timestamp};
use tokio_util::sync::CancellationToken;

use crate::convert::{room_from_api, room_to_api, timestamp_to_api};
use crate::services::activity::ActivityLog;
use crate::services::rooms::RoomsService;
use crate::services::routines::RoutineService;
use crate::services::talk::{TalkError, TalkService};
use crate::services::threads::{FollowedEnd, ThreadService};

/// Whether the assistant is on (not paused); on until switched off.
const ON: SettingKey<bool> = SettingKey::new("on");
/// The check-ins, soonest first.
const CHECKINS: SettingKey<Vec<Checkin>> = SettingKey::new("checkins");
/// The watches, by number.
const WATCHES: SettingKey<Vec<Watch>> = SettingKey::new("watches");
/// The last watch's number.
const WATCH_SEQ: SettingKey<i64> = SettingKey::new("watchSeq");

/// Check-ins kept at most for each room (its soonest), so one room's can't push out another's.
pub const MAX_CHECKINS: usize = 20;

/// How often the server looks for what should wake the assistant.
pub const TICK_EVERY: Duration = Duration::from_secs(30);

/// Whose check-ins and watches a change reaches: every room's (the owner's rooms), or one
/// room's (a configured agent's room, which reaches only what it scheduled itself).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WakeScope {
    /// Every room's.
    Every,
    /// Only what this room scheduled.
    Room(RoomId),
}

impl WakeScope {
    /// Whether it reaches a check-in or watch scheduled by `room` (as stored: none for Main).
    #[must_use]
    pub fn reaches(self, room: Option<emberhut_api::ids::RoomId>) -> bool {
        match self {
            Self::Every => true,
            Self::Room(own) => room.map_or(RoomId::MAIN, room_from_api) == own,
        }
    }
}

/// Why the wake rule could not be applied.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum WakeError {
    /// No watch has this number.
    #[error("no watch {0}")]
    NoWatch(i64),
    /// The database failed.
    #[error("database error")]
    Database(#[from] DbError),
    /// A room's turn could not start.
    #[error("a turn could not start")]
    Talk(#[from] TalkError),
    /// The coding threads could not be looked at.
    #[error("the coding threads could not be looked at")]
    Threads(#[source] crate::services::threads::ThreadError),
}

/// The event a followed thread's end is: in Main, once per look of the thread.
fn thread_event(end: &FollowedEnd) -> NewEvent {
    let thread = &end.thread;
    let why = &end.followed.why;
    let text = match end.ending {
        emberhut_core::thread::Ending::Ended => {
            let last = thread
                .last
                .as_deref()
                .filter(|last| !last.is_empty())
                .map_or_else(String::new, |last| {
                    format!(
                        " Its last answer: {}",
                        emberhut_core::message::preview(last, 400)
                    )
                });
            format!(
                "The thread \"{}\" on {} you followed ({why}) is {}.{last}",
                thread.title, thread.machine, thread.status
            )
        }
        emberhut_core::thread::Ending::Waiting => {
            let ask = thread
                .last
                .as_deref()
                .filter(|last| !last.is_empty())
                .map_or_else(String::new, |last| {
                    format!(" It asks: {}", emberhut_core::message::preview(last, 400))
                });
            format!(
                "The thread \"{}\" on {} you followed ({why}) waits for an answer.{ask}",
                thread.title, thread.machine
            )
        }
    };
    NewEvent::new(RoomId::MAIN, "thread", text).once(format!(
        "thread:{}:{}:{}",
        thread.machine,
        thread.id,
        thread.updated_at.as_millisecond()
    ))
}

/// A new watch: what to keep an eye on, how often, for how long.
#[derive(Clone, Debug)]
pub struct NewWatch {
    /// A few plain words for the app.
    pub title: String,
    /// The coding thread it watches, if one.
    pub thread: Option<String>,
    /// What to look at and what counts as news.
    pub what: String,
    /// Minutes between reminders.
    pub every_minutes: f64,
    /// Hours it lasts.
    pub hours: f64,
    /// What is seen now.
    pub note: String,
}

/// Keeps the check-ins and watches, and wakes rooms.
#[derive(Clone, Debug)]
pub struct WakeService {
    inner: Arc<Inner>,
}

#[derive(Debug)]
struct Inner {
    settings: SettingStore,
    events: EventStore,
    rooms: RoomStore,
    talk: TalkService,
    threads: ThreadService,
    routines: RoutineService,
    room_service: RoomsService,
    activity: ActivityLog,
    clock: Arc<dyn Clock>,
    budget: Mutex<WakeBudget>,
    /// One change of the check-ins or the watches at a time (each is read, changed, written).
    writing: tokio::sync::Mutex<()>,
}

/// What the wake service is built from.
#[derive(Debug)]
pub struct WakeParts {
    /// The settings, where check-ins, watches and the switch are kept.
    pub settings: SettingStore,
    /// The events.
    pub events: EventStore,
    /// The rooms.
    pub rooms: RoomStore,
    /// The turns.
    pub talk: TalkService,
    /// The coding threads a watch may be on.
    pub threads: ThreadService,
    /// The routines.
    pub routines: RoutineService,
    /// The rooms as the assistant sees them.
    pub room_service: RoomsService,
    /// The activity log.
    pub activity: ActivityLog,
    /// What time it is.
    pub clock: Arc<dyn Clock>,
}

impl WakeService {
    /// A service from its parts.
    #[must_use]
    pub fn new(parts: WakeParts) -> Self {
        let WakeParts {
            settings,
            events,
            rooms,
            talk,
            threads,
            routines,
            room_service,
            activity,
            clock,
        } = parts;
        Self {
            inner: Arc::new(Inner {
                settings,
                events,
                rooms,
                talk,
                threads,
                routines,
                room_service,
                activity,
                clock,
                budget: Mutex::default(),
                writing: tokio::sync::Mutex::new(()),
            }),
        }
    }

    fn now(&self) -> Timestamp {
        self.inner.clock.now()
    }

    // ---- The switch ----

    /// Whether the assistant is on.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn is_on(&self) -> Result<bool, DbError> {
        self.inner.settings.get_or(&ON, true).await
    }

    /// Switches the assistant on or off. Off, what it is doing now stops in every room, and
    /// nothing wakes it until it is on again.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn set_on(&self, on: bool) -> Result<(), DbError> {
        self.inner.settings.set(&ON, &on).await?;
        if !on {
            self.inner.talk.stop_all();
        }
        self.inner
            .activity
            .note(if on {
                "The owner turned the assistant on"
            } else {
                "The owner paused the assistant"
            })
            .await
    }

    /// Events waiting to be read, every room's.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn pending(&self) -> Result<u64, DbError> {
        self.inner.events.count_waiting().await
    }

    /// Tells `room`'s assistant `text` (a program's note, kind `kind`): an event it reads at
    /// its next wake.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn event(&self, room: RoomId, kind: &str, text: &str) -> Result<(), DbError> {
        self.inner
            .events
            .add(NewEvent::new(room, kind, text), self.now())
            .await?;
        Ok(())
    }

    // ---- Check-ins ----

    /// The check-ins, soonest first.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn checkins(&self) -> Result<Vec<Checkin>, DbError> {
        Ok(self
            .inner
            .settings
            .get(&CHECKINS)
            .await?
            .unwrap_or_default())
    }

    /// Schedules a check-in in `room` `minutes` from now (one at least) with `why`; its time.
    /// Only each room's [`MAX_CHECKINS`] soonest are kept.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn wake_me_at(
        &self,
        minutes: u32,
        why: &str,
        room: RoomId,
    ) -> Result<emberhut_api::time::Timestamp, DbError> {
        let _writing = self.inner.writing.lock().await;
        let at = self
            .now()
            .checked_add(SignedDuration::from_mins(i64::from(minutes.max(1))))
            .unwrap_or_else(|_| self.now());
        let at = timestamp_to_api(at);
        let mut checkins = self.checkins().await?;
        checkins.push(Checkin {
            at: at.clone(),
            why: why.to_owned(),
            room: (room != RoomId::MAIN).then(|| room_to_api(room)),
        });
        checkins.sort_by(|a, b| a.at.cmp(&b.at));
        let mut kept = HashMap::new();
        checkins.retain(|checkin| {
            let count = kept.entry(checkin.room).or_insert(0_usize);
            *count += 1;
            *count <= MAX_CHECKINS
        });
        self.inner.settings.set(&CHECKINS, &checkins).await?;
        self.inner
            .activity
            .note(format!("Scheduled a check-in: {why}"))
            .await?;
        Ok(at)
    }

    /// Cancels the check-ins at `at` that `scope` reaches; whether there was one. Another
    /// room's at the same time stay when `scope` is one room.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn cancel_checkin(&self, at: &str, scope: WakeScope) -> Result<bool, DbError> {
        let _writing = self.inner.writing.lock().await;
        let mut checkins = self.checkins().await?;
        let before = checkins.len();
        checkins.retain(|checkin| checkin.at.as_str() != at || !scope.reaches(checkin.room));
        if checkins.len() == before {
            return Ok(false);
        }
        self.inner.settings.set(&CHECKINS, &checkins).await?;
        Ok(true)
    }

    // ---- Watches ----

    /// The watches, by number.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn watches(&self) -> Result<Vec<Watch>, DbError> {
        Ok(self.inner.settings.get(&WATCHES).await?.unwrap_or_default())
    }

    async fn save_watches(&self, mut watches: Vec<Watch>) -> Result<(), DbError> {
        watches.sort_by_key(|watch| watch.id);
        self.inner.settings.set(&WATCHES, &watches).await
    }

    /// Starts a watch in `room`; its number.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn add_watch(&self, new: NewWatch, room: RoomId) -> Result<i64, DbError> {
        let _writing = self.inner.writing.lock().await;
        let id = self.inner.settings.get_or(&WATCH_SEQ, 0).await? + 1;
        self.inner.settings.set(&WATCH_SEQ, &id).await?;
        let now = self.now();
        let after = |minutes: f64| {
            let millis = (minutes * 60_000.0).round();
            #[expect(
                clippy::cast_possible_truncation,
                reason = "the tool bounds the minutes to a week and the hours to 168"
            )]
            let millis = millis as i64;
            timestamp_to_api(
                now.checked_add(SignedDuration::from_millis(millis))
                    .unwrap_or(now),
            )
        };
        let mut watches = self.watches().await?;
        watches.push(Watch {
            id,
            title: Some(new.title.clone()),
            thread: new.thread,
            what: new.what,
            every: new.every_minutes,
            until: after(new.hours * 60.0),
            next_at: after(new.every_minutes),
            note_at: (!new.note.is_empty()).then(|| timestamp_to_api(now)),
            note: new.note,
            fails: 0,
            room: (room != RoomId::MAIN).then(|| room_to_api(room)),
            check_at: None,
        });
        self.save_watches(watches).await?;
        self.inner
            .activity
            .note(format!("Started watching {}", new.title))
            .await?;
        Ok(id)
    }

    /// Notes what watch `id` last saw, when `scope` reaches it.
    ///
    /// # Errors
    ///
    /// [`WakeError::NoWatch`] (also for a watch `scope` doesn't reach) and
    /// [`WakeError::Database`].
    pub async fn note_watch(&self, id: i64, note: &str, scope: WakeScope) -> Result<(), WakeError> {
        let _writing = self.inner.writing.lock().await;
        let mut watches = self.watches().await?;
        let watch = watches
            .iter_mut()
            .find(|watch| watch.id == id && scope.reaches(watch.room))
            .ok_or(WakeError::NoWatch(id))?;
        note.clone_into(&mut watch.note);
        watch.note_at = Some(timestamp_to_api(self.now()));
        self.save_watches(watches).await?;
        Ok(())
    }

    /// Stops watch `id` when `scope` reaches it; the watch, if there was one.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn stop_watch(&self, id: i64, scope: WakeScope) -> Result<Option<Watch>, DbError> {
        let _writing = self.inner.writing.lock().await;
        let mut watches = self.watches().await?;
        let stopped = watches
            .iter()
            .position(|watch| watch.id == id && scope.reaches(watch.room))
            .map(|at| watches.remove(at));
        self.save_watches(watches).await?;
        if let Some(watch) = &stopped {
            let name = watch.title.clone().unwrap_or_else(|| format!("#{id}"));
            self.inner
                .activity
                .note(format!("Stopped watching {name}"))
                .await?;
        }
        Ok(stopped)
    }

    // ---- The tick ----

    /// One look, as the server takes every [`TICK_EVERY`]: the assistant's idle sessions close
    /// ([`crate::services::talk::Assistant::close_idle`]), due routines and check-ins become
    /// events, each room not in a turn wakes for its waiting events (by the wake rule), and
    /// each watch due reminds its room. Nothing but the closing while the assistant is paused.
    ///
    /// # Errors
    ///
    /// [`WakeError`].
    pub async fn tick(&self) -> Result<(), WakeError> {
        // Sessions idle too long close whether or not anything wakes (also while paused).
        self.inner.talk.assistant().close_idle();
        if !self.is_on().await? {
            return Ok(());
        }
        self.inner.routines.tick(&self.inner.room_service).await?;
        self.checkins_due().await?;
        for room in self.inner.rooms.list().await? {
            if self.inner.talk.busy(room.id) {
                continue;
            }
            // The owner first: a message of theirs a restart left unanswered.
            match self.inner.talk.redeliver(room.id).await {
                Ok(true) | Err(TalkError::Busy | TalkError::ShuttingDown) => continue,
                Ok(false) => {}
                Err(error) => return Err(error.into()),
            }
            self.wake_room(room.id).await?;
        }
        self.watches_due().await?;
        Ok(())
    }

    /// Keeps ticking every `every` until `stop`; a failed tick is logged and the next one
    /// tries again.
    pub async fn keep_ticking(self, every: Duration, stop: CancellationToken) {
        let mut ticks = tokio::time::interval(every);
        ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                () = stop.cancelled() => return,
                _ = ticks.tick() => {}
            }
            if let Err(error) = self.tick().await {
                tracing::error!(%error, "the wake tick failed");
            }
        }
    }

    /// Looks at the coding threads once ([`ThreadService::poll`]): each followed thread that
    /// came to an end, or waits for an answer, becomes a `thread` event in Main, which wakes it
    /// at once. How many did.
    ///
    /// # Errors
    ///
    /// [`WakeError::Database`]; the app's failures are kept as the machines' errors.
    pub async fn follow_threads(&self) -> Result<usize, WakeError> {
        let ends = self
            .inner
            .threads
            .poll()
            .await
            .map_err(|error| match error {
                crate::services::threads::ThreadError::Database(error) => {
                    WakeError::Database(error)
                }
                other => WakeError::Threads(other),
            })?;
        for end in &ends {
            self.inner.events.add(thread_event(end), self.now()).await?;
        }
        Ok(ends.len())
    }

    /// Follows the coding threads every `every` until `stop` ([`Self::follow_threads`]); a
    /// failed look is logged and the next one tries again.
    pub async fn keep_following(self, every: Duration, stop: CancellationToken) {
        let mut ticks = tokio::time::interval(every);
        ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                () = stop.cancelled() => return,
                _ = ticks.tick() => {}
            }
            if let Err(error) = self.follow_threads().await {
                tracing::warn!(%error, "could not look at the coding threads");
            }
        }
    }

    /// Check-ins that came due become `checkin` events in the room that asked.
    async fn checkins_due(&self) -> Result<(), DbError> {
        let _writing = self.inner.writing.lock().await;
        let now = timestamp_to_api(self.now());
        let (due, later): (Vec<Checkin>, Vec<Checkin>) = self
            .checkins()
            .await?
            .into_iter()
            .partition(|checkin| checkin.at <= now);
        if due.is_empty() {
            return Ok(());
        }
        for checkin in &due {
            let room = checkin
                .room
                .map_or(RoomId::MAIN, |room| RoomId::new(room.get()));
            let event = NewEvent::new(room, "checkin", format!("Your check-in: {}", checkin.why))
                .once(format!("checkin:{room}:{}", checkin.at));
            self.inner.events.add(event, self.now()).await?;
        }
        self.inner.settings.set(&CHECKINS, &later).await
    }

    /// Wakes `room` for its waiting events when the wake rule says so: one turn with the lines
    /// of the oldest [`PER_WAKE`] events, those marked read; the rest wait for the next wake.
    async fn wake_room(&self, room: RoomId) -> Result<(), WakeError> {
        let waiting = self.inner.events.waiting(room, PER_WAKE).await?;
        let (Some(first), Some(last)) = (waiting.first(), waiting.last()) else {
            return Ok(());
        };
        let kinds: Vec<&str> = waiting.iter().map(|event| event.kind.as_str()).collect();
        let now = self.now();
        if !wakes_now(&kinds, first.at, now) {
            return Ok(());
        }
        if !self
            .inner
            .budget
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take(now)
        {
            return Ok(());
        }
        let text = waiting
            .iter()
            .map(|event| event_line(&event.kind, &event.text))
            .collect::<Vec<_>>()
            .join("\n");
        self.inner.events.handle_through(room, last.id).await?;
        match self.inner.talk.post(room, text).await {
            Ok(_) | Err(TalkError::Busy | TalkError::ShuttingDown) => Ok(()),
            Err(error) => Err(error.into()),
        }
    }

    /// Each watch past its time ends (its room hears so); each one due reminds its room to
    /// look, unless it watches a thread that hasn't changed since its last note.
    async fn watches_due(&self) -> Result<(), WakeError> {
        let _writing = self.inner.writing.lock().await;
        let now_at = self.now();
        let now = timestamp_to_api(now_at);
        let threads = match self.inner.threads.recent().await {
            Ok(threads) => threads,
            Err(error) => {
                tracing::warn!(%error, "watches: the coding threads could not be read");
                Vec::new()
            }
        };
        let mut kept = Vec::new();
        for mut watch in self.watches().await? {
            let room = watch
                .room
                .map_or(RoomId::MAIN, |room| RoomId::new(room.get()));
            if watch.until <= now {
                let seen = if watch.note.is_empty() {
                    "nothing"
                } else {
                    watch.note.as_str()
                };
                let text = format!(
                    "Watch #{} ended (time limit): {}. Last seen: {seen}",
                    watch.id, watch.what
                );
                self.inner
                    .events
                    .add(NewEvent::new(room, "watch", text), now_at)
                    .await?;
                continue;
            }
            if watch.next_at > now {
                kept.push(watch);
                continue;
            }
            let millis = (watch.every * 60_000.0).round();
            #[expect(
                clippy::cast_possible_truncation,
                reason = "a watch's minutes are bounded to a day by the tool"
            )]
            let millis = millis as i64;
            watch.next_at = timestamp_to_api(
                now_at
                    .checked_add(SignedDuration::from_millis(millis))
                    .unwrap_or(now_at),
            );
            let unchanged = watch.thread.as_deref().is_some_and(|thread| {
                threads.iter().any(|known| {
                    known.id == thread
                        && watch
                            .note_at
                            .as_ref()
                            .is_some_and(|noted| known.updated_at <= *noted)
                })
            });
            if !unchanged {
                let last = watch
                    .note_at
                    .as_ref()
                    .map_or_else(|| "never".to_owned(), ToString::to_string);
                let seen = if watch.note.is_empty() {
                    "nothing yet"
                } else {
                    watch.note.as_str()
                };
                let name = watch.title.as_deref().unwrap_or(&watch.what);
                let text = format!(
                    "Watch #{} ({name}): look now. What to watch: {}. Last time ({last}): \
                     {seen}. Update it with watch_note; tell the owner only if something \
                     changed that matters.",
                    watch.id, watch.what
                );
                self.inner
                    .events
                    .add(NewEvent::new(room, "check", text), now_at)
                    .await?;
                watch.check_at = Some(now.clone());
            }
            kept.push(watch);
        }
        self.save_watches(kept).await?;
        Ok(())
    }
}
