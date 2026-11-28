//! Saying something in a room, and the assistant's turn that answers it.
//!
//! A message is stored at once and the request answers straight away; the turn runs on its own
//! task, one turn at a time per room, at most [`MAX_WAITING`] per room at once (the one running
//! included). While it runs, the room's live value says what the assistant is doing
//! ([`LiveSink`]); at its end the reply (if any) is stored and the live value goes back to
//! `None`, which the live stream sends as `data: null`. A turn whose assistant fails or panics
//! ends the same way, with a short note in the chat instead of a reply.
//!
//! On shutdown no new turn is taken, turns still queued are dropped (each with a log line), and
//! the running ones get the grace period to finish ([`TalkService::finish`]).
//!
//! What answers is behind the [`Assistant`] trait: the agent runtime
//! ([`crate::services::claude`], a CLI session per room) when one is configured, [`Silent`]
//! (which answers nothing) otherwise.
//!
//! Besides the person's messages, the server's own events become turns ([`TalkService::post`]):
//! they show in the chat as event lines and reach the assistant as `Event: ...`. A turn can be
//! stopped ([`TalkService::interrupt`], the app's stop button) and every turn stopped at once
//! (pausing). Files the assistant sends during a turn ([`TalkService::send_file`]) go out with
//! its reply; a reply that is exactly `NO_REPLY` puts nothing in the chat (only a line in the
//! activity log). A reply to an event turn reaches the phones only outside quiet hours.

use std::collections::HashMap;
use std::error::Error;
use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use emberhut_core::message::{Author, HistoryQuery, Message, NewMessage};
use emberhut_core::push::Alert;
use emberhut_core::upload::FileKind;
use emberhut_core::{Clock, FileId, MessageId, RoomId, Token};
use emberhut_db::{
    AgentStore, DbError, MessageRefStore, MessageStore, RoomStore, SettingKey, SettingStore,
};
use futures_util::future::BoxFuture;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use crate::mcp::tokens::AgentTokens;
use crate::services::activity::ActivityLog;
use crate::services::learning::LearningService;
use crate::services::live::LiveHub;
use crate::services::notes::NoteService;
use crate::services::push::PushService;
use crate::services::routines::RoutineService;
use crate::services::same_task::SameTaskService;
use crate::services::uploads::{Attachment, describe};

/// Turns a room may have at once, the running one included; one more message is refused.
pub const MAX_WAITING: usize = 8;

/// What the assistant is doing at the start of a turn, before it says otherwise.
const READING: &str = "Reading your new messages";

/// The event line stored when the assistant fails; the cause goes to the log only.
const FAILED: &str = "Something went wrong while answering. Try again.";

/// The event line stored when the owner stops a turn.
const STOPPED: &str = "Stopped.";

/// A reply that is only this says the assistant looked and has nothing to say.
pub const NO_REPLY: &str = "NO_REPLY";

/// Characters of a quoted message the assistant is shown with a reply to it.
const QUOTED_LENGTH: usize = 600;

/// One turn: the message to answer, in its room, with the conversation before it.
#[derive(Clone, Debug)]
pub struct Turn {
    /// The room.
    pub room: RoomId,
    /// The message to answer.
    pub said: Message,
    /// The room's latest page of messages, oldest first, `said` last.
    pub history: Vec<Message>,
    /// The files the person attached to `said`, in the uploads folder (pictures to look at;
    /// [`crate::services::uploads::describe`] words them for a prompt).
    pub files: Vec<Attachment>,
    /// Cancelled when the server starts shutting down: a long turn can save what it has and
    /// end, within the shutdown's grace period, instead of being cut off at its end.
    pub stopping: CancellationToken,
    /// What the assistant is sent: the message (with the one it replies to and its files
    /// described), or `Event: ...` for the server's events.
    pub prompt: String,
    /// The owner's own message started this turn (an event otherwise).
    pub from_owner: bool,
    /// Cancelled when the owner stops this turn or pauses the assistant: the assistant stops
    /// what it is doing and ends the turn.
    pub interrupt: CancellationToken,
    /// The room's token for the MCP endpoint (`Authorization: Bearer ...`); none when it could
    /// not be made.
    pub agent_token: Option<Token>,
}

/// Where an assistant says what it is doing while it works: the room's live value.
#[derive(Clone, Debug)]
pub struct LiveSink {
    hub: LiveHub,
    room: RoomId,
}

