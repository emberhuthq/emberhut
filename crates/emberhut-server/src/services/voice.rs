//! Voice: calls with the assistant from the phone, and dictation.
//!
//! - A call: the phone talks to a realtime voice model directly, with a key for one call made
//!   here ([`RealtimeVoice`]). The voice model is only the assistant's mouth and ears: for
//!   anything it can't answer at once it calls its one tool, and the phone hands that request
//!   to `POST /api/voice/ask`, which says it in the room's chat as the person's words and
//!   waits for the assistant's reply. When the call ends, what was said goes to the assistant
//!   as an event, so it follows up.
//! - Dictation: the app records and sends the audio here; [`SpeechToText`] turns it into text.
//!
//! [`OpenAiVoice`] is the real provider of both (its key in the secrets folder, read at each
//! call, never sent to the app); [`NoVoice`] stands in when none is set up, and tests swap in
//! fakes.

use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use axum::body::Bytes;
use emberhut_api::voice::{ASK_TOOL, LastCall, NEUTRAL_PERSON_LABEL, PERSON_LABEL, VoiceSession};
use emberhut_core::calls::cut;
use emberhut_core::event::NewEvent;
use emberhut_core::message::{Author, HistoryQuery};
use emberhut_core::{Clock, RoomId};
use emberhut_db::{DbError, EventStore, MessageStore, SettingKey, SettingStore};
use futures_util::future::BoxFuture;
use jiff::tz::TimeZone;
use serde_json::{Value, json};

use crate::convert::timestamp_to_api;
use crate::http::read_body;
use crate::services::talk::{TalkError, TalkService};

/// The secrets folder's file with the speech provider's API key.
pub const KEY_FILE: &str = "openai-api-key";

/// The phone's report of the last call.
const LAST_CALL: SettingKey<LastCall> = SettingKey::new("lastCall");

/// How long `POST /api/voice/ask` waits for the assistant's answer.
const ASK_WAIT: Duration = Duration::from_secs(120);
/// How often it looks for the answer.
const ASK_POLL: Duration = Duration::from_millis(700);
/// How long a call to the provider may take (a ten-minute recording takes a while).
const PROVIDER_WAIT: Duration = Duration::from_secs(120);
/// How long a voice call's key works, in seconds.
const KEY_SECONDS: u64 = 600;
/// The longest answer read from the provider (a transcript of a long recording is the largest).
const ANSWER_LIMIT: usize = 4 << 20;

/// Why the voice provider did not do it.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct VoiceError(pub String);

/// A key for one realtime voice call.
#[derive(Clone, Debug, PartialEq)]
pub struct VoiceKey {
    /// The key.
    pub key: String,
    /// When it stops working, in seconds since the epoch.
    pub expires_at: f64,
    /// The model the session runs.
    pub model: String,
}

/// What turns speech into text.
pub trait SpeechToText: fmt::Debug + Send + Sync {
    /// Whether it is set up (dictation shows only then).
    fn ready(&self) -> BoxFuture<'_, bool>;

    /// What `audio` (a file called `name`) says; `prompt` says what to expect (names to spell).
    fn transcribe(
        &self,
        audio: Bytes,
        name: String,
        prompt: String,
    ) -> BoxFuture<'_, Result<String, VoiceError>>;
}

/// What runs realtime voice calls.
pub trait RealtimeVoice: fmt::Debug + Send + Sync {
    /// A key for one call, with the session set up: `instructions`, and the one tool
    /// ([`ASK_TOOL`]).
    fn session(&self, instructions: String) -> BoxFuture<'_, Result<VoiceKey, VoiceError>>;
}

/// No provider set up.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoVoice;

impl SpeechToText for NoVoice {
    fn ready(&self) -> BoxFuture<'_, bool> {
        Box::pin(async { false })
    }

    fn transcribe(
        &self,
        _audio: Bytes,
        _name: String,
        _prompt: String,
    ) -> BoxFuture<'_, Result<String, VoiceError>> {
        Box::pin(async { Err(not_set_up()) })
    }
}

impl RealtimeVoice for NoVoice {
    fn session(&self, _instructions: String) -> BoxFuture<'_, Result<VoiceKey, VoiceError>> {
        Box::pin(async { Err(not_set_up()) })
    }
}

