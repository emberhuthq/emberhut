//! The `OpenAI` bridge: Anthropic's Messages API, served to the agent CLI on this computer and
//! answered by an `OpenAI` Chat Completions endpoint (`OpenAI`'s, or any that speaks its API:
//! vLLM, LM Studio, Groq, Together...). Claude Code speaks Anthropic's API only; with the bridge
//! it runs on the `openai` and `openai-compatible` providers.
//!
//! - **Its own listener, on loopback.** [`OpenAiBridge::bind`] listens on `127.0.0.1` at a port
//!   the system picks, apart from the API's listener (which may face the network, a relay or a
//!   sealed transport): no other device can reach it, whatever `EMBERHUT_LISTEN` says, and a
//!   request from any other peer is refused too.
//! - **A token per launch.** Each launch of the CLI on a bridged provider gets its own
//!   ([`BridgeAccess::grant`]) as `ANTHROPIC_AUTH_TOKEN`, held by the launch
//!   ([`BridgeGrant`]) and revoked once its CLI has exited. Only the tokens' hashes are kept,
//!   compared in constant time. The provider's own key never leaves the server: the bridge
//!   reads it from the secrets folder at each request.
//! - **Read at each request.** The provider saved now, its key, base URL and models, so a key
//!   saved again is used at once; a provider changed to one the bridge doesn't serve is
//!   refused (the session restarts on the new one at its next turn).
//! - **Routes.** `POST /v1/messages` (streamed or not; `?beta=true` taken), and `POST
//!   /v1/messages/count_tokens`, answered with an estimate (Chat Completions has no such
//!   route). Anything else is a `not_found_error`. Errors are the Messages API's
//!   (`{"type":"error","error":{"type":…,"message":…}}`), the endpoint's status mapped to
//!   its kind (`refused`, in this module).
//! - **Models.** The CLI asks for the models its environment names (the saved model, the quick
//!   one for `haiku`); another name (one the CLI has built in) is mapped to the saved quick
//!   model when it is a `haiku` model, to the saved model otherwise.
//!
//! What is translated, and what isn't, is in the module's `request`, `reply` and `stream` parts
//! (and ADR 0029).

mod reply;
mod request;
mod stream;

use std::collections::{HashMap, VecDeque};
use std::convert::Infallible;
use std::fmt;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::Duration;

