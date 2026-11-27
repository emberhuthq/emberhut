//! An answer as a transport hands it back.

use std::fmt;
use std::io::Read;

use http::{HeaderMap, StatusCode};

/// A machine's answer: its status and headers, and its body, read as it arrives (a whole JSON
/// document, a file, or an event stream that lasts as long as the server keeps it open).
///
/// The body is the content as sent, once the transport has undone what it took care of on the
/// wire (chunking, and a compression it asked for). A body that breaks off fails a read with
/// the error the transport met, often with a [`TransportError`](super::TransportError) inside
/// the [`io::Error`](std::io::Error).
/// [`io::ErrorKind::TimedOut`](std::io::ErrorKind::TimedOut) means a wait ran out.
pub struct Answer {
    status: StatusCode,
    headers: HeaderMap,
    body: Box<dyn Read + Send>,
}

impl Answer {
    /// An answer with this status, these headers and this body.
    #[must_use]
    pub fn new(status: StatusCode, headers: HeaderMap, body: impl Read + Send + 'static) -> Self {
        Self {
            status,
            headers,
            body: Box::new(body),
        }
    }

    /// An answer from the parts [`into_parts`](Self::into_parts) gave, its body kept as it is.
    #[must_use]
    pub fn from_parts(status: StatusCode, headers: HeaderMap, body: Box<dyn Read + Send>) -> Self {
        Self {
            status,
            headers,
            body,
        }
    }

    /// Its status.
    #[must_use]
    pub fn status(&self) -> StatusCode {
        self.status
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

    /// Its body, to read.
    pub fn body_mut(&mut self) -> &mut (dyn Read + Send) {
        &mut *self.body
    }

    /// Its body, kept after the head is dropped (to follow a stream).
    #[must_use]
    pub fn into_body(self) -> Box<dyn Read + Send> {
        self.body
    }

    /// Its status, headers and body, to build another answer from.
    #[must_use]
    pub fn into_parts(self) -> (StatusCode, HeaderMap, Box<dyn Read + Send>) {
        (self.status, self.headers, self.body)
    }
}

/// The status and headers; the body is a stream, not shown.
impl fmt::Debug for Answer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Answer")
            .field("status", &self.status)
            .field("headers", &self.headers)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use http::HeaderValue;
    use http::header::ETAG;

    use super::*;

    #[test]
    fn its_parts_come_back_as_given() {
        let mut headers = HeaderMap::new();
        headers.insert(ETAG, HeaderValue::from_static("\"q1\""));
        let mut answer = Answer::new(StatusCode::OK, headers, &b"body"[..]);
        assert_eq!(answer.status(), StatusCode::OK);
        assert_eq!(answer.headers()[ETAG], "\"q1\"");
        assert!(format!("{answer:?}").contains("200"));
        let mut first = [0; 2];
        answer.body_mut().read_exact(&mut first).unwrap();
        assert_eq!(&first, b"bo");
        answer.headers_mut().remove(ETAG);
        let (status, headers, body) = answer.into_parts();
        let again = Answer::from_parts(status, headers, body);
        assert_eq!(again.status(), StatusCode::OK);
        let (status, headers, mut body) = again.into_parts();
        let mut rest = String::new();
        body.read_to_string(&mut rest).unwrap();
        assert_eq!(
            (status, headers.len(), rest.as_str()),
            (StatusCode::OK, 0, "dy")
        );
    }
}
