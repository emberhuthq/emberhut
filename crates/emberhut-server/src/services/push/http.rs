//! What the push services send through: the server's one HTTP client ([`crate::http`]), with a
//! time limit on each exchange and a cap on how much of an answer is read.

use std::time::Duration;

use axum::body::Bytes;
use axum::http::{Request, StatusCode};
use reqwest::Client;

use crate::http::{ClientError, Protocol, client, read_body};

/// How long one request to a push service may take, from sending to the whole answer.
pub(super) const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// The longest answer read from a push service; the rest is dropped.
const ANSWER_LIMIT: usize = 64 * 1024;

/// Why a request to a push service got no answer.
#[derive(Debug, thiserror::Error)]
pub(super) enum HttpError {
    /// The client could not be made.
    #[error("the HTTP client could not be made")]
    Client(#[source] ClientError),
    /// The request could not be made from its parts.
    #[error("not a request")]
    Request(#[source] reqwest::Error),
    /// The request failed on the way (connect, TLS, HTTP, the time limit), or its answer could
    /// not be read.
    #[error("the request failed")]
    Send(#[source] reqwest::Error),
}

/// A status and the answer's text (at most [`ANSWER_LIMIT`] bytes of it).
#[derive(Clone, Debug)]
pub(super) struct Answer {
    /// The status.
    pub(super) status: StatusCode,
    /// The body, as text.
    pub(super) text: String,
}

/// A pooled HTTPS client for one push service.
#[derive(Clone, Debug)]
pub(super) struct HttpClient {
    client: Client,
}

impl HttpClient {
    /// A client speaking `protocol`.
    pub(super) fn new(protocol: Protocol) -> Result<Self, HttpError> {
        Ok(Self {
            client: client(protocol).map_err(HttpError::Client)?,
        })
    }

    /// Sends `request` and reads its answer, within [`REQUEST_TIMEOUT`].
    pub(super) async fn send(&self, request: Request<Bytes>) -> Result<Answer, HttpError> {
        let mut request = reqwest::Request::try_from(request).map_err(HttpError::Request)?;
        *request.timeout_mut() = Some(REQUEST_TIMEOUT);
        let response = self
            .client
            .execute(request)
            .await
            .map_err(HttpError::Send)?;
        let status = response.status();
        // Over the limit, the status is what counts.
        let body = read_body(response, ANSWER_LIMIT)
            .await
            .map_err(HttpError::Send)?;
        let text = if body.cut { String::new() } else { body.text() };
        Ok(Answer { status, text })
    }
}
