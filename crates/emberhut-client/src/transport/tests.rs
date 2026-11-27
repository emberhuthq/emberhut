//! The client over a fake transport: what it hands the transport for each kind of request, and
//! how it reads answers and streams that come back through it.

use std::borrow::Cow;
use std::collections::VecDeque;
use std::io::{self, Read};
use std::ops::ControlFlow;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use emberhut_api::auth::{AccessToken, RefreshToken};
use emberhut_api::headers::{self, ETag};
use emberhut_api::ids::{DeviceId, InstallId, RoomId};
use emberhut_api::live::{Live, LiveEvent};
use emberhut_api::talk::TalkRequest;
use http::{HeaderMap, HeaderValue, Method, StatusCode};
use serde_json::{Value, json};

use super::{Answer, Body, Request, Timeouts, Transport, TransportError, TransportErrorKind};
use crate::{
    Client, ClientConfig, Error, LiveEnd, MemorySessionStore, Session, SessionTokens, StatePoll,
    StateReply, Upload,
};

/// What the fake transport was handed.
#[derive(Debug)]
struct Sent {
    method: Method,
    path: String,
    headers: HeaderMap,
    body: Vec<u8>,
    streamed: bool,
    timeouts: Timeouts,
}

impl Sent {
    /// Its headers as (name, value) text, in the order they were added.
    fn header_list(&self) -> Vec<(String, String)> {
        self.headers
            .iter()
            .map(|(name, value)| (name.to_string(), value.to_str().unwrap().to_owned()))
            .collect()
    }
}

/// What the fake transport does with the next request.
#[derive(Debug)]
enum Then {
    Answer(u16, Vec<(&'static str, &'static str)>, Vec<u8>),
    Fail(TransportError),
}

/// A transport that records each request (reading its body whole) and gives the next scripted
/// answer.
#[derive(Debug)]
struct Fake {
    base: String,
    sent: Mutex<Vec<Sent>>,
    script: Mutex<VecDeque<Then>>,
}

impl Fake {
    fn new(script: impl IntoIterator<Item = Then>) -> Arc<Self> {
        Arc::new(Self {
            base: "http://route.invalid".to_owned(),
            sent: Mutex::new(Vec::new()),
            script: Mutex::new(script.into_iter().collect()),
        })
    }

    fn sent(&self) -> Vec<Sent> {
        std::mem::take(&mut *self.sent.lock().unwrap())
    }
}

impl Transport for Fake {
    fn send(&self, request: Request<'_>, timeouts: Timeouts) -> Result<Answer, TransportError> {
        let (method, path, headers, body) = request.into_parts();
        let (body, streamed) = match body {
            Body::Empty => (Vec::new(), false),
            Body::Bytes(bytes) => (bytes.into_owned(), false),
            Body::Stream(mut reader) => {
                let mut bytes = Vec::new();
                reader.read_to_end(&mut bytes)?;
                (bytes, true)
            }
        };
        self.sent.lock().unwrap().push(Sent {
            method,
            path: path.to_string(),
            headers,
            body,
            streamed,
            timeouts,
        });
        match self
            .script
            .lock()
            .unwrap()
            .pop_front()
            .expect("a scripted answer")
        {
            Then::Answer(status, extra, body) => {
                let mut headers = HeaderMap::new();
                for (name, value) in extra {
                    headers.append(name, HeaderValue::from_static(value));
                }
                let status = StatusCode::from_u16(status).unwrap();
                Ok(Answer::new(status, headers, io::Cursor::new(body)))
            }
            Then::Fail(error) => Err(error),
        }
    }

    fn base_url(&self) -> Cow<'_, str> {
        Cow::Borrowed(&self.base)
    }
}

const USER_AGENT: &str = concat!("emberhut-client/", env!("CARGO_PKG_VERSION"));

fn config() -> ClientConfig {
    let second = Duration::from_secs;
    ClientConfig::new("http://configured.invalid")
        .unwrap()
        .with_connect_timeout(second(1))
        .with_request_timeout(second(2))
        .with_live_read_timeout(second(3))
        .with_upload_timeout(second(4))
}

