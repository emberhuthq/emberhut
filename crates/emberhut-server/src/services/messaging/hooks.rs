//! What messaging needs from other areas, behind traits so it can be wired to them: the tasks
//! (where the owner is asked, and answers, and where what was sent is logged) and the other
//! ways the owner can OK a message (the guard rails' standing OKs, an OK in the chat, a tap on
//! a card). The server's tasks are [`ServerTasks`]; [`NoTasks`] stands in where there are none
//! (with no tasks nothing can be asked, so nothing is ever sent as the owner). Until the guard
//! rails are wired in, [`CardOnly`] stands in: the owner's Send it on the task's card is the one
//! OK there is. [`WhatsAppAnswers`] hands the tasks' answers on the assistant's own WhatsApp
//! number's tasks back to it.

use std::fmt;

use emberhut_core::RoomId;
pub use emberhut_core::task::AskId;
use futures_util::future::BoxFuture;
use jiff::Timestamp;

/// A task's id.
pub type TaskId = i64;

/// Why another area didn't do what was asked.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum HookError {
    /// That area isn't wired into this server.
    #[error("{0} isn't set up on this server")]
    NotWired(&'static str),
    /// It failed; its words.
    #[error("{0}")]
    Failed(String),
}

/// A question put to the owner on a task, with the answers to tap.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ask {
    /// The question.
    pub question: String,
    /// The answers offered; none for a free reply.
    pub options: Vec<String>,
    /// The card's title.
    pub title: String,
    /// The lines that go with it (to, from, the message).
    pub brief: Vec<String>,
    /// It approves something to go out (a send card).
    pub approve: bool,
}

/// An answer the owner chose on a task.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chosen {
    /// When the owner chose it.
    pub at: Timestamp,
    /// The answer chosen (`Send it`, `Cancel`, `Allow`...).
    pub choice: String,
}

/// The owner's answers to one ask on a task, and the task's latest ask.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Answers {
    /// The answers chosen to that ask, oldest first.
    pub chosen: Vec<Chosen>,
    /// The task's latest ask: when it is another, that ask was replaced.
    pub latest: Option<AskId>,
}

/// The tasks, as messaging uses them.
pub trait Tasks: fmt::Debug + Send + Sync {
    /// Makes a task for the owner titled `title`, in `room`'s project.
    fn create<'a>(
        &'a self,
        title: &'a str,
        room: RoomId,
    ) -> BoxFuture<'a, Result<TaskId, HookError>>;

    /// Whether `task` is still open; none when there is no such task.
    fn is_open(&self, task: TaskId) -> BoxFuture<'_, Result<Option<bool>, HookError>>;

    /// Asks the owner `ask` on `task` (and pushes it to the phones); the ask's id.
    fn ask(&self, task: TaskId, ask: Ask) -> BoxFuture<'_, Result<AskId, HookError>>;

    /// Notes `text` on `task`, as the assistant.
    fn note(&self, task: TaskId, text: String) -> BoxFuture<'_, Result<(), HookError>>;

    /// The answers the owner chose on `task` to the ask `ask`, and the task's latest ask.
    fn answers(&self, task: TaskId, ask: AskId) -> BoxFuture<'_, Result<Answers, HookError>>;

    /// Marks `task` done.
    fn done(&self, task: TaskId) -> BoxFuture<'_, Result<(), HookError>>;

    /// The room of the project called `name`, made (with `instructions` as its standing
    /// instructions) when there is none and `make` is set; none when there is no such project
    /// and it wasn't made.
    fn project_room<'a>(
        &'a self,
        name: &'a str,
        instructions: &'a str,
        make: bool,
    ) -> BoxFuture<'a, Result<Option<RoomId>, HookError>>;
}

/// No tasks on this server: nothing can be asked or noted, and no project has a room of its
/// own (the main room stands in for the one messaging would make).
#[derive(Clone, Copy, Debug, Default)]
pub struct NoTasks;

impl Tasks for NoTasks {
    fn create<'a>(&'a self, _: &'a str, _: RoomId) -> BoxFuture<'a, Result<TaskId, HookError>> {
        Box::pin(async { Err(HookError::NotWired("tasks")) })
    }

