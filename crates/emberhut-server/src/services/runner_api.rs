//! The runners on the person's machines, as the server reaches them.
//!
//! A runner is a program on one of the person's machines that runs coding-agent sessions and
//! serves a small HTTP API of its own: `POST /sessions` (a new session), `POST
//! /sessions/{id}/send|stop|interrupt|settings`, `GET /sessions/{id}/transcript|live`, `GET
//! /folders`, `GET /branches`, `GET /accounts`, `POST /accounts/check`, `GET /health`, `GET
//! /files` and `PUT /sessions/{id}/files`. Every request carries that runner's token as
//! `Authorization: Bearer <token>`; the runner presents the same token when it reports back to
//! the server (`POST /api/runner/events`, `POST /api/runner/machine`), which is how the server
//! knows which runner it is.
//!
//! Runners are configured, not discovered: `--runner NAME=URL` for each, their tokens as
//! `NAME=TOKEN` lines of `--runner-tokens` (read again when the file changes, so a new token
//! works without a restart), and their names in words with `--runner-name NAME=WORDS`.
//!
//! Runners don't prove the machine on a route as devices do (ADR 0007): they don't pin its key.
//! A runner's address must be a trusted one (TLS the caller checks, or a network that limits
//! who connects), and never the relay, which reads every header (ADR 0017).
//!
//! [`RunnerApi`] is the seam: [`HttpRunners`] is the real one; tests use a fake.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, SystemTime};

use axum::body::Bytes;
use futures_util::StreamExt as _;
use futures_util::future::BoxFuture;
use futures_util::stream::BoxStream;
use serde_json::{Map, Value};
use tokio::io::AsyncWriteExt as _;

use crate::http::read_body;

/// How long a call to a runner may take.
pub const CALL_WAIT: Duration = Duration::from_secs(30);

/// How long a file's way to or from a runner may take.
pub const FILE_WAIT: Duration = Duration::from_mins(20);

/// How long a runner's health may take.
pub const HEALTH_WAIT: Duration = Duration::from_secs(5);

/// The longest answer read from a runner's API (a session's log page is the largest).
const ANSWER_LIMIT: usize = 16 << 20;

/// The longest refusal read from a runner: its error, in a few words.
const REFUSAL_LIMIT: usize = 64 << 10;

/// Who a turn is from, in the runners' words.
pub mod by {
    /// The person, from the app.
    pub const PERSON: &str = "owner";
    /// The assistant.
    pub const ASSISTANT: &str = emberhut_api::wire::RUNNER_BY_ASSISTANT;
    /// The session itself (a turn it queued).
    pub const SELF: &str = "self";
    /// The runner (a turn it queued on its own).
    pub const RUNNER: &str = "runner";
}

/// A runner's bytes as they come (a live stream).
pub type ByteStream = BoxStream<'static, Result<Bytes, RunnerError>>;

/// Why a runner did not do what was asked.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum RunnerError {
    /// No runner of that name is configured.
    #[error("no runner called {0}")]
    Unknown(String),
    /// The runner didn't answer.
    #[error("the runner on {machine} didn't answer ({cause})")]
    Unreachable {
        /// The runner.
        machine: String,
        /// Why, as the connection said.
        cause: String,
    },
    /// The runner answered with a refusal: its words, or its status.
    #[error("{0}")]
    Refused(String),
    /// Something on this side failed (a file to send or to write).
    #[error("{0}")]
    Local(String),
}

/// A request's method.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verb {
    /// `GET`.
    Get,
    /// `POST`.
    Post,
}

/// The runners, as the server reaches them.
pub trait RunnerApi: fmt::Debug + Send + Sync {
    /// The configured runners' names, in their configured order.
    fn machines(&self) -> Vec<String>;

    /// A runner's machine in words (its configured name, or its own name).
    fn place(&self, machine: &str) -> String;

    /// The tokens the runners are sent and present, by runner: read once for all of them.
    fn tokens(&self) -> BoxFuture<'_, BTreeMap<String, String>>;