impl LiveSink {
    /// The sink of `room`'s live value in `hub`.
    #[must_use]
    pub fn new(hub: LiveHub, room: RoomId) -> Self {
        Self { hub, room }
    }

    /// Says what the assistant is doing now, in one plain line.
    pub fn doing(&self, step: impl Into<String>) {
        let step = step.into();
        self.hub.update(self.room, |live| live.doing = Some(step));
    }

    /// Shows the answer as it is written so far.
    pub fn writing(&self, text: impl Into<String>) {
        let text = text.into();
        self.hub.update(self.room, |live| live.text = text);
    }
}

/// Why an assistant could not answer.
#[derive(Debug, thiserror::Error)]
#[error("the assistant could not answer")]
pub struct AssistantError(#[source] Box<dyn Error + Send + Sync>);

impl AssistantError {
    /// The error for an assistant that failed with `cause`.
    pub fn new(cause: impl Error + Send + Sync + 'static) -> Self {
        Self(Box::new(cause))
    }
}

/// What answers the person: given a turn, the reply's text, or `None` for no reply.
pub trait Assistant: fmt::Debug + Send + Sync {
    /// Answers `turn`, saying on `live` what it is doing meanwhile.
    fn respond(
        &self,
        turn: Turn,
        live: LiveSink,
    ) -> BoxFuture<'_, Result<Option<String>, AssistantError>>;

    /// Takes `room`'s next answer as its account running out (the `testLimit` setting: a test
    /// of moving to another account). An assistant without accounts ignores it.
    fn test_limit(&self, room: RoomId) {
        let _ = room;
    }

    /// Closes whatever it keeps running between turns (the agent CLI's sessions), and resolves
    /// once it has all ended: the server is stopping, its turns are over. Nothing by default.
    fn close_all(&self) -> BoxFuture<'_, ()> {
        Box::pin(async {})
    }

    /// Closes what has sat idle too long (the agent CLI's room sessions): the server calls it
    /// at every wake tick. Nothing by default.
    fn close_idle(&self) {}

    /// Whether it has something to think with now, so a turn nobody asked for (a routine, a
    /// check-in, the wake rule's) may start; a person's own message is answered either way.
    /// Yes by default.
    fn can_think(&self) -> BoxFuture<'_, bool> {
        Box::pin(async { true })
    }
}

/// The assistant until the agent runtime lands: it answers nothing.
#[derive(Clone, Copy, Debug, Default)]
pub struct Silent;

impl Assistant for Silent {
    fn respond(
        &self,
        _turn: Turn,
        _live: LiveSink,
    ) -> BoxFuture<'_, Result<Option<String>, AssistantError>> {
        Box::pin(async { Ok(None) })
    }

    /// Nothing: no agent is configured, so nothing wakes it.
    fn can_think(&self) -> BoxFuture<'_, bool> {
        Box::pin(async { false })
    }
}

/// Why a message could not be said, or a room marked seen.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum TalkError {
    /// The message is blank once trimmed, and has no files.
    #[error("a blank message")]
    Blank,
    /// The room has [`MAX_WAITING`] turns already.
    #[error("too many messages are waiting in the room")]
    Busy,
    /// The server is shutting down and takes no new turns.
    #[error("the server is shutting down")]
    ShuttingDown,
    /// The database failed.
    #[error("database error")]
    Database(#[from] DbError),
}

/// How a message the server tells a room is put to its assistant.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Framing {
    /// The server's own word: `Event: ...`.
    Event,
    /// A message from an agent's channel, framed already: never the server's word.
    Channel,
}

/// Stores what the person says, runs the assistant's turns, and marks rooms seen.
#[derive(Clone, Debug)]
pub struct TalkService {
    inner: Arc<Inner>,
}

