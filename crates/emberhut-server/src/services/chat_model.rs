//! A language model to chat with, streamed: what the side apps (parties, the off-the-record
//! room) talk through. Each sends a system prompt and a conversation, hears the answer as it is
//! written, and gets the whole text and its token counts at the end.
//!
//! The model is behind the [`ChatModel`] trait. The server's implementation is
//! [`OpenAiChat`]: any endpoint that speaks the OpenAI-style chat completions API with
//! server-sent events (a local llama-server, a cloud gateway, a provider's OpenAI-style
//! endpoint), at a configured base URL, with an optional bearer key read from a file. Tests use a
//! scripted stand-in; [`NoChatModel`] answers that none is set up.

use std::fmt;
use std::path::PathBuf;
use std::time::Duration;

use futures_util::future::BoxFuture;
use serde_json::{Map, Value, json};
use tokio_util::sync::CancellationToken;

use crate::http::read_body;

/// Who said a line of a conversation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChatRole {
    /// The person (or the prompt the server writes for the model).
    User,
    /// The model.
    Assistant,
}

impl ChatRole {
    /// The role's word on the wire.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Assistant => "assistant",
        }
    }
}

/// One line of a conversation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChatLine {
    /// Who said it.
    pub role: ChatRole,
    /// What was said.
    pub content: String,
}

impl ChatLine {
    /// A line the person (or the server's prompt) says.
    #[must_use]
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: ChatRole::User,
            content: content.into(),
        }
    }
}

/// What to ask the model.
#[derive(Clone, Debug, PartialEq)]
pub struct ChatRequest {
    /// The model's id (the endpoint's own; empty: the endpoint's default).
    pub model: String,
    /// The system prompt.
    pub system: String,
    /// The conversation, oldest first, ending with the line to answer.
    pub lines: Vec<ChatLine>,
    /// The most tokens the answer may take.
    pub max_tokens: Option<u32>,
    /// Sampling settings sent as they are (`temperature`, `top_k`, ...).
    pub sampling: Map<String, Value>,
}

/// A piece of the answer as it comes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChatPiece {
    /// The model is reasoning before it answers (said once, before any text).
    Thinking,
    /// More of the answer.
    Text(String),
}

/// The whole answer.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChatReply {
    /// Its text.
    pub text: String,
    /// Tokens in (the prompt), as the endpoint counted them.
    pub tokens_in: u32,
    /// Tokens out (the answer).
    pub tokens_out: u32,
}

/// Why the model gave no answer.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ChatModelError {
    /// No model is set up on this server.
    #[error("no model is set up on this server")]
    NotConfigured,
    /// The key file could not be read.
    #[error("the model's key could not be read")]
    Key(#[source] std::io::Error),
    /// The endpoint could not be reached.
    #[error("fetch failed")]
    Unreachable(#[source] Box<dyn std::error::Error + Send + Sync>),
    /// The endpoint refused: its status and the start of its answer.
    #[error("{status}: {text}")]
    Refused {
        /// The status.
        status: u16,
        /// The start of its answer.
        text: String,
    },
    /// The stream said there was an error.
    #[error("{0}")]
    Stream(String),
    /// The endpoint stopped sending.
    #[error("the model stopped answering")]
    Idle,
    /// The stream ended with no text.
    #[error("The model sent no answer.")]
    Empty,
    /// The request was cancelled.
    #[error("cancelled")]
    Cancelled,
}

impl ChatModelError {
    /// What kind of failure it is, in a word or two: what a log line may say (never the
    /// model's or the endpoint's own text, which may quote the conversation).
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            Self::NotConfigured => "not set up",
            Self::Key(_) => "key unreadable",
            Self::Unreachable(_) => "unreachable",
            Self::Refused { .. } => "refused",
            Self::Stream(_) => "stream error",
            Self::Idle => "stopped answering",
            Self::Empty => "no answer",
            Self::Cancelled => "cancelled",
        }
    }

    /// The endpoint's status, when it refused.
    #[must_use]
    pub fn status(&self) -> Option<u16> {
        match self {
            Self::Refused { status, .. } => Some(*status),
            _ => None,
        }
    }
}

/// Where the pieces of an answer go as they come.
pub type Pieces<'a> = &'a (dyn Fn(ChatPiece) + Send + Sync);