fn not_set_up() -> VoiceError {
    VoiceError("no speech provider is set up on this server yet".to_owned())
}

/// How the `OpenAI` provider is configured.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenAiConfig {
    /// The API's base URL, without a final `/` (`https://api.openai.com/v1`).
    pub url: String,
    /// The file with the API key.
    pub key_file: PathBuf,
    /// The realtime model for calls.
    pub voice_model: String,
    /// The speech-to-text model.
    pub transcribe_model: String,
    /// The realtime voice.
    pub voice: String,
}

/// `OpenAI`'s realtime voice and speech to text.
#[derive(Clone, Debug)]
pub struct OpenAiVoice {
    inner: Arc<OpenAiInner>,
}

struct OpenAiInner {
    config: OpenAiConfig,
    client: reqwest::Client,
}

impl fmt::Debug for OpenAiInner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OpenAiInner")
            .field("url", &self.config.url)
            .finish_non_exhaustive()
    }
}

impl OpenAiVoice {
    /// The provider configured with `config`, reached with `client`.
    #[must_use]
    pub fn new(config: OpenAiConfig, client: reqwest::Client) -> Self {
        Self {
            inner: Arc::new(OpenAiInner { config, client }),
        }
    }

    /// Posts `body` (of `content_type`) to the API's `path`: its JSON.
    async fn post(
        &self,
        path: &'static str,
        content_type: String,
        body: Vec<u8>,
    ) -> Result<Value, VoiceError> {
        self.inner.post(path, &content_type, body).await
    }
}

impl OpenAiInner {
    /// The key, read now; empty when there is none.
    async fn key(&self) -> String {
        tokio::fs::read_to_string(&self.config.key_file)
            .await
            .map_or_default(|key| key.trim().to_owned())
    }

    async fn post(
        &self,
        path: &str,
        content_type: &str,
        body: Vec<u8>,
    ) -> Result<Value, VoiceError> {
        let key = self.key().await;
        if key.is_empty() {
            return Err(VoiceError("no OpenAI key on the server yet".to_owned()));
        }
        let failed = |error: reqwest::Error| {
            VoiceError(if error.is_timeout() {
                "The operation was aborted due to timeout".to_owned()
            } else {
                "fetch failed".to_owned()
            })
        };
        let response = self
            .client
            .post(format!("{}{path}", self.config.url))
            .bearer_auth(&key)
            .header(reqwest::header::CONTENT_TYPE, content_type)
            .timeout(PROVIDER_WAIT)
            .body(body)
            .send()
            .await
            .map_err(failed)?;
        let status = response.status().as_u16();
        let answer = read_body(response, ANSWER_LIMIT).await.map_err(failed)?;
        let parsed: Value = if answer.cut {
            json!({})
        } else {
            serde_json::from_slice(&answer.bytes).unwrap_or_else(|_| json!({}))
        };
        if !(200..300).contains(&status) {
            let reason = parsed
                .pointer("/error/message")
                .and_then(Value::as_str)
                .unwrap_or("no reason");
            return Err(VoiceError(format!("OpenAI said {status}: {reason}")));
        }
        Ok(parsed)
    }
}

impl SpeechToText for OpenAiVoice {
    fn ready(&self) -> BoxFuture<'_, bool> {
        Box::pin(async { !self.inner.key().await.is_empty() })
    }

    fn transcribe(
        &self,
        audio: Bytes,
        name: String,
        prompt: String,
    ) -> BoxFuture<'_, Result<String, VoiceError>> {
        Box::pin(async move {
            let mut fields = vec![("model", self.inner.config.transcribe_model.clone())];
            if !prompt.is_empty() {
                fields.push(("prompt", prompt));
            }
            let (content_type, body) = form(&name, &audio, &fields);
            let answer = self
                .post("/audio/transcriptions", content_type, body)
                .await?;
            Ok(answer.get("text").map_or_default(|text| {
                text.as_str()
                    .map_or_else(|| text.to_string(), str::to_owned)
            }))
        })
    }
}

