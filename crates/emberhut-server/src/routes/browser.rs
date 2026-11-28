//! The browser the assistant drives, as the owner's app sees it (`/api/browser/*`), and the owner's
//! Chrome profiles: the app's changes (`POST /api/chrome`), the extension's WebSocket
//! (`/api/chrome/ws`) and the pages a profile opens to pair (`/chrome/*`, no sign-in: the
//! pairing code is the key).

use axum::Json;
use axum::body::{Body, Bytes};
use axum::extract::rejection::QueryRejection;
use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use std::time::Duration;

use emberhut_api::browser::{BrowserReply, BrowserSessionReply, ChromePairQuery, ChromePairStatus};
use emberhut_api::chrome::ChromeReply;
use emberhut_api::error::ErrorBody;
use futures_util::stream::SplitStream;
use futures_util::{SinkExt as _, StreamExt as _};
use serde_json::Value;
use tokio::sync::{mpsc, oneshot};

use crate::app::AppState;
use crate::error::ApiError;
use crate::extract::{SignedIn, Viewer};
use crate::services::browser::BrowserInput;
use crate::services::chrome::{ChromeError, ChromeInput, ChromeService, Outgoing};

/// `GET /api/browser/session`'s query.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Deserialize)]
pub(super) struct BrowserSessionQuery {
    /// The device asking: the one that took over gets control.
    #[serde(default)]
    pub(super) device: Option<String>,
}

/// `POST /api/browser/upload`'s query.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Deserialize)]
pub(super) struct BrowserUploadQuery {
    /// The file's name.
    #[serde(default)]
    pub(super) name: Option<String>,
}

/// The biggest file the app's picker sends a page: 100 MB.
const UPLOAD_LIMIT: u64 = 100_000_000;

/// A refusal in the routes' `{ ok: false, error }` shape, with its status; or the server's own
/// answer (no session).
pub(super) enum Refused {
    /// Refused, as `error` says.
    Said(StatusCode, String),
    /// The server's own answer.
    Api(ApiError),
}

impl From<ApiError> for Refused {
    fn from(error: ApiError) -> Self {
        Self::Api(error)
    }
}

impl IntoResponse for Refused {
    fn into_response(self) -> Response {
        match self {
            Self::Said(status, error) => (
                status,
                Json(ErrorBody {
                    ok: Some(false),
                    error,
                    sign_in: false,
                }),
            )
                .into_response(),
            Self::Api(error) => error.into_response(),
        }
    }
}

fn conflict(error: impl std::fmt::Display) -> Refused {
    let text = error.to_string();
    Refused::Said(
        StatusCode::CONFLICT,
        text.split('\n').next().unwrap_or_default().to_owned(),
    )
}

/// The body as JSON (empty: `{}`), read as `T`, leniently: a field of the wrong kind is left
/// out.
fn read<T: serde::de::DeserializeOwned + Default>(body: &Bytes) -> Result<T, Refused> {
    if body.iter().all(u8::is_ascii_whitespace) {
        return Ok(T::default());
    }
    let value: Value =
        serde_json::from_slice(body).map_err(|_| conflict("The request isn't JSON"))?;
    Ok(serde_json::from_value(value).unwrap_or_default())
}

/// The live view's address and password: the control one when this device took over, else
/// the view one.
#[utoipa::path(
    get,
    path = "/api/browser/session",
    tag = "browser",
    security(("bearer" = [])),
    params(emberhut_api::browser::BrowserSessionQuery),
    responses(
        (status = 200, description = "How to reach the live view", body = BrowserSessionReply),
        (status = 401, description = "No signed-in device (in either auth mode)", body = ErrorBody),
        (status = 503, description = "The live view isn't set up on this server", body = ErrorBody),
    )
)]
pub(super) async fn session(
    State(app): State<AppState>,
    device: Result<SignedIn, ApiError>,
    query: Result<Query<BrowserSessionQuery>, QueryRejection>,
) -> Result<Json<BrowserSessionReply>, Refused> {
    // A password to watch or drive the logged-in browser: a signed-in device only.
    super::acting(device)?;
    let device = query.ok().and_then(|Query(q)| q.device);
    app.browser()
        .session(device.as_deref())
        .await
        .map(Json)
        .map_err(|e| Refused::Said(StatusCode::SERVICE_UNAVAILABLE, e.to_string()))
}