use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::{ConnectInfo, DefaultBodyLimit, Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use base64::Engine as _;
use emberhut_core::{RandomError, RandomSource, SecretHash, Token};
use serde_json::{Value, json};
use subtle::ConstantTimeEq as _;
use tokio::net::TcpListener;

use self::request::{Dialect, MessagesRequest, ToolNames};
use self::stream::{Event, Translator};
use super::{ApiKey, Provider, ProviderStore};
use crate::error::chain;
use crate::http::read_body;

/// The largest request the bridge reads: the Messages API's own limit.
const REQUEST_LIMIT: usize = 32 << 20;

/// The largest whole (not streamed) answer it reads.
const REPLY_LIMIT: usize = 32 << 20;

/// The start of an endpoint's refusal that is read.
const REFUSAL_LIMIT: usize = 64 << 10;

/// How long the endpoint may take to answer, and then stay silent in a stream: the agent CLI's
/// own wait for Anthropic's API.
const WAIT: Duration = Duration::from_mins(10);

/// How often a silent stream gets a `ping`, as Anthropic's API sends one.
const PING_EVERY: Duration = Duration::from_secs(15);

/// The characters of an endpoint's refusal passed on.
const SAID_LENGTH: usize = 500;

/// What the bridge is made of.
#[derive(Clone, Debug)]
pub struct BridgeParts {
    /// Where the provider is saved: read at each request, for its endpoint, key and models.
    pub store: ProviderStore,
    /// The client requests go out with.
    pub client: reqwest::Client,
    /// Where the tokens' and the messages' randomness comes from.
    pub random: Arc<dyn RandomSource>,
}

/// The bridge, listening on loopback; [`Self::serve`] answers on it.
#[derive(Debug)]
pub struct OpenAiBridge {
    listener: std::net::TcpListener,
    access: BridgeAccess,
    state: Bridge,
}

impl OpenAiBridge {
    /// The bridge over `parts`, listening on `127.0.0.1` at a port the system picks (from now:
    /// a request that comes before [`Self::serve`] waits for it).
    ///
    /// # Errors
    ///
    /// The listener's own, when no port can be had.
    pub fn bind(parts: BridgeParts) -> std::io::Result<Self> {
        let listener = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
        listener.set_nonblocking(true)?;
        let address = listener.local_addr()?;
        let grants = Grants {
            random: Arc::clone(&parts.random),
            map: Arc::default(),
        };
        let access = BridgeAccess {
            url: format!("http://{address}"),
            grants: grants.clone(),
        };
        let state = Bridge {
            inner: Arc::new(Inner { parts, grants }),
        };
        Ok(Self {
            listener,
            access,
            state,
        })
    }

    /// What a launch of the agent CLI is given to reach it.
    #[must_use]
    pub fn access(&self) -> BridgeAccess {
        self.access.clone()
    }

    /// The address it listens on.
    ///
    /// # Errors
    ///
    /// The system's, when it won't name it.
    pub fn local_addr(&self) -> std::io::Result<SocketAddr> {
        self.listener.local_addr()
    }

    /// Answers until `stop` resolves.
    ///
    /// # Errors
    ///
    /// The listener's own.
    pub async fn serve(
        self,
        stop: impl Future<Output = ()> + Send + 'static,
    ) -> std::io::Result<()> {
        let listener = TcpListener::from_std(self.listener)?;
        let app = router(self.state).into_make_service_with_connect_info::<SocketAddr>();
        axum::serve(listener, app)
            .with_graceful_shutdown(stop)
            .await
    }
}

/// The bridge's routes, behind its guard: a loopback peer with a live token.
#[expect(
    clippy::disallowed_methods,
    reason = "the bridge serves the agent CLI on its own loopback listener, not the server's \
              documented API, so no OpenAPI document lists its routes"
)]
fn router(state: Bridge) -> Router {
    Router::new()
        .route("/v1/messages", post(messages))
        .route("/v1/messages/count_tokens", post(count_tokens))
        .fallback(not_found)
        .layer(DefaultBodyLimit::max(REQUEST_LIMIT))
        .layer(middleware::from_fn_with_state(state.clone(), guard))
        .with_state(state)
}

/// How the agent CLI reaches the bridge: its address, and a token for each launch. Clones
/// share the tokens.
#[derive(Clone)]
pub struct BridgeAccess {
    url: String,
    grants: Grants,
}

impl fmt::Debug for BridgeAccess {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BridgeAccess")
            .field("url", &self.url)
            .finish_non_exhaustive()
    }
}

impl PartialEq for BridgeAccess {
    fn eq(&self, other: &Self) -> bool {
        self.url == other.url && Arc::ptr_eq(&self.grants.map, &other.grants.map)
    }
}

impl Eq for BridgeAccess {}

impl BridgeAccess {
    /// Its address, the CLI's `ANTHROPIC_BASE_URL`.
    #[must_use]
    pub fn url(&self) -> &str {
        &self.url
    }

    /// A new token for one launch, live while the [`BridgeGrant`] (or a clone of it) is held.
    ///
    /// # Errors
    ///
    /// [`RandomError`] when there is no randomness for it.
    pub fn grant(&self) -> Result<(Token, BridgeGrant), RandomError> {
        self.grants.grant()
    }

    /// Whether `presented` is a live token.
    #[must_use]
    pub fn admits(&self, presented: &str) -> bool {
        self.grants.admits(presented)
    }

