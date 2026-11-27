//! The transport to one base URL, over `ureq`.

use std::borrow::Cow;
use std::io::{self, Read};
use std::{error, fmt};

use http::Uri;
use ureq::config::AutoHeaderValue;
use ureq::{Agent, AsSendBody, BodyReader, SendBody};

use super::{Answer, Body, Request, Timeouts, Transport, TransportError, TransportErrorKind};
use crate::config::checked_base_url;
use crate::{ClientConfig, InvalidBaseUrl};

/// Requests to one base URL, over HTTP or HTTPS: whatever reaches the machine at an address
/// (loopback, the local network, a VPN, a public name, a relay's URL).
///
/// The request URL is the base URL followed by the request's path, so a base URL with a path
/// prefix serves the machine under it. Redirects aren't followed: a redirect comes back as the
/// answer it is. When the request names no `Accept-Encoding`, it asks for gzip and undoes it
/// before the body is read; a request that names one gets the body as the server sent it. It
/// adds no `User-Agent`: that is the request's.
///
/// A failure is [`TransportErrorKind::NotSent`] only when it is sure nothing went out: the name
/// didn't resolve, the connection was refused or took too long. A failed TLS handshake comes
/// back as [`TransportErrorKind::Sent`], the safe side, since `ureq` gives it as an io error
/// that can't be told from one on an established connection.
#[derive(Debug)]
pub struct Direct {
    base: String,
    agent: Agent,
}

impl Direct {
    /// A transport to `base_url`, an address as [`ClientConfig::new`] takes it.
    ///
    /// # Errors
    ///
    /// [`InvalidBaseUrl`] when it isn't such an address.
    pub fn new(base_url: &str) -> Result<Self, InvalidBaseUrl> {
        Ok(Self::to(checked_base_url(base_url)?))
    }

    /// A transport to the base URL of `config`, which [`ClientConfig::new`] has checked.
    #[must_use]
    pub(crate) fn for_config(config: &ClientConfig) -> Self {
        Self::to(config.base_url().to_owned())
    }

    /// A transport to `base`, a checked base URL without its trailing `/`.
    fn to(base: String) -> Self {
        let agent = Agent::config_builder()
            .http_status_as_error(false)
            .max_redirects(0)
            .user_agent(AutoHeaderValue::None)
            .build()
            .new_agent();
        Self { base, agent }
    }

    fn run(
        &self,
        head: http::Request<()>,
        body: impl AsSendBody,
        timeouts: Timeouts,
    ) -> Result<http::Response<ureq::Body>, ureq::Error> {
        let request = self
            .agent
            .configure_request(head.map(|()| body))
            .timeout_connect(timeouts.connect())
            .timeout_global(timeouts.total())
            .timeout_per_read(timeouts.read())
            .timeout_per_write(timeouts.write())
            .build();
        self.agent.run(request)
    }
}

impl Transport for Direct {
    fn send(&self, request: Request<'_>, timeouts: Timeouts) -> Result<Answer, TransportError> {
        let (method, path, headers, body) = request.into_parts();
        let uri = Uri::try_from(format!("{}{path}", self.base))
            .map_err(|error| TransportError::new(error).with_kind(TransportErrorKind::NotSent))?;
        let mut head = http::Request::new(());
        *head.method_mut() = method;
        *head.uri_mut() = uri;
        *head.headers_mut() = headers;
        let response = match body {
            Body::Empty => self.run(head, (), timeouts),
            Body::Bytes(bytes) => self.run(head, &*bytes, timeouts),
            Body::Stream(reader) => {
                let mut outgoing = Outgoing(reader);
                self.run(head, SendBody::from_reader(&mut outgoing), timeouts)
            }
        }
        .map_err(failure)?;
        let (head, body) = response.into_parts();
        Ok(Answer::new(
            head.status,
            head.headers,
            Incoming(body.into_reader()),
        ))
    }

    fn base_url(&self) -> Cow<'_, str> {
        Cow::Borrowed(&self.base)
    }
}

