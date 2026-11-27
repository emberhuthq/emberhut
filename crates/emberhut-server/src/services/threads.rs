//! Coding threads: the latest look at every coding agent's thread the server knows about, and
//! the threads the assistant handed work to and follows.
//!
//! Whatever learns of a thread's new look (a runner's events, a poll of the coding-thread app)
//! hands it to [`ThreadService::record`]. When the thread is followed and has just come to an
//! end, `record` answers with a [`FollowedEnd`]: the caller tells the assistant at once, and
//! the thread is no longer followed (one that only waits for an answer stays followed). The
//! state shows the newest threads, every followed one with its latest look, and the
//! coding-thread app's threads still running.
//!
//! The coding-thread app ([`CodingThreadApp`]) is polled every [`POLL_EVERY`]: each machine's
//! threads are listed, the ones that changed are read (what they were asked and their latest
//! answer, a few per look), and a machine that can't be reached keeps its last list and its
//! error ([`ThreadService::machine_errors`]).
//!
//! The assistant's tools `threads`, `thread_send` and `follow` are [`ThreadService::list`],
//! [`ThreadService::send`] and [`ThreadService::follow`].

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use emberhut_api as api;
use emberhut_core::thread::{
    ACTIVITY_ITEMS, ACTIVITY_TEXT, CodingThread, DIGEST_ANSWER, DIGEST_ASK, Digest, Ending,
    Followed, ThreadStatus, WHY_LENGTH, resolve_thread_id, shown_activity, tail_start,
};
use emberhut_core::{Clock, RoomId};
use emberhut_db::{DbError, ThreadStore};
use jiff::Timestamp;
use tokio_util::sync::CancellationToken;

use crate::convert::{room_to_api, timestamp_to_api};
use crate::services::coding_app::{CodingAppError, CodingThreadApp, ReadAsk, Sent};

/// How many threads the state lists.
pub const STATE_THREADS: u64 = 40;

/// How many of the coding-thread app's running threads the state lists.
pub const STATE_RUNNING: u64 = 30;

/// How often the coding-thread app is polled.
pub const POLL_EVERY: Duration = Duration::from_secs(30);

/// The most threads one poll reads (a backlog fills in over a few polls).
pub const READS_PER_POLL: usize = 8;

/// The latest items a digest reads.
const DIGEST_TAIL: usize = 15;

/// The longest error kept for a machine, in characters.
const ERROR_LENGTH: usize = 160;

/// Why a thread operation failed.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ThreadError {
    /// The database failed.
    #[error("database error")]
    Database(#[from] DbError),
    /// The coding-thread app has no such machine, could not be reached, or refused.
    #[error(transparent)]
    App(#[from] CodingAppError),
}

/// A followed thread that has just come to an end: news for the assistant.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FollowedEnd {
    /// Its follow: why it was followed, and since when.
    pub followed: Followed,
    /// Its new look.
    pub thread: CodingThread,
    /// Whether it ended (and is no longer followed) or waits for an answer.
    pub ending: Ending,
}

/// The `threads` tool's answer, as far as threads go: every known thread, and the machines
/// whose coding-thread app could not be reached at the last poll, with why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThreadList {
    /// The threads, most recently changed first.
    pub threads: Vec<api::threads::CodingThread>,
    /// Each machine that could not be reached, and its error.
    pub unreachable: BTreeMap<String, String>,
}

/// What `thread_send` did: where the message went.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SentTo {
    /// The thread's full id, as the app knows it (the id given may have lacked its prefix).
    pub thread: String,
    /// What the app said.
    pub sent: Sent,
}

/// Records coding threads and their follows, and shows them.
#[derive(Clone, Debug)]
pub struct ThreadService {
    store: ThreadStore,
    clock: Arc<dyn Clock>,
    app: Arc<dyn CodingThreadApp>,
    /// Each coding-thread app machine's error at the last poll; `None` when it answered.
    errors: Arc<Mutex<BTreeMap<String, Option<String>>>>,
    /// When each machine's app last answered a poll.
    polled: Arc<Mutex<BTreeMap<String, Timestamp>>>,
}

impl ThreadService {
    /// A service over `store`, dating follows with `clock`, following the coding-thread `app`.
    #[must_use]
    pub fn new(store: ThreadStore, clock: Arc<dyn Clock>, app: Arc<dyn CodingThreadApp>) -> Self {
        Self {
            store,
            clock,
            app,
            errors: Arc::default(),
            polled: Arc::default(),
        }
    }