    /// An access to a bridge at `url` that isn't served, for tests of what launches get.
    #[cfg(test)]
    pub(crate) fn unserved(url: &str) -> Self {
        Self {
            url: url.to_owned(),
            grants: Grants {
                random: Arc::new(emberhut_core::OsRandom),
                map: Arc::default(),
            },
        }
    }
}

/// A launch's hold on its bridge token: the token is revoked once the last clone is dropped.
/// Its `Debug` shows nothing of the token.
#[derive(Clone)]
pub struct BridgeGrant(Arc<Held>);

impl fmt::Debug for BridgeGrant {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("BridgeGrant(..)")
    }
}

impl PartialEq for BridgeGrant {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for BridgeGrant {}

/// A token's place among the live ones; dropping it revokes the token.
struct Held {
    id: u64,
    map: Weak<Mutex<GrantMap>>,
}

impl Drop for Held {
    fn drop(&mut self) {
        if let Some(map) = self.map.upgrade() {
            lock(&map).live.remove(&self.id);
        }
    }
}

/// The live tokens' hashes.
#[derive(Debug, Default)]
struct GrantMap {
    next: u64,
    live: HashMap<u64, SecretHash>,
}

#[derive(Clone)]
struct Grants {
    random: Arc<dyn RandomSource>,
    map: Arc<Mutex<GrantMap>>,
}

impl Grants {
    fn grant(&self) -> Result<(Token, BridgeGrant), RandomError> {
        let token = Token::generate(self.random.as_ref())?;
        let mut map = lock(&self.map);
        let id = map.next;
        map.next += 1;
        map.live.insert(id, token.hash());
        let held = Held {
            id,
            map: Arc::downgrade(&self.map),
        };
        Ok((token, BridgeGrant(Arc::new(held))))
    }

    /// Whether `presented` is a live token: its hash compared with every live one's, each in
    /// constant time and all of them every time.
    fn admits(&self, presented: &str) -> bool {
        let hash = SecretHash::of(presented);
        let map = lock(&self.map);
        map.live.values().fold(false, |found, live| {
            found | bool::from(live.as_bytes().ct_eq(hash.as_bytes()))
        })
    }
}

fn lock(map: &Mutex<GrantMap>) -> MutexGuard<'_, GrantMap> {
    map.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The bridge's state, shared by its routes.
#[derive(Clone)]
struct Bridge {
    inner: Arc<Inner>,
}

impl fmt::Debug for Bridge {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Bridge").finish_non_exhaustive()
    }
}

struct Inner {
    parts: BridgeParts,
    grants: Grants,
}

/// A refusal, as the Messages API words one.
#[derive(Debug)]
pub(crate) struct Failure {
    status: StatusCode,
    /// Its `error.type`.
    pub(crate) kind: &'static str,
    /// Its `error.message`.
    pub(crate) message: String,
    retry_after: Option<HeaderValue>,
}

impl Failure {
    fn new(status: StatusCode, kind: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            kind,
            message: message.into(),
            retry_after: None,
        }
    }

    /// The request isn't one the bridge can pass on.
    pub(crate) fn invalid(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "invalid_request_error", message)
    }

    fn permission(message: impl Into<String>) -> Self {
        Self::new(StatusCode::FORBIDDEN, "permission_error", message)
    }

    fn upstream(status: StatusCode, message: impl Into<String>) -> Self {
        Self::new(status, "api_error", message)
    }
}

impl IntoResponse for Failure {
    fn into_response(self) -> Response {
        let body =
            json!({ "type": "error", "error": { "type": self.kind, "message": self.message } });
        let mut response = (self.status, axum::Json(body)).into_response();
        if let Some(after) = self.retry_after {
            response.headers_mut().insert(header::RETRY_AFTER, after);
        }
        response
    }
}