#[derive(Debug)]
struct Inner {
    messages: MessageStore,
    rooms: RoomStore,
    live: LiveHub,
    clock: Arc<dyn Clock>,
    assistant: Arc<dyn Assistant>,
    stopping: CancellationToken,
    push: PushService,
    tokens: AgentTokens,
    routines: RoutineService,
    activity: ActivityLog,
    refs: MessageRefStore,
    same_task: SameTaskService,
    settings: SettingStore,
    notes: NoteService,
    learning: LearningService,
    agents: AgentStore,
    /// The owner's messages sent again after they went unanswered, never twice.
    redelivered: Mutex<std::collections::HashSet<MessageId>>,
    tracker: TaskTracker,
    /// Each room's turns: their lock (one runs at a time, in order) and how many there are.
    turns: Mutex<HashMap<RoomId, RoomTurns>>,
    /// Turns not yet ended, every room's: zero when the service is idle.
    in_flight: watch::Sender<usize>,
    /// Replies whose push to the phones has not gone out yet.
    pushing: watch::Sender<usize>,
}

#[derive(Debug, Default)]
struct RoomTurns {
    lock: Arc<tokio::sync::Mutex<()>>,
    count: usize,
    /// The running turn's stop.
    running: Option<CancellationToken>,
    /// Files the assistant sent during the turn, for its reply.
    outbox: Vec<Attachment>,
    /// What started the running turn.
    started: Option<TurnStart>,
}

/// What started a turn, as the memory tools ask (ADR 0034): what the assistant keeps from a
/// turn the owner didn't start waits for the owner to confirm it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TurnStart {
    /// The owner's own message (an event otherwise: someone else's message, mail, a voice
    /// note, a scheduled wake).
    pub by_owner: bool,
    /// The message the turn answers.
    pub message: MessageId,
}

/// What the service is built from.
#[derive(Debug)]
pub struct TalkParts {
    /// The messages.
    pub messages: MessageStore,
    /// The rooms.
    pub rooms: RoomStore,
    /// The rooms' live values.
    pub live: LiveHub,
    /// What time it is.
    pub clock: Arc<dyn Clock>,
    /// What answers.
    pub assistant: Arc<dyn Assistant>,
    /// Cancelled when the server starts shutting down.
    pub stopping: CancellationToken,
    /// Where replies are pushed to the phones.
    pub push: PushService,
    /// The rooms' tokens for the MCP endpoint, handed to each turn.
    pub tokens: AgentTokens,
    /// The quiet hours.
    pub routines: RoutineService,
    /// The activity log.
    pub activity: ActivityLog,
    /// The answers' chips.
    pub refs: MessageRefStore,
    /// What the other rooms said about a turn's tasks.
    pub same_task: SameTaskService,
    /// The settings, where each room's last stopped message is kept.
    pub settings: SettingStore,
    /// The owner's quick notes: the new ones ride along with Main's next turn.
    pub notes: NoteService,
    /// The corrections: an owner's message that sounds like one is logged.
    pub learning: LearningService,
    /// The configured agents: no other room's words are put in an agent's room's turns.
    pub agents: AgentStore,
}

impl TalkService {
    /// A service from its parts.
    #[must_use]
    pub fn new(parts: TalkParts) -> Self {
        let TalkParts {
            messages,
            rooms,
            live,
            clock,
            assistant,
            stopping,
            push,
            tokens,
            routines,
            activity,
            refs,
            same_task,
            settings,
            notes,
            learning,
            agents,
        } = parts;
        Self {
            inner: Arc::new(Inner {
                messages,
                rooms,
                live,
                clock,
                assistant,
                stopping,
                push,
                tokens,
                routines,
                activity,
                refs,
                same_task,
                settings,
                notes,
                learning,
                agents,
                redelivered: Mutex::default(),
                tracker: TaskTracker::new(),
                turns: Mutex::default(),
                in_flight: watch::Sender::new(0),
                pushing: watch::Sender::new(0),
            }),
        }
    }

    /// `prompt`, with the owner's new quick notes after it when it goes to Main: they ride
    /// along with whatever reaches Main next.
    async fn with_notes(&self, room: RoomId, mut prompt: String) -> String {
        if room == RoomId::MAIN {
            match self.inner.notes.take_new().await {
                Ok(news) => prompt.push_str(&news),
                Err(error) => tracing::warn!(%error, "the new quick notes weren't read"),
            }
        }
        prompt
    }

    /// Logs what the owner says in `room` as a candidate correction when it sounds like one,
    /// with the assistant's answer before it. Never fails the message.
    async fn notice(&self, room: RoomId, said: &str) {
        let last = match self.inner.messages.last_said(room).await {
            Ok(last) => last
                .filter(|m| m.author == Author::Assistant)
                .map(|m| m.text),
            Err(error) => {
                tracing::warn!(%room, %error, "the last answer wasn't read");
                None
            }
        };
        if let Err(error) = self.inner.learning.notice(said, last.as_deref()).await {
            tracing::warn!(%room, %error, "a correction wasn't logged");
        }
    }

