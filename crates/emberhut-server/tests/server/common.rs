//! The test harness: a fresh app over its own database, with a clock the test moves and an
//! assistant the test scripts; requests through the router, checked against the `OpenAPI`
//! document.
#![expect(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test helpers: a failure here fails the test that called them"
)]

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, Response, StatusCode};
use emberhut_api::mcp::{CallToolResult, RpcResponse};
use emberhut_core::pairing::PairingRefusal;
use emberhut_core::session::Session;
use emberhut_core::{DeviceDetails, ManualClock, OsRandom, RoomId};
use emberhut_db::{Database, DatabaseUrl};
use emberhut_server::app::{AppState, Dependencies};
use emberhut_server::config::AuthMode;
use emberhut_server::services::dark::DarkModels;
use emberhut_server::services::pairing::PairingError;
use emberhut_server::services::push::PushSenders;
use emberhut_server::services::talk::{Assistant, AssistantError, LiveSink, Turn};
use emberhut_server::services::tasks::{TaskWorkers, Thumbnailer};
use futures_util::future::BoxFuture;
use http_body_util::BodyExt as _;
use jiff::{SignedDuration, Timestamp};
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::sync::Notify;
use tower::ServiceExt as _;

use crate::fake_browser::{FakeBrowser, FakeDesk};
use crate::fake_coding_app::FakeCodingApp;
use crate::fake_personal::Fakes;
use crate::fake_runner::FakeRunner;
use crate::fake_workers::FakeWorkers;
use crate::family::FakeFamily;
use crate::mac_routes::FakeMac;
use crate::parties::FakeModel;

/// Every (method, path, status) a request sent through a `TestApp` got, in this process.
static STATUSES_GOT: Mutex<BTreeSet<(String, String, u16)>> = Mutex::new(BTreeSet::new());

/// What requests in this process got so far: (method, path, status).
pub(crate) fn statuses_got() -> BTreeSet<(String, String, u16)> {
    STATUSES_GOT.lock().unwrap().clone()
}

/// A reqwest client for plain `http://` on loopback. The server's reqwest has no default
/// crypto provider (it is given ring's, ready-made), so the tests give theirs the same way.
pub(crate) fn http_client() -> reqwest::Client {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let tls = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_root_certificates(rustls::RootCertStore::empty())
        .with_no_client_auth();
    reqwest::Client::builder()
        .tls_backend_preconfigured(tls)
        .build()
        .unwrap()
}
/// Makes an app's assistant over its database and clock, given the app's own directory.
type AgentMaker<'a> =
    Box<dyn FnOnce(&Database, Arc<ManualClock>, &std::path::Path) -> Arc<dyn Assistant> + 'a>;

/// Changes an app's dependencies, given the default ones and the app's own directory.
type Change<'a> = Box<dyn FnOnce(Dependencies, &std::path::Path) -> Dependencies + 'a>;

/// How long a test request may take before the app answers 503: well above any route's real
/// time, so a machine busy with other builds doesn't turn a slow answer into a failure (the
/// limit itself is tested in the app's unit tests, with a limit of its own).
pub(crate) const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// A workers' area that only notes which tasks it was asked to start and stop.
#[derive(Debug, Default)]
pub(crate) struct FakeTaskWorkers {
    /// The tasks a worker was started on, in order.
    pub(crate) started: Mutex<Vec<i64>>,
    /// The tasks whose worker was stopped, in order.
    pub(crate) stopped: Mutex<Vec<i64>>,
}

impl TaskWorkers for FakeTaskWorkers {
    fn start(&self, task: emberhut_core::task::TaskId) -> BoxFuture<'_, Result<(), String>> {
        self.started.lock().unwrap().push(task.get());
        Box::pin(async { Ok(()) })
    }

    fn stop(&self, task: emberhut_core::task::TaskId) -> BoxFuture<'_, Result<(), String>> {
        self.stopped.lock().unwrap().push(task.get());
        Box::pin(async { Ok(()) })
    }
}

/// A thumbnailer that writes a few bytes where the thumbnail goes, instead of running ffmpeg.
#[derive(Debug)]
pub(crate) struct FakeThumbs;