    fn is_open(&self, _: TaskId) -> BoxFuture<'_, Result<Option<bool>, HookError>> {
        Box::pin(async { Ok(None) })
    }

    fn ask(&self, _: TaskId, _: Ask) -> BoxFuture<'_, Result<AskId, HookError>> {
        Box::pin(async { Err(HookError::NotWired("tasks")) })
    }

    fn note(&self, _: TaskId, _: String) -> BoxFuture<'_, Result<(), HookError>> {
        Box::pin(async { Err(HookError::NotWired("tasks")) })
    }

    fn answers(&self, _: TaskId, _: AskId) -> BoxFuture<'_, Result<Answers, HookError>> {
        Box::pin(async { Ok(Answers::default()) })
    }

    fn done(&self, _: TaskId) -> BoxFuture<'_, Result<(), HookError>> {
        Box::pin(async { Err(HookError::NotWired("tasks")) })
    }

    fn project_room<'a>(
        &'a self,
        _: &'a str,
        _: &'a str,
        make: bool,
    ) -> BoxFuture<'a, Result<Option<RoomId>, HookError>> {
        Box::pin(async move { Ok(make.then_some(RoomId::MAIN)) })
    }
}

/// An OK the owner gave in the chat: their go as their next message after they were shown the
/// exact text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChatOk {
    /// Its id, to spend it.
    pub id: String,
    /// The line the task's log gets for it.
    pub line: String,
}

/// An OK of an exact text given elsewhere (on another card): how it was given.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextOk {
    /// How the owner OK'd it, for the log.
    pub how: String,
}

/// The ways besides the task's card that the owner OKs a message (the guard rails' area).
pub trait SendOks: fmt::Debug + Send + Sync {
    /// Whether the owner tapped the card's send answer for the card asked at `asked`.
    fn tapped(&self, task: TaskId, asked: Timestamp) -> BoxFuture<'_, bool>;

    /// The owner's go in `room` for exactly `text` to `to`, when they gave one (after the card
    /// asked at `asked`, when there is one). `to_self`: the owner's own chat.
    fn chat_ok<'a>(
        &'a self,
        room: Option<RoomId>,
        text: &'a str,
        to: &'a str,
        to_self: bool,
        asked: Option<Timestamp>,
    ) -> BoxFuture<'a, Option<ChatOk>>;

    /// Spends `ok` on one message; false when it was spent already.
    fn spend<'a>(&'a self, ok: &'a ChatOk) -> BoxFuture<'a, bool>;

    /// The owner's OK of exactly `text` on another card of `task`, if there is one.
    fn text_ok<'a>(&'a self, task: TaskId, text: &'a str) -> BoxFuture<'a, Option<TextOk>>;

    /// The owner's standing OK for these chats (a 1:1 chat on their list), as a line for the
    /// log.
    fn standing_ok<'a>(
        &'a self,
        chats: &'a [String],
        one_to_one: bool,
    ) -> BoxFuture<'a, Option<String>>;

    /// `text` went out (or was tried) on `task` through `via`: its OK is used up.
    fn sent<'a>(&'a self, task: TaskId, text: &'a str, via: &'a str) -> BoxFuture<'a, ()>;

    /// The card `question` on `task` is answered by an OK given another way: it closes.
    fn close_card<'a>(&'a self, task: TaskId, question: &'a str) -> BoxFuture<'a, ()>;
}

/// Only the card's Send it counts: no standing OKs, no OK in the chat, no taps.
#[derive(Clone, Copy, Debug, Default)]
pub struct CardOnly;

