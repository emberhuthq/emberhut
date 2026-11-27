//! The account's HTTP: the provider's discovery, device authorization and token endpoints, and
//! the directory. Behind [`Http`], so the flows are tested against a scripted fake; the real
//! one is [`UreqHttp`]. Nothing here logs a request's body or its bearer token.

use std::fmt;
use std::time::Duration;

use ureq::config::AutoHeaderValue;
use ureq::{Agent, RequestBuilder};

/// The longest answer read: discovery documents, tokens and machine lists are far smaller.
const MAX_ANSWER: u64 = 1 << 20;
/// How long one request may take, from connecting to the last byte.
const TIMEOUT: Duration = Duration::from_secs(30);

/// A request's method.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Method {
    /// `GET`.
    Get,
    /// `POST`.
    Post,
    /// `DELETE`.
    Delete,
}

/// A request's body.
#[derive(Clone, PartialEq, Eq)]
pub(crate) enum Body {
    /// None.
    Empty,
    /// `application/x-www-form-urlencoded` (the OAuth endpoints).
    Form(Vec<(&'static str, String)>),
    /// `application/json` (the directory).
    Json(Vec<u8>),
}

/// One request.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct Request {
    /// Its method.
    pub(crate) method: Method,
    /// Its whole URL.
    pub(crate) url: String,
    /// The token for `Authorization: Bearer`, if any.
    pub(crate) bearer: Option<String>,
    /// Its body.
    pub(crate) body: Body,
}

impl Request {
    /// A `GET` of `url`.
    pub(crate) fn get(url: impl Into<String>) -> Self {
        Self {
            method: Method::Get,
            url: url.into(),
            bearer: None,
            body: Body::Empty,
        }
    }

    /// A form `POST` to `url`.
    pub(crate) fn form(url: impl Into<String>, fields: Vec<(&'static str, String)>) -> Self {
        Self {
            method: Method::Post,
            url: url.into(),
            bearer: None,
            body: Body::Form(fields),
        }
    }

    /// A JSON `POST` to `url`.
    pub(crate) fn json(url: impl Into<String>, body: Vec<u8>) -> Self {
        Self {
            method: Method::Post,
            url: url.into(),
            bearer: None,
            body: Body::Json(body),
        }
    }

    /// A `DELETE` of `url`.
    pub(crate) fn delete(url: impl Into<String>) -> Self {
        Self {
            method: Method::Delete,
            url: url.into(),
            bearer: None,
            body: Body::Empty,
        }
    }

    /// The same request with `token` as its bearer.
    #[must_use]
    pub(crate) fn bearer(mut self, token: &str) -> Self {
        self.bearer = Some(token.to_owned());
        self
    }

    /// The value of form field `name`, if the body is a form with one.
    #[cfg(test)]
    pub(crate) fn field(&self, name: &str) -> Option<&str> {
        match &self.body {
            Body::Form(fields) => fields
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| value.as_str()),
            Body::Empty | Body::Json(_) => None,
        }
    }
}

// Its `Debug` shows the method and the URL only: the bodies carry codes and refresh tokens.
impl fmt::Debug for Request {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Request")
            .field("method", &self.method)
            .field("url", &self.url)
            .finish_non_exhaustive()
    }
}

/// An answer, whatever its status.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Response {
    /// The HTTP status.
    pub(crate) status: u16,
    /// `Retry-After`, when it gave a number of seconds.
    pub(crate) retry_after: Option<Duration>,
    /// The body.
    pub(crate) body: Vec<u8>,
}

/// A request that got no whole answer: no connection, a timeout, a broken stream.
#[derive(Debug, thiserror::Error)]
#[error("can't reach {host}")]
pub(crate) struct Unreachable {
    /// The host it was for.
    pub(crate) host: String,
    /// What went wrong.
    #[source]
    pub(crate) cause: Box<dyn std::error::Error + Send + Sync>,
}

/// Sends the account's requests. Blocks: call it on a background thread.
pub(crate) trait Http: Send + Sync {
    /// Sends `request` and reads its answer.
    ///
    /// # Errors
    ///
    /// [`Unreachable`] when no whole answer came.
    fn send(&self, request: &Request) -> Result<Response, Unreachable>;
}

/// [`Http`] over ureq, with rustls.
#[derive(Debug)]
pub(crate) struct UreqHttp {
    agent: Agent,
}

impl UreqHttp {
    /// The client: no redirects followed, statuses as answers, 30 s a request.
    pub(crate) fn new() -> Self {
        let agent = Agent::config_builder()
            .http_status_as_error(false)
            .max_redirects(0)
            .https_only(false)
            .timeout_global(Some(TIMEOUT))
            .user_agent(AutoHeaderValue::Provided(std::sync::Arc::new(format!(
                "emberhut-desktop/{}",
                env!("CARGO_PKG_VERSION")
            ))))
            .build()
            .new_agent();
        Self { agent }
    }

    fn run(&self, request: &Request) -> Result<ureq::http::Response<ureq::Body>, ureq::Error> {
        let bearer = request.bearer.as_deref();
        let url = request.url.as_str();
        match (&request.method, &request.body) {
            (Method::Get, _) => headers(self.agent.get(url), bearer).call(),
            (Method::Delete, _) => headers(self.agent.delete(url), bearer).call(),
            (Method::Post, Body::Empty) => headers(self.agent.post(url), bearer).send_empty(),
            (Method::Post, Body::Form(fields)) => headers(self.agent.post(url), bearer)
                .send_form(fields.iter().map(|(key, value)| (*key, value.as_str()))),
            (Method::Post, Body::Json(bytes)) => headers(self.agent.post(url), bearer)
                .header("Content-Type", "application/json")
                .send(&bytes[..]),
        }
    }
}