/// A language model to chat with.
pub trait ChatModel: fmt::Debug + Send + Sync {
    /// Answers `request`, each piece to `pieces` as it comes; ends early, with
    /// [`ChatModelError::Cancelled`], when `stop` is cancelled.
    fn chat<'a>(
        &'a self,
        request: ChatRequest,
        pieces: Pieces<'a>,
        stop: &'a CancellationToken,
    ) -> BoxFuture<'a, Result<ChatReply, ChatModelError>>;

    /// Whether the model answers now (a quick health check); a model that has none is up.
    fn is_up(&self) -> BoxFuture<'_, bool> {
        Box::pin(async { true })
    }
}

/// No model: every chat answers [`ChatModelError::NotConfigured`].
#[derive(Clone, Copy, Debug, Default)]
pub struct NoChatModel;

impl ChatModel for NoChatModel {
    fn chat<'a>(
        &'a self,
        _request: ChatRequest,
        _pieces: Pieces<'a>,
        _stop: &'a CancellationToken,
    ) -> BoxFuture<'a, Result<ChatReply, ChatModelError>> {
        Box::pin(async { Err(ChatModelError::NotConfigured) })
    }

    fn is_up(&self) -> BoxFuture<'_, bool> {
        Box::pin(async { false })
    }
}

/// How long the endpoint may send nothing, first byte included, before the answer is given up.
const IDLE: Duration = Duration::from_secs(60);
/// The longest an answer may take in all.
const WHOLE: Duration = Duration::from_mins(10);
/// How long the health check may take.
const HEALTH: Duration = Duration::from_secs(4);
/// The longest refusal read: its first 200 characters are kept.
const REFUSAL_LIMIT: usize = 64 << 10;

/// An OpenAI-style chat completions endpoint, streamed.
#[derive(Clone)]
pub struct OpenAiChat {
    client: reqwest::Client,
    base: String,
    key_file: Option<PathBuf>,
    health: Option<String>,
    model: Option<String>,
}

impl fmt::Debug for OpenAiChat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OpenAiChat")
            .field("base", &self.base)
            .field("model", &self.model)
            .finish_non_exhaustive()
    }
}

impl OpenAiChat {
    /// The endpoint at `base` (`https://gateway.example/v1`; `/chat/completions` is added),
    /// reached with `client` and sent the key in `key_file` (read at every call, trimmed) as
    /// its bearer when the file is there.
    #[must_use]
    pub fn new(client: reqwest::Client, base: &str, key_file: Option<PathBuf>) -> Self {
        Self {
            client,
            base: base.trim_end_matches('/').to_owned(),
            key_file,
            health: None,
            model: None,
        }
    }

    /// With a health check: `GET` of the base's server root plus `/health` (llama-server's),
    /// whose 2xx means the model is up.
    #[must_use]
    pub fn with_health_check(mut self) -> Self {
        let root = self.base.strip_suffix("/v1").unwrap_or(&self.base);
        self.health = Some(format!("{root}/health"));
        self
    }

    /// Asking for `model` whenever a request names none.
    #[must_use]
    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }

    /// The key in the key file; none when there is no file (a local endpoint needs none).
    async fn key(&self) -> Result<Option<String>, ChatModelError> {
        let Some(file) = &self.key_file else {
            return Ok(None);
        };
        match tokio::fs::read_to_string(file).await {
            Ok(key) => Ok(Some(key.trim().to_owned()).filter(|key| !key.is_empty())),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(ChatModelError::Key(error)),
        }
    }

    fn body(&self, request: ChatRequest) -> Value {
        let mut messages = vec![json!({ "role": "system", "content": request.system })];
        messages.extend(
            request
                .lines
                .into_iter()
                .map(|line| json!({ "role": line.role.as_str(), "content": line.content })),
        );
        let mut body = request.sampling;
        let model = if request.model.is_empty() {
            self.model.clone()
        } else {
            Some(request.model)
        };
        if let Some(model) = model {
            body.insert("model".to_owned(), Value::String(model));
        }
        body.insert("stream".to_owned(), Value::Bool(true));
        body.insert(
            "stream_options".to_owned(),
            json!({ "include_usage": true }),
        );
        if let Some(max) = request.max_tokens {
            body.insert("max_tokens".to_owned(), Value::from(max));
        }
        body.insert("messages".to_owned(), Value::Array(messages));
        Value::Object(body)
    }

    async fn run(
        &self,
        request: ChatRequest,
        pieces: Pieces<'_>,
    ) -> Result<ChatReply, ChatModelError> {
        let mut builder = self
            .client
            .post(format!("{}/chat/completions", self.base))
            .header(reqwest::header::CONTENT_TYPE, "application/json");
        if let Some(key) = self.key().await? {
            builder = builder.bearer_auth(key);
        }
        let body = self.body(request).to_string();
        let mut response = tokio::time::timeout(IDLE, builder.body(body).send())
            .await
            .map_err(|_| ChatModelError::Idle)?
            .map_err(|error| ChatModelError::Unreachable(Box::new(error)))?;
        let status = response.status();
        let mut reader = StreamReader::default();
        if !status.is_success() {
            let text = tokio::time::timeout(IDLE, read_body(response, REFUSAL_LIMIT))
                .await
                .ok()
                .and_then(Result::ok)
                .map_or_else(String::new, |body| body.text());
            return Err(ChatModelError::Refused {
                status: status.as_u16(),
                text: text.chars().take(200).collect(),
            });
        }
        loop {
            let chunk = tokio::time::timeout(IDLE, response.chunk())
                .await
                .map_err(|_| ChatModelError::Idle)?
                .map_err(|error| ChatModelError::Unreachable(Box::new(error)))?;
            let Some(data) = chunk else { break };
            reader.feed(&data, pieces)?;
        }
        reader.finish()
    }
}