/// Lets through a request from a loopback peer that carries a live token, as a bearer or as
/// `x-api-key`; refuses any other.
async fn guard(State(bridge): State<Bridge>, request: Request, next: Next) -> Response {
    let peer = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ConnectInfo(peer)| *peer);
    if !peer.is_some_and(|peer| peer.ip().to_canonical().is_loopback()) {
        return Failure::permission("The OpenAI bridge answers this computer's agent CLI only.")
            .into_response();
    }
    if !token(request.headers()).is_some_and(|token| bridge.inner.grants.admits(token)) {
        return Failure::new(
            StatusCode::UNAUTHORIZED,
            "authentication_error",
            "The OpenAI bridge takes only the token the server gave this agent CLI.",
        )
        .into_response();
    }
    next.run(request).await
}

/// The token `headers` carry: the bearer, or `x-api-key`.
fn token(headers: &HeaderMap) -> Option<&str> {
    let text = |name| {
        headers
            .get(name)
            .and_then(|value: &HeaderValue| value.to_str().ok())
    };
    text(header::AUTHORIZATION.as_str())
        .and_then(|value| value.strip_prefix("Bearer "))
        .or_else(|| text("x-api-key"))
        .map(str::trim)
        .filter(|token| !token.is_empty())
}

async fn messages(State(bridge): State<Bridge>, body: Bytes) -> Response {
    bridge
        .messages(&body)
        .await
        .unwrap_or_else(IntoResponse::into_response)
}

async fn count_tokens(body: Bytes) -> Response {
    match read_request(&body)
        .and_then(|request| request::translate(&request, &request.model, Dialect::Compatible))
    {
        Ok(body) => {
            axum::Json(json!({ "input_tokens": request::estimate_tokens(&body) })).into_response()
        }
        Err(failure) => failure.into_response(),
    }
}

async fn not_found() -> Response {
    Failure::new(
        StatusCode::NOT_FOUND,
        "not_found_error",
        "The OpenAI bridge serves POST /v1/messages and /v1/messages/count_tokens only.",
    )
    .into_response()
}

fn read_request(body: &[u8]) -> Result<MessagesRequest, Failure> {
    serde_json::from_slice(body)
        .map_err(|error| Failure::invalid(format!("That isn't a Messages API request: {error}")))
}

/// The endpoint a request goes to: the provider saved now.
struct Upstream {
    provider: Provider,
    base: String,
    key: Option<ApiKey>,
    model: String,
    fast: String,
}

impl Upstream {
    fn dialect(&self) -> Dialect {
        match self.provider {
            Provider::OpenAi => Dialect::OpenAi,
            _ => Dialect::Compatible,
        }
    }

    /// The model asked for as `asked`: the saved model or quick model when it is one of them,
    /// else the quick one for a `haiku` model and the saved one for any other.
    fn model_for<'a>(&'a self, asked: &'a str) -> &'a str {
        if asked == self.model || asked == self.fast {
            asked
        } else if asked.to_lowercase().contains("haiku") {
            &self.fast
        } else {
            &self.model
        }
    }
}

impl Bridge {
    /// The provider saved now, if the bridge serves it.
    async fn upstream(&self) -> Result<Upstream, Failure> {
        let current = self.inner.parts.store.current().await.map_err(|error| {
            tracing::warn!(%error, "the OpenAI bridge couldn't read the saved provider");
            Failure::upstream(
                StatusCode::INTERNAL_SERVER_ERROR,
                "The server couldn't read the saved model provider.",
            )
        })?;
        let Some(current) = current.filter(|current| current.provider.bridged()) else {
            return Err(Failure::permission(
                "The agent's saved model provider isn't OpenAI or an OpenAI-compatible endpoint \
                 now: the agent runs on the one saved from its next turn.",
            ));
        };
        let provider = current.provider;
        let name = provider.name();
        let Some(base) = current
            .base_url
            .or_else(|| provider.default_url().map(str::to_owned))
        else {
            return Err(Failure::permission(format!(
                "{name} has no base URL saved: save it again."
            )));
        };
        if provider == Provider::OpenAi && current.key.is_none() {
            return Err(Failure::new(
                StatusCode::UNAUTHORIZED,
                "authentication_error",
                format!("{name}'s key isn't saved: save it again."),
            ));
        }
        let Some(model) = current.model else {
            return Err(Failure::permission(format!(
                "No model is saved for {name}: choose one."
            )));
        };
        Ok(Upstream {
            provider,
            base: base.trim_end_matches('/').to_owned(),
            key: current.key,
            fast: current.fast_model.unwrap_or_else(|| model.clone()),
            model,
        })
    }