impl Thumbnailer for FakeThumbs {
    fn make<'a>(
        &'a self,
        _: &'a std::path::Path,
        dest: &'a std::path::Path,
        _: bool,
    ) -> BoxFuture<'a, Result<(), String>> {
        Box::pin(async move { std::fs::write(dest, b"jpeg").map_err(|e| e.to_string()) })
    }
}

/// The token and key files a test app is configured with, in its own directory.
pub(crate) const PRIVATE_FILES: [&str; 3] =
    ["runner-tokens", "coding-app-tokens", "mac-helper-token"];

/// The app over a fresh database in its own directory, deleted when this is dropped. Its home
/// (where a task's files may come from, and files sent to a runner) is `home` in that
/// directory, its thumbnails are [`FakeThumbs`]' and the tasks' workers [`FakeTaskWorkers`]'.
pub(crate) struct TestApp {
    /// The clock every service reads; tests move it.
    pub(crate) clock: Arc<ManualClock>,
    /// The services, as handlers see them.
    pub(crate) state: AppState,
    /// The database's file, for a second connection or a URL.
    pub(crate) url: DatabaseUrl,
    /// The assistant the app answers with.
    pub(crate) assistant: Arc<Scripted>,
    /// The Mac helper the app reaches.
    pub(crate) mac: Arc<FakeMac>,
    /// The coding-thread app the server follows.
    pub(crate) coding_app: Arc<FakeCodingApp>,
    /// The workers' area the tasks start and stop workers in.
    pub(crate) task_workers: Arc<FakeTaskWorkers>,
    /// The runners the server reaches.
    pub(crate) runners: Arc<FakeRunner>,
    /// The assistant's workers and background tasks.
    pub(crate) workers: Arc<FakeWorkers>,
    root: std::path::PathBuf,
    /// The browser the assistant drives.
    pub(crate) browser: Arc<FakeBrowser>,
    /// The tasks, as the browser and the Mac tools reach them.
    pub(crate) desk: Arc<FakeDesk>,
    /// The model parties talk through.
    pub(crate) party_model: Arc<FakeModel>,
    /// The off-the-record room's cloud model.
    pub(crate) dark_cloud: Arc<FakeModel>,
    /// The off-the-record room's local model, tried first.
    pub(crate) dark_local: Arc<FakeModel>,
    _dir: TempDir,
}

impl TestApp {
    /// A new app that enforces sign-in, whose clock starts now and moves only when the test
    /// moves it.
    pub(crate) async fn new() -> Self {
        Self::with_mode(AuthMode::Enforce).await
    }

    /// A new app in `mode`.
    pub(crate) async fn with_mode(mode: AuthMode) -> Self {
        Self::with_push(mode, PushSenders::default()).await
    }

    /// A new app in `mode` that pushes through `push`. Its secrets directory is in the app's
    /// own directory.
    pub(crate) async fn with_push(mode: AuthMode, push: PushSenders) -> Self {
        Self::build(mode, push, None, None).await
    }

    /// A new app whose assistant `agent` makes, over the app's database and clock (the
    /// scripted assistant is there too, unused).
    pub(crate) async fn with_agent(
        agent: impl FnOnce(&Database, Arc<ManualClock>) -> Arc<dyn Assistant>,
    ) -> Self {
        Self::build(
            AuthMode::Enforce,
            PushSenders::default(),
            Some(Box::new(move |db, clock, _| agent(db, clock))),
            None,
        )
        .await
    }

    /// A new app whose assistant `agent` makes, over the app's database and clock and given
    /// the app's own directory (its secrets folder is `secrets` there), and whose dependencies
    /// `change` sets.
    pub(crate) async fn with_agent_and(
        agent: impl FnOnce(&Database, Arc<ManualClock>, &std::path::Path) -> Arc<dyn Assistant>,
        change: impl FnOnce(Dependencies, &std::path::Path) -> Dependencies,
    ) -> Self {
        Self::build(
            AuthMode::Enforce,
            PushSenders::default(),
            Some(Box::new(agent)),
            Some(Box::new(change)),
        )
        .await
    }