/// The headers every request has: JSON accepted, and the bearer when there is one.
fn headers<B>(builder: RequestBuilder<B>, bearer: Option<&str>) -> RequestBuilder<B> {
    let builder = builder.header("Accept", "application/json");
    match bearer {
        Some(token) => builder.header("Authorization", format!("Bearer {token}")),
        None => builder,
    }
}

impl Http for UreqHttp {
    fn send(&self, request: &Request) -> Result<Response, Unreachable> {
        let unreachable = |cause: Box<dyn std::error::Error + Send + Sync>| Unreachable {
            host: host_of(&request.url).to_owned(),
            cause,
        };
        let mut answer = self
            .run(request)
            .map_err(|error| unreachable(Box::new(error)))?;
        let status = answer.status().as_u16();
        let retry_after = answer
            .headers()
            .get("retry-after")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.trim().parse::<u64>().ok())
            .map(Duration::from_secs);
        let body = answer
            .body_mut()
            .with_config()
            .limit(MAX_ANSWER)
            .read_to_vec()
            .map_err(|error| unreachable(Box::new(error)))?;
        Ok(Response {
            status,
            retry_after,
            body,
        })
    }
}

/// The host of `url`, for words about it.
fn host_of(url: &str) -> &str {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    rest.split(['/', '?', '#']).next().unwrap_or(rest)
}

#[cfg(test)]
pub(crate) mod fake {
    //! A scripted [`Http`]: answers by method and URL, in turn, and records every request.

    use std::collections::{HashMap, VecDeque};
    use std::fmt;
    use std::sync::{Mutex, PoisonError};

    use super::{Http, Method, Request, Response, Unreachable};

    /// One scripted answer.
    enum Scripted {
        /// This response.
        Answer(Response),
        /// No answer at all.
        Unanswered,
        /// What this runs gives, run while the request is out (to act in the middle of it).
        Run(Box<dyn FnOnce() -> Response + Send>),
    }

    impl fmt::Debug for Scripted {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self {
                Self::Answer(response) => f.debug_tuple("Answer").field(response).finish(),
                Self::Unanswered => f.write_str("Unanswered"),
                Self::Run(_) => f.write_str("Run"),
            }
        }
    }

    /// The fake.
    #[derive(Debug, Default)]
    pub(crate) struct FakeHttp {
        answers: Mutex<HashMap<(Method, String), VecDeque<Scripted>>>,
        sent: Mutex<Vec<Request>>,
    }

    impl FakeHttp {
        /// Queues `status` with `body` as the next answer to `method` on `url`.
        pub(crate) fn answer(&self, method: Method, url: &str, status: u16, body: &str) {
            self.respond(method, url, response(status, body));
        }

        /// Queues `response` as the next answer to `method` on `url`.
        pub(crate) fn respond(&self, method: Method, url: &str, response: Response) {
            self.queue(method, url, Scripted::Answer(response));
        }

        /// Queues `run` for the next request to `method` on `url`: it runs while the request
        /// is out, and what it gives is the answer.
        pub(crate) fn meanwhile(
            &self,
            method: Method,
            url: &str,
            run: impl FnOnce() -> Response + Send + 'static,
        ) {
            self.queue(method, url, Scripted::Run(Box::new(run)));
        }

        /// The next request to `method` on `url` gets no answer.
        pub(crate) fn fail(&self, method: Method, url: &str) {
            self.queue(method, url, Scripted::Unanswered);
        }

        fn queue(&self, method: Method, url: &str, answer: Scripted) {
            self.answers
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .entry((method, url.to_owned()))
                .or_default()
                .push_back(answer);
        }

        /// Every request sent, in order.
        pub(crate) fn sent(&self) -> Vec<Request> {
            self.sent
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone()
        }

        /// The requests sent to `url`.
        pub(crate) fn sent_to(&self, url: &str) -> Vec<Request> {
            self.sent()
                .into_iter()
                .filter(|request| request.url == url)
                .collect()
        }
    }

    /// An answer of `status` with `body`.
    pub(crate) fn response(status: u16, body: &str) -> Response {
        Response {
            status,
            retry_after: None,
            body: body.as_bytes().to_vec(),
        }
    }

    impl Http for FakeHttp {
        fn send(&self, request: &Request) -> Result<Response, Unreachable> {
            self.sent
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(request.clone());
            let next = self
                .answers
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .get_mut(&(request.method, request.url.clone()))
                .and_then(VecDeque::pop_front);
            match next {
                Some(Scripted::Answer(response)) => Ok(response),
                Some(Scripted::Run(run)) => Ok(run()),
                Some(Scripted::Unanswered) => Err(Unreachable {
                    host: "fake".to_owned(),
                    cause: "no route".into(),
                }),
                None => panic!("no answer scripted for {request:?}"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_requests_debug_shows_no_secret() {
        let request = Request::form(
            "https://accounts.example/oauth/token",
            vec![("refresh_token", "refresh-secret".to_owned())],
        )
        .bearer("access-secret");
        let shown = format!("{request:?}");
        assert!(!shown.contains("secret"), "{shown}");
        assert!(shown.contains("accounts.example/oauth/token"), "{shown}");
        assert_eq!(request.field("refresh_token"), Some("refresh-secret"));
    }

    #[test]
    fn the_host_is_read_from_the_url() {
        assert_eq!(
            host_of("https://relay.example/v1/machines"),
            "relay.example"
        );
        assert_eq!(host_of("http://127.0.0.1:8443?x"), "127.0.0.1:8443");
    }
}