    /// Stores what the person said in `room` and starts the assistant's turn on it. A reply to
    /// a message that doesn't exist is stored as a plain message.
    ///
    /// The room's live value says the assistant is reading as soon as this returns; the turn
    /// itself waits for any turn already running in the room.
    ///
    /// # Errors
    ///
    /// [`TalkError::Blank`] for a blank message, [`TalkError::Busy`] when the room has
    /// [`MAX_WAITING`] turns, [`TalkError::ShuttingDown`] once shutdown has begun, and
    /// [`TalkError::Database`].
    pub async fn talk(
        &self,
        room: RoomId,
        text: String,
        reply_to: Option<MessageId>,
    ) -> Result<Message, TalkError> {
        self.talk_with_files(room, text, Vec::new(), reply_to).await
    }

    /// [`Self::talk`], with `files` (stored already, and checked: see
    /// [`crate::services::uploads::UploadService::known`]) attached in this order. Text may be
    /// blank when files come with it.
    ///
    /// # Errors
    ///
    /// As [`Self::talk`]; blank is a blank text with no files.
    pub async fn talk_with_files(
        &self,
        room: RoomId,
        text: String,
        files: Vec<Attachment>,
        reply_to: Option<MessageId>,
    ) -> Result<Message, TalkError> {
        if text.trim().is_empty() && files.is_empty() {
            return Err(TalkError::Blank);
        }
        if self.inner.stopping.is_cancelled() {
            return Err(TalkError::ShuttingDown);
        }
        let slot = self.reserve(room)?;
        let messages = &self.inner.messages;
        self.notice(room, &text).await;
        let reply_to = match reply_to {
            Some(id) => messages.get(id).await?.map(|quoted| quoted.id),
            None => None,
        };
        let ids: Vec<FileId> = files.iter().map(|file| file.file.id).collect();
        let said = messages
            .append_with_files(
                NewMessage {
                    room,
                    author: Author::User,
                    text,
                    reply_to,
                },
                &ids,
                self.inner.clock.now(),
            )
            .await?;
        if self.inner.live.now(room).is_none() {
            self.sink(room).doing(READING);
        }
        let quoted = match reply_to {
            Some(id) => messages.get(id).await?,
            None => None,
        };
        let same = self.same_task(room, &said).await;
        let prompt = format!(
            "{}{}{}{same}",
            replying_to(quoted.as_ref()),
            said.text,
            describe(&files)
        );
        let turn = self.clone();
        let message = said.clone();
        let start = Start {
            prompt,
            from_owner: true,
            doing: READING.to_owned(),
        };
        self.inner
            .tracker
            .spawn(async move { turn.run(message, files, slot, start).await });
        Ok(said)
    }

    /// The server tells `room`'s assistant something (`text`: an event batch's lines, a
    /// check-in): an event line in the chat, and a turn that reads it as `Event: ...`.
    ///
    /// # Errors
    ///
    /// [`TalkError::Blank`] for a blank text, [`TalkError::Busy`],
    /// [`TalkError::ShuttingDown`] and [`TalkError::Database`].
    pub async fn post(&self, room: RoomId, text: String) -> Result<Message, TalkError> {
        self.post_as(room, text, Framing::Event).await
    }

    /// A message from one of a configured agent's channels, as its bridge passed it on (`text`,
    /// framed already: the server's header line, then the person's words, quoted): a line in
    /// the chat, and a turn that reads it as it is, never as `Event: ...` (the server's word).
    ///
    /// # Errors
    ///
    /// As [`Self::post`].
    pub async fn channel_message(&self, room: RoomId, text: String) -> Result<Message, TalkError> {
        self.post_as(room, text, Framing::Channel).await
    }