impl SendOks for CardOnly {
    fn tapped(&self, _: TaskId, _: Timestamp) -> BoxFuture<'_, bool> {
        Box::pin(async { false })
    }

    fn chat_ok<'a>(
        &'a self,
        _: Option<RoomId>,
        _: &'a str,
        _: &'a str,
        _: bool,
        _: Option<Timestamp>,
    ) -> BoxFuture<'a, Option<ChatOk>> {
        Box::pin(async { None })
    }

    fn spend<'a>(&'a self, _: &'a ChatOk) -> BoxFuture<'a, bool> {
        Box::pin(async { false })
    }

    fn text_ok<'a>(&'a self, _: TaskId, _: &'a str) -> BoxFuture<'a, Option<TextOk>> {
        Box::pin(async { None })
    }

    fn standing_ok<'a>(&'a self, _: &'a [String], _: bool) -> BoxFuture<'a, Option<String>> {
        Box::pin(async { None })
    }

    fn sent<'a>(&'a self, _: TaskId, _: &'a str, _: &'a str) -> BoxFuture<'a, ()> {
        Box::pin(async {})
    }

    fn close_card<'a>(&'a self, _: TaskId, _: &'a str) -> BoxFuture<'a, ()> {
        Box::pin(async {})
    }
}

/// The server's tasks, as messaging uses them: a project's room is the room named after it
/// (made, with its standing instructions as its first event, when messaging needs it), an
/// owner's answer is a `Chose: <answer>` line of the owner's in the task's history that names
/// the ask it answered (the task's latest ask when it was given), and an ask is also pushed to
/// the phones.
#[derive(Clone, Debug)]
pub struct ServerTasks {
    tasks: crate::services::tasks::TaskService,
    rooms: emberhut_db::RoomStore,
    events: emberhut_db::EventStore,
    push: crate::services::push::PushService,
    clock: std::sync::Arc<dyn emberhut_core::Clock>,
}

impl ServerTasks {
    /// The tasks of `tasks`, the rooms of `rooms` (a project's room's instructions go to
    /// `events`), asks pushed through `push`.
    #[must_use]
    pub fn new(
        tasks: crate::services::tasks::TaskService,
        rooms: emberhut_db::RoomStore,
        events: emberhut_db::EventStore,
        push: crate::services::push::PushService,
        clock: std::sync::Arc<dyn emberhut_core::Clock>,
    ) -> Self {
        Self {
            tasks,
            rooms,
            events,
            push,
            clock,
        }
    }

    /// The room named `name` exactly (in any case).
    async fn room_named(&self, name: &str) -> Result<Option<RoomId>, HookError> {
        let key = name.trim().to_lowercase();
        Ok(self
            .rooms
            .list()
            .await
            .map_err(failed)?
            .into_iter()
            .find(|room| room.name.to_lowercase() == key)
            .map(|room| room.id))
    }
}

/// A failure of another area, in its words.
fn failed(error: impl fmt::Display) -> HookError {
    HookError::Failed(error.to_string())
}

/// The id of task `task`.
fn task_id(task: TaskId) -> emberhut_core::task::TaskId {
    emberhut_core::task::TaskId::new(task)
}

/// The answer in a history line of the owner's, `Chose: <answer>`.
fn chose(text: &str) -> Option<&str> {
    text.strip_prefix("Chose: ").map(str::trim)
}

