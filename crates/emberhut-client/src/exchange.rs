//! Requests to the server's routes and what their answers mean, over the client's
//! [`Transport`].
//!
//! Answers of every status come back as answers (not transport errors); [`refusal`] turns a
//! non-2xx one into an [`Error`] with the server's own words.

use std::borrow::Cow;
use std::io::Read;
use std::time::{Duration, SystemTime};

use emberhut_api::auth::AccessToken;
use emberhut_api::error::ErrorBody;
use emberhut_api::headers;
use http::header::{ACCEPT, ACCEPT_ENCODING, CONTENT_LENGTH, CONTENT_TYPE, DATE, USER_AGENT};
use http::uri::PathAndQuery;
use http::{HeaderName, HeaderValue, Method, StatusCode};
use serde::Serialize;
use serde::de::DeserializeOwned;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc2822;

use crate::dialect::Dialect;
use crate::transport::{
    Answer, Body, Request, Timeouts, Transport, TransportError, TransportErrorKind,
};
use crate::{ClientConfig, Error};

/// The largest answer read: the state is 1 to 2 MB today.
const MAX_BODY: u64 = 64 << 20;

/// The `User-Agent` of every request.
const USER_AGENT_VALUE: &str = concat!("emberhut-client/", env!("CARGO_PKG_VERSION"));

/// The transport to the server, and how long each kind of request waits.
#[derive(Debug)]
pub(crate) struct Http {
    transport: Box<dyn Transport>,
    /// The server's spelling of the wire words.
    dialect: Dialect,
    /// For requests: a whole answer within the request timeout.
    request: Timeouts,
    /// For the live stream: no overall timeout, only one per read.
    live: Timeouts,
    /// For uploads and downloads: no overall timeout, only one per write and per read.
    transfer: Timeouts,
}

impl Http {
    pub(crate) fn new(transport: Box<dyn Transport>, config: &ClientConfig) -> Self {
        Self {
            transport,
            dialect: config.dialect(),
            request: config.request_timeouts(),
            live: config.live_timeouts(),
            transfer: config.transfer_timeouts(),
        }
    }

    pub(crate) fn base_url(&self) -> Cow<'_, str> {
        self.transport.base_url()
    }

    /// The server's spelling of the wire words.
    pub(crate) fn dialect(&self) -> Dialect {
        self.dialect
    }

    /// Sends `request` in the server's spelling of the wire words and reads its answer in the
    /// contract's ([`crate::dialect`]: as they are, unless the crate was built with one and
    /// the server speaks it).
    fn send(&self, request: Request<'_>, timeouts: Timeouts) -> Result<Answer, Error> {
        let dialect = self.dialect.table();
        let answer = self.transport.send(dialect.request(request), timeouts)?;
        Ok(dialect.answer(answer))
    }

    /// A `GET` of a route, signed when there is a token, with these extra headers.
    pub(crate) fn get(
        &self,
        path: &str,
        query: &str,
        bearer: Option<&AccessToken>,
        extra: &[(&'static str, String)],
    ) -> Result<Answer, Error> {
        let mut request = request(Method::GET, path, query, bearer)?;
        for (name, value) in extra {
            let name = HeaderName::from_bytes(name.as_bytes()).map_err(invalid)?;
            request = request.with_header(name, HeaderValue::try_from(value).map_err(invalid)?);
        }
        self.send(request, self.request)
    }

    /// An unsigned `GET` of a route, given `within` for the whole answer in place of the
    /// request timeout (a quick probe).
    pub(crate) fn get_within(&self, path: &str, within: Duration) -> Result<Answer, Error> {
        let request = request(Method::GET, path, "", None)?;
        self.send(
            request,
            self.request.with_connect(within).with_total(within),
        )
    }

    /// A `GET` of a stream.
    pub(crate) fn get_stream(
        &self,
        path: &str,
        query: &str,
        bearer: Option<&AccessToken>,
    ) -> Result<Answer, Error> {
        let request = request(Method::GET, path, query, bearer)?
            .with_header(ACCEPT, HeaderValue::from_static("text/event-stream"));
        self.send(request, self.live)
    }

    /// A `POST` of a JSON body (see [`encode`]) to a route, signed when there is a token.
    pub(crate) fn post_json(
        &self,
        path: &str,
        body: &[u8],
        bearer: Option<&AccessToken>,
    ) -> Result<Answer, Error> {
        let request = request(Method::POST, path, "", bearer)?
            .with_header(CONTENT_TYPE, HeaderValue::from_static("application/json"))
            .with_body(Body::Bytes(Cow::Borrowed(body)));
        self.send(request, self.request)
    }

    /// A `POST` as [`Http::post_json`], given `within` for the whole answer in place of the
    /// request timeout (a route the server answers slowly on purpose).
    pub(crate) fn post_json_within(
        &self,
        path: &str,
        body: &[u8],
        within: Duration,
        bearer: Option<&AccessToken>,
    ) -> Result<Answer, Error> {
        let request = request(Method::POST, path, "", bearer)?
            .with_header(CONTENT_TYPE, HeaderValue::from_static("application/json"))
            .with_body(Body::Bytes(Cow::Borrowed(body)));
        self.send(request, self.request.with_total(within))
    }

    /// A `GET` of a file to stream to disk: no overall timeout, and not compressed, so its
    /// `Content-Length` is the file's size.
    pub(crate) fn get_file(
        &self,
        path: &str,
        query: &str,
        bearer: Option<&AccessToken>,
    ) -> Result<Answer, Error> {
        let request = request(Method::GET, path, query, bearer)?
            .with_header(ACCEPT_ENCODING, HeaderValue::from_static("identity"));
        self.send(request, self.transfer)
    }

    /// A `POST` of `length` raw bytes read from `body` (`application/octet-stream`, with its
    /// `Content-Length`) to a route with a query, signed when there is a token.
    pub(crate) fn post_raw(
        &self,
        path: &str,
        query: &str,
        length: u64,
        body: &mut dyn Read,
        bearer: Option<&AccessToken>,
    ) -> Result<Answer, Error> {
        let request = request(Method::POST, path, query, bearer)?
            .with_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/octet-stream"),
            )
            .with_header(CONTENT_LENGTH, HeaderValue::from(length))
            .with_body(Body::Stream(Box::new(body)));
        self.send(request, self.transfer)
    }
}