    /// One request to `machine`'s runner: its JSON answer (`{}` when it had none).
    fn call<'a>(
        &'a self,
        machine: &'a str,
        verb: Verb,
        path: &'a str,
        body: Option<Value>,
    ) -> BoxFuture<'a, Result<Value, RunnerError>>;

    /// `GET path` on `machine`'s runner, its body as it comes (a live stream).
    fn stream<'a>(
        &'a self,
        machine: &'a str,
        path: &'a str,
    ) -> BoxFuture<'a, Result<ByteStream, RunnerError>>;

    /// `PUT path` on `machine`'s runner with the file at `file` as the body: its JSON answer.
    fn upload<'a>(
        &'a self,
        machine: &'a str,
        path: &'a str,
        file: &'a Path,
    ) -> BoxFuture<'a, Result<Value, RunnerError>>;

    /// `GET path` on `machine`'s runner into the file `to`, `limit` bytes at most: how many came.
    fn download<'a>(
        &'a self,
        machine: &'a str,
        path: &'a str,
        to: &'a Path,
        limit: u64,
    ) -> BoxFuture<'a, Result<u64, RunnerError>>;

    /// Whether `machine`'s runner answers its health (an error in words otherwise).
    fn health<'a>(&'a self, machine: &'a str) -> BoxFuture<'a, Result<(), String>>;
}

/// No runner configured.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoRunners;

impl RunnerApi for NoRunners {
    fn machines(&self) -> Vec<String> {
        Vec::new()
    }

    fn place(&self, machine: &str) -> String {
        machine.to_owned()
    }

    fn tokens(&self) -> BoxFuture<'_, BTreeMap<String, String>> {
        Box::pin(async { BTreeMap::new() })
    }

    fn call<'a>(
        &'a self,
        machine: &'a str,
        _verb: Verb,
        _path: &'a str,
        _body: Option<Value>,
    ) -> BoxFuture<'a, Result<Value, RunnerError>> {
        Box::pin(async move { Err(RunnerError::Unknown(machine.to_owned())) })
    }

    fn stream<'a>(
        &'a self,
        machine: &'a str,
        _path: &'a str,
    ) -> BoxFuture<'a, Result<ByteStream, RunnerError>> {
        Box::pin(async move { Err(RunnerError::Unknown(machine.to_owned())) })
    }

    fn upload<'a>(
        &'a self,
        machine: &'a str,
        _path: &'a str,
        _file: &'a Path,
    ) -> BoxFuture<'a, Result<Value, RunnerError>> {
        Box::pin(async move { Err(RunnerError::Unknown(machine.to_owned())) })
    }

    fn download<'a>(
        &'a self,
        machine: &'a str,
        _path: &'a str,
        _to: &'a Path,
        _limit: u64,
    ) -> BoxFuture<'a, Result<u64, RunnerError>> {
        Box::pin(async move { Err(RunnerError::Unknown(machine.to_owned())) })
    }

    fn health<'a>(&'a self, _machine: &'a str) -> BoxFuture<'a, Result<(), String>> {
        Box::pin(async { Err("not configured".to_owned()) })
    }
}

/// One configured runner: its name and its API's address.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunnerAt {
    /// The runner's name, as it names itself in what it reports.
    pub name: String,
    /// Its API's address, `http://` or `https://`, without a final `/`.
    pub url: String,
}

/// The runners over HTTP.
pub struct HttpRunners {
    client: reqwest::Client,
    runners: Vec<RunnerAt>,
    places: BTreeMap<String, String>,
    tokens: Option<PathBuf>,
    /// The tokens file as last read, kept while its modification time and size stay the same.
    kept: Mutex<Option<KeptTokens>>,
}

/// The tokens file's tokens, and the modification time and size it had when they were read.
#[derive(Clone, Debug)]
struct KeptTokens {
    stamp: (Option<SystemTime>, u64),
    tokens: BTreeMap<String, String>,
}

impl fmt::Debug for HttpRunners {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HttpRunners")
            .field("runners", &self.runners)
            .field("places", &self.places)
            .finish_non_exhaustive()
    }
}

