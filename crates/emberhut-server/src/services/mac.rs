//! The owner's Mac, through the Mac helper on it: the app's Mac row (online, locked, idle) and
//! "Lock now" (`GET /api/mac`, `POST /api/mac/lock`). Nothing here ever unlocks the Mac.
//!
//! The helper is a small HTTP service on the Mac that the server calls: `POST <url>/health`
//! answers `{ locked, idle, ... }`, and `POST <url>/lock` locks the screen. Each request carries
//! the helper's token as `Authorization: Bearer <token>` and a JSON body (`{}` here). A refusal
//! is a non-2xx answer whose `error` says why. The rest of what the helper does (an app's window,
//! the whole screen) arrives with the assistant's Mac tools, on this same trait. The helper also
//! keeps the browser extension's folder on the Mac in step with the zip the server serves
//! (`GET <url>/extension`, `POST <url>/extension/install` with the zip; `docs/mac-helper.md`).
//!
//! [`MacHelper`] is the seam: [`HttpMacHelper`] is the real one, configured with the helper's URL
//! and token; [`NoMacHelper`] stands in when none is set up, and tests swap in a fake.

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use emberhut_api::mac::{ExtensionFolder, MacStatus, helper};
use futures_util::future::BoxFuture;
use serde_json::{Map, Value};

use crate::http::{ClientError, Protocol, client, read_body};

/// How long the Mac row waits for the helper's health before it shows the Mac offline.
const HEALTH_WAIT: Duration = Duration::from_secs(5);
/// How long "Lock now" waits for the helper.
const LOCK_WAIT: Duration = Duration::from_secs(10);
/// How long asking for the extension's folder waits for the helper.
const EXTENSION_WAIT: Duration = Duration::from_secs(5);
/// How long installing the extension's zip waits for the helper (it unpacks it first).
const INSTALL_WAIT: Duration = Duration::from_secs(60);
/// The longest answer read from the helper; a longer one reads as `{}`.
const ANSWER_LIMIT: usize = 64 * 1024;
/// The longest answer to the Mac tools (`hands`): a snapshot or a screen shot carries its
/// picture as base64, and the helper keeps a picture's JPEG under 6 MB (8 MB as base64).
const HANDS_ANSWER_LIMIT: usize = 16 * 1024 * 1024;

/// Why the helper's client could not be made.
#[derive(Debug, thiserror::Error)]
#[error("the Mac helper's HTTP client could not be made")]
pub struct MacSetupError(#[source] ClientError);
/// How long starting a call's recording waits for the helper.
const RECORD_START_WAIT: Duration = Duration::from_secs(20);
/// How long stopping a recording waits for the helper (it sends the tracks first).
const RECORD_STOP_WAIT: Duration = Duration::from_secs(30);
/// How long asking whether it records waits for the helper.
const RECORD_STATUS_WAIT: Duration = Duration::from_secs(8);

/// What the Mac's call recorder is asked to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecorderCommand {
    /// Start recording (the body names the call: `{ id, title }`).
    Start,
    /// Stop, and send the tracks to the server.
    Stop,
    /// Say whether it records.
    Status,
}

impl RecorderCommand {
    /// The helper's path for it.
    #[must_use]
    pub const fn path(self) -> &'static str {
        match self {
            Self::Start => "/record/start",
            Self::Stop => "/record/stop",
            Self::Status => "/record/status",
        }
    }

    /// How long the helper is waited for.
    #[must_use]
    pub const fn wait(self) -> Duration {
        match self {
            Self::Start => RECORD_START_WAIT,
            Self::Stop => RECORD_STOP_WAIT,
            Self::Status => RECORD_STATUS_WAIT,
        }
    }
}

/// Why the helper did not do what was asked.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum MacError {
    /// No helper is configured on this server.
    #[error("No Mac helper is set up on this server. Nothing was done.")]
    NotSetUp,
    /// The helper didn't answer: it is off, or the Mac is asleep or off the network.
    #[error(
        "The Mac helper isn't answering (it's off, or the Mac is asleep or away from the \
         network). Nothing was done."
    )]
    Unreachable(#[source] Box<dyn std::error::Error + Send + Sync>),
    /// The helper answered with a refusal; its words, or its status when it gave none.
    #[error("{0}")]
    Refused(String),
}