/// The latest picture of the browser's tab, for a card; none while the owner drives.
#[utoipa::path(
    get,
    path = "/api/browser/frame.jpg",
    tag = "browser",
    security(("bearer" = [])),
    responses(
        (status = 200, description = "The picture", body = Vec<u8>, content_type = "image/jpeg"),
        (status = 401, description = "Sign-in is enforced and there is no session", body = ErrorBody),
        (status = 404, description = "No picture now: no browser, or the owner drives", body = String, content_type = "text/plain"),
    )
)]
pub(super) async fn frame(State(app): State<AppState>, _viewer: Viewer) -> Response {
    let Some(frame) = app.browser().frame().await else {
        return (StatusCode::NOT_FOUND, "No picture now").into_response();
    };
    let mut response = Response::new(Body::from(frame));
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("image/jpeg"));
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

/// Take over, hand back, and while the owner drives, the live view's toolbar.
#[utoipa::path(
    post,
    path = "/api/browser/input",
    tag = "browser",
    security(("bearer" = [])),
    request_body = emberhut_api::browser::BrowserInput,
    responses(
        (status = 200, description = "Done: the browser now", body = BrowserReply),
        (status = 401, description = "No signed-in device (in either auth mode)", body = ErrorBody),
        (status = 409, description = "Not done: not taken over first, or not an input", body = ErrorBody),
    )
)]
pub(super) async fn input(
    State(app): State<AppState>,
    device: Result<SignedIn, ApiError>,
    body: Bytes,
) -> Result<Json<BrowserReply>, Refused> {
    // Drives the logged-in browser: a signed-in device only.
    super::acting(device)?;
    let input: BrowserInput = read(&body)?;
    let browser = app.browser().input(&input).await.map_err(conflict)?;
    Ok(Json(BrowserReply { ok: true, browser }))
}

/// The file the page asked for (`fileRequest`), as the body: into the page's file field.
#[utoipa::path(
    post,
    path = "/api/browser/upload",
    tag = "browser",
    security(("bearer" = [])),
    params(emberhut_api::uploads::UploadQuery),
    request_body(content = Vec<u8>, content_type = "application/octet-stream", description = "The file itself, raw"),
    responses(
        (status = 200, description = "The page has it: the browser now", body = BrowserReply),
        (status = 401, description = "No signed-in device (in either auth mode)", body = ErrorBody),
        (status = 409, description = "Not taken: the page isn't asking for a file, or it is over 100 MB", body = ErrorBody),
    )
)]
pub(super) async fn upload(
    State(app): State<AppState>,
    device: Result<SignedIn, ApiError>,
    query: Result<Query<BrowserUploadQuery>, QueryRejection>,
    body: Body,
) -> Result<Json<BrowserReply>, Refused> {
    // Puts a file into the logged-in browser's page: a signed-in device only.
    super::acting(device)?;
    let name = query
        .ok()
        .and_then(|Query(q)| q.name)
        .unwrap_or_else(|| "file".to_owned());
    let browser = app
        .browser()
        .upload(&name, body.into_data_stream(), UPLOAD_LIMIT)
        .await
        .map_err(conflict)?;
    Ok(Json(BrowserReply { ok: true, browser }))
}

