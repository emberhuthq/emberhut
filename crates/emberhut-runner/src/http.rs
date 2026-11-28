//! The runner's HTTP API, which its server calls with the runner's token: open a session, send
//! it a turn, stop or interrupt it, change its settings, read its transcript or watch it live,
//! the folders and branches for a new session, the accounts and their use, the background jobs,
//! files in and out, and the switched-off MCP servers and skills. `/health` answers without the
//! token. A refusal is a status and `{ "error": "<words for a person>" }`.
//!
//! The routes are the runner protocol's (ADR 0017), not the server's documented API, so they
//! are matched here by method and path in one handler rather than listed in the API document.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::{Query, State};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use futures_util::StreamExt as _;
use serde_json::{Map, Value, json};
use subtle::ConstantTimeEq as _;

use crate::accounts::Accounts;
use crate::config::Config;
use crate::files::Files;
use crate::sessions::engine::Sessions;
use crate::sessions::folders;
use crate::store::Store;
use crate::wire::{Refused, live};

/// The largest JSON body taken (a prompt with a long brief fits many times over).
const MAX_BODY: usize = 4 * 1024 * 1024;
/// How often an open live stream gets a comment, so nothing between closes it as idle.
const PING: Duration = Duration::from_secs(15);

/// What the API needs.
#[derive(Debug)]
pub struct Api {
    /// The settings.
    pub config: Arc<Config>,
    /// The store.
    pub store: Store,
    /// The sessions.
    pub sessions: Arc<Sessions>,
    /// The accounts.
    pub accounts: Arc<Accounts>,
    /// The file rules.
    pub files: Arc<Files>,
    /// The machine's latest sample.
    pub machine: Arc<Mutex<Option<Value>>>,
    /// When the runner started.
    pub started: String,
}

/// The API as a router: everything goes to one handler.
pub fn router(api: Arc<Api>) -> Router {
    Router::new().fallback(handle).with_state(api)
}

fn reply(status: StatusCode, body: &Value) -> Response {
    (
        status,
        [(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        )],
        body.to_string(),
    )
        .into_response()
}

fn ok(body: &Value) -> Response {
    reply(StatusCode::OK, body)
}

fn refusal(r: &Refused) -> Response {
    reply(
        StatusCode::from_u16(r.status).unwrap_or(StatusCode::BAD_REQUEST),
        &json!({ "error": r.message }),
    )
}

/// A JSON object from a request body: `{}` when empty.
fn object(bytes: &[u8]) -> Result<Map<String, Value>, Refused> {
    if bytes.iter().all(u8::is_ascii_whitespace) {
        return Ok(Map::new());
    }
    match serde_json::from_slice::<Value>(bytes) {
        Ok(Value::Object(m)) => Ok(m),
        Ok(_) => Err(Refused::bad("not a JSON object")),
        Err(_) => Err(Refused::bad("not JSON")),
    }
}

async fn json_body(body: Body) -> Result<Map<String, Value>, Refused> {
    let bytes = axum::body::to_bytes(body, MAX_BODY)
        .await
        .map_err(|_| Refused::new(413, "the request is too big"))?;
    object(&bytes)
}

/// A positive integer from a query value, or the default (empty, missing or not a number).
fn int(q: &HashMap<String, String>, k: &str, fallback: u64) -> u64 {
    q.get(k)
        .and_then(|v| v.trim().parse::<f64>().ok())
        .filter(|n| n.is_finite() && *n >= 1.0)
        .map_or(fallback, |n| {
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "a positive whole number from a query, truncated as the protocol says"
            )]
            let whole = n.trunc() as u64;
            whole
        })
}

/// Why a request without this runner's token is turned away, or `None` when it has it.
fn unauthorized(api: &Api, headers: &HeaderMap) -> Option<&'static str> {
    let Some(token) = api.config.token() else {
        return Some("this runner isn't paired yet");
    };
    let given = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    let given = given
        .get(..7)
        .filter(|p| p.eq_ignore_ascii_case("bearer "))
        .map_or("", |_| given[7..].trim());
    (!bool::from(given.as_bytes().ct_eq(token.as_bytes()))).then_some("not this runner's token")
}