/// What the helper says of the Mac's health.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MacHealth {
    /// The screen is locked.
    pub locked: bool,
    /// Seconds since the last mouse or key input, when the helper says.
    pub idle: Option<f64>,
}

/// The Mac helper, as the server reaches it.
pub trait MacHelper: fmt::Debug + Send + Sync {
    /// The Mac's health: locked or not, and idle for how long.
    fn health(&self) -> BoxFuture<'_, Result<MacHealth, MacError>>;

    /// Locks the Mac's screen.
    fn lock(&self) -> BoxFuture<'_, Result<(), MacError>>;

    /// Asks the Mac's call recorder to `action`, with `body` (`{ id, title }` to start): its
    /// answer, whose `text` says what it did. A helper that doesn't record calls is
    /// [`MacError::NotSetUp`].
    fn record(
        &self,
        _action: RecorderCommand,
        _body: Value,
    ) -> BoxFuture<'_, Result<Map<String, Value>, MacError>> {
        Box::pin(async { Err(MacError::NotSetUp) })
    }

    /// `POST`s `body` to the helper's `path` (`/apps`, `/snapshot`, `/press`, `/screen/act`…:
    /// the assistant's Mac tools) and reads its JSON answer, waiting at most `wait`.
    fn hands(
        &self,
        path: &str,
        body: Value,
        wait: Duration,
    ) -> BoxFuture<'_, Result<Map<String, Value>, MacError>> {
        let _ = (path, body, wait);
        Box::pin(async { Err(MacError::NotSetUp) })
    }

    /// The browser extension's folder on the Mac: the version unpacked there.
    fn extension(&self) -> BoxFuture<'_, Result<ExtensionFolder, MacError>> {
        Box::pin(async { Err(MacError::NotSetUp) })
    }

    /// Unpacks the extension's `zip` into its folder on the Mac, replacing what was there: the
    /// folder as it is then. The helper refuses a zip that isn't the extension (another id) or
    /// that reaches outside the folder.
    fn install_extension(&self, zip: Vec<u8>) -> BoxFuture<'_, Result<ExtensionFolder, MacError>> {
        drop(zip);
        Box::pin(async { Err(MacError::NotSetUp) })
    }
}

/// No helper set up: the Mac is offline, and locking says so.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoMacHelper;

impl MacHelper for NoMacHelper {
    fn health(&self) -> BoxFuture<'_, Result<MacHealth, MacError>> {
        Box::pin(async { Err(MacError::NotSetUp) })
    }

    fn lock(&self) -> BoxFuture<'_, Result<(), MacError>> {
        Box::pin(async { Err(MacError::NotSetUp) })
    }
}

/// The helper over HTTP, at its URL with its token.
pub struct HttpMacHelper {
    client: reqwest::Client,
    url: reqwest::Url,
    token: String,
}

impl fmt::Debug for HttpMacHelper {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HttpMacHelper")
            .field("url", &self.url.as_str())
            .finish_non_exhaustive()
    }
}

impl HttpMacHelper {
    /// The helper at `url` (`http://mac.example:8793`), sent `token` as its bearer.
    ///
    /// # Errors
    ///
    /// [`MacSetupError`] when its HTTP client cannot be made.
    pub fn new(url: reqwest::Url, token: String) -> Result<Self, MacSetupError> {
        Ok(Self {
            client: client(Protocol::Any).map_err(MacSetupError)?,
            url,
            token,
        })
    }

    /// `POST`s `{}` to the helper's `path` and reads its JSON answer (an object; anything else
    /// reads as `{}`).
    async fn call(&self, path: &str, wait: Duration) -> Result<Map<String, Value>, MacError> {
        self.send(path, "{}".to_owned(), wait, ANSWER_LIMIT).await
    }