    /// A new message's id.
    fn message_id(&self) -> Result<String, Failure> {
        let mut bytes = [0_u8; 12];
        self.inner.parts.random.fill(&mut bytes).map_err(|_| {
            Failure::upstream(
                StatusCode::INTERNAL_SERVER_ERROR,
                "The server has no randomness for a message's id.",
            )
        })?;
        let id = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes);
        Ok(format!("msg_bridge_{id}"))
    }

    /// Answers a Messages API request `body` through the saved endpoint.
    async fn messages(&self, body: &[u8]) -> Result<Response, Failure> {
        let request = read_request(body)?;
        let upstream = self.upstream().await?;
        let model = upstream.model_for(&request.model).to_owned();
        let names = ToolNames::of(&request);
        let sent = request::translate(&request, &model, upstream.dialect())?;
        let id = self.message_id()?;
        let name = upstream.provider.name();
        let mut outgoing = self
            .inner
            .parts
            .client
            .post(format!("{}/chat/completions", upstream.base))
            .header(header::CONTENT_TYPE, "application/json")
            .body(sent.to_string());
        if let Some(key) = &upstream.key {
            outgoing = outgoing.bearer_auth(key.as_str());
        }
        let response = match tokio::time::timeout(WAIT, outgoing.send()).await {
            Ok(Ok(response)) => response,
            Ok(Err(error)) => {
                return Err(Failure::upstream(
                    StatusCode::BAD_GATEWAY,
                    format!("{name} couldn't be reached: {}", chain(&error)),
                ));
            }
            Err(_) => {
                return Err(Failure::upstream(
                    StatusCode::GATEWAY_TIMEOUT,
                    format!(
                        "{name} didn't answer within {} minutes.",
                        WAIT.as_secs() / 60
                    ),
                ));
            }
        };
        let status = response.status();
        tracing::debug!(
            provider = upstream.provider.kind().as_str(),
            model,
            stream = request.stream,
            status = status.as_u16(),
            "the OpenAI bridge passed a request on"
        );
        if !status.is_success() {
            let retry_after = response.headers().get(header::RETRY_AFTER).cloned();
            let said = tokio::time::timeout(WAIT, read_body(response, REFUSAL_LIMIT))
                .await
                .ok()
                .and_then(Result::ok)
                .map_or_else(Vec::new, |body| body.bytes);
            let mut failure = refused(upstream.provider, status, &said, upstream.key.as_ref());
            if failure.status == StatusCode::TOO_MANY_REQUESTS {
                failure.retry_after = retry_after;
            }
            return Err(failure);
        }
        if request.stream {
            let translator = Translator::new(id, model, names);
            return Ok(streamed(response, translator, name));
        }
        let whole = tokio::time::timeout(WAIT, read_body(response, REPLY_LIMIT))
            .await
            .ok()
            .and_then(Result::ok)
            .filter(|body| !body.cut)
            .and_then(|body| serde_json::from_slice::<Value>(&body.bytes).ok())
            .filter(|reply| reply.get("choices").is_some_and(Value::is_array))
            .ok_or_else(|| {
                Failure::upstream(
                    StatusCode::BAD_GATEWAY,
                    format!("{name} didn't answer with a chat completion."),
                )
            })?;
        Ok(axum::Json(reply::message(&whole, &id, &model, &names)).into_response())
    }
}