    async fn post_as(
        &self,
        room: RoomId,
        text: String,
        framing: Framing,
    ) -> Result<Message, TalkError> {
        if text.trim().is_empty() {
            return Err(TalkError::Blank);
        }
        if self.inner.stopping.is_cancelled() {
            return Err(TalkError::ShuttingDown);
        }
        let slot = self.reserve(room)?;
        let said = self
            .inner
            .messages
            .append(
                NewMessage {
                    room,
                    author: Author::Event,
                    text,
                    reply_to: None,
                },
                self.inner.clock.now(),
            )
            .await?;
        let about = emberhut_core::wake::about(&said.text);
        if self.inner.live.now(room).is_none() {
            self.sink(room).doing(about.clone());
        }
        let same = self.same_task(room, &said).await;
        let prompt = match framing {
            Framing::Event => format!("Event: {}{same}", said.text),
            Framing::Channel => format!("{}{same}", said.text),
        };
        let start = Start {
            prompt,
            from_owner: false,
            doing: about,
        };
        let turn = self.clone();
        let message = said.clone();
        self.inner
            .tracker
            .spawn(async move { turn.run(message, Vec::new(), slot, start).await });
        Ok(said)
    }

    /// What the rooms said about the keys `said` names, for its turn; nothing when that
    /// can't be read (the turn goes on without it).
    async fn same_task(&self, room: RoomId, said: &Message) -> String {
        // A configured agent's room hears nothing of the other rooms, the owner's least of all.
        match self.inner.agents.by_room(room).await {
            Ok(agent) if agent.id.is_main() => {}
            Ok(_) => return String::new(),
            Err(error) => {
                tracing::warn!(%room, %error, "whose room it is wasn't read: no other room's words");
                return String::new();
            }
        }
        match self
            .inner
            .same_task
            .for_turn(room, &said.text, Some(said.id))
            .await
        {
            Ok(block) => block,
            Err(error) => {
                tracing::warn!(%room, %error, "the same-task lines could not be read");
                String::new()
            }
        }
    }

    /// Whether `room` has a turn running or waiting.
    #[must_use]
    pub fn busy(&self, room: RoomId) -> bool {
        self.turns().get(&room).is_some_and(|turns| turns.count > 0)
    }

    /// The owner's stop button in `room`: the turn running there stops now (the ones waiting
    /// still run), and the chat says `Stopped.`.
    ///
    /// # Errors
    ///
    /// [`TalkError::Database`].
    pub async fn interrupt(&self, room: RoomId) -> Result<(), TalkError> {
        let running = self
            .turns()
            .get(&room)
            .and_then(|turns| turns.running.clone());
        if let Some(running) = running {
            running.cancel();
        }
        // What the owner sent so far is settled: it is not sent again as unanswered.
        if let Some(last) = self
            .inner
            .messages
            .page(room, HistoryQuery::default())
            .await?
            .last()
        {
            self.inner
                .settings
                .set(&stopped_through(room), &last.id.get())
                .await?;
        }
        self.inner
            .messages
            .append(
                NewMessage {
                    room,
                    author: Author::Event,
                    text: STOPPED.to_owned(),
                    reply_to: None,
                },
                self.inner.clock.now(),
            )
            .await?;
        self.inner.activity.note("Stopped by the owner").await?;
        Ok(())
    }

    /// Sends `room`'s assistant again the owner's messages that never got an answer (a restart
    /// dropped their turn, or it failed): those after the room's last answer and its last
    /// stop, a minute old, while the room has no turn. Each is sent again once. Whether any
    /// was.
    ///
    /// # Errors
    ///
    /// [`TalkError::Busy`], [`TalkError::ShuttingDown`] and [`TalkError::Database`].
    pub async fn redeliver(&self, room: RoomId) -> Result<bool, TalkError> {
        if self.busy(room) || self.inner.stopping.is_cancelled() {
            return Ok(false);
        }
        let page = self
            .inner
            .messages
            .page(room, HistoryQuery::default())
            .await?;
        let stopped = self
            .inner
            .settings
            .get(&stopped_through(room))
            .await?
            .unwrap_or(0);
        let answered = page
            .iter()
            .rposition(|message| message.author == Author::Assistant)
            .map_or(0, |at| at + 1);
        let now = self.inner.clock.now();
        let lost: Vec<Message> = {
            let mut redelivered = self
                .inner
                .redelivered
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            page[answered..]
                .iter()
                .filter(|message| message.author == Author::User)
                .filter(|message| message.id.get() > stopped)
                .filter(|message| now.duration_since(message.created_at) > REDELIVER_AFTER)
                .filter(|message| redelivered.insert(message.id))
                .cloned()
                .collect()
        };
        let Some(said) = lost.last().cloned() else {
            return Ok(false);
        };
        let mut parts = Vec::with_capacity(lost.len());
        for message in &lost {
            let quoted = match message.reply_to {
                Some(id) => self.inner.messages.get(id).await?,
                None => None,
            };
            parts.push(format!("{}{}", replying_to(quoted.as_ref()), message.text));
        }
        let slot = self.reserve(room)?;
        if self.inner.live.now(room).is_none() {
            self.sink(room).doing(READING);
        }
        let start = Start {
            prompt: parts.join("\n\n"),
            from_owner: true,
            doing: READING.to_owned(),
        };
        let turn = self.clone();
        self.inner
            .tracker
            .spawn(async move { turn.run(said, Vec::new(), slot, start).await });
        Ok(true)
    }