    /// `POST`s `body` (JSON) to the helper's `path` and reads its JSON answer, as [`Self::call`].
    async fn send(
        &self,
        path: &str,
        body: String,
        wait: Duration,
        limit: usize,
    ) -> Result<Map<String, Value>, MacError> {
        let request = self
            .client
            .post(self.at(path))
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body);
        self.answer(request, wait, limit).await
    }

    /// The helper's address for `path`.
    fn at(&self, path: &str) -> String {
        format!("{}{path}", self.url.as_str().trim_end_matches('/'))
    }

    /// Sends `request` with the helper's token and reads its JSON answer, up to `limit` bytes (an
    /// object; anything else, or a longer answer, reads as `{}`); a non-2xx answer is the helper's
    /// refusal.
    async fn answer(
        &self,
        request: reqwest::RequestBuilder,
        wait: Duration,
        limit: usize,
    ) -> Result<Map<String, Value>, MacError> {
        let response = request
            .bearer_auth(&self.token)
            .timeout(wait)
            .send()
            .await
            .map_err(|error| MacError::Unreachable(Box::new(error)))?;
        let status = response.status();
        let body = read_body(response, limit)
            .await
            .map_err(|error| MacError::Unreachable(Box::new(error)))?;
        let answer = match serde_json::from_slice(&body.bytes) {
            Ok(Value::Object(answer)) if !body.cut => answer,
            _ => Map::new(),
        };
        if status.is_success() {
            Ok(answer)
        } else {
            Err(refusal(&answer, status.as_u16()))
        }
    }
}

/// The helper's refusal: its `error`, or its status.
fn refusal(answer: &Map<String, Value>, status: u16) -> MacError {
    MacError::Refused(match answer.get("error") {
        Some(Value::String(error)) => error.clone(),
        Some(error) if !error.is_null() => error.to_string(),
        _ => format!("The Mac helper said {status}"),
    })
}

/// The helper's health answer: `locked` as JavaScript reads truth (any truthy value), `idle`
/// when it is a number.
fn health_from(answer: &Map<String, Value>) -> MacHealth {
    let locked = match answer.get("locked") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(locked)) => *locked,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|n| n != 0.0 && !n.is_nan()),
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Array(_) | Value::Object(_)) => true,
    };
    let idle = answer.get("idle").and_then(Value::as_f64);
    MacHealth { locked, idle }
}

impl MacHelper for HttpMacHelper {
    fn health(&self) -> BoxFuture<'_, Result<MacHealth, MacError>> {
        Box::pin(async move { Ok(health_from(&self.call("/health", HEALTH_WAIT).await?)) })
    }

    fn lock(&self) -> BoxFuture<'_, Result<(), MacError>> {
        Box::pin(async move { self.call("/lock", LOCK_WAIT).await.map(drop) })
    }

    fn record(
        &self,
        action: RecorderCommand,
        body: Value,
    ) -> BoxFuture<'_, Result<Map<String, Value>, MacError>> {
        Box::pin(async move {
            self.send(action.path(), body.to_string(), action.wait(), ANSWER_LIMIT)
                .await
        })
    }

    fn hands(
        &self,
        path: &str,
        body: Value,
        wait: Duration,
    ) -> BoxFuture<'_, Result<Map<String, Value>, MacError>> {
        let path = path.to_owned();
        Box::pin(async move {
            self.send(&path, body.to_string(), wait, HANDS_ANSWER_LIMIT)
                .await
        })
    }

    fn extension(&self) -> BoxFuture<'_, Result<ExtensionFolder, MacError>> {
        Box::pin(async move {
            let request = self.client.get(self.at(helper::EXTENSION));
            folder_from(self.answer(request, EXTENSION_WAIT, ANSWER_LIMIT).await?)
        })
    }

    fn install_extension(&self, zip: Vec<u8>) -> BoxFuture<'_, Result<ExtensionFolder, MacError>> {
        Box::pin(async move {
            if zip.len() > helper::EXTENSION_ZIP_MAX {
                return Err(MacError::Refused(
                    "The extension's zip is over 20 MB: the Mac helper takes no more".to_owned(),
                ));
            }
            let request = self
                .client
                .post(self.at(helper::EXTENSION_INSTALL))
                .header(reqwest::header::CONTENT_TYPE, "application/zip")
                .body(zip);
            folder_from(self.answer(request, INSTALL_WAIT, ANSWER_LIMIT).await?)
        })
    }
}

/// The helper's answer about the extension's folder; one that isn't its shape is a refusal.
fn folder_from(answer: Map<String, Value>) -> Result<ExtensionFolder, MacError> {
    serde_json::from_value(Value::Object(answer)).map_err(|_| {
        MacError::Refused(
            "The Mac helper's answer about the extension's folder isn't one".to_owned(),
        )
    })
}

/// The Mac row and "Lock now", over the helper.
#[derive(Clone, Debug)]
pub struct MacService {
    helper: Arc<dyn MacHelper>,
}