/// `ureq`'s error as a transport's: how far the request got, and whether a wait ran out.
fn failure(error: ureq::Error) -> TransportError {
    use TransportErrorKind::{Body, NotSent, Sent};
    use ureq::Error as E;
    use ureq::Timeout as T;

    let kind = match &error {
        E::Io(io)
            if io
                .get_ref()
                .is_some_and(<dyn error::Error + Send + Sync>::is::<CallerBody>) =>
        {
            Body
        }
        // Only a refusal is sure to come from the dial: the socket's reads and writes give
        // their errors as io errors too, unreachable hosts and lost addresses included.
        E::Io(io) if io.kind() == io::ErrorKind::ConnectionRefused => NotSent,
        E::Timeout(T::Resolve | T::Connect)
        | E::HostNotFound
        | E::ConnectionFailed
        | E::Tls(_)
        | E::Rustls(_)
        | E::TlsRequired
        | E::BadUri(_)
        | E::Http(_)
        | E::RequireHttpsOnly(_)
        | E::InvalidProxyUrl
        | E::ConnectProxyFailed(_) => NotSent,
        _ => Sent,
    };
    let failure = if matches!(error, E::Timeout(_)) {
        TransportError::timeout(error)
    } else {
        TransportError::new(error)
    };
    failure.with_kind(kind)
}

/// The caller's request body, whose failures carry [`CallerBody`], so that they come back as
/// [`TransportErrorKind::Body`] and not as the way to the machine failing.
struct Outgoing<'a>(Box<dyn Read + 'a>);

/// A read that was interrupted is tried again, so every failure that leaves carries the mark.
impl Read for Outgoing<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        loop {
            match self.0.read(buffer) {
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => return Err(io::Error::new(error.kind(), CallerBody(error))),
                read => return read,
            }
        }
    }
}

/// The mark on a failure of the caller's request body.
#[derive(Debug)]
struct CallerBody(io::Error);

impl fmt::Display for CallerBody {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

impl error::Error for CallerBody {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        self.0.source()
    }
}

/// An answer's body. A read fails with the [`TransportError`] inside the [`io::Error`], of
/// kind [`io::ErrorKind::TimedOut`] when a wait ran out (`ureq` gives that as another kind).
struct Incoming(BodyReader<'static>);

impl Read for Incoming {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.0.read(buffer).map_err(|error| {
            let kind = error.kind();
            let failure = failure(ureq::Error::from(error)).with_kind(TransportErrorKind::Sent);
            let kind = if failure.is_timeout() {
                io::ErrorKind::TimedOut
            } else {
                kind
            };
            io::Error::new(kind, failure)
        })
    }
}

#[cfg(test)]
mod tests {
    use std::io::{BufRead, BufReader, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::mpsc::{self, Sender};
    use std::thread::JoinHandle;
    use std::time::Duration;

    use http::header::{ACCEPT_ENCODING, CONTENT_LENGTH, CONTENT_TYPE, LOCATION, USER_AGENT};
    use http::uri::PathAndQuery;
    use http::{HeaderValue, Method, StatusCode};

    use super::*;

    /// "hello, emberhut", gzipped.
    const GZIPPED: &[u8] = b"\x1f\x8b\x08\x00\x00\x00\x00\x00\x02\x03\xcb\x48\xcd\xc9\xc9\xd7\x51\x48\xcd\x4d\x4a\x2d\xca\x28\x2d\x01\x00\x67\x35\xe1\x1b\x0f\x00\x00\x00";

    /// A server on an OS-chosen port that takes one connection, reads its request (the body as
    /// long as its `Content-Length` says), writes `head` and `body`, and holds the connection
    /// open until it is finished.
    struct Once {
        url: String,
        release: Sender<()>,
        server: JoinHandle<(String, Vec<u8>)>,
    }

    impl Once {
        fn serve(head: &'static str, body: &'static [u8]) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let url = format!("http://{}", listener.local_addr().unwrap());
            let (release, released) = mpsc::channel::<()>();
            let server = std::thread::spawn(move || {
                let (stream, _) = listener.accept().unwrap();
                let request = read_request(&stream);
                (&stream).write_all(head.as_bytes()).unwrap();
                (&stream).write_all(body).unwrap();
                // Until the test is done, or has dropped its end.
                released.recv().ok();
                request
            });
            Self {
                url,
                release,
                server,
            }
        }

        /// Closes the connection, and gives the request head (lower case) and body.
        fn finish(self) -> (String, Vec<u8>) {
            self.release.send(()).ok();
            self.server.join().unwrap()
        }
    }