    /// Pausing: the turn running in every room stops now.
    pub fn stop_all(&self) {
        for turns in self.turns().values() {
            if let Some(running) = &turns.running {
                running.cancel();
            }
        }
    }

    /// A file the assistant sends in `room`: it goes out with the reply of the turn running
    /// there (or the next one).
    pub fn send_file(&self, room: RoomId, file: Attachment) {
        self.turns().entry(room).or_default().outbox.push(file);
    }

    /// The assistant itself.
    #[must_use]
    pub fn assistant(&self) -> &dyn Assistant {
        self.inner.assistant.as_ref()
    }

    /// Marks `room`'s messages up to now seen.
    ///
    /// # Errors
    ///
    /// [`TalkError::Database`].
    pub async fn seen(&self, room: RoomId) -> Result<(), TalkError> {
        self.inner
            .rooms
            .mark_seen(room, self.inner.clock.now())
            .await?;
        Ok(())
    }

    /// Resolves once no turn is running or queued, in any room.
    pub async fn idle(&self) {
        let mut in_flight = self.inner.in_flight.subscribe();
        // The sender lives as long as the service, so this can only end with the count at
        // zero; were it ever dropped, nothing would be running either.
        if let Err(closed) = in_flight.wait_for(|count| *count == 0).await {
            tracing::debug!(%closed, "the turn counter is gone: nothing runs");
        }
    }

    /// Resolves once every reply's push to the phones has gone out (or failed).
    ///
    /// A hook for tests, which wait on it before they look at what was pushed; the server
    /// itself waits for pushes through [`Self::finish`]. Hidden from the docs, as it is not
    /// part of the service's API.
    #[doc(hidden)]
    pub async fn pushed(&self) {
        let mut pushing = self.inner.pushing.subscribe();
        if let Err(closed) = pushing.wait_for(|count| *count == 0).await {
            tracing::debug!(%closed, "the push counter is gone: nothing is pushed");
        }
    }

    /// Lets the running turns finish, for up to `grace`, once shutdown has begun (queued
    /// turns have been dropped by then); `false` when some were still running at the end.
    pub async fn finish(&self, grace: Duration) -> bool {
        self.inner.tracker.close();
        let done = tokio::time::timeout(grace, self.inner.tracker.wait())
            .await
            .is_ok();
        if !done {
            tracing::warn!(
                ?grace,
                "turns still running after the grace period; stopping anyway"
            );
        }
        done
    }

    /// Takes one of `room`'s turn places.
    fn reserve(&self, room: RoomId) -> Result<Slot, TalkError> {
        let mut turns = self.turns();
        let entry = turns.entry(room).or_default();
        if entry.count >= MAX_WAITING {
            return Err(TalkError::Busy);
        }
        entry.count += 1;
        self.inner.in_flight.send_modify(|count| *count += 1);
        Ok(Slot {
            service: self.clone(),
            room,
        })
    }

    /// What started `room`'s running turn; none when no turn runs there.
    #[must_use]
    pub fn started(&self, room: RoomId) -> Option<TurnStart> {
        self.turns().get(&room).and_then(|turns| turns.started)
    }

    fn turns(&self) -> MutexGuard<'_, HashMap<RoomId, RoomTurns>> {
        // Counters and locks: a panic elsewhere cannot leave the map half-written.
        self.inner
            .turns
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    fn sink(&self, room: RoomId) -> LiveSink {
        LiveSink {
            hub: self.inner.live.clone(),
            room,
        }
    }