impl Http {
    /// A `DELETE` of a route, signed when there is a token.
    pub(crate) fn delete(&self, path: &str, bearer: Option<&AccessToken>) -> Result<Answer, Error> {
        self.send(request(Method::DELETE, path, "", bearer)?, self.request)
    }

    /// A `POST` of a JSON body whose answer is a stream: no overall timeout, and as long as
    /// an upload's for each read (a model may think quietly for minutes before it writes).
    pub(crate) fn post_stream(
        &self,
        path: &str,
        body: &[u8],
        bearer: Option<&AccessToken>,
    ) -> Result<Answer, Error> {
        let request = request(Method::POST, path, "", bearer)?
            .with_header(CONTENT_TYPE, HeaderValue::from_static("application/json"))
            .with_header(ACCEPT, HeaderValue::from_static("text/event-stream"))
            .with_body(Body::Bytes(Cow::Borrowed(body)));
        self.send(request, self.transfer)
    }
}

/// A request of `path` and `query` with this client's `User-Agent`, signed when there is a
/// token. The token's header is marked sensitive, so no `Debug` shows it.
fn request(
    method: Method,
    path: &str,
    query: &str,
    bearer: Option<&AccessToken>,
) -> Result<Request<'static>, Error> {
    let target = if query.is_empty() {
        PathAndQuery::try_from(path)
    } else {
        PathAndQuery::try_from(format!("{path}?{query}"))
    }
    .map_err(invalid)?;
    let mut request = Request::new(method, target)
        .with_header(USER_AGENT, HeaderValue::from_static(USER_AGENT_VALUE));
    if let Some(token) = bearer {
        let mut value = HeaderValue::try_from(headers::bearer(token)).map_err(invalid)?;
        value.set_sensitive(true);
        request = request.with_header(http::header::AUTHORIZATION, value);
    }
    Ok(request)
}

/// A request that couldn't be written (a path or header value HTTP doesn't take): it fails as
/// a request that didn't get through, with nothing sent.
fn invalid(error: impl Into<Box<dyn std::error::Error + Send + Sync>>) -> Error {
    TransportError::new(error)
        .with_kind(TransportErrorKind::NotSent)
        .into()
}

