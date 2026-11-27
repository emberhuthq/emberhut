//! A request as a transport carries it.

use std::borrow::Cow;
use std::fmt;
use std::io::Read;

use http::uri::PathAndQuery;
use http::{HeaderMap, HeaderName, HeaderValue, Method};

/// A request to a machine: a method, a path and query on the machine (no scheme or host: the
/// transport knows where the machine is), headers and a body.
///
/// The headers are the request's own: the transport adds only what its wire needs (`Host`,
/// framing, the compression it can undo). A header whose value is marked sensitive
/// ([`HeaderValue::set_sensitive`], as the `Authorization` this crate sends is) never shows in
/// `Debug`.
#[derive(Debug)]
pub struct Request<'a> {
    method: Method,
    path: PathAndQuery,
    headers: HeaderMap,
    body: Body<'a>,
}

/// The body of a [`Request`].
pub enum Body<'a> {
    /// None.
    Empty,
    /// Bytes held whole (a JSON body).
    Bytes(Cow<'a, [u8]>),
    /// Bytes read as they go out (an upload), as many as the request's `Content-Length` says;
    /// without one, it is sent chunked until the reader ends. It is read once, so a request
    /// with such a body can't be sent again ([`Request::try_clone`] has no copy of it).
    Stream(Box<dyn Read + 'a>),
}

impl<'a> Request<'a> {
    /// A request of `path` (and its query) with `method`, no headers and no body.
    #[must_use]
    pub fn new(method: Method, path: PathAndQuery) -> Self {
        Self {
            method,
            path,
            headers: HeaderMap::new(),
            body: Body::Empty,
        }
    }

    /// The same request with this header added (beside any of the same name).
    #[must_use]
    pub fn with_header(mut self, name: HeaderName, value: HeaderValue) -> Self {
        self.headers.append(name, value);
        self
    }

    /// The same request with these headers in place of its own.
    #[must_use]
    pub fn with_headers(mut self, headers: HeaderMap) -> Self {
        self.headers = headers;
        self
    }

    /// The same request with this body.
    #[must_use]
    pub fn with_body(mut self, body: Body<'a>) -> Self {
        self.body = body;
        self
    }

    /// Its method.
    #[must_use]
    pub fn method(&self) -> &Method {
        &self.method
    }

    /// Its path on the machine, with the query.
    #[must_use]
    pub fn path(&self) -> &PathAndQuery {
        &self.path
    }

    /// Its headers.
    #[must_use]
    pub fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    /// Its headers, to change.
    pub fn headers_mut(&mut self) -> &mut HeaderMap {
        &mut self.headers
    }

    /// Its body.
    #[must_use]
    pub fn body(&self) -> &Body<'a> {
        &self.body
    }

    /// Its body, to change or to read from.
    pub fn body_mut(&mut self) -> &mut Body<'a> {
        &mut self.body
    }

    /// A copy to send again, borrowing this one's body; none when the body is a
    /// [`Body::Stream`], which can be read only once.
    #[must_use]
    pub fn try_clone(&self) -> Option<Request<'_>> {
        let body = match &self.body {
            Body::Empty => Body::Empty,
            Body::Bytes(bytes) => Body::Bytes(Cow::Borrowed(bytes)),
            Body::Stream(_) => return None,
        };
        Some(Request {
            method: self.method.clone(),
            path: self.path.clone(),
            headers: self.headers.clone(),
            body,
        })
    }

    /// Its method, path, headers and body, to send or to build another request from.
    #[must_use]
    pub fn into_parts(self) -> (Method, PathAndQuery, HeaderMap, Body<'a>) {
        (self.method, self.path, self.headers, self.body)
    }
}

/// The body's kind and, when it is held whole, its size; never its contents, which may be a
/// token.
impl fmt::Debug for Body<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("Empty"),
            Self::Bytes(bytes) => f.debug_tuple("Bytes").field(&bytes.len()).finish(),
            Self::Stream(_) => f.write_str("Stream"),
        }
    }
}

#[cfg(test)]
mod tests {
    use http::header::{AUTHORIZATION, CONTENT_TYPE};

    use super::*;

    #[test]
    fn debug_hides_sensitive_headers_and_the_body() {
        let mut bearer = HeaderValue::from_static("Bearer secret-token");
        bearer.set_sensitive(true);
        let request = Request::new(Method::POST, PathAndQuery::from_static("/api/talk"))
            .with_header(AUTHORIZATION, bearer)
            .with_header(CONTENT_TYPE, HeaderValue::from_static("application/json"))
            .with_body(Body::Bytes(Cow::Borrowed(
                b"{\"refresh\":\"secret-refresh\"}",
            )));
        let debug = format!("{request:?}");
        assert!(!debug.contains("secret"), "{debug}");
        assert!(debug.contains("/api/talk"), "{debug}");
        assert!(debug.contains("application/json"), "{debug}");
        assert!(debug.contains("Bytes(28)"), "{debug}");
        let stream = Request::new(Method::POST, PathAndQuery::from_static("/api/uploads"))
            .with_body(Body::Stream(Box::new(&b"file"[..])));
        assert!(format!("{stream:?}").contains("Stream"));
    }

    #[test]
    fn a_copy_borrows_the_body_and_a_stream_has_none() {
        let mut bearer = HeaderValue::from_static("Bearer token");
        bearer.set_sensitive(true);
        let request = Request::new(Method::POST, PathAndQuery::from_static("/api/talk"))
            .with_header(AUTHORIZATION, bearer)
            .with_body(Body::Bytes(Cow::Owned(b"{}".to_vec())));
        let copy = request.try_clone().unwrap();
        assert_eq!(copy.method(), Method::POST);
        assert_eq!(copy.path().as_str(), "/api/talk");
        assert_eq!(copy.headers(), request.headers());
        assert!(copy.headers()[AUTHORIZATION].is_sensitive());
        assert!(matches!(copy.body(), Body::Bytes(Cow::Borrowed(b"{}"))));
        let empty = Request::new(Method::GET, PathAndQuery::from_static("/api/state"));
        assert!(matches!(empty.try_clone().unwrap().body(), Body::Empty));
        let stream = Request::new(Method::POST, PathAndQuery::from_static("/api/uploads"))
            .with_body(Body::Stream(Box::new(&b"file"[..])));
        assert!(stream.try_clone().is_none());
    }

    #[test]
    fn parts_come_back_as_given() {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("text/plain"));
        let mut request = Request::new(Method::GET, PathAndQuery::from_static("/a?b=c"))
            .with_headers(headers.clone());
        request.headers_mut().remove(CONTENT_TYPE);
        *request.body_mut() = Body::Bytes(Cow::Owned(b"x".to_vec()));
        assert!(matches!(request.body(), Body::Bytes(bytes) if bytes.as_ref() == b"x"));
        let (method, path, kept, body) = request.into_parts();
        assert_eq!(method, Method::GET);
        assert_eq!(path.as_str(), "/a?b=c");
        assert!(kept.is_empty());
        assert!(matches!(body, Body::Bytes(_)));
    }
}
