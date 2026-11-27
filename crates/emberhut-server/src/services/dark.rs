//! The off-the-record room (`emberhut-core`'s [`dark`] rules): a chat with a model that the
//! assistant never sees. It is kept in a database of its own ([`DarkStore`], beside the
//! server's and left out of backups), opened the first time the room is used; there is no
//! room or chat row, no event, push, memory or tool, and nothing of what is said is logged.
//!
//! A reply is written on its own task, so it is kept even when the app goes away mid-answer;
//! its pieces go to whoever is listening. One reply at a time.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use emberhut_api::dark::{DarkEvent, DarkMessage, DarkRoom};
use emberhut_core::Clock;
use emberhut_core::dark::{self, DarkRole};
use emberhut_db::{DARK_FILE, DarkLine, DarkStore, DbError};
use serde_json::{Map, Value, json};
use tokio::sync::{OnceCell, mpsc};
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use crate::convert::timestamp_to_api;
use crate::services::chat_model::{
    ChatLine, ChatModel, ChatModelError, ChatPiece, ChatReply, ChatRequest, ChatRole, NoChatModel,
};

/// The models the room talks with: a cloud one, and a local one tried first if there is one.
#[derive(Clone, Debug)]
pub struct DarkModels {
    /// The cloud model.
    pub cloud: Arc<dyn ChatModel>,
    /// Its name, as the room shows it.
    pub cloud_label: String,
    /// A local model tried first, and its name.
    pub local: Option<(Arc<dyn ChatModel>, String)>,
}

impl Default for DarkModels {
    /// No model.
    fn default() -> Self {
        Self {
            cloud: Arc::new(NoChatModel),
            cloud_label: "no model".to_owned(),
            local: None,
        }
    }
}

/// Why the room did not do what was asked.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum DarkError {
    /// The message is blank.
    #[error("Say something.")]
    Blank,
    /// A reply is still being written.
    #[error("A reply is still being written.")]
    Busy,
    /// The room's database failed.
    #[error("database error")]
    Database(#[from] DbError),
}

/// The off-the-record room.
#[derive(Clone, Debug)]
pub struct DarkService {
    inner: Arc<Inner>,
}

#[derive(Debug)]
struct Inner {
    path: PathBuf,
    store: OnceCell<DarkStore>,
    clock: Arc<dyn Clock>,
    models: DarkModels,
    busy: AtomicBool,
    stopping: CancellationToken,
    tracker: TaskTracker,
}