/// A request body as JSON.
pub(crate) fn encode<B: Serialize>(body: &B) -> Result<Vec<u8>, Error> {
    serde_json::to_vec(body).map_err(Error::Encode)
}

/// When the server says it answered (its `Date` header), for durations it gives as instants.
pub(crate) fn server_date(answer: &Answer) -> Option<SystemTime> {
    let date = answer.headers().get(DATE)?.to_str().ok()?;
    OffsetDateTime::parse(date, &Rfc2822)
        .ok()
        .map(SystemTime::from)
}

/// The whole body of an answer, up to 64 MiB.
pub(crate) fn body_bytes(answer: &mut Answer) -> Result<Vec<u8>, Error> {
    let mut bytes = Vec::new();
    Read::take(answer.body_mut(), MAX_BODY + 1)
        .read_to_end(&mut bytes)
        .map_err(TransportError::from)?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_BODY {
        return Err(TransportError::new("the answer is longer than 64 MiB").into());
    }
    Ok(bytes)
}

/// The body of a 2xx answer as `T`; any other status as its refusal.
pub(crate) fn json<T: DeserializeOwned>(mut answer: Answer) -> Result<T, Error> {
    if !answer.status().is_success() {
        return Err(refusal(answer));
    }
    let bytes = body_bytes(&mut answer)?;
    serde_json::from_slice(&bytes).map_err(Error::Decode)
}

/// What a non-2xx answer means: a 401 asking for sign-in, or the status with the server's
/// `error` text when its body has one.
pub(crate) fn refusal(mut answer: Answer) -> Error {
    let status = answer.status();
    match body_bytes(&mut answer) {
        Ok(bytes) => refusal_of(status, &bytes),
        Err(error) => {
            tracing::debug!(%status, %error, "couldn't read a refusal's body");
            Error::Status {
                status: status.as_u16(),
                message: None,
            }
        }
    }
}

/// What a non-2xx answer with this body means.
pub(crate) fn refusal_of(status: StatusCode, body: &[u8]) -> Error {
    match serde_json::from_slice::<ErrorBody>(body).ok() {
        Some(body) if status == StatusCode::UNAUTHORIZED && body.sign_in => Error::SignInRequired,
        body => Error::Status {
            status: status.as_u16(),
            message: body.map(|b| b.error),
        },
    }
}

#[cfg(test)]
mod tests {
    use std::io;
    use std::time::Duration;

    use http::HeaderMap;

    use super::*;

    fn answer(headers: HeaderMap, body: impl Read + Send + 'static) -> Answer {
        Answer::new(StatusCode::OK, headers, body)
    }

    #[test]
    fn reads_an_http_date() {
        let mut headers = HeaderMap::new();
        headers.insert(
            DATE,
            HeaderValue::from_static("Tue, 24 Nov 2026 22:57:53 GMT"),
        );
        let expected = SystemTime::UNIX_EPOCH + Duration::from_secs(1_795_561_073);
        assert_eq!(server_date(&answer(headers, io::empty())), Some(expected));
        let undated = answer(HeaderMap::new(), io::empty());
        assert_eq!(server_date(&undated), None);
    }

    #[test]
    fn a_body_is_read_up_to_its_limit() {
        let limit = usize::try_from(MAX_BODY).unwrap();
        let mut longest = answer(HeaderMap::new(), io::repeat(b'a').take(MAX_BODY));
        assert_eq!(body_bytes(&mut longest).unwrap().len(), limit);
        let mut longer = answer(HeaderMap::new(), io::repeat(b'a').take(MAX_BODY + 1));
        let error = body_bytes(&mut longer).unwrap_err();
        assert!(matches!(error, Error::Network(_)), "{error}");
    }

    #[test]
    fn a_body_that_breaks_is_a_network_error() {
        struct Slow;
        impl Read for Slow {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                Err(io::Error::new(io::ErrorKind::TimedOut, "slow"))
            }
        }
        let error = body_bytes(&mut answer(HeaderMap::new(), Slow)).unwrap_err();
        let Error::Network(network) = error else {
            panic!("a network error")
        };
        assert!(network.is_timeout());
    }

    #[test]
    fn a_path_http_doesnt_take_never_leaves() {
        let error = request(Method::GET, "/a b", "", None).unwrap_err();
        assert!(matches!(error, Error::Network(_)), "{error}");
    }
}