/// A client over `fake`, signed in with an access token good for an hour.
fn client(fake: &Arc<Fake>) -> Client {
    let session = Session {
        install: InstallId::new("zr-test-install"),
        tokens: Some(SessionTokens {
            access: AccessToken::new("access-token"),
            refresh: RefreshToken::new("refresh-token"),
            device: Some(DeviceId::new(3)),
            access_expires: SystemTime::now() + Duration::from_secs(3600),
        }),
    };
    let store = MemorySessionStore::with_session(session);
    Client::with_transport(&config(), Arc::clone(fake), store).unwrap()
}

fn fixture(name: &str) -> Vec<u8> {
    let path = format!(
        "{}/../emberhut-api/tests/fixtures/{name}",
        env!("CARGO_MANIFEST_DIR")
    );
    std::fs::read(path).unwrap()
}

fn json_answer(status: u16, body: &Value) -> Then {
    let content = vec![("content-type", "application/json")];
    Then::Answer(status, content, serde_json::to_vec(body).unwrap())
}

/// The headers every request of a signed-in device carries, then `more`.
fn signed(more: &[(&str, &str)]) -> Vec<(String, String)> {
    let mut headers = vec![
        ("user-agent".to_owned(), USER_AGENT.to_owned()),
        ("authorization".to_owned(), "Bearer access-token".to_owned()),
    ];
    headers.extend(more.iter().map(|(n, v)| ((*n).to_owned(), (*v).to_owned())));
    headers
}

#[test]
fn a_poll_sends_the_held_state_and_the_looking_room() {
    let fake = Fake::new([Then::Answer(304, vec![("etag", "\"q1\"")], Vec::new())]);
    let poll = StatePoll::new(RoomId::MAIN)
        .held(Some(ETag::new("\"q1\"")))
        .looking(true);
    assert_eq!(client(&fake).state(&poll).unwrap(), StateReply::Unchanged);

    let [sent] = <[Sent; 1]>::try_from(fake.sent()).unwrap();
    assert_eq!(sent.method, Method::GET);
    assert_eq!(sent.path, "/api/state?room=1");
    assert_eq!(
        sent.header_list(),
        signed(&[("if-none-match", "\"q1\""), (headers::LOOKING, "1")])
    );
    assert!(sent.headers["authorization"].is_sensitive());
    assert!(sent.body.is_empty() && !sent.streamed);
    assert_eq!(sent.timeouts, config().request_timeouts());
}

#[test]
fn a_fresh_state_comes_with_its_etag() {
    let fake = Fake::new([Then::Answer(
        200,
        vec![("etag", "\"q2\"")],
        fixture("state.json"),
    )]);
    let reply = client(&fake).state(&StatePoll::new(RoomId::MAIN)).unwrap();
    let StateReply::Fresh { state, etag } = reply else {
        panic!("a fresh state: {reply:?}")
    };
    assert_eq!(etag, Some(ETag::new("\"q2\"")));
    assert_ne!(state.rooms.len(), 0);
    assert_eq!(fake.sent()[0].header_list(), signed(&[]));
}

#[test]
fn a_post_sends_its_json_body() {
    let talk: TalkRequest = serde_json::from_slice(&fixture("talk_request.json")).unwrap();
    let reply: Value = serde_json::from_slice(&fixture("talk_reply.json")).unwrap();
    let fake = Fake::new([json_answer(200, &reply)]);
    let answer = client(&fake).talk(&talk).unwrap();
    assert!(answer.started);

    let [sent] = <[Sent; 1]>::try_from(fake.sent()).unwrap();
    assert_eq!(
        (&sent.method, sent.path.as_str()),
        (&Method::POST, "/api/talk")
    );
    assert_eq!(
        sent.header_list(),
        signed(&[("content-type", "application/json")])
    );
    assert_eq!(sent.body, serde_json::to_vec(&talk).unwrap());
    assert!(!sent.streamed);
    assert_eq!(sent.timeouts, config().request_timeouts());
}