impl DarkService {
    /// The room kept in `folder`'s [`DARK_FILE`], talking with `models`.
    #[must_use]
    pub fn new(
        folder: &Path,
        clock: Arc<dyn Clock>,
        models: DarkModels,
        stopping: CancellationToken,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                path: folder.join(DARK_FILE),
                store: OnceCell::new(),
                clock,
                models,
                busy: AtomicBool::new(false),
                stopping,
                tracker: TaskTracker::new(),
            }),
        }
    }

    async fn store(&self) -> Result<&DarkStore, DbError> {
        self.inner
            .store
            .get_or_try_init(|| DarkStore::open(&self.inner.path))
            .await
    }

    /// The model, where it runs, whether a reply is being written, and the chat.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn page(&self) -> Result<DarkRoom, DbError> {
        let models = &self.inner.models;
        let (model, place) = match &models.local {
            Some((_, local)) => (
                local.clone(),
                format!(
                    "{local} on your own machine ({} when it's offline)",
                    models.cloud_label
                ),
            ),
            None => (models.cloud_label.clone(), models.cloud_label.clone()),
        };
        Ok(DarkRoom {
            model,
            place: Some(place),
            busy: self.inner.busy.load(Ordering::SeqCst),
            chat: self
                .store()
                .await?
                .lines()
                .await?
                .iter()
                .map(message)
                .collect(),
        })
    }

    /// Clears the chat.
    ///
    /// # Errors
    ///
    /// [`DarkError::Busy`] while a reply is being written, [`DarkError::Database`].
    pub async fn clear(&self) -> Result<(), DarkError> {
        // Claimed for the clear, as a reply claims it: no reply starts while the chat is
        // being cleared.
        if self
            .inner
            .busy
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return Err(DarkError::Busy);
        }
        let cleared = match self.store().await {
            Ok(store) => store.clear().await,
            Err(error) => Err(error),
        };
        self.inner.busy.store(false, Ordering::SeqCst);
        cleared?;
        Ok(())
    }

    /// Stores what the person says and starts the reply on its own task: its events come on
    /// the receiver, the last one `done` or `error`.
    ///
    /// # Errors
    ///
    /// [`DarkError::Blank`], [`DarkError::Busy`], [`DarkError::Database`].
    pub async fn say(&self, text: &str) -> Result<mpsc::UnboundedReceiver<DarkEvent>, DarkError> {
        let text = text.trim();
        if text.is_empty() {
            return Err(DarkError::Blank);
        }
        if self
            .inner
            .busy
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return Err(DarkError::Busy);
        }
        let store = match self.store().await {
            Ok(store) => store.clone(),
            Err(error) => {
                self.inner.busy.store(false, Ordering::SeqCst);
                return Err(error.into());
            }
        };
        if let Err(error) = store
            .add(DarkRole::User, text, self.inner.clock.now())
            .await
        {
            self.inner.busy.store(false, Ordering::SeqCst);
            return Err(error.into());
        }
        let (events, receiver) = mpsc::unbounded_channel();
        let service = self.clone();
        self.inner.tracker.spawn(async move {
            let last = match service.reply(&store, &events).await {
                Ok(line) => DarkEvent {
                    done: Some(message(&line)),
                    ..DarkEvent::default()
                },
                Err(error) => DarkEvent {
                    error: Some(error.chars().take(300).collect()),
                    ..DarkEvent::default()
                },
            };
            service.inner.busy.store(false, Ordering::SeqCst);
            // The app may be gone: the reply is kept all the same.
            let _unheard = events.send(last);
        });
        Ok(receiver)
    }

    /// Asks the models and stores the reply.
    async fn reply(
        &self,
        store: &DarkStore,
        events: &mpsc::UnboundedSender<DarkEvent>,
    ) -> Result<DarkLine, String> {
        let history = store.lines().await.map_err(|error| error.to_string())?;
        let lines: Vec<ChatLine> = history
            .iter()
            .map(|line| ChatLine {
                role: match line.role {
                    DarkRole::User => ChatRole::User,
                    DarkRole::Assistant => ChatRole::Assistant,
                },
                content: match line.role {
                    DarkRole::User => line.text.clone(),
                    DarkRole::Assistant => dark::unmarked(&line.text).to_owned(),
                },
            })
            .collect();
        let emit = |event: DarkEvent| {
            let _unheard = events.send(event);
        };
        let reply = self
            .ask(lines, &emit)
            .await
            .map_err(|error| error.to_string())?;
        store
            .add(DarkRole::Assistant, &reply, self.inner.clock.now())
            .await
            .map_err(|error| error.to_string())
    }

    /// The local model first when there is one and it is up; the cloud one when it is off or
    /// fails before it has written a word (the reply then says so at its top). Once it has
    /// written, a failure is just a failure: the person's message stays, to send again.
    async fn ask(
        &self,
        lines: Vec<ChatLine>,
        emit: &(dyn Fn(DarkEvent) + Send + Sync),
    ) -> Result<String, ChatModelError> {
        let pieces = |piece: ChatPiece| {
            emit(match piece {
                ChatPiece::Thinking => DarkEvent {
                    thinking: Some(true),
                    ..DarkEvent::default()
                },
                ChatPiece::Text(text) => DarkEvent {
                    delta: Some(text),
                    ..DarkEvent::default()
                },
            });
        };
        let models = &self.inner.models;
        let stop = &self.inner.stopping;
        let Some((local, _)) = &models.local else {
            return Ok(models
                .cloud
                .chat(request(lines, Map::new()), &pieces, stop)
                .await?
                .text);
        };
        let up = local.is_up().await;
        if up {
            let lengths: Vec<usize> = lines.iter().map(|l| l.content.chars().count()).collect();
            let fitted = lines[dark::fit(&lengths)..].to_vec();
            let wrote = AtomicBool::new(false);
            let local_pieces = |piece: ChatPiece| {
                if matches!(piece, ChatPiece::Text(_)) {
                    wrote.store(true, Ordering::SeqCst);
                }
                pieces(piece);
            };
            match local
                .chat(request(fitted, local_sampling()), &local_pieces, stop)
                .await
            {
                Ok(ChatReply { text, .. }) => return Ok(text),
                Err(error) if wrote.load(Ordering::SeqCst) => return Err(error),
                Err(error) => {
                    // Only its kind and status: the error's text may quote the chat.
                    tracing::info!(
                        kind = error.kind(),
                        status = error.status(),
                        "the local model didn't answer; the cloud one does"
                    );
                }
            }
        }
        let mark = if up {
            dark::FALLBACK_ERROR
        } else {
            dark::FALLBACK
        };
        pieces(ChatPiece::Text(mark.to_owned()));
        let reply = models
            .cloud
            .chat(request(lines, Map::new()), &pieces, stop)
            .await?;
        Ok(format!("{mark}{}", reply.text))
    }
}

/// The request for `lines`, with the room's system prompt.
fn request(lines: Vec<ChatLine>, sampling: Map<String, Value>) -> ChatRequest {
    let max_tokens = sampling
        .get("max_tokens")
        .and_then(Value::as_u64)
        .and_then(|n| u32::try_from(n).ok());
    let mut sampling = sampling;
    sampling.remove("max_tokens");
    ChatRequest {
        model: String::new(),
        system: dark::SYSTEM.to_owned(),
        lines,
        max_tokens,
        sampling,
    }
}

/// A small local model's sampling (low temperature, top-k 50, a light repetition penalty),
/// with room for its reasoning before it answers.
fn local_sampling() -> Map<String, Value> {
    let Value::Object(sampling) = json!({
        "temperature": 0.1, "top_k": 50, "repeat_penalty": 1.05, "max_tokens": 8192,
    }) else {
        return Map::new();
    };
    sampling
}

fn message(line: &DarkLine) -> DarkMessage {
    DarkMessage {
        id: emberhut_api::ids::DarkMessageId::new(line.id),
        role: line.role.as_str().into(),
        text: line.text.clone(),
        at: timestamp_to_api(line.at),
    }
}
