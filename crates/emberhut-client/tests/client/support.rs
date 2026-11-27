//! A test HTTP server bound to an OS-chosen port, recording what it is sent.

use std::io::Read;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, SystemTime};

use emberhut_api::auth::{AccessToken, RefreshToken};
use emberhut_api::ids::{DeviceId, InstallId};
use emberhut_client::{Client, ClientConfig, MemorySessionStore, Session, SessionTokens};
use serde_json::Value;
use tiny_http::{Header, Response, Server, StatusCode};

/// A request as the server got it.
#[derive(Debug, Clone)]
pub(crate) struct Recorded {
    pub(crate) method: String,
    pub(crate) path: String,
    pub(crate) query: String,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) body: Vec<u8>,
}

impl Recorded {
    pub(crate) fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    pub(crate) fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap()
    }
}

/// What the server answers.
pub(crate) enum Reply {
    /// A JSON body.
    Json(u16, Value),
    /// A JSON body with extra headers.
    JsonWith(u16, Value, Vec<(&'static str, String)>),
    /// No body.
    Empty(u16, Vec<(&'static str, String)>),
    /// Bytes that aren't JSON.
    Text(u16, &'static str),
    /// A file's bytes, with its `Content-Length`.
    Bytes(u16, Vec<u8>),
    /// An event stream, written chunk by chunk.
    Stream(Vec<&'static str>),
}

type Handler = dyn Fn(&Recorded) -> Reply + Send + Sync;

/// The server. Dropping it stops it.
pub(crate) struct TestServer {
    url: String,
    server: Arc<Server>,
    accept: Option<JoinHandle<()>>,
    requests: Arc<Mutex<Vec<Recorded>>>,
}

impl TestServer {
    pub(crate) fn start(handler: impl Fn(&Recorded) -> Reply + Send + Sync + 'static) -> Self {
        let server = Arc::new(Server::http("127.0.0.1:0").unwrap());
        let url = format!("http://{}", server.server_addr().to_ip().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let handler: Arc<Handler> = Arc::new(handler);
        let accept = {
            let (server, requests) = (Arc::clone(&server), Arc::clone(&requests));
            std::thread::spawn(move || {
                while let Ok(request) = server.recv() {
                    let (handler, requests) = (Arc::clone(&handler), Arc::clone(&requests));
                    std::thread::spawn(move || answer(request, &*handler, &requests));
                }
            })
        };
        Self {
            url,
            server,
            accept: Some(accept),
            requests,
        }
    }

    pub(crate) fn requests_to(&self, path: &str) -> Vec<Recorded> {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.path == path)
            .cloned()
            .collect()
    }

    /// A client of this server with this session.
    pub(crate) fn client(&self, session: Session) -> Client {
        self.client_with(Arc::new(MemorySessionStore::with_session(session)))
    }

    /// A client of this server over this store.
    pub(crate) fn client_with(&self, store: Arc<MemorySessionStore>) -> Client {
        Client::new(&self.config(), store).unwrap()
    }

    /// A client of this server with this session, taking downloads up to `limit` bytes.
    pub(crate) fn client_limited(&self, session: Session, limit: u64) -> Client {
        let store = Arc::new(MemorySessionStore::with_session(session));
        Client::new(&self.config().with_download_limit(limit), store).unwrap()
    }

    /// A client of this server with no session, told whether the server speaks the built
    /// dialect.
    pub(crate) fn client_speaking(&self, built_dialect: bool) -> Client {
        let config = self.config().with_built_dialect(built_dialect);
        Client::new(&config, MemorySessionStore::new()).unwrap()
    }

    fn config(&self) -> ClientConfig {
        ClientConfig::new(&self.url)
            .unwrap()
            .with_device("Emberhut on a test machine", "linux")
            .with_request_timeout(Duration::from_secs(10))
            .with_live_read_timeout(Duration::from_secs(10))
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.server.unblock();
        if let Some(accept) = self.accept.take() {
            accept.join().unwrap();
        }
    }
}

fn answer(mut request: tiny_http::Request, handler: &Handler, requests: &Mutex<Vec<Recorded>>) {
    let (path, query) = match request.url().split_once('?') {
        Some((path, query)) => (path.to_owned(), query.to_owned()),
        None => (request.url().to_owned(), String::new()),
    };
    let mut body = Vec::new();
    request.as_reader().read_to_end(&mut body).unwrap();
    let recorded = Recorded {
        method: request.method().to_string(),
        path,
        query,
        headers: request
            .headers()
            .iter()
            .map(|h| (h.field.to_string(), h.value.to_string()))
            .collect(),
        body,
    };
    requests.lock().unwrap().push(recorded.clone());
    let header =
        |name: &str, value: &str| Header::from_bytes(name.as_bytes(), value.as_bytes()).unwrap();
    let json = |status: u16, value: &Value, extra: &[(&'static str, String)]| {
        let mut response = Response::from_data(serde_json::to_vec(value).unwrap())
            .with_status_code(status)
            .with_header(header("Content-Type", "application/json"));
        for (name, value) in extra {
            response.add_header(header(name, value));
        }
        response.boxed()
    };
    let response = match handler(&recorded) {
        Reply::Json(status, value) => json(status, &value, &[]),
        Reply::JsonWith(status, value, extra) => json(status, &value, &extra),
        Reply::Empty(status, extra) => {
            let mut response = Response::empty(status);
            for (name, value) in &extra {
                response.add_header(header(name, value));
            }
            response.boxed()
        }
        Reply::Text(status, text) => Response::from_string(text).with_status_code(status).boxed(),
        // Whole, with its Content-Length, as the reference server sends a file (not chunked).
        Reply::Bytes(status, bytes) => Response::from_data(bytes)
            .with_status_code(status)
            .with_chunked_threshold(usize::MAX)
            .boxed(),
        Reply::Stream(chunks) => Response::new(
            StatusCode(200),
            vec![
                header("Content-Type", "text/event-stream"),
                header("Cache-Control", "no-cache"),
            ],
            Chunks(chunks.into_iter().map(str::as_bytes).collect()),
            None,
            None,
        )
        .boxed(),
    };
    // The client may have gone (a stopped stream): nothing to do about it.
    if let Err(error) = request.respond(response) {
        eprintln!("test server: the client went away: {error}");
    }
}

/// A body read in the chunks given.
struct Chunks(std::collections::VecDeque<&'static [u8]>);

impl FromIterator<&'static [u8]> for Chunks {
    fn from_iter<I: IntoIterator<Item = &'static [u8]>>(iter: I) -> Self {
        Self(iter.into_iter().collect())
    }
}

impl Read for Chunks {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let Some(chunk) = self.0.front_mut() else {
            return Ok(0);
        };
        let n = chunk.len().min(buffer.len());
        buffer[..n].copy_from_slice(&chunk[..n]);
        *chunk = &chunk[n..];
        if chunk.is_empty() {
            self.0.pop_front();
        }
        Ok(n)
    }
}

/// A fixture of the contract crate, shaped as the reference server writes it.
pub(crate) fn fixture(name: &str) -> Value {
    let path = format!(
        "{}/../emberhut-api/tests/fixtures/{name}",
        env!("CARGO_MANIFEST_DIR")
    );
    serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap()
}

/// A session that isn't signed in.
pub(crate) fn signed_out() -> Session {
    Session {
        install: InstallId::new("zr-test-install"),
        tokens: None,
    }
}

/// A signed-in session with these tokens, its access token good until `expires`.
pub(crate) fn signed_in(refresh: &str, expires: SystemTime) -> Session {
    Session {
        install: InstallId::new("zr-test-install"),
        tokens: Some(SessionTokens {
            access: AccessToken::new(access_for(refresh)),
            refresh: RefreshToken::new(refresh),
            device: Some(DeviceId::new(3)),
            access_expires: expires,
        }),
    }
}

/// The access token that goes with a refresh token, in these tests.
pub(crate) fn access_for(refresh: &str) -> String {
    format!("access-for-{refresh}")
}

/// An access token good for an hour.
pub(crate) fn fresh(refresh: &str) -> Session {
    signed_in(refresh, SystemTime::now() + Duration::from_secs(3600))
}

/// An access token that ran out long ago.
pub(crate) fn expired(refresh: &str) -> Session {
    signed_in(refresh, SystemTime::UNIX_EPOCH)
}