    async fn build(
        mode: AuthMode,
        push: PushSenders,
        agent: Option<AgentMaker<'_>>,
        change: Option<Change<'_>>,
    ) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let url: DatabaseUrl = format!("sqlite://{}", dir.path().join("emberhut.db").display())
            .parse()
            .unwrap();
        let db = Database::connect(&url).await.unwrap();
        let clock = Arc::new(ManualClock::new(Timestamp::now()));
        let assistant = Arc::new(Scripted::default());
        let mac = Arc::new(FakeMac::default());
        let coding_app = Arc::new(FakeCodingApp::default());
        let task_workers = Arc::new(FakeTaskWorkers::default());
        let answering: Arc<dyn Assistant> = match agent {
            Some(agent) => agent(&db, Arc::clone(&clock), dir.path()),
            None => Arc::clone(&assistant) as _,
        };
        let runners = Arc::new(FakeRunner::default());
        let workers = Arc::new(FakeWorkers::default());
        let browser = Arc::new(FakeBrowser::default());
        let desk = Arc::new(FakeDesk::default());
        let party_model = Arc::new(FakeModel::default());
        let dark_cloud = Arc::new(FakeModel::default());
        let dark_local = Arc::new(FakeModel::default());
        let dependencies = Dependencies {
            clock: Arc::clone(&clock) as _,
            random: Arc::new(OsRandom),
            assistant: answering,
            push,
            secrets_dir: dir.path().join("secrets"),
            mac: Arc::clone(&mac) as _,
            coding_app: Arc::clone(&coding_app) as _,
            workers: Arc::clone(&task_workers) as _,
            thumbnailer: Arc::new(FakeThumbs),
            runners: Arc::clone(&runners) as _,
            home: Some(dir.path().join("home")),
            runner_mcp: Some(dir.path().join("runner-mcp.json")),
            worker_sessions: Arc::clone(&workers) as _,
            assistant_tasks: Arc::clone(&workers) as _,
            private_files: PRIVATE_FILES
                .iter()
                .map(|name| dir.path().join(name))
                .collect(),
            browser: Arc::clone(&browser) as _,
            desk: Arc::clone(&desk) as _,
            chrome_extension_zip: Some(dir.path().join("extension.zip")),
            browser_view: Some("https://view.example.test/".to_owned()),
            party_model: Arc::clone(&party_model) as _,
            dark: DarkModels {
                cloud: Arc::clone(&dark_cloud) as _,
                cloud_label: "Cloud".to_owned(),
                local: Some((Arc::clone(&dark_local) as _, "Local".to_owned())),
            },
            family_directory: Arc::new(FakeFamily::default()),
            ..Dependencies::default()
        };
        let dependencies = match change {
            Some(change) => change(dependencies, dir.path()),
            None => dependencies,
        };
        let state = AppState::new(&db, dependencies, mode);
        Self {
            clock,
            state,
            url,
            assistant,
            mac,
            coding_app,
            task_workers,
            runners,
            workers,
            root: dir.path().to_path_buf(),
            browser,
            desk,
            party_model,
            dark_cloud,
            dark_local,
            _dir: dir,
        }
    }

    /// A new app in `mode` whose dependencies `change` sets (given the default ones and the
    /// app's own directory).
    pub(crate) async fn with_dependencies(
        mode: AuthMode,
        change: impl FnOnce(Dependencies, &std::path::Path) -> Dependencies,
    ) -> Self {
        Self::build(mode, PushSenders::default(), None, Some(Box::new(change))).await
    }

    /// A new app that enforces sign-in, whose Mac (its recorder and calendars) and calls,
    /// voice and health providers are `fakes`.
    pub(crate) async fn with_fakes(fakes: &Fakes) -> Self {
        let mac = Arc::clone(&fakes.mac);
        let personal = fakes.dependencies();
        Self::with_dependencies(AuthMode::Enforce, move |dependencies, _| Dependencies {
            mac: mac as _,
            personal,
            ..dependencies
        })
        .await
    }

    /// A new app that enforces sign-in, with an identity of its own: the key [`MACHINE_SEED`]
    /// makes, called "Test machine", whose pairing links name [`PAIRING_ROUTE`].
    pub(crate) async fn with_machine() -> Self {
        Self::with_dependencies(AuthMode::Enforce, |dependencies, _| Dependencies {
            machine: Some(Arc::new(test_machine())),
            pairing_routes: vec![PAIRING_ROUTE.to_owned()],
            ..dependencies
        })
        .await
    }

    /// Stands for the server's first start with this app's database: it makes the machine's
    /// key beside the database, and the database remembers it.
    pub(crate) async fn serve_once(&self) -> emberhut_identity::SecretKey {
        let db = Database::connect(&self.url).await.unwrap();
        let path = self.dir().join("machine.key");
        let key = emberhut_server::identity::serving_key(&path, &db.settings(), false)
            .await
            .unwrap();
        db.close().await.unwrap();
        key
    }

    /// The app's own directory (its database, uploads and secrets are in it).
    pub(crate) fn dir(&self) -> &std::path::Path {
        &self.root
    }

    /// What the app's clock reads.
    pub(crate) fn clock_now(&self) -> Timestamp {
        emberhut_core::Clock::now(self.clock.as_ref())
    }

    /// Moves the app's clock on by `minutes`.
    pub(crate) fn advance_minutes(&self, minutes: i64) {
        self.clock.advance(SignedDuration::from_mins(minutes));
    }

    /// The whole app, middleware included.
    pub(crate) fn router(&self) -> Router {
        emberhut_server::app::router(self.state.clone(), REQUEST_TIMEOUT)
    }

    /// Sends `request` through the app, without a socket.
    pub(crate) async fn send(&self, request: Request<Body>) -> Response<Body> {
        let method = request.method().to_string();
        let path = documented_path(request.uri().path());
        let response = self.router().oneshot(request).await.unwrap();
        STATUSES_GOT
            .lock()
            .unwrap()
            .insert((method, path, response.status().as_u16()));
        response
    }

    /// Waits until no turn runs or waits, in any room (the talk service's own count of them),
    /// and every reply's push has gone out.
    pub(crate) async fn settle(&self) {
        tokio::time::timeout(Duration::from_secs(10), async {
            self.state.talk().idle().await;
            self.state.talk().pushed().await;
        })
        .await
        .expect("the turns and their pushes end within 10 s");
    }

    /// A signed-in device's session.
    pub(crate) async fn sign_in(&self) -> Session {
        self.state
            .auth()
            .open_session(&device("laptop-1"))
            .await
            .unwrap()
    }

    /// Sends a request and reads its answer, which must be one the document lists for that
    /// route, with a body that matches the document's schema.
    pub(crate) async fn call(&self, call: Call) -> Answer {
        let method = call.method.clone();
        let path = documented_path(&call.path);
        let response = self.send(call.request()).await;
        let answer = Answer::read(response).await;
        check_against_document(&method, &path, &answer);
        answer
    }
}