#[test]
fn the_live_stream_is_read_as_it_comes_through_the_transport() {
    let stream = "data: {\"text\":\"\",\"doing\":\"Reading\"}\n\n: ping\n\ndata: null\n\n";
    let fake = Fake::new([Then::Answer(
        200,
        vec![("content-type", "text/event-stream")],
        stream.as_bytes().to_vec(),
    )]);
    let mut events = Vec::new();
    let end = client(&fake)
        .follow_live(RoomId::MAIN, || false, |event| events.push(event))
        .unwrap();
    assert_eq!(end, LiveEnd::Closed);
    let working = LiveEvent::Working(Live {
        text: String::new(),
        doing: Some("Reading".to_owned()),
    });
    assert_eq!(events, [working, LiveEvent::Idle]);

    let [sent] = <[Sent; 1]>::try_from(fake.sent()).unwrap();
    assert_eq!(
        (&sent.method, sent.path.as_str()),
        (&Method::GET, "/api/live?room=1")
    );
    assert_eq!(
        sent.header_list(),
        signed(&[("accept", "text/event-stream")])
    );
    assert_eq!(sent.timeouts, config().live_timeouts());
}

#[test]
fn an_upload_streams_its_bytes_with_their_length() {
    let reply: Value = serde_json::from_slice(&fixture("upload_reply.json")).unwrap();
    let fake = Fake::new([json_answer(200, &reply)]);
    let mut told = Vec::new();
    let file = client(&fake)
        .upload(&Upload::bytes("a b.png", vec![1, 2, 3, 4]), |progress| {
            told.push(progress.done);
            ControlFlow::Continue(())
        })
        .unwrap();
    assert_eq!(file.name, "Week 3 example.png");
    assert_eq!(
        told,
        [4],
        "the transport read the body through the progress"
    );

    let [sent] = <[Sent; 1]>::try_from(fake.sent()).unwrap();
    assert_eq!(
        (&sent.method, sent.path.as_str()),
        (&Method::POST, "/api/uploads?name=a%20b.png")
    );
    assert_eq!(
        sent.header_list(),
        signed(&[
            ("content-type", "application/octet-stream"),
            ("content-length", "4")
        ])
    );
    assert_eq!(sent.body, [1, 2, 3, 4]);
    assert!(sent.streamed);
    assert_eq!(sent.timeouts, config().transfer_timeouts());
}

#[test]
fn a_download_is_written_as_it_is_read() {
    let fake = Fake::new([Then::Answer(
        200,
        vec![("content-length", "5")],
        b"abcde".to_vec(),
    )]);
    let mut file = Vec::new();
    let mut told = Vec::new();
    let size = client(&fake)
        .download("/uploads/a.png?v=2", &mut file, |progress| {
            told.push(progress);
            ControlFlow::Continue(())
        })
        .unwrap();
    assert_eq!((size, file.as_slice()), (5, &b"abcde"[..]));
    assert_eq!(told.last().map(|p| (p.done, p.total)), Some((5, Some(5))));

    let [sent] = <[Sent; 1]>::try_from(fake.sent()).unwrap();
    assert_eq!(
        (&sent.method, sent.path.as_str()),
        (&Method::GET, "/uploads/a.png?v=2")
    );
    assert_eq!(
        sent.header_list(),
        signed(&[("accept-encoding", "identity")])
    );
    assert_eq!(sent.timeouts, config().transfer_timeouts());
}