/// The endpoint's refusal (`status`, and the start of its body `said`) as the Messages API's:
///
/// | the endpoint's status | status | `error.type` |
/// |---|---|---|
/// | 400, 422 | 400 | `invalid_request_error` (a context too long reads `prompt is too long`, as the CLI compacts on it) |
/// | 401 | 401 | `authentication_error` |
/// | 403 | 403 | `permission_error` |
/// | 404 | 404 | `not_found_error` |
/// | 413 | 413 | `request_too_large` |
/// | 429 | 429 | `rate_limit_error`, with its `retry-after` |
/// | 503, 529 | 529 | `overloaded_error` |
/// | any other | 502 | `api_error` |
///
/// The message is the endpoint's own (its `error.message`), its key taken out if it quotes it.
fn refused(provider: Provider, status: StatusCode, said: &[u8], key: Option<&ApiKey>) -> Failure {
    let body: Value = serde_json::from_slice(said).unwrap_or(Value::Null);
    let error = body.get("error").unwrap_or(&body);
    let mut text = error
        .get("message")
        .and_then(Value::as_str)
        .or_else(|| error.as_str())
        .map_or_else(|| String::from_utf8_lossy(said).into_owned(), str::to_owned);
    if let Some(key) = key {
        text = text.replace(key.as_str(), "…");
    }
    let text: String = text.trim().chars().take(SAID_LENGTH).collect();
    let code = error
        .get("code")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let too_long =
        code == "context_length_exceeded" || text.to_lowercase().contains("maximum context length");
    let name = provider.name();
    let said = if text.is_empty() {
        format!("{name} answered {}.", status.as_u16())
    } else {
        format!("{name} answered {}: {text}", status.as_u16())
    };
    let (status, kind) = match status.as_u16() {
        400 | 422 if too_long => {
            return Failure::invalid(format!("prompt is too long: {said}"));
        }
        400 | 422 => (StatusCode::BAD_REQUEST, "invalid_request_error"),
        401 => (StatusCode::UNAUTHORIZED, "authentication_error"),
        403 => (StatusCode::FORBIDDEN, "permission_error"),
        404 => (StatusCode::NOT_FOUND, "not_found_error"),
        413 => (StatusCode::PAYLOAD_TOO_LARGE, "request_too_large"),
        429 => (StatusCode::TOO_MANY_REQUESTS, "rate_limit_error"),
        503 | 529 => (OVERLOADED, "overloaded_error"),
        _ => (StatusCode::BAD_GATEWAY, "api_error"),
    };
    Failure::new(status, kind, said)
}

/// Anthropic's status for an overloaded API.
const OVERLOADED: StatusCode = match StatusCode::from_u16(529) {
    Ok(status) => status,
    Err(_) => StatusCode::SERVICE_UNAVAILABLE,
};

/// The endpoint's streamed answer `response` as the Messages API's event stream.
fn streamed(response: reqwest::Response, translator: Translator, name: &'static str) -> Response {
    let queue: VecDeque<Event> = translator.start().into();
    let pump = Pump {
        response,
        translator,
        queue,
        silent: Duration::ZERO,
        name,
    };
    let events = futures_util::stream::unfold(pump, Pump::next);
    let mut response = Response::new(Body::from_stream(events));
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/event-stream"),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    response
}

/// Reads the endpoint's stream on as the CLI reads the bridge's: one event at a time.
struct Pump {
    response: reqwest::Response,
    translator: Translator,
    queue: VecDeque<Event>,
    silent: Duration,
    name: &'static str,
}