impl Tasks for ServerTasks {
    fn create<'a>(
        &'a self,
        title: &'a str,
        room: RoomId,
    ) -> BoxFuture<'a, Result<TaskId, HookError>> {
        Box::pin(async move {
            let project = if room == RoomId::MAIN {
                None
            } else {
                self.rooms
                    .get(room)
                    .await
                    .map_err(failed)?
                    .map(|room| room.name)
            };
            let new = emberhut_db::NewTask {
                title: title.to_owned(),
                project,
                ..emberhut_db::NewTask::default()
            };
            let made = self
                .tasks
                .create(new, emberhut_core::task::Who::Assistant)
                .await
                .map_err(failed)?;
            Ok(made.id.get())
        })
    }

    fn is_open(&self, task: TaskId) -> BoxFuture<'_, Result<Option<bool>, HookError>> {
        Box::pin(async move {
            let status = self.tasks.status_of(task_id(task)).await.map_err(failed)?;
            Ok(status.map(emberhut_core::task::Status::is_open))
        })
    }

    fn ask(&self, task: TaskId, ask: Ask) -> BoxFuture<'_, Result<AskId, HookError>> {
        Box::pin(async move {
            let Ask {
                question,
                options,
                title,
                brief,
                approve,
            } = ask;
            let input = crate::services::tasks::AskInput {
                question: question.clone(),
                options: options
                    .into_iter()
                    .map(emberhut_core::task::ask::OptionIn::Plain)
                    .collect(),
                action: Some(title.clone()).filter(|t| !t.trim().is_empty()),
                kind: approve.then(|| "approve".to_owned()),
                brief: Some(brief).filter(|b| !b.is_empty()),
                ..crate::services::tasks::AskInput::default()
            };
            let (_, id) = self
                .tasks
                .ask_with_id(task_id(task), input, emberhut_core::task::Who::Assistant)
                .await
                .map_err(failed)?;
            let head = if title.trim().is_empty() {
                "Needs you".to_owned()
            } else {
                title
            };
            let alert = emberhut_core::push::Alert::new(head, &question, None);
            self.push.push(&alert).await;
            Ok(id)
        })
    }

    fn note(&self, task: TaskId, text: String) -> BoxFuture<'_, Result<(), HookError>> {
        Box::pin(async move {
            self.tasks
                .note(
                    task_id(task),
                    emberhut_core::task::Who::Assistant,
                    &text,
                    false,
                )
                .await
                .map_err(failed)
        })
    }

    fn answers(&self, task: TaskId, ask: AskId) -> BoxFuture<'_, Result<Answers, HookError>> {
        Box::pin(async move {
            let id = task_id(task);
            let chosen = self
                .tasks
                .answers_to(id, ask)
                .await
                .map_err(failed)?
                .into_iter()
                .filter(|line| line.who == emberhut_core::task::Who::Person)
                .filter_map(|line| {
                    chose(&line.text).map(|choice| Chosen {
                        at: line.at,
                        choice: choice.to_owned(),
                    })
                })
                .collect();
            let latest = self.tasks.last_ask(id).await.map_err(failed)?;
            Ok(Answers { chosen, latest })
        })
    }

    fn done(&self, task: TaskId) -> BoxFuture<'_, Result<(), HookError>> {
        Box::pin(async move {
            self.tasks
                .update(
                    task_id(task),
                    emberhut_core::task::TaskChange::status(emberhut_core::task::Status::Done),
                    emberhut_core::task::Who::Assistant,
                    None,
                )
                .await
                .map(|_| ())
                .map_err(failed)
        })
    }

    fn project_room<'a>(
        &'a self,
        name: &'a str,
        instructions: &'a str,
        make: bool,
    ) -> BoxFuture<'a, Result<Option<RoomId>, HookError>> {
        Box::pin(async move {
            if let Some(room) = self.room_named(name).await? {
                return Ok(Some(room));
            }
            if !make || name.trim().is_empty() {
                return Ok(None);
            }
            let now = self.clock.now();
            let room = self.rooms.create(name.trim(), now).await.map_err(failed)?;
            if !instructions.trim().is_empty() {
                let event = emberhut_core::event::NewEvent::new(
                    room.id,
                    "instructions",
                    instructions.trim().to_owned(),
                );
                self.events.add(event, now).await.map_err(failed)?;
            }
            Ok(Some(room.id))
        })
    }
}

/// The assistant's own WhatsApp number takes the person's answers on its linking task first
/// (`Link`, `New code`, `Linked it`, a phone number written on the task), as the reference
/// server does.
#[derive(Clone, Debug)]
pub struct WhatsAppAnswers(pub super::whatsapp::WhatsAppService);

impl crate::services::tasks::AnswerTaker for WhatsAppAnswers {
    fn decide<'a>(
        &'a self,
        task: emberhut_core::task::TaskId,
        choice: &'a str,
    ) -> BoxFuture<'a, bool> {
        Box::pin(async move {
            match self.0.decide(task.get(), choice).await {
                Ok(taken) => taken,
                Err(error) => {
                    tracing::warn!(%error, %task, "the WhatsApp number couldn't read its task");
                    false
                }
            }
        })
    }

    fn message<'a>(
        &'a self,
        task: emberhut_core::task::TaskId,
        text: &'a str,
    ) -> BoxFuture<'a, bool> {
        Box::pin(async move { self.0.task_message(task.get(), text).await })
    }
}