/// Answers one request.
async fn handle(
    State(api): State<Arc<Api>>,
    method: Method,
    uri: Uri,
    Query(q): Query<HashMap<String, String>>,
    headers: HeaderMap,
    body: Body,
) -> Response {
    let path = uri.path().to_owned();
    if path == "/health" {
        return ok(
            &json!({ "ok": true, "machine": api.config.name, "version": crate::version(), "running": api.sessions.running_count().await }),
        );
    }
    if let Some(why) = unauthorized(&api, &headers) {
        return reply(StatusCode::UNAUTHORIZED, &json!({ "error": why }));
    }
    match route(&api, &method, &path, &q, &headers, body).await {
        Ok(r) => r,
        Err(r) => refusal(&r),
    }
}

async fn route(
    api: &Api,
    method: &Method,
    path: &str,
    q: &HashMap<String, String>,
    headers: &HeaderMap,
    body: Body,
) -> Result<Response, Refused> {
    let name = api.config.name.as_str();
    match (method.as_str(), path) {
        ("GET", "/info") => Ok(ok(&json!({
            "machine": name, "version": crate::version(), "started": api.started,
            "platform": std::env::consts::OS, "arch": std::env::consts::ARCH, "running": api.sessions.running_count().await,
        }))),
        ("POST", "/sessions") => Ok(ok(&api.sessions.open(&json_body(body).await?).await?)),
        ("GET", "/sessions") => {
            let limit = int(q, "limit", 100).min(500);
            let mut list = Vec::new();
            for s in api.store.sessions(limit).await.unwrap_or_default() {
                if let Some(h) = api.sessions.head_of(&s.id).await {
                    list.push(h);
                }
            }
            Ok(ok(&json!({ "sessions": list })))
        }
        ("GET", "/machine") => Ok(ok(&api
            .machine
            .lock()
            .ok()
            .and_then(|m| m.clone())
            .unwrap_or_else(|| json!({})))),
        ("POST", "/off") => {
            let off = api.sessions.set_off(&Value::Object(json_body(body).await?));
            Ok(ok(
                &json!({ "ok": true, "mcp": off.mcp, "skills": off.skills }),
            ))
        }
        ("GET", "/accounts") => Ok(ok(
            &json!({ "machine": name, "accounts": api.accounts.summary() }),
        )),
        ("POST", "/accounts/check") => {
            api.accounts.probe_all().await;
            Ok(ok(&json!({ "accounts": api.accounts.summary() })))
        }
        ("GET", "/jobs") => Ok(ok(&crate::jobs::scan(api.config.job_labels.as_ref()).await)),
        ("POST", "/jobs/stop") => stop_job(&json_body(body).await?).await,
        ("GET", "/files") => send_file(api, q).await,
        ("GET", "/folders") => {
            let home = api.config.user.clone();
            match q.get("path").filter(|p| !p.is_empty()) {
                Some(p) => Ok(ok(&folders::folders_in(p, name, &home).await?)),
                None => Ok(ok(&folders::start_folders(&api.store, name, &home).await)),
            }
        }
        ("GET", "/branches") => Ok(ok(&folders::branches(
            q.get("cwd").map_or("", String::as_str),
        )
        .await?)),
        _ => {
            let Some(rest) = path.strip_prefix("/sessions/") else {
                return Err(Refused::missing("not found"));
            };
            let (id, action) = rest
                .split_once('/')
                .map_or((rest, None), |(i, a)| (i, Some(a)));
            if !id.starts_with("r-")
                || id.len() < 3
                || !id[2..]
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            {
                return Err(Refused::missing("not found"));
            }
            session(api, id, action, method, q, headers, body).await
        }
    }
}

