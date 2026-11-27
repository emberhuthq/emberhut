//! The HTTP client the messaging services' real implementations use: the server's one client
//! (`crate::http`), with a time limit and a size limit per request.

use std::time::Duration;

use axum::body::Bytes;
use axum::http::{HeaderMap, Method, StatusCode};

use crate::http::read_body;

/// Why a request got no answer.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum WebError {
    /// The request could not be made (a bad address or header).
    #[error("the request could not be made: {0}")]
    Build(String),
    /// The request failed on the way (connect, TLS, HTTP).
    #[error("fetch failed")]
    Request(#[source] reqwest::Error),
    /// The answer's body could not be read.
    #[error("the answer could not be read")]
    Body(#[source] reqwest::Error),
    /// The answer's body was over the request's limit.
    #[error("the answer is over {0} bytes")]
    TooLarge(usize),
    /// No whole answer in time.
    #[error("no answer within {0:?}")]
    Timeout(Duration),
}

/// An answer: its status, headers and body.
#[derive(Clone, Debug)]
pub struct WebAnswer {
    /// Its status.
    pub status: StatusCode,
    /// Its headers.
    pub headers: HeaderMap,
    /// Its body.
    pub body: Bytes,
}

impl WebAnswer {
    /// The body as JSON; `null` when it isn't.
    #[must_use]
    pub fn json(&self) -> serde_json::Value {
        serde_json::from_slice(&self.body).unwrap_or(serde_json::Value::Null)
    }

    /// A header's value as text.
    #[must_use]
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).and_then(|value| value.to_str().ok())
    }

    /// Whether the status is a success (2xx).
    #[must_use]
    pub fn ok(&self) -> bool {
        self.status.is_success()
    }
}

/// A request to send.
#[derive(Clone, Debug)]
pub struct WebRequest {
    /// Its method.
    pub method: Method,
    /// Its address.
    pub url: String,
    /// Its headers.
    pub headers: Vec<(String, String)>,
    /// Its body.
    pub body: Bytes,
    /// How long it may take, to the whole answer.
    pub wait: Duration,
    /// The longest answer read.
    pub limit: usize,
}

impl WebRequest {
    /// A `GET` of `url` that may take `wait`, reading up to 1 MiB.
    #[must_use]
    pub fn get(url: impl Into<String>, wait: Duration) -> Self {
        Self {
            method: Method::GET,
            url: url.into(),
            headers: Vec::new(),
            body: Bytes::new(),
            wait,
            limit: 1 << 20,
        }
    }

    /// A `POST` of `body` to `url`, of `content_type`.
    #[must_use]
    pub fn post(
        url: impl Into<String>,
        content_type: &str,
        body: impl Into<Bytes>,
        wait: Duration,
    ) -> Self {
        Self {
            method: Method::POST,
            body: body.into(),
            ..Self::get(url, wait)
        }
        .header("content-type", content_type)
    }

    /// A `POST` of `fields`, form-encoded.
    #[must_use]
    pub fn form(url: impl Into<String>, fields: &[(&str, &str)], wait: Duration) -> Self {
        let body = form_urlencoded::Serializer::new(String::new())
            .extend_pairs(fields)
            .finish();
        Self::post(url, "application/x-www-form-urlencoded", body, wait)
    }

    /// With this header.
    #[must_use]
    pub fn header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_owned(), value.to_owned()));
        self
    }

    /// With `token` as its bearer.
    #[must_use]
    pub fn bearer(self, token: &str) -> Self {
        self.header("authorization", &format!("Bearer {token}"))
    }

    /// Reading up to `limit` bytes of the answer.
    #[must_use]
    pub fn limit(mut self, limit: usize) -> Self {
        self.limit = limit;
        self
    }
}

/// A pooled HTTPS (and plain HTTP) client.
#[derive(Clone, Debug)]
pub struct WebClient {
    client: reqwest::Client,
}

impl WebClient {
    /// Requests sent with `client`.
    #[must_use]
    pub fn new(client: reqwest::Client) -> Self {
        Self { client }
    }

    /// Sends `request` and reads its answer.
    ///
    /// # Errors
    ///
    /// [`WebError`] when there is no whole answer in time.
    pub async fn send(&self, request: WebRequest) -> Result<WebAnswer, WebError> {
        let url = reqwest::Url::parse(&request.url)
            .map_err(|error| WebError::Build(error.to_string()))?;
        let mut builder = self.client.request(request.method, url);
        for (name, value) in &request.headers {
            builder = builder.header(name.as_str(), value.as_str());
        }
        let outgoing = builder
            .body(request.body)
            .build()
            .map_err(|error| WebError::Build(error.to_string()))?;
        let limit = request.limit;
        let exchange = async {
            let response = self
                .client
                .execute(outgoing)
                .await
                .map_err(WebError::Request)?;
            let status = response.status();
            let headers = response.headers().clone();
            let body = read_body(response, limit).await.map_err(WebError::Body)?;
            if body.cut {
                return Err(WebError::TooLarge(limit));
            }
            Ok(WebAnswer {
                status,
                headers,
                body: Bytes::from(body.bytes),
            })
        };
        tokio::time::timeout(request.wait, exchange)
            .await
            .map_err(|_| WebError::Timeout(request.wait))?
    }
}
