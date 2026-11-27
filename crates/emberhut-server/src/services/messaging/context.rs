//! What every messaging service shares: the clock, the stores it writes beside its own (the
//! settings, the activity log, the events that wake the assistant, the chat), the phones, the
//! tasks and the other ways the owner OKs a message, the secrets, and one lock per thing that must
//! happen one at a time.
//!
//! The owner's messaging is the owner's assistant's alone: an event or a chat line meant for a
//! configured agent's room (a chat bound to a room an agent was later given) goes to Main
//! instead, so nothing of the owner's chats reaches an agent.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use base64::Engine as _;
use emberhut_core::event::NewEvent;
use emberhut_core::message::{Author, NewMessage};
use emberhut_core::push::Alert;
use emberhut_core::{Clock, RandomError, RandomSource, RoomId};
use emberhut_db::{AgentStore, EventStore, LogStore, MessageStore, SettingStore};
use jiff::Timestamp;

use super::hooks::{Ask, AskId, HookError, SendOks, TaskId, Tasks};
use super::secrets::Secrets;
use crate::services::push::PushService;

/// The shared parts.
#[derive(Clone, Debug)]
pub struct Context {
    pub(super) clock: Arc<dyn Clock>,
    random: Arc<dyn RandomSource>,
    pub(super) settings: SettingStore,
    log: LogStore,
    events: EventStore,
    chat: MessageStore,
    agents: AgentStore,
    push: PushService,
    pub(super) tasks: Arc<dyn Tasks>,
    pub(super) oks: Arc<dyn SendOks>,
    pub(super) secrets: Secrets,
    pub(super) card_oks_only: bool,
    locks: Arc<Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>>,
}

/// What a [`Context`] is made of.
#[derive(Clone, Debug)]
pub struct ContextParts {
    /// What time it is.
    pub clock: Arc<dyn Clock>,
    /// Where one-time links get their randomness.
    pub random: Arc<dyn RandomSource>,
    /// The settings.
    pub settings: SettingStore,
    /// The activity log.
    pub log: LogStore,
    /// The events that wake the assistant.
    pub events: EventStore,
    /// The rooms' chat.
    pub chat: MessageStore,
    /// The configured agents, whose rooms the owner's messaging never reaches.
    pub agents: AgentStore,
    /// The phones.
    pub push: PushService,
    /// The tasks.
    pub tasks: Arc<dyn Tasks>,
    /// The other ways the owner OKs a message.
    pub oks: Arc<dyn SendOks>,
    /// The secrets.
    pub secrets: Secrets,
    /// Only the owner's answer on a message's own card (which takes a signed-in device) and
    /// their standing list OK a message sent as them: set while sign-in is only reported, when
    /// a go in the chat or another card's OK may come from a request without a session.
    pub card_oks_only: bool,
}

impl Context {
    /// The context of `parts`.
    #[must_use]
    pub fn new(parts: ContextParts) -> Self {
        Self {
            clock: parts.clock,
            random: parts.random,
            settings: parts.settings,
            log: parts.log,
            events: parts.events,
            chat: parts.chat,
            agents: parts.agents,
            push: parts.push,
            tasks: parts.tasks,
            oks: parts.oks,
            secrets: parts.secrets,
            card_oks_only: parts.card_oks_only,
            locks: Arc::default(),
        }
    }

    /// What time it is.
    pub(super) fn now(&self) -> Timestamp {
        self.clock.now()
    }

    /// A new one-time link's token: `bytes` random bytes, base64url without padding.
    pub(super) fn token(&self, bytes: usize) -> Result<String, RandomError> {
        let mut random = vec![0; bytes];
        self.random.fill(&mut random)?;
        Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(random))
    }

    /// Writes `text` to the activity log; a failure is logged, never thrown.
    pub(super) async fn activity(&self, text: impl Into<String>) {
        let text = text.into();
        tracing::info!(%text, "messaging");
        if let Err(error) = self.log.add(text, self.now()).await {
            tracing::warn!(%error, "an activity line was not kept");
        }
    }

    /// Whether `room` is a configured agent's (not the default agent's); a failed look counts
    /// as an agent's, so nothing of the owner's goes there on a doubt.
    pub(super) async fn is_agents_room(&self, room: RoomId) -> bool {
        if room == RoomId::MAIN {
            return false;
        }
        match self.agents.by_room(room).await {
            Ok(agent) => !agent.id.is_main(),
            Err(error) => {
                tracing::warn!(%error, %room, "a room's agent couldn't be read");
                true
            }
        }
    }

    /// `room`, or Main when it is a configured agent's room: the owner's messaging is the
    /// owner's assistant's alone.
    async fn owners_room(&self, room: RoomId) -> RoomId {
        if self.is_agents_room(room).await {
            tracing::info!(%room, "an agent's room: the owner's messaging goes to Main");
            RoomId::MAIN
        } else {
            room
        }
    }

    /// Tells the assistant in `room` (Main when it is an agent's), as a `kind` event, at most
    /// once under `key`.
    pub(super) async fn event(&self, room: RoomId, kind: &str, text: String, key: Option<String>) {
        let room = self.owners_room(room).await;
        let mut event = NewEvent::new(room, kind, text);
        event.key = key;
        if let Err(error) = self.events.add(event, self.now()).await {
            tracing::warn!(%error, %room, kind, "an event was not kept");
        }
    }

    /// Marks every waiting `kind` event read: none of them wakes the assistant now.
    pub(super) async fn handle_kind(&self, kind: &str) {
        if let Err(error) = self.events.handle_kind(kind).await {
            tracing::warn!(%error, kind, "waiting events weren't dropped");
        }
    }

    /// Shows `text` in `room`'s chat (Main's when it is an agent's), as a note from the server.
    pub(super) async fn line(&self, room: RoomId, text: String) {
        let room = self.owners_room(room).await;
        let line = NewMessage {
            room,
            author: Author::Event,
            text,
            reply_to: None,
        };
        if let Err(error) = self.chat.append(line, self.now()).await {
            tracing::warn!(%error, %room, "a chat line was not kept");
        }
    }

    /// Pushes `title` and `body` to the phones, in the background.
    pub(super) fn push(&self, title: String, body: &str) {
        let push = self.push.clone();
        let alert = Alert::new(title, body, None);
        drop(tokio::spawn(async move {
            push.push(&alert).await;
        }));
    }

    /// Notes `text` on `task`; a failure is logged.
    pub(super) async fn note(&self, task: TaskId, text: String) {
        if let Err(error) = self.tasks.note(task, text).await {
            tracing::warn!(%error, task, "a task note was not kept");
        }
    }

    /// Asks `ask` on `task`; the ask's id.
    pub(super) async fn ask(&self, task: TaskId, ask: Ask) -> Result<AskId, HookError> {
        self.tasks.ask(task, ask).await
    }

    /// The lock for `key`: what holds it runs alone.
    pub(super) fn lock(&self, key: &str) -> Arc<tokio::sync::Mutex<()>> {
        let mut locks = self.locks.lock().unwrap_or_else(PoisonError::into_inner);
        Arc::clone(locks.entry(key.to_owned()).or_default())
    }
}