impl ChatModel for OpenAiChat {
    fn chat<'a>(
        &'a self,
        request: ChatRequest,
        pieces: Pieces<'a>,
        stop: &'a CancellationToken,
    ) -> BoxFuture<'a, Result<ChatReply, ChatModelError>> {
        Box::pin(async move {
            tokio::select! {
                () = stop.cancelled() => Err(ChatModelError::Cancelled),
                answer = tokio::time::timeout(WHOLE, self.run(request, pieces)) => {
                    answer.unwrap_or(Err(ChatModelError::Idle))
                }
            }
        })
    }

    fn is_up(&self) -> BoxFuture<'_, bool> {
        Box::pin(async move {
            let Some(url) = &self.health else {
                return true;
            };
            matches!(
                tokio::time::timeout(HEALTH, self.client.get(url.as_str()).send()).await,
                Ok(Ok(response)) if response.status().is_success()
            )
        })
    }
}

/// Reads an OpenAI-style event stream: `data: {json}` lines, `data: [DONE]` at the end.
/// Reasoning shows only as [`ChatPiece::Thinking`], once, before any text; the text streams in,
/// its leading blank lines dropped; the usage comes at the end.
#[derive(Debug, Default)]
pub struct StreamReader {
    buffer: Vec<u8>,
    text: String,
    thinking: bool,
    tokens_in: u32,
    tokens_out: u32,
}

impl StreamReader {
    /// Reads `bytes`, sending each piece to `pieces`.
    ///
    /// # Errors
    ///
    /// [`ChatModelError::Stream`] when the stream reports an error.
    pub fn feed(&mut self, bytes: &[u8], pieces: Pieces<'_>) -> Result<(), ChatModelError> {
        self.buffer.extend_from_slice(bytes);
        while let Some(end) = self.buffer.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = self.buffer.drain(..=end).collect();
            let line = String::from_utf8_lossy(&line);
            self.line(line.trim_end_matches(['\r', '\n']), pieces)?;
        }
        Ok(())
    }

    fn line(&mut self, line: &str, pieces: Pieces<'_>) -> Result<(), ChatModelError> {
        let Some(data) = line.strip_prefix("data: ") else {
            return Ok(());
        };
        if data == "[DONE]" {
            return Ok(());
        }
        let Ok(event) = serde_json::from_str::<Value>(data) else {
            return Ok(());
        };
        let error = event.get("error").filter(|e| truthy(e)).or_else(|| {
            event.get("errors").filter(|e| match e {
                Value::Array(list) => !list.is_empty(),
                Value::String(text) => !text.is_empty(),
                _ => false,
            })
        });
        if let Some(error) = error {
            return Err(ChatModelError::Stream(
                error.to_string().chars().take(200).collect(),
            ));
        }
        if let Some(usage) = event.get("usage").filter(|u| u.is_object()) {
            let count = |key: &str| {
                usage
                    .get(key)
                    .and_then(Value::as_u64)
                    .and_then(|n| u32::try_from(n).ok())
            };
            if let Some(n) = count("prompt_tokens") {
                self.tokens_in = n;
            }
            if let Some(n) = count("completion_tokens") {
                self.tokens_out = n;
            }
        }
        let delta = event
            .get("choices")
            .and_then(|choices| choices.get(0))
            .and_then(|choice| choice.get("delta"));
        let Some(delta) = delta else {
            return Ok(());
        };
        let content = delta.get("content").and_then(Value::as_str);
        let words = if self.text.is_empty() {
            content.map(str::trim_start)
        } else {
            content
        };
        if let Some(words) = words.filter(|words| !words.is_empty()) {
            self.text.push_str(words);
            pieces(ChatPiece::Text(words.to_owned()));
        } else if !self.thinking
            && self.text.is_empty()
            && (delta.get("reasoning").is_some_and(truthy)
                || delta.get("reasoning_content").is_some_and(truthy))
        {
            self.thinking = true;
            pieces(ChatPiece::Thinking);
        }
        Ok(())
    }

    /// The whole answer.
    ///
    /// # Errors
    ///
    /// [`ChatModelError::Empty`] when no text came.
    pub fn finish(self) -> Result<ChatReply, ChatModelError> {
        if self.text.trim().is_empty() {
            return Err(ChatModelError::Empty);
        }
        Ok(ChatReply {
            text: self.text,
            tokens_in: self.tokens_in,
            tokens_out: self.tokens_out,
        })
    }
}