    /// One turn, after the room's turns before it; dropped if shutdown begins first.
    async fn run(self, said: Message, files: Vec<Attachment>, _slot: Slot, start: Start) {
        let room = said.room;
        let lock = Arc::clone(&self.turns().entry(room).or_default().lock);
        let turn = tokio::select! {
            biased;
            () = self.inner.stopping.cancelled() => None,
            turn = lock.lock() => Some(turn),
        };
        let Some(_turn) = turn.filter(|_| !self.inner.stopping.is_cancelled()) else {
            tracing::warn!(
                %room,
                message = %said.id,
                "shutting down: dropped a turn that had not started; the message stays unanswered"
            );
            return;
        };
        // Whatever happens below, the room's live value ends with the turn.
        let _end = EndOfTurn {
            service: self.clone(),
            room,
        };
        self.sink(room).doing(start.doing.clone());
        if let Err(error) = self.answer(said, files, start).await {
            tracing::error!(%room, error = %error_chain(&error), "a turn failed");
        }
    }

    #[expect(
        clippy::too_many_lines,
        reason = "one turn from its start to its reply, each step in order"
    )]
    async fn answer(
        &self,
        said: Message,
        files: Vec<Attachment>,
        start: Start,
    ) -> Result<(), TalkError> {
        let messages = &self.inner.messages;
        let room = said.room;
        let history = messages
            .page(
                room,
                HistoryQuery {
                    before: Some(MessageId::new(said.id.get() + 1)),
                    after: None,
                },
            )
            .await?;
        let interrupt = CancellationToken::new();
        {
            let mut turns = self.turns();
            let turn = turns.entry(room).or_default();
            turn.running = Some(interrupt.clone());
            turn.started = Some(TurnStart {
                by_owner: start.from_owner,
                message: said.id,
            });
        }
        let agent_token = match self.inner.tokens.for_room(room) {
            Ok(token) => Some(token),
            Err(error) => {
                tracing::error!(%room, %error, "no agent token for the turn");
                None
            }
        };
        let prompt = self.with_notes(room, start.prompt).await;
        let turn = Turn {
            room,
            said,
            history,
            files,
            stopping: self.inner.stopping.child_token(),
            prompt,
            from_owner: start.from_owner,
            interrupt,
            agent_token,
        };
        // On a task of its own, so a panicking assistant ends its turn rather than the server's
        // bookkeeping of it.
        let assistant = Arc::clone(&self.inner.assistant);
        let sink = self.sink(room);
        let answered = self
            .inner
            .tracker
            .spawn(async move { assistant.respond(turn, sink).await })
            .await;
        let reply = match answered {
            Ok(Ok(reply)) => reply.unwrap_or_default(),
            Ok(Err(error)) => {
                tracing::error!(%room, error = %error_chain(&error), "the assistant failed");
                return self.note_failure(room).await;
            }
            Err(join) => {
                tracing::error!(%room, error = %join, "the assistant panicked");
                return self.note_failure(room).await;
            }
        };
        let sent = std::mem::take(&mut self.turns().entry(room).or_default().outbox);
        let quiet = reply.trim() == NO_REPLY;
        if quiet && sent.is_empty() {
            self.inner
                .activity
                .note("Checked · nothing for the owner")
                .await?;
            return Ok(());
        }
        let text = if quiet { String::new() } else { reply };
        if text.is_empty() && sent.is_empty() {
            self.inner
                .activity
                .note("Turn ended without a reply")
                .await?;
            return Ok(());
        }
        let ids: Vec<FileId> = sent.iter().map(|file| file.file.id).collect();
        let answer = messages
            .append_with_files(
                NewMessage {
                    room,
                    author: Author::Assistant,
                    text: text.clone(),
                    reply_to: None,
                },
                &ids,
                self.inner.clock.now(),
            )
            .await?;
        crate::services::refs::keep(&self.inner.refs, answer.id, &text).await?;
        // The reply is in the chat: the room is no longer busy, and the phones are told off
        // the turn, so the room's next turn doesn't wait for APNs or FCM. The assistant's own
        // news (an event's turn) stays off the phones in quiet hours.
        self.inner.live.set(room, None);
        let quiet_hours = !start.from_owner && self.inner.routines.is_quiet().await?;
        if !quiet_hours {
            let push = self.inner.push.clone();
            let pushing = Pushing::start(&self.inner.pushing);
            let body = if text.is_empty() {
                sent_files(&sent)
            } else {
                text
            };
            self.inner.tracker.spawn(async move {
                push.reply(room, &body).await;
                drop(pushing);
            });
        }
        Ok(())
    }

    /// A turn whose assistant failed: a note in the chat in place of a reply.
    async fn note_failure(&self, room: RoomId) -> Result<(), TalkError> {
        self.inner
            .messages
            .append(
                NewMessage {
                    room,
                    author: Author::Event,
                    text: FAILED.to_owned(),
                    reply_to: None,
                },
                self.inner.clock.now(),
            )
            .await?;
        Ok(())
    }

    /// Pushes `title` and `detail` to the phones as an alert from `room`, whatever the quiet
    /// hours (the assistant's `tell_owner`); whether any phone could be told.
    pub async fn alert(&self, room: RoomId, title: &str, detail: &str) -> bool {
        let alert = Alert::new(title, detail, Some(room));
        self.inner.push.push(&alert).await;
        self.inner.push.ready().await
    }
}