async fn session(
    api: &Api,
    id: &str,
    action: Option<&str>,
    method: &Method,
    q: &HashMap<String, String>,
    headers: &HeaderMap,
    body: Body,
) -> Result<Response, Refused> {
    let name = api.config.name.as_str();
    let s = api
        .store
        .session(id)
        .await
        .ok()
        .flatten()
        .ok_or_else(|| Refused::missing(format!("no session {id} on {name}")))?;
    match (method.as_str(), action) {
        ("GET", None) => {
            let mut head = api.sessions.head_of(id).await.unwrap_or(Value::Null);
            let turns: Vec<Value> = api
                .store
                .turns(id, 50)
                .await
                .unwrap_or_default()
                .iter()
                .map(|t| Sessions::turn_shape(Some(t)))
                .collect();
            if let Value::Object(h) = &mut head {
                h.insert("turns".to_owned(), Value::Array(turns));
            }
            Ok(ok(&head))
        }
        ("GET", Some("transcript")) => {
            // after 0 (or empty): the newest `limit` steps and whether older ones exist; else the
            // steps after it.
            let after = int(q, "after", 0);
            let limit = int(q, "limit", 400).min(1000);
            let after_seq = i64::try_from(after).unwrap_or(i64::MAX);
            let rows = if after > 0 {
                api.store.events_after(id, after_seq, limit).await
            } else {
                api.store.events_newest(id, limit).await
            }
            .unwrap_or_default();
            let earlier = match rows.first() {
                Some(first) if after == 0 => api
                    .store
                    .has_events_before(id, first.seq)
                    .await
                    .unwrap_or(false),
                _ => false,
            };
            let head = api
                .sessions
                .head_of(id)
                .await
                .unwrap_or_else(|| Value::Object(api.sessions.shape(&s)));
            Ok(ok(
                &json!({ "session": head, "events": rows.iter().map(Sessions::step_of).collect::<Vec<_>>(), "earlier": earlier }),
            ))
        }
        ("GET", Some("live")) => Ok(live_stream(api, id, int(q, "after", 0)).await),
        ("PUT", Some("files")) => {
            let file_name = q.get("name").map_or("", String::as_str);
            let by = q.get("by").map_or("", String::as_str);
            let said = q
                .get("size")
                .and_then(|n| n.parse::<u64>().ok())
                .or_else(|| {
                    headers
                        .get(header::CONTENT_LENGTH)
                        .and_then(|v| v.to_str().ok())
                        .and_then(|v| v.parse().ok())
                });
            let got = api
                .files
                .receive_sized(&s, file_name, by, said, body.into_data_stream())
                .await?;
            let from = if by.is_empty() { "the server" } else { by };
            api.sessions
                .note_system(id, &format!("File from {from}: {}", got.path))
                .await;
            Ok(ok(&serde_json::to_value(&got).unwrap_or_default()))
        }
        ("POST", Some("send")) => {
            let b = json_body(body).await?;
            let prompt = b.get("prompt").and_then(Value::as_str).unwrap_or_default();
            let by = b.get("by").and_then(Value::as_str).unwrap_or_default();
            let (turn, queued) = api.sessions.send(id, prompt, by, b.get("files")).await?;
            Ok(ok(&json!({ "ok": true, "turn": turn, "queued": queued })))
        }
        ("POST", Some("settings")) => Ok(ok(&api
            .sessions
            .settings(id, &json_body(body).await?)
            .await?)),
        ("POST", Some("interrupt")) => Ok(ok(
            &json!({ "ok": true, "running": api.sessions.interrupt(id, false).await? }),
        )),
        ("POST", Some("stop")) => Ok(ok(
            &json!({ "ok": true, "running": api.sessions.interrupt(id, true).await? }),
        )),
        _ => Err(Refused::missing("not found")),
    }
}