/// JavaScript's truthiness of a JSON value.
fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|n| n != 0.0),
        Value::String(s) => !s.is_empty(),
        Value::Array(_) | Value::Object(_) => true,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    #[test]
    fn a_failure_s_kind_and_status_never_carry_its_text() {
        let refused = ChatModelError::Refused {
            status: 429,
            text: "what the person said".to_owned(),
        };
        assert_eq!((refused.kind(), refused.status()), ("refused", Some(429)));
        let stream = ChatModelError::Stream("a quoted line".to_owned());
        assert_eq!((stream.kind(), stream.status()), ("stream error", None));
    }

    #[test]
    fn the_stream_says_thinking_once_then_the_text_then_its_usage() {
        let got = Mutex::new(Vec::new());
        let pieces = |piece: ChatPiece| got.lock().unwrap().push(piece);
        let mut reader = StreamReader::default();
        let stream = concat!(
            "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"hm\"}}]}\n",
            "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"hm\"}}]}\n",
            ": a comment\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"\\n\\nHel\"}}]}\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"lo\"}}]}\n",
            "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":12,\"completion_tokens\":3}}\n",
            "data: [DONE]\n",
        );
        // Split mid-line, as a network would.
        let (a, b) = stream.split_at(70);
        reader.feed(a.as_bytes(), &pieces).unwrap();
        reader.feed(b.as_bytes(), &pieces).unwrap();
        let reply = reader.finish().unwrap();
        assert_eq!(reply.text, "Hello");
        assert_eq!((reply.tokens_in, reply.tokens_out), (12, 3));
        assert_eq!(
            *got.lock().unwrap(),
            vec![
                ChatPiece::Thinking,
                ChatPiece::Text("Hel".to_owned()),
                ChatPiece::Text("lo".to_owned())
            ]
        );
    }

    #[test]
    fn an_error_in_the_stream_or_no_text_is_a_failure() {
        let pieces = |_: ChatPiece| {};
        let mut reader = StreamReader::default();
        let failed = reader.feed(b"data: {\"errors\":[{\"code\":7}]}\n", &pieces);
        assert!(matches!(failed, Err(ChatModelError::Stream(text)) if text.contains('7')));
        assert!(matches!(
            StreamReader::default().finish(),
            Err(ChatModelError::Empty)
        ));
    }

    #[test]
    fn the_body_has_the_system_prompt_first_and_asks_for_usage() {
        let chat = OpenAiChat::new(
            crate::http::shared_client().unwrap(),
            "http://127.0.0.1:9/v1/",
            None,
        )
        .with_model("m1");
        let mut sampling = Map::new();
        sampling.insert("temperature".to_owned(), json!(0.1));
        let body = chat.body(ChatRequest {
            model: String::new(),
            system: "be brief".to_owned(),
            lines: vec![ChatLine::user("hi")],
            max_tokens: Some(50),
            sampling,
        });
        assert_eq!(body["model"], "m1");
        assert_eq!(body["messages"][0]["role"], "system");
        assert_eq!(body["messages"][1]["content"], "hi");
        assert_eq!(body["stream_options"]["include_usage"], true);
        assert_eq!(body["temperature"], 0.1);
        assert_eq!(body["max_tokens"], 50);
        assert_eq!(
            chat.with_health_check().health.as_deref(),
            Some("http://127.0.0.1:9/health")
        );
    }
}