/// A request to send.
#[derive(Clone, Debug)]
pub(crate) struct Call {
    method: Method,
    path: String,
    query: String,
    headers: Vec<(String, String)>,
    body: Option<String>,
    peer: Option<std::net::SocketAddr>,
    direct: bool,
    local: Option<std::net::SocketAddr>,
}

impl Call {
    /// A `GET` of `path`.
    pub(crate) fn get(path: &str) -> Self {
        Self::new(Method::GET, path)
    }

    /// A `DELETE` of `path`.
    pub(crate) fn delete(path: &str) -> Self {
        Self::new(Method::DELETE, path)
    }

    /// A `POST` of `path`, with no body.
    pub(crate) fn post(path: &str) -> Self {
        Self::new(Method::POST, path)
    }

    /// A `PUT` of `path`, with no body.
    pub(crate) fn put(path: &str) -> Self {
        Self::new(Method::PUT, path)
    }

    /// A `PATCH` of `path`, with no body.
    pub(crate) fn patch(path: &str) -> Self {
        Self::new(Method::PATCH, path)
    }

    fn new(method: Method, path: &str) -> Self {
        Self {
            method,
            path: path.to_owned(),
            query: String::new(),
            headers: Vec::new(),
            body: None,
            peer: None,
            direct: false,
            local: None,
        }
    }

    /// From this TCP peer, over a connection the server's own TCP listener accepted (its
    /// connection info and the direct-TCP mark). Without it: a transport that names no peer.
    pub(crate) fn peer(mut self, peer: &str) -> Self {
        self.peer = Some(peer.parse().unwrap());
        self.direct = true;
        self
    }