impl RealtimeVoice for OpenAiVoice {
    fn session(&self, instructions: String) -> BoxFuture<'_, Result<VoiceKey, VoiceError>> {
        Box::pin(async move {
            let config = &self.inner.config;
            let body = json!({
                "expires_after": { "anchor": "created_at", "seconds": KEY_SECONDS },
                "session": {
                    "type": "realtime",
                    "model": config.voice_model,
                    "instructions": instructions,
                    "audio": {
                        "input": {
                            "format": { "type": "audio/pcm", "rate": 24000 },
                            "noise_reduction": { "type": "near_field" },
                            "transcription": { "model": config.transcribe_model },
                            "turn_detection": { "type": "semantic_vad", "eagerness": "medium" },
                        },
                        "output": {
                            "format": { "type": "audio/pcm", "rate": 24000 },
                            "voice": config.voice,
                        },
                    },
                    "tools": [{
                        "type": "function",
                        "name": ASK_TOOL,
                        "description": "Ask the real assistant, who has the user's messages, \
                            calendar, work and memory, and can act. Pass what the user wants, in \
                            their words plus any detail from the call.",
                        "parameters": {
                            "type": "object",
                            "properties": { "request": { "type": "string" } },
                            "required": ["request"],
                        },
                    }],
                    "tool_choice": "auto",
                },
            });
            let answer = self
                .post(
                    "/realtime/client_secrets",
                    "application/json".to_owned(),
                    body.to_string().into_bytes(),
                )
                .await?;
            Ok(VoiceKey {
                key: answer
                    .get("value")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                expires_at: answer
                    .get("expires_at")
                    .and_then(Value::as_f64)
                    .unwrap_or(f64::NAN),
                model: config.voice_model.clone(),
            })
        })
    }
}

/// The boundary between a form's parts.
const BOUNDARY: &str = "emberhut-form-6f1c0b9a4e";

/// A multipart form: the file `name` with `audio` (no type of its own), then `fields` (line
/// breaks as CRLF); its content type and body.
fn form(name: &str, audio: &[u8], fields: &[(&str, String)]) -> (String, Vec<u8>) {
    let escape = |s: &str| {
        s.replace('\n', "%0A")
            .replace('\r', "%0D")
            .replace('"', "%22")
    };
    let name = if name.is_empty() {
        "dictation.m4a"
    } else {
        name
    };
    let mut body = Vec::new();
    body.extend_from_slice(
        format!(
            "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{}\"\r\n\
             Content-Type: application/octet-stream\r\n\r\n",
            escape(name)
        )
        .as_bytes(),
    );
    body.extend_from_slice(audio);
    body.extend_from_slice(b"\r\n");
    for (field, value) in fields {
        let value = value
            .replace("\r\n", "\n")
            .replace('\r', "\n")
            .replace('\n', "\r\n");
        body.extend_from_slice(
            format!(
                "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"{}\"\r\n\r\n{value}\r\n",
                escape(field)
            )
            .as_bytes(),
        );
    }
    body.extend_from_slice(format!("--{BOUNDARY}--\r\n").as_bytes());
    (format!("multipart/form-data; boundary={BOUNDARY}"), body)
}