#[test]
fn a_refused_token_is_renewed_and_the_request_sent_again() {
    let sign_in: Value = serde_json::from_slice(&fixture("error_sign_in.json")).unwrap();
    let mut renewed: Value = serde_json::from_slice(&fixture("auth_accepted.json")).unwrap();
    renewed["expires"] = json!("2099-01-01T00:00:00.000Z");
    let machines: Value = serde_json::from_slice(&fixture("machines.json")).unwrap();
    let fake = Fake::new([
        json_answer(401, &sign_in),
        json_answer(200, &renewed),
        json_answer(200, &machines),
    ]);
    let page = client(&fake).machines().unwrap();
    assert_ne!(page.machines.len(), 0);

    let [first, renewal, again] = <[Sent; 3]>::try_from(fake.sent()).unwrap();
    assert_eq!(
        (&first.method, first.path.as_str()),
        (&Method::GET, "/api/machines")
    );
    assert_eq!(first.header_list(), signed(&[]));
    assert_eq!(
        (&renewal.method, renewal.path.as_str()),
        (&Method::POST, "/api/auth/refresh")
    );
    assert_eq!(
        renewal.header_list(),
        [
            ("user-agent".to_owned(), USER_AGENT.to_owned()),
            ("content-type".to_owned(), "application/json".to_owned())
        ],
        "a renewal carries no access token"
    );
    let body: Value = serde_json::from_slice(&renewal.body).unwrap();
    assert_eq!(
        body,
        json!({ "refresh": "refresh-token", "install": "zr-test-install" })
    );
    assert_eq!(again.path, "/api/machines");
    assert_eq!(
        again.headers["authorization"],
        "Bearer access-token-for-tests"
    );
}

#[test]
fn a_refusal_in_an_answer_is_the_servers_words() {
    let refused: Value = serde_json::from_slice(&fixture("error_refused.json")).unwrap();
    let fake = Fake::new([json_answer(400, &refused)]);
    let talk: TalkRequest = serde_json::from_slice(&fixture("talk_request.json")).unwrap();
    let error = client(&fake).talk(&talk).unwrap_err();
    assert!(
        matches!(error, Error::Status { status: 400, ref message } if message.as_deref() == refused["error"].as_str()),
        "{error}"
    );
}

#[test]
fn a_transport_failure_is_a_network_error() {
    let fake = Fake::new([Then::Fail(TransportError::timeout("no answer in time"))]);
    let error = client(&fake).machines().unwrap_err();
    assert!(error.is_transient());
    let Error::Network(network) = error else {
        panic!("a network error: {error}")
    };
    assert!(network.is_timeout());
    assert_eq!(network.to_string(), "no answer in time");
}

#[test]
fn a_stream_that_breaks_is_a_network_error() {
    /// A body that fails after its first event.
    struct Breaks(io::Cursor<&'static [u8]>);
    impl Read for Breaks {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            match self.0.read(buffer)? {
                0 => Err(io::Error::new(io::ErrorKind::ConnectionReset, "reset")),
                read => Ok(read),
            }
        }
    }
    /// A transport whose live stream breaks.
    #[derive(Debug)]
    struct Breaking(String);
    impl Transport for Breaking {
        fn send(&self, _: Request<'_>, _: Timeouts) -> Result<Answer, TransportError> {
            let body = Breaks(io::Cursor::new(b"data: null\n\n"));
            Ok(Answer::new(StatusCode::OK, HeaderMap::new(), body))
        }

        fn base_url(&self) -> Cow<'_, str> {
            Cow::Borrowed(&self.0)
        }
    }
    let client = Client::with_transport(
        &config(),
        Breaking(String::new()),
        MemorySessionStore::new(),
    )
    .unwrap();
    let mut events = Vec::new();
    let error = client
        .follow_live(RoomId::MAIN, || false, |event| events.push(event))
        .unwrap_err();
    assert_eq!(events, [LiveEvent::Idle]);
    assert!(matches!(error, Error::Network(_)), "{error}");
}

#[test]
fn the_base_url_is_the_transports() {
    let fake = Fake::new([]);
    assert_eq!(client(&fake).base_url(), "http://route.invalid");
    let direct = Client::new(&config(), MemorySessionStore::new()).unwrap();
    assert_eq!(direct.base_url(), "http://configured.invalid");
}

