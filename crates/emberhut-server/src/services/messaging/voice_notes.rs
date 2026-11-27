//! Voice notes as text: each clip people send the owner (a Slack clip, a WhatsApp voice note,
//! an iMessage audio message) is transcribed once, in the background, and kept; readers wait a
//! little for it, then show its file name with why there is no text yet. The rules (how a note
//! reads, when a failed one is tried again) are `emberhut_core::messaging::voice`'s.
//!
//! [`Transcriber`] is the speech-to-text seam: [`OpenAiTranscriber`] is the real one (an
//! OpenAI-style `/audio/transcriptions`, its key in the secret `openai-api-key`), and
//! [`NoTranscriber`] stands in when none is set up.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use axum::body::Bytes;
use emberhut_core::messaging::OWNER;
use emberhut_core::messaging::text::first_chars;
use emberhut_core::messaging::voice::{
    self, MAX_BYTES, SeenNote, VoiceNote, VoiceSource, VoiceStatus, next_try,
};
use emberhut_db::messaging::Outcome;
use emberhut_db::{DbError, SlackStore, VoiceNoteStore};
use futures_util::future::BoxFuture;
use serde::Serialize;
use serde_json::Value;
use tokio::sync::{Semaphore, watch};

use super::context::Context;
use super::mac::{MacMessages, encode};
use super::slack::SlackApi;
use super::web::{WebClient, WebRequest};

/// The secret that holds the speech-to-text service's key.
pub const OPENAI_KEY_SECRET: &str = "openai-api-key";

/// How many notes are transcribed at once.
const AT_ONCE: usize = 3;

/// How long a fetch of a note's sound may take.
const FETCH_WAIT: Duration = Duration::from_secs(60);

/// A speech-to-text service.
pub trait Transcriber: fmt::Debug + Send + Sync {
    /// What is said in `audio` (a file called `name`, whose extension says its format),
    /// helped by `prompt`.
    fn transcribe<'a>(
        &'a self,
        audio: Bytes,
        name: &'a str,
        prompt: &'a str,
    ) -> BoxFuture<'a, Result<String, String>>;
}

/// No speech-to-text set up: every note fails, and is tried again later.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoTranscriber;

impl Transcriber for NoTranscriber {
    fn transcribe<'a>(
        &'a self,
        _: Bytes,
        _: &'a str,
        _: &'a str,
    ) -> BoxFuture<'a, Result<String, String>> {
        Box::pin(async { Err("no speech-to-text is set up on this server".to_owned()) })
    }
}

/// An OpenAI-style speech-to-text service.
#[derive(Clone)]
pub struct OpenAiTranscriber {
    web: WebClient,
    base: String,
    model: String,
    secrets: super::secrets::Secrets,
}

impl fmt::Debug for OpenAiTranscriber {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OpenAiTranscriber")
            .field("base", &self.base)
            .field("model", &self.model)
            .finish_non_exhaustive()
    }
}

impl OpenAiTranscriber {
    /// The service at `base` (`https://api.openai.com/v1`) with `model`, its key read from the
    /// secrets at every call.
    #[must_use]
    pub fn new(web: WebClient, base: &str, model: &str, secrets: super::secrets::Secrets) -> Self {
        Self {
            web,
            base: base.trim_end_matches('/').to_owned(),
            model: model.to_owned(),
            secrets,
        }
    }
}

impl Transcriber for OpenAiTranscriber {
    fn transcribe<'a>(
        &'a self,
        audio: Bytes,
        name: &'a str,
        prompt: &'a str,
    ) -> BoxFuture<'a, Result<String, String>> {
        Box::pin(async move {
            let key = self.secrets.read(OPENAI_KEY_SECRET).await;
            if key.is_empty() {
                return Err("no OpenAI key on the server yet".to_owned());
            }
            let (content_type, body) = multipart(&audio, name, &self.model, prompt);
            let request = WebRequest::post(
                format!("{}/audio/transcriptions", self.base),
                &content_type,
                body,
                Duration::from_secs(120),
            )
            .bearer(&key);
            let answer = self.web.send(request).await.map_err(|e| e.to_string())?;
            let json = answer.json();
            if !answer.ok() {
                let why = json["error"]["message"].as_str().unwrap_or("no reason");
                return Err(format!("OpenAI said {}: {why}", answer.status.as_u16()));
            }
            Ok(json["text"].as_str().unwrap_or_default().to_owned())
        })
    }
}