impl HttpRunners {
    /// `runners`, named in words by `places`, sent the tokens of the `NAME=TOKEN` file
    /// `tokens`, reached with `client`.
    #[must_use]
    pub fn new(
        client: reqwest::Client,
        runners: Vec<RunnerAt>,
        places: BTreeMap<String, String>,
        tokens: Option<PathBuf>,
    ) -> Self {
        Self {
            client,
            runners: runners
                .into_iter()
                .map(|runner| RunnerAt {
                    url: runner.url.trim_end_matches('/').to_owned(),
                    name: runner.name,
                })
                .collect(),
            places,
            tokens,
            kept: Mutex::new(None),
        }
    }

    /// The tokens file's tokens: read again only when its modification time or size changed,
    /// so a request costs one look at the file's metadata. None when no file is set or it
    /// can't be read.
    async fn read_tokens(&self) -> BTreeMap<String, String> {
        let Some(path) = &self.tokens else {
            return BTreeMap::new();
        };
        let unreadable = |error: &std::io::Error| {
            tracing::warn!(%error, path = %path.display(), "the runners' tokens could not be read");
            BTreeMap::new()
        };
        let meta = match tokio::fs::metadata(path).await {
            Ok(meta) => meta,
            Err(error) => return unreadable(&error),
        };
        let stamp = (meta.modified().ok(), meta.len());
        if let Some(kept) = self
            .kept
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .filter(|kept| kept.stamp == stamp)
        {
            return kept.tokens.clone();
        }
        match tokio::fs::read_to_string(path).await {
            Ok(text) => {
                let tokens = tokens_in(&text);
                *self.kept.lock().unwrap_or_else(PoisonError::into_inner) = Some(KeptTokens {
                    stamp,
                    tokens: tokens.clone(),
                });
                tokens
            }
            Err(error) => unreadable(&error),
        }
    }

    fn url(&self, machine: &str, path: &str) -> Result<String, RunnerError> {
        self.runners
            .iter()
            .find(|runner| runner.name == machine)
            .map(|runner| format!("{}{path}", runner.url))
            .ok_or_else(|| RunnerError::Unknown(machine.to_owned()))
    }

    async fn request(
        &self,
        machine: &str,
        method: reqwest::Method,
        path: &str,
    ) -> Result<reqwest::RequestBuilder, RunnerError> {
        let url = self.url(machine, path)?;
        let request = self.client.request(method, url);
        Ok(match self.read_tokens().await.remove(machine) {
            Some(token) => request.bearer_auth(token),
            None => request,
        })
    }
}

/// Each runner's token in the `NAME=TOKEN` lines of `text` (a runner's first line with a
/// token counts; other lines are skipped).
#[must_use]
pub fn tokens_in(text: &str) -> BTreeMap<String, String> {
    let mut tokens = BTreeMap::new();
    for line in text.lines() {
        if let Some((name, token)) = line.split_once('=')
            && !token.trim().is_empty()
        {
            tokens
                .entry(name.trim().to_owned())
                .or_insert_with(|| token.trim().to_owned());
        }
    }
    tokens
}

/// A failed connection's cause, in a few words.
fn cause(error: &reqwest::Error) -> String {
    if error.is_timeout() {
        "timed out".to_owned()
    } else if error.is_connect() {
        "could not connect".to_owned()
    } else {
        "the request failed".to_owned()
    }
}

/// A runner's refusal: its `error`, or its status.
fn refusal(body: &[u8], status: u16) -> RunnerError {
    let error = match serde_json::from_slice::<Value>(body) {
        Ok(Value::Object(answer)) => match answer.get("error") {
            Some(Value::String(error)) => Some(error.clone()),
            Some(error) if !error.is_null() => Some(error.to_string()),
            _ => None,
        },
        _ => None,
    };
    RunnerError::Refused(error.unwrap_or_else(|| format!("the runner said {status}")))
}

/// A runner's JSON answer; anything that isn't JSON reads as `{}`.
fn answer_of(body: &[u8]) -> Value {
    serde_json::from_slice(body).unwrap_or_else(|_| Value::Object(Map::new()))
}