    /// Over a connection the server's own TCP listener accepted on this local address (the
    /// direct-TCP mark's), from [`Self::peer`]'s peer, or `192.0.2.20:50000` when none is set.
    pub(crate) fn local(mut self, local: &str) -> Self {
        self.local = Some(local.parse().unwrap());
        self.peer = self
            .peer
            .or_else(|| Some("192.0.2.20:50000".parse().unwrap()));
        self.direct = true;
        self
    }

    /// With connection info naming this peer, from a listener other than the server's own TCP
    /// one (a relayed or sealed listener), which doesn't mark its requests as direct.
    pub(crate) fn peer_not_direct(mut self, peer: &str) -> Self {
        self.peer = Some(peer.parse().unwrap());
        self.direct = false;
        self
    }

    /// With this query string (without the `?`).
    pub(crate) fn query(mut self, query: &str) -> Self {
        query.clone_into(&mut self.query);
        self
    }

    /// With this header.
    pub(crate) fn header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_owned(), value.to_owned()));
        self
    }

    /// With the session's access token.
    pub(crate) fn token(self, session: &Session) -> Self {
        let bearer = format!("Bearer {}", session.tokens.access.as_str());
        self.header("authorization", &bearer)
    }

    /// With this JSON body.
    pub(crate) fn json(mut self, body: &Value) -> Self {
        self.body = Some(body.to_string());
        self
    }

    /// With this raw body.
    pub(crate) fn raw(mut self, body: &str) -> Self {
        self.body = Some(body.to_owned());
        self
    }

    /// The request, to send by hand (for a streamed answer).
    pub(crate) fn request(self) -> Request<Body> {
        let uri = if self.query.is_empty() {
            self.path
        } else {
            format!("{}?{}", self.path, self.query)
        };
        let mut request = Request::builder().method(self.method).uri(uri);
        for (name, value) in &self.headers {
            request = request.header(name, value);
        }
        if self.body.is_some() {
            request = request.header("content-type", "application/json");
        }
        if let Some(peer) = self.peer {
            request = request.extension(axum::extract::ConnectInfo(peer));
        }
        if self.direct {
            request = request.extension(emberhut_server::peer::DirectTcp { local: self.local });
        }
        request
            .body(self.body.map_or_else(Body::empty, Body::from))
            .unwrap()
    }
}

/// An answer, read whole.
#[derive(Debug)]
pub(crate) struct Answer {
    /// Its status.
    pub(crate) status: StatusCode,
    /// Its headers.
    pub(crate) headers: axum::http::HeaderMap,
    /// Its body, as text (lossy for a body that isn't).
    pub(crate) text: String,
    /// Its body, as bytes.
    pub(crate) bytes: Vec<u8>,
}

impl Answer {
    /// Reads `response` whole.
    pub(crate) async fn read(response: Response<Body>) -> Self {
        let status = response.status();
        let headers = response.headers().clone();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        Self {
            status,
            headers,
            text: String::from_utf8_lossy(&bytes).into_owned(),
            bytes: bytes.to_vec(),
        }
    }

    /// The body as JSON.
    pub(crate) fn json(&self) -> Value {
        serde_json::from_str(&self.text)
            .unwrap_or_else(|error| panic!("not JSON ({error}): {}", self.text))
    }

    /// The body read as `T`: the contract type the route answers with.
    pub(crate) fn read_as<T: serde::de::DeserializeOwned>(&self) -> T {
        serde_json::from_str(&self.text)
            .unwrap_or_else(|error| panic!("not the contract's type ({error}): {}", self.text))
    }

    /// A header's value.
    pub(crate) fn header(&self, name: &str) -> &str {
        self.headers
            .get(name)
            .unwrap_or_else(|| panic!("no {name} header"))
            .to_str()
            .unwrap()
    }
}

/// The document's path for a request's `path`: a stored file's is `/uploads/{path}`; any
/// other is the path itself, or the template with parameters (`/s/{token}`) it matches,
/// segment by segment.
pub(crate) fn documented_path(path: &str) -> String {
    if path.starts_with(emberhut_api::paths::FILES) {
        return "/uploads/{path}".to_owned();
    }
    let document = document();
    let paths = document["paths"].as_object().unwrap();
    if paths.contains_key(path) {
        return path.to_owned();
    }
    let segments: Vec<&str> = path.split('/').collect();
    paths
        .keys()
        .find(|template| {
            let parts: Vec<&str> = template.split('/').collect();
            parts.len() == segments.len()
                && parts.iter().zip(&segments).all(|(part, segment)| {
                    part == segment
                        || (part.starts_with('{') && part.ends_with('}'))
                        || part.split_once('{').is_some_and(|(prefix, rest)| {
                            rest.ends_with('}') && segment.starts_with(prefix)
                        })
                })
        })
        .cloned()
        .unwrap_or_else(|| path.to_owned())
}

