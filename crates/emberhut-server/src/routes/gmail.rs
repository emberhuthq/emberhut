//! `GET /gmail/connect/{token}` and `GET /gmail/callback`: signing one of the owner's Gmail
//! accounts in, read-only. The one-time link is the key (its token is the `state` Google sends
//! back), so these need no session.

use axum::extract::{Path, RawQuery, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};

use crate::app::AppState;
use crate::services::messaging::gmail::Page;

/// On to Google's consent screen for the link's account.
#[utoipa::path(
    get,
    path = "/gmail/connect/{token}",
    tag = "messaging",
    params(("token" = String, Path, description = "The one-time link's token: the link is the key")),
    responses(
        (status = 302, description = "On to Google's consent screen for the account"),
        (
            status = 404,
            description = "The link expired, was used, or never existed",
            body = String,
            content_type = "text/html"
        ),
    )
)]
pub(super) async fn connect(State(app): State<AppState>, Path(token): Path<String>) -> Response {
    page(app.messaging().gmail().connect_page(&token).await)
}

/// Google's way back with the code: the account's refresh token is saved on the server.
#[utoipa::path(
    get,
    path = "/gmail/callback",
    tag = "messaging",
    params(
        ("state" = Option<String>, Query, description = "The one-time link's token"),
        ("code" = Option<String>, Query, description = "Google's code"),
        ("error" = Option<String>, Query, description = "Why Google gave no code"),
    ),
    responses(
        (status = 200, description = "Signed in", body = String, content_type = "text/html"),
        (
            status = 400,
            description = "Nothing was saved: Google refused, the reading scope wasn't allowed, or \
                another address signed in",
            body = String,
            content_type = "text/html"
        ),
        (
            status = 404,
            description = "The link expired, was used, or never existed",
            body = String,
            content_type = "text/html"
        ),
        (
            status = 500,
            description = "The sign-in couldn't be saved",
            body = String,
            content_type = "text/html"
        ),
    )
)]
pub(super) async fn callback(State(app): State<AppState>, RawQuery(query): RawQuery) -> Response {
    let fields: Vec<(String, String)> =
        form_urlencoded::parse(query.unwrap_or_default().as_bytes())
            .into_owned()
            .collect();
    let field = |name: &str| {
        fields
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    };
    page(
        app.messaging()
            .gmail()
            .callback(field("state"), field("code"), field("error"))
            .await,
    )
}

/// A page's answer.
fn page(page: Page) -> Response {
    match page {
        Page::Redirect(to) => {
            let mut response = StatusCode::FOUND.into_response();
            if let Ok(to) = HeaderValue::from_str(&to) {
                response.headers_mut().insert(header::LOCATION, to);
            }
            response
                .headers_mut()
                .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
            response
        }
        Page::Html(status, body) => html(status, &body),
    }
}

/// A page: `body` in the pages' frame, never cached, never sent on as a referrer.
pub(super) fn html(status: u16, body: &str) -> Response {
    let page = format!(
        "<!doctype html><meta name=viewport content=\"width=device-width\"><title>Emberhut</title>\
         <body style=\"font:17px system-ui,-apple-system,sans-serif;max-width:520px;\
         margin:40px auto;padding:0 16px;text-align:center\">{body}"
    );
    let status = StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let mut response = (status, page).into_response();
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    response
}