/// A `multipart/form-data` body with the file, the model and the prompt: its content type
/// and bytes.
fn multipart(audio: &[u8], name: &str, model: &str, prompt: &str) -> (String, Vec<u8>) {
    const BOUNDARY: &str = "emberhut-voice-note-boundary-7d1f3a";
    let quoted = |s: &str| {
        s.replace('\n', "%0A")
            .replace('\r', "%0D")
            .replace('"', "%22")
    };
    let mut body = Vec::with_capacity(audio.len() + 512);
    body.extend_from_slice(
        format!(
            "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{}\"\r\n\
             Content-Type: application/octet-stream\r\n\r\n",
            quoted(if name.is_empty() { "voice.m4a" } else { name })
        )
        .as_bytes(),
    );
    body.extend_from_slice(audio);
    for (field, value) in [("model", model), ("prompt", prompt)] {
        if value.is_empty() {
            continue;
        }
        body.extend_from_slice(
            format!(
                "\r\n--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"{field}\"\r\n\r\n{}",
                value.replace("\r\n", "\n").replace('\n', "\r\n")
            )
            .as_bytes(),
        );
    }
    body.extend_from_slice(format!("\r\n--{BOUNDARY}--\r\n").as_bytes());
    (format!("multipart/form-data; boundary={BOUNDARY}"), body)
}

/// A note's sound, fetched.
struct Audio {
    data: Bytes,
    name: String,
    seconds: Option<f64>,
    long: bool,
}

/// Why a fetch failed: its words, and whether trying again could help.
struct FetchError {
    error: String,
    last: bool,
}

impl FetchError {
    fn again(error: impl Into<String>) -> Self {
        Self {
            error: error.into(),
            last: false,
        }
    }
}

/// A WhatsApp or iMessage note found by what was said.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceHit {
    /// The chat it was sent in.
    pub chat: Option<String>,
    /// That chat's name.
    pub chat_name: Option<String>,
    /// `WhatsApp` or `iMessage`.
    pub source: &'static str,
    /// When its message was sent.
    pub at: Option<String>,
    /// Who sent it.
    pub sender: Option<String>,
    /// Its tag and what was said.
    pub text: String,
}

/// What voice notes need besides the context.
#[derive(Clone, Debug)]
pub struct VoiceParts {
    /// The notes.
    pub store: VoiceNoteStore,
    /// The Slack messages that show Slack's notes.
    pub slack_store: SlackStore,
    /// Slack's Web API, for a Slack clip's file.
    pub slack: Arc<dyn SlackApi>,
    /// The Mac helper, for a WhatsApp or iMessage note's sound.
    pub mac: Arc<dyn MacMessages>,
    /// The speech-to-text service.
    pub transcriber: Arc<dyn Transcriber>,
    /// The longest note transcribed.
    pub max_seconds: f64,
    /// The pause before a note is tried again at once, after a hiccup.
    pub retry_pause: Duration,
}

/// Voice notes, transcribed in the background.
#[derive(Clone, Debug)]
pub struct VoiceNotes {
    ctx: Context,
    parts: VoiceParts,
    inflight: Arc<Mutex<HashMap<String, watch::Receiver<bool>>>>,
    slots: Arc<Semaphore>,
}

impl VoiceNotes {
    /// The service over `parts`.
    #[must_use]
    pub fn new(ctx: Context, parts: VoiceParts) -> Self {
        Self {
            ctx,
            parts,
            inflight: Arc::default(),
            slots: Arc::new(Semaphore::new(AT_ONCE)),
        }
    }

    /// The longest note transcribed.
    #[must_use]
    pub fn max_seconds(&self) -> f64 {
        self.parts.max_seconds
    }