/// The live stream: the head, the steps after the watcher's last, what's being typed and
/// thought, then every change, with a comment every 15 s.
async fn live_stream(api: &Api, id: &str, after: u64) -> Response {
    let mut first: Vec<String> = Vec::new();
    let frame = |v: Value| format!("data: {v}\n\n");
    if let Some(h) = api.sessions.head_of(id).await {
        first.push(frame(json!({ "type": live::HEAD, "session": h })));
    }
    let after = i64::try_from(after).unwrap_or(i64::MAX);
    for e in api
        .store
        .events_after(id, after, 2000)
        .await
        .unwrap_or_default()
    {
        first.push(frame(
            json!({ "type": live::STEP, "step": Sessions::step_of(&e) }),
        ));
    }
    if let Some((turn, typed, thinking, since)) = api.sessions.typing(id).await {
        if !typed.is_empty() {
            first.push(frame(
                json!({ "type": live::DELTA, "turn": turn, "start": true, "text": typed }),
            ));
        }
        if let Some(since) = since {
            first.push(frame(json!({ "type": live::THINK, "turn": turn, "start": true, "text": thinking, "since": since })));
        }
    }
    let rx = api.sessions.watchers.add(id);
    let head = futures_util::stream::iter(
        first
            .into_iter()
            .map(|s| Ok::<_, std::io::Error>(Bytes::from(s))),
    );
    let mut pings = tokio::time::interval(PING);
    pings.reset();
    let rest = futures_util::stream::unfold((rx, pings), |(mut rx, mut pings)| async move {
        let next = tokio::select! {
            f = rx.recv() => f?,
            _ = pings.tick() => ": ping\n\n".to_owned(),
        };
        Some((Ok::<_, std::io::Error>(Bytes::from(next)), (rx, pings)))
    });
    let body = Body::from_stream(head.chain(rest));
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/event-stream")
        .header(header::CACHE_CONTROL, "no-cache")
        .header("X-Accel-Buffering", "no")
        .body(body)
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

async fn send_file(api: &Api, q: &HashMap<String, String>) -> Result<Response, Refused> {
    let raw = q.get("path").map_or("", String::as_str);
    let by = q.get("by").map_or("", String::as_str);
    let session = match q.get("session").filter(|s| !s.is_empty()) {
        Some(sid) => match api.store.session(sid).await.ok().flatten() {
            Some(s) => Some(s),
            None => {
                return Err(api
                    .files
                    .refused_out(
                        raw,
                        None,
                        by,
                        Refused::missing(format!("no session {sid} on {}", api.config.name)),
                    )
                    .await);
            }
        },
        None => None,
    };
    if q.get("stat").is_some_and(|v| v == "1" || v == "true") {
        let f = api.files.stat(raw, session.as_ref(), by).await?;
        return Ok(ok(&serde_json::to_value(&f).unwrap_or_default()));
    }
    let (f, file) = api.files.open_for_send(raw, session.as_ref(), by).await?;
    let file_name = std::path::Path::new(&f.path)
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().replace('"', ""));
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .header(header::CONTENT_LENGTH, f.size)
        .header(
            header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"{file_name}\""),
        )
        .body(Body::from_stream(tokio_util::io::ReaderStream::new(file)))
        .map_err(|_| Refused::new(500, "the file couldn't be sent"))
}

async fn stop_job(b: &Map<String, Value>) -> Result<Response, Refused> {
    let pid = b
        .get("pid")
        .and_then(Value::as_u64)
        .and_then(|p| u32::try_from(p).ok())
        .ok_or_else(|| Refused::bad("pid: a process id"))?;
    if !crate::jobs::runs().await.iter().any(|r| r.pid == pid) {
        return Err(Refused::missing(
            "no such run here (it ended, or it isn't a codex exec or claude -p run)",
        ));
    }
    crate::jobs::stop(pid).map_err(|e| Refused::bad(format!("couldn't stop {pid}: {e}")))?;
    Ok(ok(&json!({ "ok": true })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_numbers_fall_back_when_empty_or_wrong() {
        let q: HashMap<String, String> = [
            ("after".to_owned(), String::new()),
            ("limit".to_owned(), "12.7".to_owned()),
            ("x".to_owned(), "-3".to_owned()),
        ]
        .into();
        assert_eq!(int(&q, "after", 0), 0);
        assert_eq!(int(&q, "limit", 400), 12);
        assert_eq!(int(&q, "x", 5), 5);
        assert_eq!(int(&q, "missing", 7), 7);
    }

    #[test]
    fn a_body_is_an_object_or_nothing() {
        assert!(object(b"  ").unwrap().is_empty());
        assert_eq!(object(b"[1]").unwrap_err().message, "not a JSON object");
        assert_eq!(object(b"{").unwrap_err().message, "not JSON");
    }
}
