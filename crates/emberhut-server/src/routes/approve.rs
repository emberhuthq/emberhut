//! `GET /approve/{token}` and `POST /approve/{token}`: a signed link's one-time page, where an
//! approver without an account decides a request. The token is the capability (its MAC covers
//! the request and its payload hash), so the page needs no session; its form must carry the
//! hash it showed, so a page shown for one payload never decides another. A link works once.
//!
//! The page is self-contained (inline styles, light and dark, no script), every value in it
//! escaped with its invisible characters written out, the exact words in a `<pre>`, each file
//! with its name, size and SHA-256.

use std::fmt::Write as _;

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::Response;
use emberhut_core::approvals::shown::visible;
use emberhut_core::approvals::{ApprovalRequest, Decision, RequestState};

use super::agents::approval_status;
use super::secret_links::{escape, html_page};
use crate::app::AppState;
use crate::convert::timestamp_to_api;
use crate::services::approvals::{ApprovalError, LinkView};

/// The page's styles: phone-sized, in the system's light or dark.
const STYLE: &str = "\
:root{color-scheme:light dark;--bg:#fff;--fg:#1c1c1e;--muted:#6e6e73;--line:#d1d1d6;\
--card:#f5f5f7;--ok:#1a7f37;--no:#b42318}\
@media (prefers-color-scheme:dark){:root{--bg:#000;--fg:#f5f5f7;--muted:#a1a1a6;\
--line:#3a3a3c;--card:#1c1c1e;--ok:#2ea043;--no:#da3633}}\
*{box-sizing:border-box}\
body{margin:0;background:var(--bg);color:var(--fg);\
font:16px/1.45 system-ui,-apple-system,'Segoe UI',sans-serif}\
main{max-width:640px;margin:0 auto;padding:24px 16px 48px}\
h1{font-size:22px;line-height:1.25;margin:0 0 4px}\
h2{font-size:15px;margin:24px 0 8px;color:var(--muted);font-weight:600}\
.muted{color:var(--muted);font-size:14px}\
dl{display:grid;grid-template-columns:auto 1fr;gap:6px 12px;margin:16px 0}\
dt{color:var(--muted)}dd{margin:0;overflow-wrap:anywhere}\
pre{margin:0;background:var(--card);border:1px solid var(--line);border-radius:10px;\
padding:12px;white-space:pre-wrap;overflow-wrap:anywhere;\
font:14px/1.45 ui-monospace,SFMono-Regular,Menlo,Consolas,monospace}\
table{width:100%;border-collapse:collapse;font-size:14px}\
th,td{border-top:1px solid var(--line);padding:6px 4px;text-align:left;vertical-align:top}\
code{font:12px/1.4 ui-monospace,SFMono-Regular,Menlo,Consolas,monospace;overflow-wrap:anywhere}\
.actions{display:flex;gap:12px;margin-top:28px}.actions form{flex:1}\
button{width:100%;font:600 17px system-ui,-apple-system,sans-serif;padding:14px;\
border:0;border-radius:12px;color:#fff;cursor:pointer}\
.approve{background:var(--ok)}.reject{background:var(--no)}";

/// The whole page around `body`, titled `title`.
fn framed(status: StatusCode, title: &str, body: &str) -> Response {
    html_page(
        status,
        format!(
            "<!doctype html><html lang=en><meta charset=utf-8>\
             <meta name=viewport content=\"width=device-width,initial-scale=1\">\
             <title>{title}</title><style>{STYLE}</style><main>{body}</main></html>",
            title = escape(title),
        ),
    )
}

/// A short page saying one thing.
fn note(status: StatusCode, title: &str, words: &str) -> Response {
    framed(
        status,
        title,
        &format!("<h1>{}</h1><p>{}</p>", escape(title), escape(words)),
    )
}

/// What a link that doesn't work shows.
fn gone() -> Response {
    note(
        StatusCode::GONE,
        "This link no longer works",
        "It was used already, it has expired, or the request was withdrawn.",
    )
}

/// What a failure answers: its page, or the server's fault (logged).
fn failed(error: &ApprovalError) -> Response {
    match approval_status(error) {
        StatusCode::GONE => gone(),
        StatusCode::CONFLICT => note(
            StatusCode::CONFLICT,
            "Nothing was decided",
            "The request is not what this page showed. Open the link again to see it as it is.",
        ),
        StatusCode::BAD_REQUEST => note(
            StatusCode::BAD_REQUEST,
            "Nothing was decided",
            "Choose Approve or Reject on the request's page.",
        ),
        _ => {
            tracing::error!(%error, "an approval link's page failed");
            note(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Something went wrong",
                "Nothing was decided. Try again in a moment.",
            )
        }
    }
}

/// `text` as the page shows it: invisible characters written out (`[U+202E]`), then escaped.
fn shown(text: &str) -> String {
    escape(&visible(text))
}

/// An amount in cents as the page writes it.
fn amount(cents: u64) -> String {
    format!("{}.{:02}", cents / 100, cents % 100)
}