    /// Keeps a note seen in a message, and starts transcribing it the first time.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn seen(&self, seen: &SeenNote) -> Result<VoiceNote, DbError> {
        let note = self.parts.store.seen(seen, self.ctx.now()).await?;
        if note.status == VoiceStatus::New {
            self.start(&note.id);
        }
        Ok(note)
    }

    /// What a reader sees for note `id` now.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn line(&self, id: &str, name: Option<&str>) -> Result<String, DbError> {
        let note = self.parts.store.get(id).await?;
        Ok(voice::line(note.as_ref(), name, self.parts.max_seconds))
    }

    /// What a Slack message's body shows for note `id` now (its file name until there is
    /// more).
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn shown(&self, id: &str, name: &str) -> Result<String, DbError> {
        Ok(self
            .parts
            .store
            .get(id)
            .await?
            .and_then(|note| note.shown)
            .unwrap_or_else(|| format!("[{name}]")))
    }

    /// Waits up to `wait` for the notes being transcribed now: `ids`, or all of them.
    pub async fn settle(&self, wait: Duration, ids: Option<&[String]>) {
        let waiting: Vec<watch::Receiver<bool>> = {
            let inflight = self.inflight.lock().unwrap_or_else(PoisonError::into_inner);
            match ids {
                Some(ids) => ids
                    .iter()
                    .filter_map(|id| inflight.get(id).cloned())
                    .collect(),
                None => inflight.values().cloned().collect(),
            }
        };
        if waiting.is_empty() || wait.is_zero() {
            return;
        }
        let all = async {
            for mut done in waiting {
                // Ends when the note is done, or its task is gone.
                let _settled = done.wait_for(|done| *done).await;
            }
        };
        let _timed_out = tokio::time::timeout(wait, all).await;
    }

    /// Starts the notes due a try: failed ones whose time has come, and any left half-done by
    /// a restart.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn tick(&self) -> Result<(), DbError> {
        let now = self.ctx.now();
        let stale = now - jiff::SignedDuration::from_mins(2);
        for id in self.parts.store.due(now, stale).await? {
            self.start(&id);
        }
        Ok(())
    }

    /// A new Slack token: the clips that couldn't be fetched are tried again.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn retry_slack(&self) -> Result<(), DbError> {
        self.parts.store.retry_slack(self.ctx.now()).await?;
        self.tick().await
    }

    /// The done WhatsApp and iMessage notes by what was said, in the last `days`.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn search(
        &self,
        words: &str,
        days: i64,
        limit: usize,
    ) -> Result<Vec<VoiceHit>, DbError> {
        let since = self.ctx.now() - jiff::SignedDuration::from_hours(24 * days);
        let notes = self.parts.store.search(words, since, limit).await?;
        Ok(notes
            .into_iter()
            .map(|note| VoiceHit {
                chat: note.chat,
                chat_name: note.chat_name,
                source: if note.id.starts_with("wa:") {
                    VoiceSource::WhatsApp.shown()
                } else {
                    VoiceSource::IMessage.shown()
                },
                at: note.msg_at.map(|at| at.to_string()),
                sender: note.sender,
                text: format!(
                    "{} {}",
                    voice::tag(note.seconds),
                    note.text.as_deref().unwrap_or("null")
                ),
            })
            .collect())
    }

    /// The Mac helper's messages, each with a voice note (`audio: { id, name, seconds }`)
    /// given its transcript as its text (waiting up to `wait` for new ones), its `audio` gone;
    /// every other message and field as it came.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn in_mac_messages(
        &self,
        messages: &mut [Value],
        wait: Duration,
    ) -> Result<(), DbError> {
        let mut ids = Vec::new();
        for message in messages.iter() {
            let audio = &message["audio"];
            let Some(id) = audio["id"].as_str().filter(|id| !id.is_empty()) else {
                continue;
            };
            let from_me = truthy(&message["fromMe"]);
            let chat_name = message["chatName"].as_str().map(str::to_owned);
            let sender = if from_me {
                Some(OWNER.to_owned())
            } else {
                message["sender"]
                    .as_str()
                    .map(str::to_owned)
                    .or_else(|| chat_name.clone())
            };
            self.seen(&SeenNote {
                id: id.to_owned(),
                source: VoiceSource::of_mac_id(id),
                reference: id.to_owned(),
                name: audio["name"].as_str().map(str::to_owned),
                seconds: audio["seconds"].as_f64(),
                chat: message["chat"].as_str().map(str::to_owned),
                chat_name,
                sender,
                msg_at: message["at"].as_str().and_then(|at| at.parse().ok()),
                msg: None,
            })
            .await?;
            ids.push(id.to_owned());
        }
        if ids.is_empty() {
            return Ok(());
        }
        self.settle(wait, Some(&ids)).await;
        for message in messages.iter_mut() {
            let Some(id) = message["audio"]["id"].as_str().map(str::to_owned) else {
                continue;
            };
            if id.is_empty() {
                continue;
            }
            let name = message["audio"]["name"].as_str().map(str::to_owned);
            let line = self.line(&id, name.as_deref()).await?;
            if let Value::Object(object) = message {
                object.remove("audio");
                object.insert("text".to_owned(), Value::String(line));
            }
        }
        Ok(())
    }

    /// Starts transcribing note `id` in the background, unless it is already.
    fn start(&self, id: &str) {
        let (done, waiting) = watch::channel(false);
        {
            let mut inflight = self.inflight.lock().unwrap_or_else(PoisonError::into_inner);
            if inflight.contains_key(id) {
                return;
            }
            inflight.insert(id.to_owned(), waiting);
        }
        let notes = self.clone();
        let id = id.to_owned();
        drop(tokio::spawn(async move {
            {
                let _slot = notes.slots.acquire().await;
                if let Err(error) = notes.attempt(&id).await {
                    tracing::warn!(%error, id, "a voice note's try failed");
                }
            }
            notes
                .inflight
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .remove(&id);
            done.send_replace(true);
        }));
    }

    /// One round of tries at note `id`.
    async fn attempt(&self, id: &str) -> Result<(), DbError> {
        let Some(note) = self.parts.store.get(id).await? else {
            return Ok(());
        };
        if !matches!(note.status, VoiceStatus::New | VoiceStatus::Failed) {
            return Ok(());
        }
        let max = self.parts.max_seconds;
        if note.seconds.is_some_and(|s| s != 0.0 && s > max) {
            return self.finish(id, Outcome::Long { seconds: None }).await;
        }
        let mut error = String::new();
        for round in 0..2 {
            match self.try_once(&note).await {
                Ok(outcome) => return self.finish(id, outcome).await,
                Err(failed) => {
                    error = first_chars(&failed.error, 300);
                    if failed.last {
                        break;
                    }
                    if round == 0 {
                        tokio::time::sleep(self.parts.retry_pause).await;
                    }
                }
            }
        }
        let now = self.ctx.now();
        let tries = note.tries + 1;
        let blocked = error.contains("files:read");
        let sent = note.msg_at.unwrap_or(note.at);
        let next_at = next_try(tries, blocked, sent, now).map(|wait| now + wait);
        tracing::info!(
            id,
            error,
            gave_up = next_at.is_none(),
            "a voice note wasn't transcribed"
        );
        self.finish(
            id,
            Outcome::Failed {
                error,
                tries,
                next_at,
            },
        )
        .await
    }

    /// Fetches and transcribes `note` once.
    async fn try_once(&self, note: &VoiceNote) -> Result<Outcome, FetchError> {
        let audio = self.fetch(note).await?;
        let max = self.parts.max_seconds;
        if audio.long || audio.seconds.is_some_and(|s| s > max) || audio.data.len() > MAX_BYTES {
            return Ok(Outcome::Long {
                seconds: audio.seconds,
            });
        }
        if audio.data.is_empty() {
            return Err(FetchError::again("an empty file"));
        }
        let names = [note.sender.as_deref().unwrap_or_default()];
        let prompt = voice::prompt(&names);
        let text = self
            .parts
            .transcriber
            .transcribe(audio.data, &audio.name, &prompt)
            .await
            .map_err(FetchError::again)?;
        Ok(Outcome::Done {
            text: emberhut_core::messaging::text::js_trim(&text).to_owned(),
            seconds: audio.seconds,
        })
    }

    /// A note's sound: a Slack file with the owner's token, or the Mac helper's.
    async fn fetch(&self, note: &VoiceNote) -> Result<Audio, FetchError> {
        if note.source == VoiceSource::Slack {
            let (workspace, url) = note
                .reference
                .split_once('|')
                .unwrap_or((&note.reference, ""));
            let token = self
                .ctx
                .secrets
                .read(&emberhut_core::messaging::slack::user_token_secret(
                    workspace,
                ))
                .await;
            if token.is_empty() {
                return Err(FetchError {
                    error: format!("no Slack token for {workspace}"),
                    last: true,
                });
            }
            let got = self
                .parts
                .slack
                .download(url, &token)
                .await
                .map_err(|error| FetchError::again(error.to_string()))?;
            if got.status == 401 || got.status == 403 || got.content_type.contains("text/html") {
                return Err(FetchError::again(
                    "Slack wouldn't hand over the file (the Slack app needs files:read)",
                ));
            }
            if !(200..300).contains(&got.status) {
                return Err(FetchError::again(format!("Slack answered {}", got.status)));
            }
            let name = note
                .name
                .clone()
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| "clip.m4a".to_owned());
            return Ok(Audio {
                data: got.body,
                name,
                seconds: None,
                long: false,
            });
        }
        let path = format!(
            "/media?id={}&max={}",
            encode(&note.reference),
            self.parts.max_seconds
        );
        let got = self
            .parts
            .mac
            .get(&path, FETCH_WAIT)
            .await
            .map_err(|error| FetchError::again(error.to_string()))?;
        let seconds = got
            .seconds
            .as_deref()
            .and_then(|s| s.trim().parse::<f64>().ok())
            .filter(|s| *s != 0.0 && s.is_finite());
        if got.status == 413 {
            let told = got.json()["seconds"].as_f64().filter(|s| *s != 0.0);
            return Ok(Audio {
                data: Bytes::new(),
                name: String::new(),
                seconds: seconds.or(told),
                long: true,
            });
        }
        if got.status == 404 {
            let why = got.json()["error"]
                .as_str()
                .unwrap_or("not downloaded")
                .to_owned();
            return Err(FetchError::again(format!("not on the Mac ({why})")));
        }
        if !got.ok() {
            return Err(FetchError::again(format!(
                "the Mac answered {}",
                got.status
            )));
        }
        let kind = got.content_type.as_str();
        let name = if kind.contains("ogg") || kind.contains("opus") {
            "voice.ogg"
        } else if kind.contains("mpeg") || kind.contains("mp3") {
            "voice.mp3"
        } else if kind.contains("wav") {
            "voice.wav"
        } else {
            "voice.m4a"
        };
        Ok(Audio {
            data: got.body,
            name: name.to_owned(),
            seconds,
            long: false,
        })
    }

    /// Records how a try ended, and shows the note's new line in its Slack message.
    async fn finish(&self, id: &str, outcome: Outcome) -> Result<(), DbError> {
        self.parts
            .store
            .finish(id, &outcome, self.ctx.now())
            .await?;
        self.show(id).await
    }

    /// The Slack message that shows note `id` gets its new line.
    async fn show(&self, id: &str) -> Result<(), DbError> {
        let Some(note) = self.parts.store.get(id).await? else {
            return Ok(());
        };
        let Some(msg) = note.msg.as_deref().filter(|msg| !msg.is_empty()) else {
            return Ok(());
        };
        let old = note
            .shown
            .clone()
            .unwrap_or_else(|| format!("[{}]", note.name.as_deref().unwrap_or("null")));
        let line = voice::line(Some(&note), None, self.parts.max_seconds);
        if old == line {
            return Ok(());
        }
        // The line is kept first: a Slack message kept again meanwhile (its body read with the
        // old line) then finds the new one and puts it back itself.
        self.parts.store.set_shown(id, &line).await?;
        self.parts
            .slack_store
            .replace_in_body(msg, &old, &line)
            .await?;
        Ok(())
    }
}

/// Whether a JSON value is true as JavaScript reads it.
pub(crate) fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|n| n != 0.0 && !n.is_nan()),
        Value::String(s) => !s.is_empty(),
        Value::Array(_) | Value::Object(_) => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_form_carries_the_file_model_and_prompt() {
        let (kind, body) = multipart(b"RIFF", "a\"b.m4a", "speech-1", "line one\nline two");
        assert!(kind.starts_with("multipart/form-data; boundary="));
        let text = String::from_utf8_lossy(&body);
        assert!(text.contains("filename=\"a%22b.m4a\""), "{text}");
        assert!(text.contains("RIFF"));
        assert!(text.contains("name=\"model\"\r\n\r\nspeech-1"));
        assert!(text.contains("line one\r\nline two"));
        assert!(text.ends_with("--\r\n"));
    }

    #[test]
    fn truth_as_javascript_reads_it() {
        assert!(truthy(&serde_json::json!(1)));
        assert!(!truthy(&serde_json::json!("")));
        assert!(!truthy(&Value::Null));
        assert!(truthy(&serde_json::json!({})));
    }
}