impl MacService {
    /// The helper itself, for the assistant's Mac tools.
    #[must_use]
    pub fn helper(&self) -> &Arc<dyn MacHelper> {
        &self.helper
    }
}

impl MacService {
    /// The service over `helper`.
    #[must_use]
    pub fn new(helper: Arc<dyn MacHelper>) -> Self {
        Self { helper }
    }

    /// The Mac's row: offline whenever the helper doesn't answer with its health, for any
    /// reason.
    pub async fn status(&self) -> MacStatus {
        match self.helper.health().await {
            Ok(health) => MacStatus {
                online: true,
                locked: Some(health.locked),
                idle: health.idle,
            },
            Err(error) => {
                tracing::debug!(%error, "the Mac helper gave no health");
                MacStatus::OFFLINE
            }
        }
    }

    /// Asks the Mac's call recorder to `action` (see [`MacHelper::record`]).
    ///
    /// # Errors
    ///
    /// [`MacError`] when the helper is not set up, doesn't answer, or refuses.
    pub async fn record(
        &self,
        action: RecorderCommand,
        body: Value,
    ) -> Result<Map<String, Value>, MacError> {
        self.helper.record(action, body).await
    }

    /// Locks the Mac's screen.
    ///
    /// # Errors
    ///
    /// [`MacError`] when the helper is not set up, doesn't answer, or refuses.
    pub async fn lock(&self) -> Result<(), MacError> {
        self.helper
            .lock()
            .await
            .inspect_err(|error| tracing::warn!(?error, "the Mac was not locked"))?;
        tracing::info!("locked the Mac (Lock now in the app)");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn object(value: Value) -> Map<String, Value> {
        let Value::Object(map) = value else {
            panic!("an object")
        };
        map
    }

    #[test]
    fn health_reads_locked_as_truth_and_idle_as_a_number() {
        let health = health_from(&object(json!({ "locked": true, "idle": 12 })));
        assert_eq!(
            health,
            MacHealth {
                locked: true,
                idle: Some(12.0)
            }
        );
        for (locked, truthy) in [
            (json!(null), false),
            (json!(0), false),
            (json!(""), false),
            (json!(1), true),
            (json!("yes"), true),
            (json!({}), true),
        ] {
            let health = health_from(&object(json!({ "locked": locked, "idle": "soon" })));
            assert_eq!(health.locked, truthy, "{locked}");
            assert_eq!(health.idle, None);
        }
        assert!(!health_from(&Map::new()).locked);
    }

    #[test]
    fn a_refusal_carries_the_helper_s_words_or_its_status() {
        let said = refusal(
            &object(json!({ "error": "The screen can't lock now" })),
            409,
        );
        assert_eq!(said.to_string(), "The screen can't lock now");
        let silent = refusal(&Map::new(), 502);
        assert_eq!(silent.to_string(), "The Mac helper said 502");
    }

    #[tokio::test]
    async fn a_mac_tool_s_answer_holds_a_picture_bigger_than_a_plain_answer() {
        use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let picture = "A".repeat(4 * ANSWER_LIMIT);
        let answer = json!({ "text": "The main screen", "image": picture }).to_string();
        let served = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = vec![0_u8; 16 * 1024];
            let _read = socket.read(&mut request).await.unwrap();
            let head = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\
                 connection: close\r\n\r\n",
                answer.len()
            );
            socket.write_all(head.as_bytes()).await.unwrap();
            socket.write_all(answer.as_bytes()).await.unwrap();
        });
        let url = format!("http://{address}").parse().unwrap();
        let helper = HttpMacHelper::new(url, "token".to_owned()).unwrap();
        let shot = helper
            .hands("/screen/shot", json!({}), Duration::from_secs(10))
            .await
            .unwrap();
        assert_eq!(
            shot.get("image").and_then(Value::as_str).map(str::len),
            Some(4 * ANSWER_LIMIT)
        );
        served.await.unwrap();
    }

    #[tokio::test]
    async fn without_a_helper_the_mac_is_offline_and_locking_says_so() {
        let mac = MacService::new(Arc::new(NoMacHelper));
        assert_eq!(mac.status().await, MacStatus::OFFLINE);
        let refused = mac.lock().await.unwrap_err();
        assert!(matches!(refused, MacError::NotSetUp));
    }
}
