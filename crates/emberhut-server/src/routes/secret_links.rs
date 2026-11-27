//! `GET /s/{token}` and `POST /s/{token}`: the one-time secret pages. The link is the key, so
//! these need no session.
//!
//! The pages are plain HTML with no script, kept out of caches and referrers, and every value
//! the assistant gave (why, the names) is escaped.

use std::fmt::Write as _;

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};

use crate::app::AppState;
use crate::error::ApiError;
use crate::services::secret_links::Saved;

/// What an expired, used or unknown link shows.
const GONE: &str = "<p>This link has expired or was already used. Ask for a new one.</p>";

/// The form for a link's secrets.
#[utoipa::path(
    get,
    path = "/s/{token}",
    tag = "secrets",
    params(("token" = String, Path, description = "The link's token: the link is the key")),
    responses(
        (
            status = 200,
            description = "The form: one password field per secret, and why they are wanted",
            body = String,
            content_type = "text/html"
        ),
        (
            status = 404,
            description = "The link expired, was used, or never existed",
            body = String,
            content_type = "text/html"
        ),
    )
)]
pub(super) async fn page(
    State(app): State<AppState>,
    Path(token): Path<String>,
) -> Result<Response, ApiError> {
    let Some(link) = app
        .secret_links()
        .open(&token)
        .await
        .map_err(|error| fault(&error))?
    else {
        return Ok(html(StatusCode::NOT_FOUND, GONE));
    };
    let mut fields = String::new();
    for name in &link.names {
        let name = escape(name.as_str());
        // Writing to a String never fails.
        let _written = write!(
            fields,
            "<label>{name}<br><input name=\"{name}\" type=password autocomplete=off \
             style=\"width:100%;font-size:16px;padding:8px;margin:6px 0 16px\"></label>"
        );
    }
    let body = format!(
        "<h2>For Emberhut</h2><p>{why}</p><form method=post>{fields}\n    \
         <button style=\"font-size:17px;padding:10px 20px\">Save</button></form>\
         <p style=\"color:#888\">Saved only on this server. This page works once.</p>",
        why = escape(&link.why),
    );
    Ok(html(StatusCode::OK, &body))
}

/// Saves the secrets typed into the form, each to its own file, and uses the link up.
#[utoipa::path(
    post,
    path = "/s/{token}",
    tag = "secrets",
    params(("token" = String, Path, description = "The link's token: the link is the key")),
    request_body(
        content = String,
        content_type = "application/x-www-form-urlencoded",
        description = "The form's fields: each secret's name and what was typed"
    ),
    responses(
        (
            status = 200,
            description = "Saved what was filled in (or nothing); the link is used up",
            body = String,
            content_type = "text/html"
        ),
        (
            status = 404,
            description = "The link expired, was used, or never existed",
            body = String,
            content_type = "text/html"
        ),
    )
)]
pub(super) async fn save(
    State(app): State<AppState>,
    Path(token): Path<String>,
    form: Bytes,
) -> Result<Response, ApiError> {
    let fields: Vec<(String, String)> = form_urlencoded::parse(&form).into_owned().collect();
    let saved = app
        .secret_links()
        .save(&token, &fields)
        .await
        .map_err(|error| fault(&error))?;
    let body = match saved {
        Saved::Gone => return Ok(html(StatusCode::NOT_FOUND, GONE)),
        Saved::Saved(names) if names.is_empty() => "<p>Nothing was entered.</p>".to_owned(),
        Saved::Saved(names) => {
            let names: Vec<String> = names.iter().map(|name| escape(name.as_str())).collect();
            format!(
                "<p>Saved {}. You can close this page.</p>",
                names.join(", ")
            )
        }
    };
    Ok(html(StatusCode::OK, &body))
}

/// A page: `body` in the pages' frame, never cached, never sent on as a referrer.
fn html(status: StatusCode, body: &str) -> Response {
    html_page(
        status,
        format!(
            "<!doctype html><meta name=viewport content=\"width=device-width\"><title>Emberhut</title>\
             <body style=\"font:17px system-ui,-apple-system,sans-serif;max-width:520px;\
             margin:40px auto;padding:0 16px\">{body}"
        ),
    )
}

/// A whole HTML `page` answered with `status`: never cached, never sent on as a referrer, no
/// script, its forms posting only to this server, never framed.
pub(super) fn html_page(status: StatusCode, page: String) -> Response {
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
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(
            "default-src 'none'; style-src 'unsafe-inline'; form-action 'self'; \
             frame-ancestors 'none'",
        ),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    response
}

/// `text` safe inside HTML text and a double-quoted attribute.
pub(super) fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

/// A failure behind a page, logged here; the answer is the server's fault.
fn fault(error: &crate::services::secret_links::SecretLinkError) -> ApiError {
    tracing::error!(error = %error, "a secret page failed");
    ApiError::Internal
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn everything_that_could_open_markup_is_escaped() {
        assert_eq!(
            escape(r#"<a href="x">Tom & 'Jerry'</a>"#),
            "&lt;a href=&quot;x&quot;&gt;Tom &amp; &#39;Jerry&#39;&lt;/a&gt;"
        );
        assert_eq!(escape("plain"), "plain");
    }
}