    /// Stores `thread`'s new look; when it is followed and has just come to an end, what the
    /// assistant must hear of it.
    ///
    /// # Errors
    ///
    /// [`ThreadError::Database`].
    pub async fn record(&self, thread: &CodingThread) -> Result<Option<FollowedEnd>, ThreadError> {
        let recorded = self.store.record(thread).await?;
        Ok(recorded
            .followed
            .zip(recorded.ending)
            .map(|(followed, ending)| FollowedEnd {
                followed,
                thread: thread.clone(),
                ending,
            }))
    }

    /// Follows the thread `id` on `machine`, for `why` (cut to [`WHY_LENGTH`] characters), from
    /// now on. Following it again replaces the reason.
    ///
    /// # Errors
    ///
    /// [`ThreadError::Database`].
    pub async fn follow(
        &self,
        machine: &str,
        id: &str,
        why: &str,
    ) -> Result<Followed, ThreadError> {
        let follow = Followed {
            thread: id.to_owned(),
            machine: machine.to_owned(),
            why: why.trim().chars().take(WHY_LENGTH).collect(),
            at: to_millisecond(self.clock.now()),
        };
        self.store.follow(&follow).await?;
        Ok(follow)
    }

    /// Stops following the thread `id`; `false` when it wasn't followed.
    ///
    /// # Errors
    ///
    /// [`ThreadError::Database`].
    pub async fn unfollow(&self, id: &str) -> Result<bool, ThreadError> {
        Ok(self.store.unfollow(id).await?)
    }

    /// The state's `threads`: the newest top-level threads.
    ///
    /// # Errors
    ///
    /// [`ThreadError::Database`].
    pub async fn recent(&self) -> Result<Vec<api::threads::CodingThread>, ThreadError> {
        Ok(self
            .store
            .recent(STATE_THREADS)
            .await?
            .iter()
            .map(thread_to_api)
            .collect())
    }

    /// The state's `followed`: every followed thread, with its latest look.
    ///
    /// # Errors
    ///
    /// [`ThreadError::Database`].
    pub async fn followed(&self) -> Result<Vec<api::threads::FollowedThread>, ThreadError> {
        Ok(self
            .store
            .followed()
            .await?
            .into_iter()
            .map(|(follow, look)| api::threads::FollowedThread {
                id: follow.thread,
                machine: follow.machine.into(),
                why: follow.why,
                at: timestamp_to_api(follow.at),
                thread: look.as_ref().map(thread_to_api),
            })
            .collect())
    }

    /// The coding-thread app's threads still running, queued or waiting, for `room`'s state:
    /// the newest [`STATE_RUNNING`]; Main lists every room's.
    ///
    /// # Errors
    ///
    /// [`ThreadError::Database`].
    pub async fn running(
        &self,
        room: RoomId,
    ) -> Result<Vec<api::threads::ExternalThread>, ThreadError> {
        let machines = self.app.machines();
        Ok(self
            .store
            .active_on(&machines, STATE_RUNNING)
            .await?
            .into_iter()
            .filter(|thread| room == RoomId::MAIN || room_of(thread) == room)
            .map(|thread| api::threads::ExternalThread {
                room: room_to_api(room_of(&thread)),
                machine: thread.machine.into(),
                id: thread.id,
                title: thread.title,
                project: thread.project,
                status: thread.status.as_str().to_owned(),
                updated_at: timestamp_to_api(thread.updated_at),
                ask: thread.ask,
                last: thread.last,
                status_line: thread.status_line,
                // Task links are not kept yet: no thread is linked to a task.
                tasks: Vec::new(),
            })
            .collect())
    }

    /// The latest steps of the coding-thread app's thread `thread` on `machine`: its newest
    /// [`ACTIVITY_ITEMS`] items, checkpoints and blank ones left out, read from the app now.
    ///
    /// # Errors
    ///
    /// [`ThreadError::App`] when no coding-thread app is configured on `machine`
    /// ([`CodingAppError::UnknownMachine`]), or it could not be read.
    pub async fn activity(
        &self,
        machine: &str,
        thread: &str,
    ) -> Result<api::threads::ThreadActivity, ThreadError> {
        self.known_machine(machine)?;
        let head = self
            .app
            .read(machine, thread, read(1, None, ACTIVITY_TEXT))
            .await?;
        let mut ask = read(
            ACTIVITY_ITEMS,
            Some(tail_start(head.item_count, ACTIVITY_ITEMS)),
            ACTIVITY_TEXT,
        );
        ask.activity = true;
        let tail = self.app.read(machine, thread, ask).await?;
        Ok(api::threads::ThreadActivity {
            title: head.title.unwrap_or_default(),
            status: head.status.unwrap_or_default(),
            updated_at: head.updated_at,
            items: shown_activity(tail.items)
                .into_iter()
                .map(|item| api::threads::ActivityStep {
                    kind: item.kind,
                    title: item.title,
                    text: item.text,
                    at: item.at,
                })
                .collect(),
        })
    }