/// The request's page: who asks, for what, where, the exact words and files, and the two
/// forms carrying the payload hash.
fn request_page(view: &LinkView) -> String {
    let request = &view.request;
    let hash = request.payload_hash.to_hex();
    let mut body = format!(
        "<h1>{agent} asks for your approval</h1>\
         <p class=muted>{kind} · request {id}{until}</p>\
         <dl><dt>Where</dt><dd>{target}</dd><dt>What</dt><dd>{summary}</dd>",
        agent = shown(&view.agent_name),
        kind = escape(request.kind.as_str()),
        id = request.id,
        until = request.expires_at().map_or_else(String::new, |at| format!(
            " · open until {}",
            escape(timestamp_to_api(at).as_str())
        )),
        target = shown(&request.target),
        summary = shown(&request.summary),
    );
    if let Some(cents) = request.amount_cents {
        // Writing to a String never fails.
        let _written = write!(
            body,
            "<dt>Amount</dt><dd>{} {}</dd>",
            amount(cents),
            escape(view.currency.as_deref().unwrap_or_default())
        );
    }
    body.push_str("</dl>");
    if let Some(text) = &request.text {
        let _written = write!(body, "<h2>Exact text</h2><pre>{}</pre>", shown(text));
    }
    if !request.files.is_empty() {
        body.push_str("<h2>Files</h2><table><tr><th>Name</th><th>Size</th><th>SHA-256</th></tr>");
        for file in &request.files {
            let _written = write!(
                body,
                "<tr><td>{}</td><td>{}</td><td><code>{}</code></td></tr>",
                shown(&file.name),
                file.size
                    .map_or_else(|| "unknown".to_owned(), |size| format!("{size} bytes")),
                file.sha256,
            );
        }
        body.push_str("</table>");
    }
    let _written = write!(
        body,
        "<p class=muted>Payload hash <code>{hash}</code></p>\
         <div class=actions>{approve}{reject}</div>\
         <p class=muted>This page works once. Your answer covers the target, the exact text, the \
         amount and the files shown here; the summary is the agent's description of them.</p>",
        approve = form(&hash, "approve", "Approve"),
        reject = form(&hash, "reject", "Reject"),
    );
    body
}

/// One decision's form, posting to this page with the hash it showed.
fn form(hash: &str, decision: &str, label: &str) -> String {
    format!(
        "<form method=post><input type=hidden name=payload_hash value=\"{hash}\">\
         <input type=hidden name=decision value={decision}>\
         <button class={decision}>{label}</button></form>"
    )
}

/// What the page says once decided.
fn decided_page(request: &ApprovalRequest) -> Response {
    let (title, words) = if request.state == RequestState::Approved {
        (
            "Approved",
            "The step may go ahead, once, exactly as it was shown. You can close this page.",
        )
    } else {
        ("Rejected", "Nothing goes out. You can close this page.")
    };
    note(StatusCode::OK, title, words)
}

/// A signed link's page: the request it decides, with Approve and Reject.
#[utoipa::path(
    get,
    path = "/approve/{token}",
    tag = "approvals",
    params(("token" = String, Path, description = "The link's token: the link is the key")),
    responses(
        (
            status = 200,
            description = "The request's page: the agent, the target, the exact words, each file's name, size and hash, and Approve and Reject",
            body = String,
            content_type = "text/html"
        ),
        (
            status = 410,
            description = "The link was used, expired, was withdrawn, or never existed",
            body = String,
            content_type = "text/html"
        ),
    )
)]
pub(super) async fn page_of(State(app): State<AppState>, Path(token): Path<String>) -> Response {
    match app.approvals().link_view(&token).await {
        Ok(view) => framed(StatusCode::OK, "Approval", &request_page(&view)),
        Err(error) => failed(&error),
    }
}

/// A signed link's decision: the form's `decision` (`approve` or `reject`) on the
/// `payload_hash` the page showed. The link works once.
#[utoipa::path(
    post,
    path = "/approve/{token}",
    tag = "approvals",
    params(("token" = String, Path, description = "The link's token: the link is the key")),
    request_body(
        content = String,
        content_type = "application/x-www-form-urlencoded",
        description = "`decision` (approve or reject) and `payload_hash`, the hash the page showed"
    ),
    responses(
        (status = 200, description = "Decided", body = String, content_type = "text/html"),
        (status = 400, description = "No decision or no hash in the form", body = String, content_type = "text/html"),
        (status = 409, description = "The hash is not the request's: nothing was decided", body = String, content_type = "text/html"),
        (status = 410, description = "The link was used, expired, was withdrawn, or never existed", body = String, content_type = "text/html"),
    )
)]
pub(super) async fn decide(
    State(app): State<AppState>,
    Path(token): Path<String>,
    form: Bytes,
) -> Response {
    let mut decision = None;
    let mut hash = None;
    for (name, value) in form_urlencoded::parse(&form) {
        match (name.as_ref(), value.as_ref()) {
            ("decision", "approve") => decision = Some(Decision::Approve),
            ("decision", "reject") => decision = Some(Decision::Reject),
            ("payload_hash", value) => hash = Some(value.to_owned()),
            _ => {}
        }
    }
    match app
        .approvals()
        .link_decide(&token, decision, hash.as_deref())
        .await
    {
        Ok(request) => decided_page(&request),
        Err(error) => failed(&error),
    }
}