#[test]
fn a_wrapper_sees_and_changes_whole_requests_and_answers() {
    /// Adds a header to every request and every answer, and upper-cases answers' bodies.
    #[derive(Debug)]
    struct Marking<T>(T);
    impl<T: Transport> Transport for Marking<T> {
        fn send(
            &self,
            mut request: Request<'_>,
            timeouts: Timeouts,
        ) -> Result<Answer, TransportError> {
            request
                .headers_mut()
                .insert("x-wrapped", HeaderValue::from_static("yes"));
            let (status, mut headers, mut body) = self.0.send(request, timeouts)?.into_parts();
            headers.insert("x-unwrapped", HeaderValue::from_static("yes"));
            let mut text = Vec::new();
            body.read_to_end(&mut text)?;
            text.make_ascii_uppercase();
            Ok(Answer::new(status, headers, io::Cursor::new(text)))
        }

        fn base_url(&self) -> Cow<'_, str> {
            self.0.base_url()
        }
    }
    let fake = Fake::new([
        Then::Answer(200, Vec::new(), b"bytes".to_vec()),
        Then::Answer(200, Vec::new(), b"more".to_vec()),
    ]);
    let store = MemorySessionStore::new();
    let marking = Arc::new(Marking(Arc::clone(&fake)));
    let client = Client::with_transport(&config(), Arc::clone(&marking), store).unwrap();
    assert_eq!(client.file("/uploads/a.txt").unwrap(), b"BYTES");
    let [sent] = <[Sent; 1]>::try_from(fake.sent()).unwrap();
    assert_eq!(sent.headers["x-wrapped"], "yes");
    assert!(!sent.headers.contains_key("authorization"), "signed out");

    let request = Request::new(Method::GET, "/uploads/b.txt".parse().unwrap());
    let mut answer = marking.send(request, Timeouts::default()).unwrap();
    assert_eq!(answer.headers()["x-unwrapped"], "yes");
    let mut text = String::new();
    answer.body_mut().read_to_string(&mut text).unwrap();
    assert_eq!(text, "MORE");
}

/// Sends a request again, once, when the first attempt sent nothing, as the trait's contract
/// says a transport may.
#[derive(Debug)]
struct Retrying<T>(T);

impl<T: Transport> Transport for Retrying<T> {
    fn send(&self, request: Request<'_>, timeouts: Timeouts) -> Result<Answer, TransportError> {
        let Some(copy) = request.try_clone() else {
            return self.0.send(request, timeouts);
        };
        match self.0.send(copy, timeouts) {
            Err(error) if error.kind() == TransportErrorKind::NotSent => {
                self.0.send(request, timeouts)
            }
            sent => sent,
        }
    }

    fn base_url(&self) -> Cow<'_, str> {
        self.0.base_url()
    }
}

fn not_sent() -> Then {
    Then::Fail(TransportError::new("refused").with_kind(TransportErrorKind::NotSent))
}

#[test]
fn a_request_that_sent_nothing_may_go_again_whole() {
    let talk: TalkRequest = serde_json::from_slice(&fixture("talk_request.json")).unwrap();
    let reply: Value = serde_json::from_slice(&fixture("talk_reply.json")).unwrap();
    let fake = Fake::new([not_sent(), json_answer(200, &reply)]);
    let store = MemorySessionStore::new();
    let client = Client::with_transport(&config(), Retrying(Arc::clone(&fake)), store).unwrap();
    assert!(client.talk(&talk).unwrap().started);
    let [first, again] = <[Sent; 2]>::try_from(fake.sent()).unwrap();
    assert_eq!(first.path, again.path);
    assert_eq!(first.headers, again.headers);
    assert_eq!(first.body, again.body);
    assert_eq!(again.body, serde_json::to_vec(&talk).unwrap());
}

#[test]
fn a_streamed_request_goes_once_and_its_failure_back_to_the_caller() {
    let reply: Value = serde_json::from_slice(&fixture("upload_reply.json")).unwrap();
    let fake = Fake::new([not_sent(), json_answer(200, &reply)]);
    let store = MemorySessionStore::new();
    let client = Client::with_transport(&config(), Retrying(Arc::clone(&fake)), store).unwrap();
    let upload = Upload::bytes("a.png", vec![1, 2, 3]);
    let error = client
        .upload(&upload, |_| ControlFlow::Continue(()))
        .unwrap_err();
    assert!(matches!(error, Error::Network(_)), "{error}");
    assert_eq!(fake.sent().len(), 1, "sent once");
}