    fn read_request(stream: &TcpStream) -> (String, Vec<u8>) {
        let mut reader = BufReader::new(stream);
        let mut head = String::new();
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            if line == "\r\n" {
                break;
            }
            head.push_str(&line);
        }
        let length = head
            .lines()
            .find_map(|line| {
                line.to_ascii_lowercase()
                    .strip_prefix("content-length: ")?
                    .parse()
                    .ok()
            })
            .unwrap_or(0);
        let mut body = vec![0; length];
        reader.read_exact(&mut body).unwrap();
        (head.to_ascii_lowercase(), body)
    }

    fn waits() -> Timeouts {
        Timeouts::default()
            .with_connect(Duration::from_secs(5))
            .with_total(Duration::from_secs(5))
    }

    fn get(path: &'static str) -> Request<'static> {
        Request::new(Method::GET, PathAndQuery::from_static(path))
    }

    fn text(answer: &mut Answer) -> Vec<u8> {
        let mut body = Vec::new();
        answer.body_mut().read_to_end(&mut body).unwrap();
        body
    }

    #[test]
    fn sends_the_request_under_the_base_url_and_reads_the_answer() {
        let server = Once::serve(
            "HTTP/1.1 201 Created\r\nx-kind: test\r\ncontent-length: 5\r\n\r\n",
            b"hello",
        );
        let direct = Direct::new(&format!("{}/emberhut/", server.url)).unwrap();
        assert_eq!(direct.base_url(), format!("{}/emberhut", server.url));
        let request = Request::new(
            Method::POST,
            PathAndQuery::from_static("/api/talk?room=main"),
        )
        .with_header(USER_AGENT, HeaderValue::from_static("emberhut-client/test"))
        .with_header(CONTENT_TYPE, HeaderValue::from_static("application/json"))
        .with_body(Body::Bytes(Cow::Borrowed(b"{\"text\":\"hi\"}")));
        let mut answer = direct.send(request, waits()).unwrap();
        assert_eq!(answer.status(), StatusCode::CREATED);
        assert_eq!(answer.headers()["x-kind"], "test");
        assert_eq!(text(&mut answer), b"hello");

        let (head, body) = server.finish();
        assert!(
            head.starts_with("post /emberhut/api/talk?room=main http/1.1\r\n"),
            "{head}"
        );
        assert!(
            head.contains("user-agent: emberhut-client/test\r\n"),
            "{head}"
        );
        assert_eq!(head.matches("user-agent").count(), 1, "{head}");
        assert!(
            head.contains("content-type: application/json\r\n"),
            "{head}"
        );
        assert!(head.contains("content-length: 13\r\n"), "{head}");
        assert_eq!(body, b"{\"text\":\"hi\"}");
    }

    #[test]
    fn adds_no_user_agent_of_its_own() {
        let server = Once::serve("HTTP/1.1 204 No Content\r\n\r\n", b"");
        let answer = Direct::new(&server.url)
            .unwrap()
            .send(get("/api/machines"), waits())
            .unwrap();
        assert_eq!(answer.status(), StatusCode::NO_CONTENT);
        let (head, _) = server.finish();
        assert!(head.starts_with("get /api/machines http/1.1\r\n"), "{head}");
        assert!(!head.contains("user-agent"), "{head}");
    }

    #[test]
    fn a_redirect_is_an_answer_not_followed() {
        let server = Once::serve(
            "HTTP/1.1 302 Found\r\nlocation: /elsewhere\r\ncontent-length: 0\r\n\r\n",
            b"",
        );
        let answer = Direct::new(&server.url)
            .unwrap()
            .send(get("/api/state"), waits())
            .unwrap();
        assert_eq!(answer.status(), StatusCode::FOUND);
        assert_eq!(answer.headers()[LOCATION], "/elsewhere");
        server.finish();
    }

    #[test]
    fn asks_for_gzip_and_undoes_it() {
        let server = Once::serve(
            "HTTP/1.1 200 OK\r\ncontent-encoding: gzip\r\ncontent-length: 35\r\n\r\n",
            GZIPPED,
        );
        let mut answer = Direct::new(&server.url)
            .unwrap()
            .send(get("/api/state"), waits())
            .unwrap();
        assert_eq!(text(&mut answer), b"hello, emberhut");
        let (head, _) = server.finish();
        assert!(head.contains("accept-encoding: gzip\r\n"), "{head}");
    }

    #[test]
    fn a_named_encoding_passes_through_unchanged() {
        let server = Once::serve("HTTP/1.1 200 OK\r\ncontent-length: 35\r\n\r\n", GZIPPED);
        let request =
            get("/uploads/a.gz").with_header(ACCEPT_ENCODING, HeaderValue::from_static("identity"));
        let mut answer = Direct::new(&server.url)
            .unwrap()
            .send(request, waits())
            .unwrap();
        assert_eq!(answer.headers()[CONTENT_LENGTH], "35");
        assert_eq!(text(&mut answer), GZIPPED);
        let (head, _) = server.finish();
        assert!(head.contains("accept-encoding: identity\r\n"), "{head}");
        assert_eq!(head.matches("accept-encoding").count(), 1, "{head}");
    }

    #[test]
    fn streams_a_body_of_the_length_given() {
        let server = Once::serve("HTTP/1.1 200 OK\r\ncontent-length: 0\r\n\r\n", b"");
        let request = Request::new(
            Method::POST,
            PathAndQuery::from_static("/api/uploads?name=a"),
        )
        .with_header(CONTENT_LENGTH, HeaderValue::from_static("6"))
        .with_body(Body::Stream(Box::new(&b"abcdef"[..])));
        let answer = Direct::new(&server.url)
            .unwrap()
            .send(request, waits())
            .unwrap();
        assert_eq!(answer.status(), StatusCode::OK);
        let (head, body) = server.finish();
        assert!(!head.contains("transfer-encoding"), "{head}");
        assert_eq!(body, b"abcdef");
    }

    #[test]
    fn an_interrupted_read_of_the_body_is_tried_again() {
        /// Interrupted before each byte.
        struct Jumpy(&'static [u8], bool);
        impl Read for Jumpy {
            fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
                self.1 = !self.1;
                if self.1 {
                    return Err(io::Error::from(io::ErrorKind::Interrupted));
                }
                let Some((first, rest)) = self.0.split_first() else {
                    return Ok(0);
                };
                buffer[0] = *first;
                self.0 = rest;
                Ok(1)
            }
        }
        let server = Once::serve("HTTP/1.1 200 OK\r\ncontent-length: 0\r\n\r\n", b"");
        let request = Request::new(
            Method::POST,
            PathAndQuery::from_static("/api/uploads?name=a"),
        )
        .with_header(CONTENT_LENGTH, HeaderValue::from_static("3"))
        .with_body(Body::Stream(Box::new(Jumpy(b"abc", false))));
        let answer = Direct::new(&server.url)
            .unwrap()
            .send(request, waits())
            .unwrap();
        assert_eq!(answer.status(), StatusCode::OK);
        assert_eq!(server.finish().1, b"abc");
    }

    #[test]
    fn an_answer_that_never_comes_is_a_timeout_after_sending() {
        // Bound, so the connection is made, and never accepted, so nothing answers.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let waits = Timeouts::default().with_total(Duration::from_millis(200));
        let error = Direct::new(&url)
            .unwrap()
            .send(get("/api/state"), waits)
            .unwrap_err();
        assert!(error.is_timeout(), "{error}");
        assert_eq!(error.kind(), TransportErrorKind::Sent);
        drop(listener);
    }

    #[test]
    fn a_body_that_stops_coming_times_out_on_read() {
        let server = Once::serve("HTTP/1.1 200 OK\r\ncontent-length: 10\r\n\r\n", b"abc");
        let waits = Timeouts::default().with_read(Duration::from_millis(200));
        let mut answer = Direct::new(&server.url)
            .unwrap()
            .send(get("/api/live"), waits)
            .unwrap();
        let mut body = Vec::new();
        let error = answer.body_mut().read_to_end(&mut body).unwrap_err();
        assert_eq!(body, b"abc");
        assert_eq!(error.kind(), io::ErrorKind::TimedOut, "{error}");
        let failure = TransportError::from(error);
        assert!(failure.is_timeout());
        assert_eq!(failure.kind(), TransportErrorKind::Sent);
        server.finish();
    }

    #[test]
    fn a_transfer_that_cant_go_out_times_out_on_write() {
        // Bound and never accepted: nothing reads, so the socket's buffers fill and writes wait.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let length = 256_u64 << 20;
        let request = Request::new(
            Method::POST,
            PathAndQuery::from_static("/api/uploads?name=a"),
        )
        .with_header(CONTENT_LENGTH, HeaderValue::from(length))
        .with_body(Body::Stream(Box::new(io::repeat(b'a').take(length))));
        let waits = Timeouts::default().with_write(Duration::from_millis(200));
        let error = Direct::new(&url).unwrap().send(request, waits).unwrap_err();
        assert!(error.is_timeout(), "{error}");
        assert_eq!(error.kind(), TransportErrorKind::Sent);
        drop(listener);
    }

    #[test]
    fn a_failing_request_body_is_the_callers() {
        struct Broken;
        impl Read for Broken {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                Err(io::Error::other("disk gone"))
            }
        }
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let request = Request::new(
            Method::POST,
            PathAndQuery::from_static("/api/uploads?name=a"),
        )
        .with_header(CONTENT_LENGTH, HeaderValue::from_static("6"))
        .with_body(Body::Stream(Box::new(Broken)));
        let error = Direct::new(&url)
            .unwrap()
            .send(request, waits())
            .unwrap_err();
        assert_eq!(error.kind(), TransportErrorKind::Body, "{error}");
        assert!(!error.is_timeout());
        drop(listener);
    }

    #[test]
    fn a_refused_connection_sent_nothing() {
        // A port with no socket on it at all: the kernel answers the connection with a reset at
        // once, on Linux and macOS alike. (Port 0 can't be connected to on macOS, and macOS lets
        // a connection to a bound socket that isn't listening hang instead of refusing it.) The
        // port comes from a listener closed just before, which the system doesn't hand out
        // again straight away.
        let port = {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            listener.local_addr().unwrap().port()
        };
        let error = Direct::new(&format!("http://127.0.0.1:{port}"))
            .unwrap()
            .send(get("/api/state"), waits())
            .unwrap_err();
        assert_eq!(error.kind(), TransportErrorKind::NotSent, "{error}");
        assert!(!error.is_timeout());
        assert_ne!(error.to_string(), "");
    }

    #[test]
    fn ureq_errors_say_how_far_the_request_got() {
        use TransportErrorKind::{Body, NotSent, Sent};
        use ureq::Timeout as T;
        let kind = |error| {
            let failure = failure(error);
            (failure.kind(), failure.is_timeout())
        };
        assert_eq!(kind(ureq::Error::Timeout(T::Connect)), (NotSent, true));
        assert_eq!(kind(ureq::Error::Timeout(T::Resolve)), (NotSent, true));
        assert_eq!(kind(ureq::Error::Timeout(T::PerRead)), (Sent, true));
        assert_eq!(kind(ureq::Error::Timeout(T::Global)), (Sent, true));
        assert_eq!(kind(ureq::Error::HostNotFound), (NotSent, false));
        let refused = io::Error::from(io::ErrorKind::ConnectionRefused);
        assert_eq!(kind(ureq::Error::Io(refused)), (NotSent, false));
        // How rustls's failed handshake arrives, as on a connection already carrying bytes.
        let handshake = io::Error::new(io::ErrorKind::InvalidData, "bad certificate");
        assert_eq!(kind(ureq::Error::Io(handshake)), (Sent, false));
        for later in [
            io::ErrorKind::AddrNotAvailable,
            io::ErrorKind::HostUnreachable,
            io::ErrorKind::NetworkUnreachable,
        ] {
            let error = io::Error::from(later);
            assert_eq!(kind(ureq::Error::Io(error)), (Sent, false), "{later:?}");
        }
        let reset = io::Error::from(io::ErrorKind::ConnectionReset);
        assert_eq!(kind(ureq::Error::Io(reset)), (Sent, false));
        let callers = io::Error::other(CallerBody(io::Error::other("disk gone")));
        assert_eq!(kind(ureq::Error::Io(callers)), (Body, false));
    }

    #[test]
    fn refuses_what_isnt_a_server_address() {
        let error = Direct::new("ftp://example.com").unwrap_err();
        assert_eq!(error.reason(), "not http or https");
    }

    #[test]
    fn a_configs_address_is_taken_as_it_is() {
        let config = ClientConfig::new("http://127.0.0.1:1/emberhut/").unwrap();
        assert_eq!(
            Direct::for_config(&config).base_url(),
            "http://127.0.0.1:1/emberhut"
        );
    }
}