    /// The `threads` tool: every known thread, helper agents' too when `helpers`, only those in
    /// `status` when one is given; and the machines that could not be reached.
    ///
    /// # Errors
    ///
    /// [`ThreadError::Database`].
    pub async fn list(
        &self,
        status: Option<&str>,
        helpers: bool,
    ) -> Result<ThreadList, ThreadError> {
        let status = status
            .filter(|word| !word.is_empty())
            .map(ThreadStatus::from);
        let threads = self
            .store
            .all(helpers, status.as_ref())
            .await?
            .iter()
            .map(thread_to_api)
            .collect();
        let unreachable = self
            .machine_errors()
            .into_iter()
            .filter_map(|(machine, error)| Some((machine, error?)))
            .collect();
        Ok(ThreadList {
            threads,
            unreachable,
        })
    }

    /// The `thread_send` tool's sending: `message` to the coding-thread app's thread `thread`
    /// on `machine`, as a turn of its own (queued behind one running). An id given without the
    /// prefix the app puts in front of it is completed when one known thread there matches.
    ///
    /// # Errors
    ///
    /// [`ThreadError::App`] when no coding-thread app is configured on `machine`, or it could
    /// not be reached or refused (an unknown thread); [`ThreadError::Database`].
    pub async fn send(
        &self,
        machine: &str,
        thread: &str,
        message: &str,
    ) -> Result<SentTo, ThreadError> {
        self.known_machine(machine)?;
        let known = self.store.on_machine(machine).await?;
        let thread = resolve_thread_id(&known, machine, thread).to_owned();
        let sent = self.app.send(machine, &thread, message).await?;
        Ok(SentTo { thread, sent })
    }

    /// Each coding-thread app machine's error at the last poll, `None` for one that answered
    /// (a machine not polled yet is not listed).
    #[must_use]
    pub fn machine_errors(&self) -> BTreeMap<String, Option<String>> {
        self.errors
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// When each machine's app last answered a poll (a machine never polled successfully is
    /// not listed).
    #[must_use]
    pub fn polled_at(&self) -> BTreeMap<String, Timestamp> {
        self.polled
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Looks at the coding-thread app on every machine once: stores each thread's new look
    /// (reading what the changed ones were asked and answered, [`READS_PER_POLL`] at most),
    /// forgets the threads a machine no longer lists, and keeps each machine's error. A machine
    /// that can't be reached keeps its last list. Returns the followed threads that have just
    /// come to an end.
    ///
    /// # Errors
    ///
    /// [`ThreadError::Database`]; the app's failures are kept as the machines' errors.
    pub async fn poll(&self) -> Result<Vec<FollowedEnd>, ThreadError> {
        let mut ends = Vec::new();
        let mut reads = 0;
        for machine in self.app.machines() {
            let listed = match self.app.list(&machine).await {
                Ok(listed) => listed,
                Err(error) => {
                    let error: String = error.to_string().chars().take(ERROR_LENGTH).collect();
                    tracing::debug!(%machine, %error, "the coding-thread app could not be listed");
                    self.set_error(&machine, Some(error));
                    continue;
                }
            };
            let before: HashMap<String, CodingThread> = self
                .store
                .on_machine(&machine)
                .await?
                .into_iter()
                .map(|thread| (thread.id.clone(), thread))
                .collect();
            let mut keep = Vec::with_capacity(listed.len());
            for item in listed {
                let was = before.get(&item.id);
                let mut thread = CodingThread::new(
                    machine.clone(),
                    item.id,
                    item.title,
                    ThreadStatus::from(item.status.as_str()),
                    item.updated_at
                        .parse::<Timestamp>()
                        .unwrap_or_else(|_| self.clock.now()),
                );
                thread.project = item.project;
                thread.parent = item.parent.filter(|parent| !parent.is_empty());
                if let Some(was) = was {
                    thread.ask.clone_from(&was.ask);
                    thread.last_ask.clone_from(&was.last_ask);
                    thread.last.clone_from(&was.last);
                    thread.status_line.clone_from(&was.status_line);
                }
                let read_before = was.is_some_and(|was| was.ask.is_some());
                let unchanged = was.is_some_and(|was| was.updated_at == thread.updated_at);
                if thread.is_top_level() && !(read_before && unchanged) && reads < READS_PER_POLL {
                    reads += 1;
                    match self.digest(&machine, &thread.id).await {
                        Ok(digest) => {
                            // Read, even with nothing found: an empty text, not unknown.
                            thread.ask = Some(digest.ask.unwrap_or_default());
                            thread.last_ask = Some(digest.last_ask.unwrap_or_default());
                            thread.last = Some(digest.last.unwrap_or_default());
                        }
                        Err(error) => {
                            tracing::debug!(%machine, thread = %thread.id, %error, "could not read a thread");
                        }
                    }
                }
                keep.push(thread.id.clone());
                if let Some(end) = self.record(&thread).await? {
                    ends.push(end);
                }
            }
            self.store.forget_others(&machine, &keep).await?;
            self.set_error(&machine, None);
            self.polled
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .insert(machine.clone(), self.clock.now());
        }
        Ok(ends)
    }

    /// Polls the coding-thread app every `every` until `stop`, when it is on any machine. Each
    /// followed thread's end is logged for now: the assistant's events come with its runtime.
    pub async fn keep_polling(self, every: Duration, stop: CancellationToken) {
        if self.app.machines().is_empty() {
            return;
        }
        let mut ticks = tokio::time::interval(every);
        ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                () = stop.cancelled() => return,
                _ = ticks.tick() => {}
            }
            match self.poll().await {
                Ok(ends) => {
                    for end in ends {
                        tracing::info!(
                            machine = %end.thread.machine,
                            thread = %end.thread.id,
                            status = %end.thread.status,
                            why = %end.followed.why,
                            "a followed thread came to an end"
                        );
                    }
                }
                Err(error) => tracing::warn!(%error, "could not poll the coding-thread app"),
            }
        }
    }