/// Why a voice request failed.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum VoiceFailure {
    /// The provider failed (502, with its words).
    #[error(transparent)]
    Provider(#[from] VoiceError),
    /// The request could not be said in the chat.
    #[error(transparent)]
    Talk(#[from] TalkError),
    /// The database failed.
    #[error("database error")]
    Database(#[from] DbError),
}

/// Voice calls and dictation.
#[derive(Clone, Debug)]
pub struct VoiceService {
    inner: Arc<Inner>,
}

#[derive(Debug)]
struct Inner {
    speech: Arc<dyn SpeechToText>,
    realtime: Arc<dyn RealtimeVoice>,
    messages: MessageStore,
    events: EventStore,
    settings: SettingStore,
    clock: Arc<dyn Clock>,
    zone: TimeZone,
    /// The labels of the person's lines, none empty.
    person_labels: Vec<String>,
}

/// How the voice service is configured.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VoiceConfig {
    /// The labels a call's transcript gives the person's lines (`Me:`): a transcript with a
    /// line starting with one of them is followed up. Empty labels are ignored.
    pub person_labels: Vec<String>,
}

impl Default for VoiceConfig {
    /// The app's label ([`PERSON_LABEL`]) and the neutral one
    /// ([`NEUTRAL_PERSON_LABEL`]).
    fn default() -> Self {
        Self {
            person_labels: vec![PERSON_LABEL.to_owned(), NEUTRAL_PERSON_LABEL.to_owned()],
        }
    }
}

/// What the voice service is built from.
#[derive(Debug)]
pub struct VoiceParts {
    /// Dictation's speech to text.
    pub speech: Arc<dyn SpeechToText>,
    /// The calls' realtime voice.
    pub realtime: Arc<dyn RealtimeVoice>,
    /// Where its answer is read.
    pub messages: MessageStore,
    /// Where a call's end goes.
    pub events: EventStore,
    /// The settings (the last call's report).
    pub settings: SettingStore,
    /// What time it is.
    pub clock: Arc<dyn Clock>,
    /// The server's time zone, for the call's local time.
    pub zone: TimeZone,
    /// The labels of the person's lines.
    pub config: VoiceConfig,
}

impl VoiceService {
    /// The service from its parts.
    #[must_use]
    pub fn new(parts: VoiceParts) -> Self {
        let VoiceParts {
            speech,
            realtime,
            messages,
            events,
            settings,
            clock,
            zone,
            config,
        } = parts;
        let person_labels = config
            .person_labels
            .into_iter()
            .filter(|label| !label.is_empty())
            .collect();
        Self {
            inner: Arc::new(Inner {
                speech,
                realtime,
                messages,
                events,
                settings,
                clock,
                zone,
                person_labels,
            }),
        }
    }

    /// Whether dictation (and calls) work: the server has a speech provider.
    pub async fn ready(&self) -> bool {
        self.inner.speech.ready().await
    }

    /// The phone's report of the last call.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn last_call(&self) -> Result<Option<LastCall>, DbError> {
        self.inner.settings.get(&LAST_CALL).await
    }

    /// What the voice model is told before the person says a word; `name`: the one the owner
    /// gave their ember, when they gave one.
    fn instructions(&self, name: Option<&str>) -> String {
        let now = self.inner.zone.to_datetime(self.inner.clock.now());
        let local = now.strftime("%A %H:%M");
        let named = name.map_or_else(String::new, crate::services::ember_name::told);
        format!(
            "You are the user's personal assistant, on a voice call with them (they may be \
             driving: keep it short, clear, hands-free).{named} Speak like a warm, trusted assistant: \
             natural, brief sentences, no lists or markdown read out, no machine details. Answer \
             in the language they speak. Local time: {local}.\n\n\
             You are the assistant's voice. The real assistant (the one with their messages, \
             calendar, work, health, memory and the power to act) answers through the \
             {ASK_TOOL} tool: use it for anything about their messages, people, work or \
             calendar, and anything they want done, remembered or changed. Before calling it, \
             say a few words (\"Let me check.\" / \"On it.\"), then read its answer back \
             naturally and briefly. If it says it started a background job, tell them you'll \
             let them know when it's done."
        )
    }

    /// A key for one call.
    ///
    /// # Errors
    ///
    /// [`VoiceFailure::Provider`] when the provider isn't set up or refuses.
    pub async fn session(&self) -> Result<VoiceSession, VoiceFailure> {
        tracing::info!("a voice call is starting");
        let name = crate::services::ember_name::custom_name(&self.inner.settings).await?;
        let key = self
            .inner
            .realtime
            .session(self.instructions(name.as_deref()))
            .await?;
        Ok(VoiceSession {
            ok: true,
            key: key.key,
            expires_at: key.expires_at,
            model: key.model,
        })
    }

    /// The call asks the assistant: `request` goes into `room`'s chat (through `talk`) as the
    /// person's words
    /// (marked as voice), and the assistant's first answer after it comes back (two minutes at
    /// most; after that, a line saying the answer will be in the chat).
    ///
    /// # Errors
    ///
    /// [`VoiceFailure::Talk`] when it can't be said, and the database's failure.
    pub async fn ask(
        &self,
        talk: &TalkService,
        request: &str,
        room: RoomId,
    ) -> Result<String, VoiceFailure> {
        let said = talk
            .talk(room, format!("(by voice) {request}"), None)
            .await?;
        let deadline = tokio::time::Instant::now() + ASK_WAIT;
        while tokio::time::Instant::now() < deadline {
            tokio::time::sleep(ASK_POLL).await;
            let answer = self
                .inner
                .messages
                .page(
                    room,
                    HistoryQuery {
                        before: None,
                        after: Some(said.id),
                    },
                )
                .await?
                .into_iter()
                .find(|message| message.author == Author::Assistant && !message.text.is_empty());
            if let Some(answer) = answer {
                return Ok(answer.text);
            }
        }
        Ok("The assistant is still working on it; the answer will be in the chat.".to_owned())
    }

    /// The call ended: the phone's report is kept (and logged), and what was said goes to
    /// `room`'s assistant when the person said anything.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn done(
        &self,
        seconds: f64,
        diag: Option<&str>,
        transcript: &str,
        room: RoomId,
    ) -> Result<(), DbError> {
        let now = self.inner.clock.now();
        let seconds = if seconds.is_finite() { seconds } else { 0.0 };
        let report = format!(
            "{} call ended ({:.0} s): {}",
            timestamp_to_api(now).as_str(),
            (seconds + 0.5).floor(),
            cut(diag.unwrap_or("no report"), 400)
        );
        tracing::info!(%report, "the phone's report of a voice call");
        self.inner
            .settings
            .set(
                &LAST_CALL,
                &LastCall {
                    report,
                    transcript: cut(transcript, 2000),
                },
            )
            .await?;
        if said_by_the_person(transcript, &self.inner.person_labels) {
            let minutes = ((seconds / 60.0 + 0.5).floor()).max(1.0);
            let text = format!(
                "Voice call with the user ({minutes:.0} min). What was said:\n{}\nFollow up on \
                 anything they asked for that {ASK_TOOL} didn't already handle; otherwise \
                 NO_REPLY.",
                cut(transcript, 6000)
            );
            self.inner
                .events
                .add(NewEvent::new(room, "voice", text), now)
                .await?;
        }
        Ok(())
    }

    /// Dictation: what `audio` (a file called `name`) says; `words` are names it may have.
    ///
    /// # Errors
    ///
    /// [`VoiceFailure::Provider`].
    pub async fn transcribe(
        &self,
        audio: Bytes,
        name: &str,
        words: &str,
    ) -> Result<String, VoiceFailure> {
        let prompt = if words.is_empty() {
            String::new()
        } else {
            format!("Names that may come up: {words}.")
        };
        Ok(self
            .inner
            .speech
            .transcribe(audio, name.to_owned(), prompt)
            .await?)
    }
}