/// The owner's Chrome profiles: add (a pairing link), repair, rename, unpair, cancel a link,
/// hand back, pause the assistant there.
#[utoipa::path(
    post,
    path = "/api/chrome",
    tag = "browser",
    security(("bearer" = [])),
    request_body = emberhut_api::chrome::ChromeAction,
    responses(
        (status = 200, description = "Done: the profiles now", body = ChromeReply),
        (status = 401, description = "No signed-in device (in either auth mode)", body = ErrorBody),
        (status = 409, description = "Not done, as `error` says", body = ErrorBody),
    )
)]
pub(super) async fn chrome(
    State(app): State<AppState>,
    device: Result<SignedIn, ApiError>,
    body: Bytes,
) -> Result<Json<ChromeReply>, Refused> {
    // A pairing link hands a profile a lasting token: a signed-in device only.
    super::acting(device)?;
    let input: ChromeInput = read(&body)?;
    match app.chrome().input(&input).await {
        Ok(reply) => Ok(Json(reply)),
        Err(ChromeError::Refused(why)) => Err(conflict(why)),
        Err(error) => {
            tracing::error!(%error, "a Chrome profile change failed");
            Err(Refused::Said(
                StatusCode::CONFLICT,
                "The server couldn't do that now.".to_owned(),
            ))
        }
    }
}

/// The browser extension's WebSocket: its first message pairs it (a link's code) or says hello
/// (its own token); then it relays the browser tools' commands.
#[utoipa::path(
    get,
    path = "/api/chrome/ws",
    tag = "browser",
    responses(
        (status = 400, description = "Not a WebSocket upgrade (an upgrade switches protocols, 101)", body = ErrorBody),
    )
)]
pub(super) async fn chrome_ws(
    State(app): State<AppState>,
    headers: HeaderMap,
    upgrade: Result<WebSocketUpgrade, axum::extract::ws::rejection::WebSocketUpgradeRejection>,
) -> Response {
    let Ok(upgrade) = upgrade else {
        return Refused::Said(
            StatusCode::BAD_REQUEST,
            "Not a WebSocket request".to_owned(),
        )
        .into_response();
    };
    let origin = headers
        .get(header::ORIGIN)
        .and_then(|o| o.to_str().ok())
        .map(str::to_owned);
    let chrome = app.chrome().clone();
    upgrade
        .max_message_size(64 * 1024 * 1024)
        .on_upgrade(move |socket| relay(socket, chrome, origin))
}

/// The most a socket's first message (its pairing or its hello) may be. Until the hello
/// succeeded nothing else is read; a bigger first message closes the socket (1009).
const FIRST_MESSAGE_MAX: usize = 8 * 1024;

/// How long a socket has to send its first message.
const FIRST_MESSAGE_WAIT: Duration = Duration::from_secs(10);

/// The socket's next text message (none: it closed or broke). Anything but text before the
/// hello is not the extension's.
async fn next_text(stream: &mut SplitStream<WebSocket>) -> Option<Result<String, &'static str>> {
    loop {
        match stream.next().await? {
            Ok(Message::Text(text)) => return Some(Ok(text.as_str().to_owned())),
            Ok(Message::Binary(_)) => return Some(Err("Not JSON")),
            Ok(Message::Close(_)) | Err(_) => return None,
            Ok(_) => {}
        }
    }
}

