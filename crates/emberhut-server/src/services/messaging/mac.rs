//! The owner's messages on their Mac, through the Mac helper: iMessage, SMS and WhatsApp chats
//! (`/chats`, `/read`, `/search`, `/wa-contact`), their voice notes' sound (`/media`), and
//! Slack's notifications and the owner's own Slack messages from the Slack app's saved state
//! (`/notifications`, `/slack-mine`). Each is a `GET` with the helper's token as the bearer;
//! the helper is the same one `services::mac` reaches.

use std::fmt::{self, Write as _};
use std::time::Duration;

use axum::body::Bytes;
use futures_util::future::BoxFuture;

use super::web::{WebClient, WebRequest};
use crate::services::mac::MacError;

/// What the helper answered.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MacAnswer {
    /// Its status.
    pub status: u16,
    /// Its `content-type`.
    pub content_type: String,
    /// Its `x-seconds` (a voice note's length), when it sent one.
    pub seconds: Option<String>,
    /// Its body.
    pub body: Bytes,
}

impl MacAnswer {
    /// Whether the status is a success.
    #[must_use]
    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }

    /// The body as text.
    #[must_use]
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    /// The body as JSON; `null` when it isn't.
    #[must_use]
    pub fn json(&self) -> serde_json::Value {
        serde_json::from_slice(&self.body).unwrap_or(serde_json::Value::Null)
    }
}

/// The Mac helper's message routes.
pub trait MacMessages: fmt::Debug + Send + Sync {
    /// `GET`s `path` (with its query) from the helper, waiting up to `wait`.
    fn get<'a>(
        &'a self,
        path: &'a str,
        wait: Duration,
    ) -> BoxFuture<'a, Result<MacAnswer, MacError>>;
}

/// No helper set up.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoMacMessages;

impl MacMessages for NoMacMessages {
    fn get<'a>(&'a self, _: &'a str, _: Duration) -> BoxFuture<'a, Result<MacAnswer, MacError>> {
        Box::pin(async { Err(MacError::NotSetUp) })
    }
}

/// The helper over HTTP, at its URL with its token.
pub struct HttpMacMessages {
    web: WebClient,
    url: String,
    token: String,
}

impl fmt::Debug for HttpMacMessages {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HttpMacMessages")
            .field("url", &self.url)
            .finish_non_exhaustive()
    }
}

impl HttpMacMessages {
    /// The helper at `url`, sent `token` as its bearer.
    #[must_use]
    pub fn new(web: WebClient, url: &str, token: String) -> Self {
        Self {
            web,
            url: url.trim_end_matches('/').to_owned(),
            token,
        }
    }
}

impl MacMessages for HttpMacMessages {
    fn get<'a>(
        &'a self,
        path: &'a str,
        wait: Duration,
    ) -> BoxFuture<'a, Result<MacAnswer, MacError>> {
        Box::pin(async move {
            let request = WebRequest::get(format!("{}{path}", self.url), wait)
                .bearer(&self.token)
                .limit(64 << 20);
            let answer = self
                .web
                .send(request)
                .await
                .map_err(|error| MacError::Unreachable(Box::new(error)))?;
            Ok(MacAnswer {
                status: answer.status.as_u16(),
                content_type: answer.header("content-type").unwrap_or_default().to_owned(),
                seconds: answer.header("x-seconds").map(str::to_owned),
                body: answer.body,
            })
        })
    }
}

/// `text` as a URL's query value (as JavaScript's `encodeURIComponent`).
#[must_use]
pub fn encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&byte) {
            out.push(char::from(byte));
        } else {
            let _written = write!(out, "%{byte:02X}");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_values_are_encoded_as_javascript_does() {
        assert_eq!(encode("wa:12 & (x)"), "wa%3A12%20%26%20(x)");
        assert_eq!(encode("é"), "%C3%A9");
    }
}