/// Whether `transcript` has a line of the person's: one starting with one of `labels`.
fn said_by_the_person(transcript: &str, labels: &[String]) -> bool {
    transcript
        .lines()
        .any(|line| labels.iter().any(|label| line.starts_with(label.as_str())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_under_any_person_label_is_the_person_s() {
        let labels = VoiceConfig::default().person_labels;
        assert!(said_by_the_person(
            &format!("{PERSON_LABEL} what's next?\nAssistant: Lunch."),
            &labels
        ));
        assert!(said_by_the_person(
            "Assistant: Hello.\nMe: call my sister",
            &labels
        ));
        assert!(!said_by_the_person(
            "Assistant: Hello?\n  Me: indented",
            &labels
        ));
        assert!(!said_by_the_person("Assistant: Hello? Me: no", &labels));
        assert!(!said_by_the_person("", &labels));
        let own = ["You:".to_owned()];
        assert!(said_by_the_person("You: hi", &own));
        assert!(!said_by_the_person("Me: hi", &own));
    }

    #[test]
    fn a_form_has_the_file_then_its_fields() {
        let (content_type, body) = form(
            "memo \"1\".m4a",
            b"AUDIO",
            &[("model", "m".to_owned()), ("prompt", "a\nb".to_owned())],
        );
        assert_eq!(
            content_type,
            format!("multipart/form-data; boundary={BOUNDARY}")
        );
        let body = String::from_utf8(body).unwrap();
        assert!(body.contains("filename=\"memo %221%22.m4a\"\r\nContent-Type: application/octet-stream\r\n\r\nAUDIO\r\n"));
        assert!(body.contains("name=\"prompt\"\r\n\r\na\r\nb\r\n"));
        assert!(body.ends_with(&format!("--{BOUNDARY}--\r\n")));
        let (_, unnamed) = form("", b"", &[]);
        assert!(
            String::from_utf8(unnamed)
                .unwrap()
                .contains("dictation.m4a")
        );
    }
}