    /// What the thread `id` on `machine` is about, read from the app.
    async fn digest(&self, machine: &str, id: &str) -> Result<Digest, CodingAppError> {
        let head = self
            .app
            .read(machine, id, read(1, None, DIGEST_ASK))
            .await?;
        let tail = if head.item_count > 1 {
            let after = tail_start(head.item_count, DIGEST_TAIL);
            self.app
                .read(machine, id, read(DIGEST_TAIL, Some(after), DIGEST_ANSWER))
                .await?
        } else {
            head.clone()
        };
        Ok(Digest::of(&head.items, &tail.items))
    }

    fn known_machine(&self, machine: &str) -> Result<(), CodingAppError> {
        if self.app.machines().iter().any(|known| known == machine) {
            Ok(())
        } else {
            Err(CodingAppError::UnknownMachine)
        }
    }

    fn set_error(&self, machine: &str, error: Option<String>) {
        self.errors
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(machine.to_owned(), error);
    }
}

/// A read of `limit` items after `after`, each at most `max_chars` long.
fn read(limit: usize, after: Option<u64>, max_chars: usize) -> ReadAsk {
    ReadAsk {
        limit,
        after,
        activity: false,
        max_chars,
    }
}

/// The room a thread belongs to. Rooms don't carry projects yet, so every thread is Main's.
fn room_of(_thread: &CodingThread) -> RoomId {
    RoomId::MAIN
}

/// `at` to the millisecond, as the database keeps it, so a follow reads back as it was made.
fn to_millisecond(at: Timestamp) -> Timestamp {
    Timestamp::from_millisecond(at.as_millisecond()).unwrap_or(at)
}

/// A thread as the contract writes it.
fn thread_to_api(thread: &CodingThread) -> api::threads::CodingThread {
    api::threads::CodingThread {
        machine: thread.machine.clone().into(),
        id: thread.id.clone(),
        title: thread.title.clone(),
        project: thread.project.clone(),
        status: thread.status.as_str().to_owned(),
        updated_at: timestamp_to_api(thread.updated_at),
        parent: thread.parent.clone().filter(|parent| !parent.is_empty()),
        ask: thread.ask.clone(),
        last_ask: thread.last_ask.clone(),
        last: thread.last.clone(),
        status_line: thread.status_line.clone(),
        doing: thread.doing.clone(),
        account: thread.account.clone(),
        cwd: thread.cwd.clone(),
        branch: thread.branch.clone(),
        since: thread.since.map(timestamp_to_api),
        direct: thread.direct,
        model: thread.model.clone(),
        effort: thread.effort.clone(),
        question: thread.question.clone(),
    }
}