/// One extension's socket: its messages to the service, the service's back. Its first
/// message must be small and come soon; nothing more is read before its hello succeeded. When
/// the socket ends, whichever side ended it, the service lets go of the profile (it is no
/// longer listed online).
async fn relay(socket: WebSocket, chrome: ChromeService, origin: Option<String>) {
    let (mut sink, mut stream) = socket.split();
    let (to_service, from_socket) = mpsc::unbounded_channel::<String>();
    let (to_socket, mut outgoing) = mpsc::unbounded_channel::<Outgoing>();
    let (said_hello, hello_said) = oneshot::channel::<()>();
    let mut writer = tokio::spawn(async move {
        while let Some(out) = outgoing.recv().await {
            match out {
                Outgoing::Text(text) => {
                    if sink.send(Message::Text(text.into())).await.is_err() {
                        break;
                    }
                }
                Outgoing::Close(code, reason) => {
                    let reason: String = reason.chars().take(120).collect();
                    drop(
                        sink.send(Message::Close(Some(CloseFrame {
                            code,
                            reason: reason.into(),
                        })))
                        .await,
                    );
                    break;
                }
            }
        }
    });
    let closer = to_socket.clone();
    let reader = tokio::spawn(async move {
        let close = |code: u16, why: &str| drop(closer.send(Outgoing::Close(code, why.to_owned())));
        match tokio::time::timeout(FIRST_MESSAGE_WAIT, next_text(&mut stream)).await {
            Ok(Some(Ok(first))) if first.len() <= FIRST_MESSAGE_MAX => {
                if to_service.send(first).is_err() {
                    return;
                }
            }
            Ok(Some(Ok(_))) => return close(1009, "The first message is too big"),
            Ok(Some(Err(why))) => return close(1003, why),
            Ok(None) => return,
            Err(_) => return close(1008, "No hello"),
        }
        if hello_said.await.is_err() {
            return;
        }
        while let Some(Ok(message)) = stream.next().await {
            match message {
                Message::Text(text) => {
                    if to_service.send(text.as_str().to_owned()).is_err() {
                        break;
                    }
                }
                Message::Close(_) => break,
                _ => {}
            }
        }
    });
    let serve = chrome.serve(from_socket, to_socket, origin, None, Some(said_hello));
    tokio::pin!(serve);
    tokio::select! {
        () = &mut serve => {}
        _ = &mut writer => {
            // The server closed the socket (or it broke): its messages stop, and the service
            // runs its goodbye.
            reader.abort();
            serve.await;
            return;
        }
    }
    reader.abort();
}

/// The page a Chrome profile opens to pair: it waits for the extension there, or shows how to
/// add it, then says it paired.
#[utoipa::path(
    get,
    path = "/chrome/pair",
    tag = "browser",
    responses((status = 200, description = "The pairing page (its code after `#`)", body = String, content_type = "text/html")),
)]
pub(super) async fn pair_page() -> Response {
    let page = PAIR_PAGE
        .replace("{STORE}", emberhut_api::chrome::extension::STORE_PAGE)
        .replace("{ZIP}", emberhut_api::paths::CHROME_EXTENSION_ZIP);
    let mut response = Response::new(Body::from(page));
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

/// How a pairing link stands: waiting, paired (as what), or expired.
#[utoipa::path(
    get,
    path = "/chrome/pair/status",
    tag = "browser",
    params(ChromePairQuery),
    responses((status = 200, description = "The link's state", body = ChromePairStatus)),
)]
pub(super) async fn pair_status(
    State(app): State<AppState>,
    query: Result<Query<ChromePairQuery>, QueryRejection>,
) -> Response {
    let code = query.ok().and_then(|Query(q)| q.code).unwrap_or_default();
    let mut response = Json(app.chrome().pair_status(&code).await).into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

/// The browser extension, as a zip to load unpacked (`emberhut-extension-<version>.zip`).
#[utoipa::path(
    get,
    path = "/chrome/emberhut-extension.zip",
    tag = "browser",
    responses(
        (status = 200, description = "The extension", body = Vec<u8>, content_type = "application/zip"),
        (status = 404, description = "No extension build is set up on this server", body = String, content_type = "text/plain"),
    )
)]
pub(super) async fn emberhut_extension_zip(State(app): State<AppState>) -> Response {
    zip_response(app.chrome())
}

/// The zip, with its download name; 404 without one.
fn zip_response(chrome: &ChromeService) -> Response {
    let Some((bytes, name)) = chrome.extension_zip() else {
        return (
            StatusCode::NOT_FOUND,
            "No extension build is set up on this server",
        )
            .into_response();
    };
    let mut response = Response::new(Body::from(bytes));
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/zip"),
    );
    if let Ok(disposition) = HeaderValue::from_str(&format!("attachment; filename=\"{name}\"")) {
        headers.insert(header::CONTENT_DISPOSITION, disposition);
    }
    response
}