impl RunnerApi for HttpRunners {
    fn machines(&self) -> Vec<String> {
        self.runners
            .iter()
            .map(|runner| runner.name.clone())
            .collect()
    }

    fn place(&self, machine: &str) -> String {
        self.places
            .get(machine)
            .cloned()
            .unwrap_or_else(|| machine.to_owned())
    }

    fn tokens(&self) -> BoxFuture<'_, BTreeMap<String, String>> {
        Box::pin(self.read_tokens())
    }

    fn call<'a>(
        &'a self,
        machine: &'a str,
        verb: Verb,
        path: &'a str,
        body: Option<Value>,
    ) -> BoxFuture<'a, Result<Value, RunnerError>> {
        Box::pin(async move {
            let method = match verb {
                Verb::Get => reqwest::Method::GET,
                Verb::Post => reqwest::Method::POST,
            };
            let mut request = self
                .request(machine, method, path)
                .await?
                .timeout(CALL_WAIT);
            if let Some(body) = body {
                request = request
                    .header(reqwest::header::CONTENT_TYPE, "application/json")
                    .body(body.to_string());
            }
            let unreachable = |error: &reqwest::Error| RunnerError::Unreachable {
                machine: machine.to_owned(),
                cause: cause(error),
            };
            let response = request.send().await.map_err(|error| unreachable(&error))?;
            let status = response.status();
            let bytes = read_body(response, ANSWER_LIMIT)
                .await
                .map_err(|error| unreachable(&error))?
                .bytes;
            if status.is_success() {
                Ok(answer_of(&bytes))
            } else {
                Err(refusal(&bytes, status.as_u16()))
            }
        })
    }

    fn stream<'a>(
        &'a self,
        machine: &'a str,
        path: &'a str,
    ) -> BoxFuture<'a, Result<ByteStream, RunnerError>> {
        Box::pin(async move {
            let response = self
                .request(machine, reqwest::Method::GET, path)
                .await?
                .send()
                .await
                .map_err(|error| RunnerError::Unreachable {
                    machine: machine.to_owned(),
                    cause: cause(&error),
                })?;
            let status = response.status();
            if !status.is_success() {
                let bytes = read_body(response, REFUSAL_LIMIT)
                    .await
                    .map_or_default(|body| body.bytes);
                return Err(refusal(&bytes, status.as_u16()));
            }
            let owner = machine.to_owned();
            Ok(response
                .bytes_stream()
                .map(move |chunk| {
                    chunk.map_err(|error| RunnerError::Unreachable {
                        machine: owner.clone(),
                        cause: cause(&error),
                    })
                })
                .boxed())
        })
    }

    fn upload<'a>(
        &'a self,
        machine: &'a str,
        path: &'a str,
        file: &'a Path,
    ) -> BoxFuture<'a, Result<Value, RunnerError>> {
        Box::pin(async move {
            let opened = tokio::fs::File::open(file)
                .await
                .map_err(|error| RunnerError::Local(format!("could not open it: {error}")))?;
            let body = reqwest::Body::wrap_stream(tokio_util::io::ReaderStream::new(opened));
            let unreachable = |error: &reqwest::Error| RunnerError::Unreachable {
                machine: machine.to_owned(),
                cause: cause(error),
            };
            let response = self
                .request(machine, reqwest::Method::PUT, path)
                .await?
                .header(reqwest::header::CONTENT_TYPE, "application/octet-stream")
                .timeout(FILE_WAIT)
                .body(body)
                .send()
                .await
                .map_err(|error| unreachable(&error))?;
            let status = response.status();
            let bytes = read_body(response, ANSWER_LIMIT)
                .await
                .map_err(|error| unreachable(&error))?
                .bytes;
            if status.is_success() {
                Ok(answer_of(&bytes))
            } else {
                Err(refusal(&bytes, status.as_u16()))
            }
        })
    }

    fn download<'a>(
        &'a self,
        machine: &'a str,
        path: &'a str,
        to: &'a Path,
        limit: u64,
    ) -> BoxFuture<'a, Result<u64, RunnerError>> {
        Box::pin(async move {
            let unreachable = |error: &reqwest::Error| RunnerError::Unreachable {
                machine: machine.to_owned(),
                cause: cause(error),
            };
            let mut response = self
                .request(machine, reqwest::Method::GET, path)
                .await?
                .timeout(FILE_WAIT)
                .send()
                .await
                .map_err(|error| unreachable(&error))?;
            let status = response.status();
            if !status.is_success() {
                let bytes = read_body(response, REFUSAL_LIMIT)
                    .await
                    .map_or_default(|body| body.bytes);
                return Err(refusal(&bytes, status.as_u16()));
            }
            let local = |error: std::io::Error| RunnerError::Local(error.to_string());
            let mut file = tokio::fs::File::create(to).await.map_err(local)?;
            let mut got: u64 = 0;
            while let Some(chunk) = response
                .chunk()
                .await
                .map_err(|error| unreachable(&error))?
            {
                got += chunk.len() as u64;
                if got > limit {
                    return Err(RunnerError::Local("over the limit".to_owned()));
                }
                file.write_all(&chunk).await.map_err(local)?;
            }
            file.flush().await.map_err(local)?;
            Ok(got)
        })
    }

    fn health<'a>(&'a self, machine: &'a str) -> BoxFuture<'a, Result<(), String>> {
        Box::pin(async move {
            let request = self
                .request(machine, reqwest::Method::GET, "/health")
                .await
                .map_err(|error| error.to_string())?
                .timeout(HEALTH_WAIT);
            match request.send().await {
                Ok(response) if response.status().is_success() => Ok(()),
                Ok(response) => Err(format!("answered {}", response.status().as_u16())),
                Err(error) => Err(format!("not reachable ({})", cause(&error))),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn client() -> reqwest::Client {
        crate::http::shared_client().unwrap()
    }

    #[test]
    fn a_token_is_its_runners_line() {
        let file = "build=abc\n# a note\nlaptop = def \nempty=\nbuild=second\n";
        let tokens = tokens_in(file);
        assert_eq!(tokens.get("build").map(String::as_str), Some("abc"));
        assert_eq!(tokens.get("laptop").map(String::as_str), Some("def"));
        assert_eq!(tokens.get("empty"), None);
        assert_eq!(tokens.len(), 2);
    }

    #[tokio::test]
    async fn the_tokens_file_is_read_again_only_when_it_changes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tokens");
        std::fs::write(&path, "build=abc\n").unwrap();
        let runners = HttpRunners::new(client(), Vec::new(), BTreeMap::new(), Some(path.clone()));
        assert_eq!(
            runners.tokens().await.get("build").map(String::as_str),
            Some("abc")
        );
        // Kept while the file's time and size stay: a stand-in with the same stamp is what
        // the next request reads.
        let stamp = runners.kept.lock().unwrap().as_ref().unwrap().stamp;
        *runners.kept.lock().unwrap() = Some(KeptTokens {
            stamp,
            tokens: BTreeMap::from([("build".to_owned(), "kept".to_owned())]),
        });
        assert_eq!(
            runners.tokens().await.get("build").map(String::as_str),
            Some("kept")
        );
        std::fs::write(&path, "build=abcdef\nlaptop=xyz\n").unwrap();
        let tokens = runners.tokens().await;
        assert_eq!(tokens.get("build").map(String::as_str), Some("abcdef"));
        assert_eq!(tokens.get("laptop").map(String::as_str), Some("xyz"));
        std::fs::remove_file(&path).unwrap();
        assert_eq!(runners.tokens().await, BTreeMap::new());
        let unset = HttpRunners::new(client(), Vec::new(), BTreeMap::new(), None);
        assert_eq!(unset.tokens().await, BTreeMap::new());
    }

    #[test]
    fn a_refusal_is_the_runners_words_or_its_status() {
        assert_eq!(
            refusal(br#"{"error":"no such session"}"#, 404),
            RunnerError::Refused("no such session".to_owned())
        );
        assert_eq!(
            refusal(b"oops", 500),
            RunnerError::Refused("the runner said 500".to_owned())
        );
        assert_eq!(answer_of(b""), Value::Object(Map::new()));
    }
}