impl Pump {
    async fn next(mut self) -> Option<(Result<Bytes, Infallible>, Self)> {
        loop {
            if let Some(event) = self.queue.pop_front() {
                return Some((Ok(Bytes::from(event.to_sse())), self));
            }
            if self.translator.done() {
                return None;
            }
            match tokio::time::timeout(PING_EVERY, self.response.chunk()).await {
                Ok(Ok(Some(bytes))) => {
                    self.silent = Duration::ZERO;
                    let events = self.translator.feed(&bytes);
                    self.queue.extend(events);
                }
                Ok(Ok(None)) => {
                    let events = self.translator.end();
                    self.queue.extend(events);
                }
                Ok(Err(error)) => {
                    let message =
                        format!("The connection to {} broke: {}", self.name, chain(&error));
                    let event = self.translator.fail("api_error", &message);
                    self.queue.push_back(event);
                }
                Err(_) => {
                    self.silent += PING_EVERY;
                    let event = if self.silent >= WAIT {
                        let message = format!(
                            "{} sent nothing for {} minutes.",
                            self.name,
                            WAIT.as_secs() / 60
                        );
                        self.translator.fail("api_error", &message)
                    } else {
                        Event::ping()
                    };
                    self.queue.push_back(event);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use axum::http::Request as HttpRequest;
    use emberhut_core::OsRandom;
    use http_body_util::BodyExt as _;
    use tower::ServiceExt as _;

    use super::*;
    use crate::services::messaging::secrets::Secrets;

    async fn bridge(dir: &tempfile::TempDir) -> OpenAiBridge {
        let url: emberhut_db::DatabaseUrl =
            emberhut_db::DatabaseUrl::from_path(&dir.path().join("emberhut.db")).unwrap();
        let db = emberhut_db::Database::connect(&url).await.unwrap();
        OpenAiBridge::bind(BridgeParts {
            store: ProviderStore::new(Secrets::new(dir.path().join("secrets")), db.settings()),
            client: crate::http::shared_client().unwrap(),
            random: Arc::new(OsRandom),
        })
        .unwrap()
    }

    /// `router`'s answer to a `count_tokens` request from `peer` with `token`.
    async fn asked(router: Router, peer: Option<&str>, token: Option<&str>) -> (u16, Value) {
        let mut request = HttpRequest::post("/v1/messages/count_tokens?beta=true")
            .header("content-type", "application/json");
        if let Some(token) = token {
            request = request.header("authorization", format!("Bearer {token}"));
        }
        let mut request = request
            .body(Body::from(
                r#"{"model":"m","messages":[{"role":"user","content":"Hello there"}]}"#,
            ))
            .unwrap();
        if let Some(peer) = peer {
            let peer: SocketAddr = peer.parse().unwrap();
            request.extensions_mut().insert(ConnectInfo(peer));
        }
        let response = router.oneshot(request).await.unwrap();
        let status = response.status().as_u16();
        let body = response.into_body().collect().await.unwrap().to_bytes();
        (status, serde_json::from_slice(&body).unwrap())
    }

    #[tokio::test]
    async fn it_listens_on_loopback_only() {
        let dir = tempfile::tempdir().unwrap();
        let bridge = bridge(&dir).await;
        let address = bridge.local_addr().unwrap();
        assert!(address.ip().is_loopback(), "{address}");
        assert_ne!(address.port(), 0);
        assert_eq!(bridge.access().url(), format!("http://{address}"));
    }

    #[tokio::test]
    async fn only_a_loopback_peer_with_a_live_token_gets_in() {
        let dir = tempfile::tempdir().unwrap();
        let bridge = bridge(&dir).await;
        let access = bridge.access();
        let (token, grant) = access.grant().unwrap();
        let router = router(bridge.state.clone());
        let token = token.as_str();

        let (status, body) = asked(router.clone(), Some("127.0.0.1:50000"), Some(token)).await;
        assert_eq!(status, 200, "{body}");
        assert!(body["input_tokens"].as_u64().unwrap() > 0);
        let (status, _) = asked(router.clone(), Some("[::1]:50000"), Some(token)).await;
        assert_eq!(status, 200, "IPv6 loopback");

        for peer in [Some("192.0.2.7:50000"), Some("[::ffff:192.0.2.7]:1"), None] {
            let (status, body) = asked(router.clone(), peer, Some(token)).await;
            assert_eq!(status, 403, "{peer:?}");
            assert_eq!(body["error"]["type"], "permission_error");
        }
        for wrong in [None, Some("not-the-token"), Some("")] {
            let (status, body) = asked(router.clone(), Some("127.0.0.1:1"), wrong).await;
            assert_eq!(status, 401, "{wrong:?}");
            assert_eq!(body["type"], "error");
            assert_eq!(body["error"]["type"], "authentication_error");
        }

        // A clone holds it too; the last one dropped revokes it.
        let held = grant.clone();
        drop(grant);
        assert!(access.admits(token));
        drop(held);
        assert!(!access.admits(token));
        let (status, _) = asked(router, Some("127.0.0.1:1"), Some(token)).await;
        assert_eq!(status, 401, "revoked once its launch is gone");
    }

    #[test]
    fn tokens_are_their_own_and_never_shown() {
        let grants = Grants {
            random: Arc::new(OsRandom),
            map: Arc::default(),
        };
        let (one, held_one) = grants.grant().unwrap();
        let (two, _held_two) = grants.grant().unwrap();
        assert_ne!(one.as_str(), two.as_str());
        assert!(grants.admits(one.as_str()) && grants.admits(two.as_str()));
        assert_eq!(format!("{held_one:?}"), "BridgeGrant(..)");
        drop(held_one);
        assert!(!grants.admits(one.as_str()) && grants.admits(two.as_str()));
    }

    #[test]
    fn an_endpoint_s_refusal_is_the_messages_api_s() {
        let key = ApiKey::new("sk-proj-not-a-real-one");
        let refusal = |status: u16, said: &str| {
            refused(
                Provider::OpenAi,
                StatusCode::from_u16(status).unwrap(),
                said.as_bytes(),
                Some(&key),
            )
        };
        // (the endpoint's status and body, the status passed on, its type)
        for (status, said, passed, kind) in [
            (
                400,
                r#"{"error":{"message":"bad"}}"#,
                400,
                "invalid_request_error",
            ),
            (422, "unprocessable", 400, "invalid_request_error"),
            (
                401,
                r#"{"error":{"message":"no"}}"#,
                401,
                "authentication_error",
            ),
            (403, "", 403, "permission_error"),
            (
                404,
                r#"{"error":{"message":"The model `x` does not exist"}}"#,
                404,
                "not_found_error",
            ),
            (413, "", 413, "request_too_large"),
            (
                429,
                r#"{"error":{"message":"Rate limit reached"}}"#,
                429,
                "rate_limit_error",
            ),
            (500, "oops", 502, "api_error"),
            (503, "busy", 529, "overloaded_error"),
        ] {
            let failure = refusal(status, said);
            assert_eq!(failure.status.as_u16(), passed, "{status}");
            assert_eq!(failure.kind, kind, "{status}");
            assert!(
                failure
                    .message
                    .starts_with(&format!("OpenAI answered {status}"))
            );
        }
        let quoted = refusal(
            401,
            r#"{"error":{"message":"Incorrect API key provided: sk-proj-not-a-real-one."}}"#,
        );
        assert_eq!(
            quoted.message,
            "OpenAI answered 401: Incorrect API key provided: …."
        );
        let long = refusal(
            400,
            r#"{"error":{"message":"This model's maximum context length is 8192 tokens.",
                "code":"context_length_exceeded"}}"#,
        );
        assert!(
            long.message.starts_with("prompt is too long: "),
            "{}",
            long.message
        );
    }

    #[test]
    fn models_the_cli_has_built_in_map_to_the_saved_ones() {
        let upstream = Upstream {
            provider: Provider::OpenAiCompatible,
            base: "http://127.0.0.1:9".to_owned(),
            key: None,
            model: "big".to_owned(),
            fast: "small".to_owned(),
        };
        assert_eq!(upstream.model_for("big"), "big");
        assert_eq!(upstream.model_for("small"), "small");
        assert_eq!(upstream.model_for("claude-haiku-4-5-20251001"), "small");
        assert_eq!(upstream.model_for("claude-opus-5-5"), "big");
        assert_eq!(upstream.dialect(), Dialect::Compatible);
    }
}