/// How old an unanswered message is before it is sent again.
const REDELIVER_AFTER: jiff::SignedDuration = jiff::SignedDuration::from_mins(1);

/// Where `room`'s last stopped message is kept.
fn stopped_through(room: RoomId) -> SettingKey<i64> {
    if room == RoomId::MAIN {
        SettingKey::new("stoppedThrough")
    } else {
        SettingKey::named(format!("stoppedThrough:{room}"))
    }
}

/// How a turn starts: what the assistant is sent, who started it, and the room's first status
/// line.
#[derive(Debug)]
struct Start {
    prompt: String,
    from_owner: bool,
    doing: String,
}

/// The line in front of a reply's text that says which message it answers:
/// `(replying to your message, 2026-11-26 10:41 UTC: "...")` and a blank line.
fn replying_to(quoted: Option<&Message>) -> String {
    let Some(quoted) = quoted else {
        return String::new();
    };
    let who = match quoted.author {
        Author::User => "their own message",
        Author::Assistant => "your message",
        Author::Event => "an event line",
    };
    let when = quoted.created_at.strftime("%Y-%m-%d %H:%M UTC");
    let text = emberhut_core::message::preview(&quoted.text, QUOTED_LENGTH);
    let text = if text.is_empty() {
        "(no text)".to_owned()
    } else {
        text
    };
    format!("(replying to {who}, {when}: \u{201c}{text}\u{201d})\n\n")
}

/// A push's words for a reply that is files alone.
fn sent_files(files: &[Attachment]) -> String {
    match files {
        [one] if one.file.kind == FileKind::Image => "Sent a picture".to_owned(),
        [_] => "Sent a file".to_owned(),
        many => format!("Sent {} files", many.len()),
    }
}

/// A turn's place in its room, given back when the turn ends (or is dropped).
#[derive(Debug)]
struct Slot {
    service: TalkService,
    room: RoomId,
}

impl Drop for Slot {
    fn drop(&mut self) {
        if let Some(entry) = self.service.turns().get_mut(&self.room) {
            entry.count = entry.count.saturating_sub(1);
        }
        self.service
            .inner
            .in_flight
            .send_modify(|count| *count = count.saturating_sub(1));
    }
}

/// One reply's push on its way: counted from its start until it is dropped, however it ends.
struct Pushing(watch::Sender<usize>);

impl Pushing {
    fn start(count: &watch::Sender<usize>) -> Self {
        count.send_modify(|count| *count += 1);
        Self(count.clone())
    }
}

impl Drop for Pushing {
    fn drop(&mut self) {
        self.0.send_modify(|count| *count = count.saturating_sub(1));
    }
}

/// Sets the room's live value back to none when a turn ends, however it ends, and forgets
/// its stop.
struct EndOfTurn {
    service: TalkService,
    room: RoomId,
}

impl Drop for EndOfTurn {
    fn drop(&mut self) {
        self.service.inner.live.set(self.room, None);
        if let Some(turns) = self.service.turns().get_mut(&self.room) {
            turns.running = None;
            turns.started = None;
        }
    }
}

/// An error and its sources, joined with ": ", for a log line.
fn error_chain(error: &dyn Error) -> String {
    let mut text = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        text.push_str(": ");
        text.push_str(&cause.to_string());
        source = cause.source();
    }
    text
}