/// The document, as JSON.
pub(crate) fn document() -> Value {
    serde_json::from_str(emberhut_server::openapi_json().unwrap()).unwrap()
}

/// Fails unless the document lists `answer`'s status for `method` `path` and, for a JSON
/// answer, the body matches that response's schema.
pub(crate) fn check_against_document(method: &Method, path: &str, answer: &Answer) {
    let document = document();
    let operation = &document["paths"][path][method.as_str().to_lowercase()];
    assert!(
        operation.is_object(),
        "{method} {path} is not in the document"
    );
    let status = answer.status.as_u16().to_string();
    let response = &operation["responses"][&status];
    assert!(
        response.is_object(),
        "{method} {path} answered {status}, which the document doesn't list: {}",
        answer.text
    );
    let Some(schema) = response["content"]["application/json"]["schema"].as_object() else {
        return;
    };
    let mut root = document.clone();
    for (key, value) in schema {
        root[key] = value.clone();
    }
    let validator = jsonschema::validator_for(&root)
        .unwrap_or_else(|error| panic!("{method} {path} {status}: not a schema: {error}"));
    let body = answer.json();
    let errors: Vec<String> = validator
        .iter_errors(&body)
        .map(|error| format!("{} at {}", error, error.instance_path()))
        .collect();
    assert!(
        errors.is_empty(),
        "{method} {path} {status}: the body doesn't match the document: {errors:#?}\n{}",
        answer.text
    );
}

/// A fixture of the contract (`crates/emberhut-api/tests/fixtures`), as JSON.
pub(crate) fn fixture(name: &str) -> Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../emberhut-api/tests/fixtures")
        .join(name);
    serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap()
}

/// Fails unless every key `ours` writes is one the fixture has, at every depth (arrays by
/// their first element): the server writes the contract's shape, never a field of its own.
pub(crate) fn assert_keys_within(ours: &Value, fixture: &Value, at: &str) {
    match (ours, fixture) {
        (Value::Object(ours), Value::Object(theirs)) => {
            for (key, value) in ours {
                let theirs = theirs
                    .get(key)
                    .unwrap_or_else(|| panic!("{at}.{key} is not in the fixture"));
                assert_keys_within(value, theirs, &format!("{at}.{key}"));
            }
        }
        (Value::Array(ours), Value::Array(theirs)) => {
            if let (Some(ours), Some(theirs)) = (ours.first(), theirs.first()) {
                assert_keys_within(ours, theirs, &format!("{at}[0]"));
            }
        }
        _ => {}
    }
}

/// The seed of the key [`TestApp::with_machine`]'s machine has.
pub(crate) const MACHINE_SEED: [u8; 32] = [7; 32];

/// The route [`TestApp::with_machine`]'s pairing links name.
pub(crate) const PAIRING_ROUTE: &str = "http://192.0.2.10:18990";

/// The machine [`TestApp::with_machine`] has.
pub(crate) fn test_machine() -> emberhut_server::identity::Machine {
    emberhut_server::identity::Machine::new(
        emberhut_identity::SecretKey::from_bytes(&MACHINE_SEED),
        "Test machine".parse().unwrap(),
    )
}

/// A device as an app would describe itself.
pub(crate) fn device(install: &str) -> DeviceDetails {
    DeviceDetails::new(Some("Room 2 laptop"), Some("Example 1"), Some(install))
}

/// A `GET` of `path` with no body, as a raw request.
pub(crate) fn get(path: &str) -> Request<Body> {
    Request::get(path).body(Body::empty()).unwrap()
}