/// The pairing page: it polls its link's state, and shows how to add the extension (from the
/// Chrome Web Store; loading a build unpacked is the developers' way) when it doesn't come.
const PAIR_PAGE: &str = r#"<!doctype html><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>Pair this Chrome profile with Emberhut</title>
<style>
  :root { color-scheme: light dark; --p: #3b6cf6; --ok: #2e9e5b; --bad: #d64545; }
  body { font: 15px/1.45 -apple-system, BlinkMacSystemFont, "Segoe UI", system-ui, sans-serif; max-width: 560px; margin: 48px auto; padding: 0 20px; }
  h1 { font-size: 24px; margin: 0 0 4px; } .sub { color: #888; margin: 0 0 24px; }
  .status { display: flex; gap: 12px; align-items: center; padding: 16px; border-radius: 14px; background: color-mix(in srgb, var(--p) 10%, transparent); font-weight: 600; }
  .dot { width: 12px; height: 12px; border-radius: 50%; background: var(--p); animation: p 1.2s infinite; flex: none; } @keyframes p { 50% { opacity: .3 } }
  .ok .dot { background: var(--ok); animation: none; } .bad .dot { background: var(--bad); animation: none; }
  ol { padding-left: 22px; } li { margin: 0 0 14px; } li b { display: block; }
  a.btn { font-weight: 600; display: inline-block; margin-top: 6px; padding: 7px 14px; border-radius: 9px; background: var(--p); color: #fff; text-decoration: none; }
  code { font: 13px ui-monospace, Menlo, monospace; background: color-mix(in srgb, currentColor 8%, transparent); padding: 2px 6px; border-radius: 5px; }
  .fine { color: #888; font-size: 13px; margin-top: 28px; } .hide { display: none; }
</style>
<h1>Pair this Chrome profile with Emberhut</h1>
<p class="sub" id="as">…</p>
<div class="status" id="status"><span class="dot"></span><span id="line">Looking for the Emberhut extension in this profile…</span></div>
<section id="install" class="hide">
  <p>The Emberhut extension isn't in this Chrome profile yet. Add it once; this page pairs as soon as it's there.</p>
  <ol>
    <li><b>Add it from the Chrome Web Store</b><a class="btn" href="{STORE}" target="_blank" rel="noopener">Add Emberhut to Chrome</a></li>
    <li><b>Come back to this tab</b>The extension finds this page and asks before it pairs: it says what it sends to your hut, and you press Pair.</li>
  </ol>
  <details class="fine"><summary>Developing Emberhut?</summary>Load your own build instead: <a href="{ZIP}">download it</a> (or build <code>apps/extension</code>), unzip it into a folder that stays, then in <code>chrome://extensions</code> turn on Developer mode and click Load unpacked. Remove the store copy first.</details>
</section>
<p class="fine">Once paired, Emberhut works in this profile when it needs to (Pause on its icon lets it go for 10 minutes), only in its own “Emberhut” tab group, never with passwords, and it asks you before any final step. Code for Emberhut's popup: <code id="code"></code></p>
<script>
  const code = decodeURIComponent(location.hash.slice(1)).trim().toUpperCase(), $ = (id) => document.getElementById(id);
  $("code").textContent = code || "none";
  const started = Date.now();
  const set = (cls, text) => { $("status").className = "status " + cls; $("line").textContent = text; };
  async function tick() {
    let s = { state: "expired" };
    try { s = await (await fetch("/chrome/pair/status?code=" + encodeURIComponent(code), { cache: "no-store" })).json(); } catch { return setTimeout(tick, 3000); }
    if (s.label) $("as").textContent = "As “" + s.label + "”";
    if (s.state === "paired") { $("install").classList.add("hide"); set("ok", "Paired as “" + s.label + "”" + (s.email ? " · " + s.email : "") + ". You can close this tab."); return; }
    if (s.state === "expired") { $("install").classList.add("hide"); set("bad", "This link was used or has expired. Get a new one in the Emberhut app: Settings, Chrome profiles."); return; }
    if (Date.now() - started > 3000) { $("install").classList.remove("hide"); set("", "Waiting for the Emberhut extension in this profile…"); }
    setTimeout(tick, 1500);
  }
  addEventListener("hashchange", () => location.reload());
  if (!code) set("bad", "This link has no code. Open the whole link from the Emberhut app."); else tick();
</script>
"#;