/// A response's body, as JSON.
pub(crate) async fn json(response: Response<Body>) -> Value {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

/// The refusal in `result`; fails the test on anything else.
pub(crate) fn refusal<T: std::fmt::Debug>(result: Result<T, PairingError>) -> PairingRefusal {
    match result {
        Err(PairingError::Refused(refusal)) => refusal,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

/// The assistant of the tests: it says its steps, then gives its reply. A turn can be held at
/// its first step until the test lets it go, to watch it while it runs.
#[derive(Debug, Default)]
pub(crate) struct Scripted {
    script: Mutex<Script>,
    gate: Notify,
}

#[derive(Debug, Default)]
struct Script {
    steps: Vec<String>,
    reply: Option<String>,
    fail: bool,
    panic: bool,
    held: bool,
    turns: Vec<Turn>,
}

impl Scripted {
    /// Its next turns say `steps`, then answer `reply`.
    pub(crate) fn answers(&self, steps: &[&str], reply: Option<&str>) {
        let mut script = self.script.lock().unwrap();
        script.steps = steps.iter().map(|step| (*step).to_owned()).collect();
        script.reply = reply.map(str::to_owned);
        script.fail = false;
        script.panic = false;
    }

    /// Its next turns fail.
    pub(crate) fn fails(&self) {
        self.script.lock().unwrap().fail = true;
    }

    /// Its next turns panic, after their steps.
    pub(crate) fn panics(&self) {
        self.script.lock().unwrap().panic = true;
    }

    /// Its next turns wait after their first step until [`Self::release`].
    pub(crate) fn hold(&self) {
        self.script.lock().unwrap().held = true;
    }

    /// Lets a held turn go on.
    pub(crate) fn release(&self) {
        self.script.lock().unwrap().held = false;
        // A permit, so a turn that has not reached the gate yet still passes it.
        self.gate.notify_one();
    }

    /// The turns it was given.
    pub(crate) fn turns(&self) -> Vec<Turn> {
        self.script.lock().unwrap().turns.clone()
    }
}

/// A failing assistant's error.
#[derive(Debug, thiserror::Error)]
#[error("the scripted assistant was told to fail")]
struct ScriptedFailure;

impl Assistant for Scripted {
    fn respond(
        &self,
        turn: Turn,
        live: LiveSink,
    ) -> BoxFuture<'_, Result<Option<String>, AssistantError>> {
        Box::pin(async move {
            let (steps, reply, fail, panic, held) = {
                let mut script = self.script.lock().unwrap();
                script.turns.push(turn);
                (
                    script.steps.clone(),
                    script.reply.clone(),
                    script.fail,
                    script.panic,
                    script.held,
                )
            };
            for (n, step) in steps.iter().enumerate() {
                live.doing(step.clone());
                if n == 0 && held {
                    self.gate.notified().await;
                }
            }
            assert!(!panic, "the scripted assistant was told to panic");
            if fail {
                return Err(AssistantError::new(ScriptedFailure));
            }
            Ok(reply)
        })
    }
}

/// The MCP endpoint's path.
const MCP: &str = emberhut_api::paths::MCP_ROOM;

impl TestApp {
    /// A JSON-RPC request to the endpoint as Main's agent session.
    pub(crate) async fn rpc(&self, method: &str, params: Value) -> Answer {
        let token = self.state.agent_tokens().for_room(RoomId::MAIN).unwrap();
        let body = json!({ "jsonrpc": "2.0", "id": 7, "method": method, "params": params });
        self.call(
            Call::post(MCP)
                .header("authorization", &format!("Bearer {}", token.as_str()))
                .header("accept", "application/json, text/event-stream")
                .json(&body),
        )
        .await
    }

    /// `tools/call` of `name` with `arguments`: its result.
    pub(crate) async fn call_tool(&self, name: &str, arguments: Value) -> CallToolResult {
        let answer = self
            .rpc(
                "tools/call",
                json!({ "name": name, "arguments": arguments }),
            )
            .await;
        assert_eq!(answer.status, 200, "{}", answer.text);
        let reply: RpcResponse = answer.read_as();
        assert_eq!(reply.id, json!(7));
        serde_json::from_value(reply.result.unwrap()).unwrap()
    }
}

/// The one text of a tool's result.
pub(crate) fn text(result: &CallToolResult) -> &str {
    assert_eq!(result.content.len(), 1, "{result:?}");
    result.content[0].as_text().unwrap()
}
