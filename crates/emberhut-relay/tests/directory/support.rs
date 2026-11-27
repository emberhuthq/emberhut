//! The harness: a stand-in for Clerk's JWKS URL, test-only RSA keys made when a test first
//! needs them, a clock the test sets, and the directory served over real HTTP on loopback.
#![expect(
    clippy::unwrap_used,
    reason = "test helpers: a failure here fails the test that called them"
)]

use std::num::NonZeroU32;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use axum::Router;
use axum::http::{StatusCode, header};
use axum::response::IntoResponse;
use emberhut_api::connect::SignedLink;
use emberhut_api::directory::UnlinkRequest;
use emberhut_identity::{AccountId, MachineName, SecretKey, UnixMillis};
use emberhut_relay::directory::{
    ClerkSettings, ClerkVerifier, Clock, Directory, DirectoryConfig, InProcessDirectory, KeyTiming,
    Rate, RequestLimits,
};
use jsonwebtoken::jwk::Jwk;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use rsa::Generate as _;
use rsa::pkcs1::EncodeRsaPrivateKey as _;
use serde_json::{Value, json};
use tempfile::TempDir;

/// The issuer the stand-in's tokens carry.
pub(crate) const ISSUER: &str = "https://clerk.test";
/// The session tokens' audience.
pub(crate) const AUDIENCE: &str = "emberhut-directory";
/// The native OAuth application's client id.
pub(crate) const NATIVE: &str = "native-client";
/// The web app's OAuth application's client id.
pub(crate) const WEB: &str = "web-client";
/// The web app's origin, the one allowed `azp` and CORS origin.
pub(crate) const APP: &str = "https://app.emberhut.com";
/// The relay domain the vectors name.
pub(crate) const RELAY: &str = "relay.example";
/// The account the vectors' link names.
pub(crate) const ACCOUNT: &str = "user_2example0account";
/// When the vectors were signed, in milliseconds.
pub(crate) const VECTOR_TIME: u64 = 1_795_561_073_000;

/// A test-only RSA key, with the `kid` the stand-in publishes it under.
pub(crate) struct TestKey {
    pub(crate) kid: String,
    encoding: EncodingKey,
    jwk: Value,
}

impl TestKey {
    fn generate(kid: &str) -> Self {
        let key = rsa::PrivateKey::generate();
        let der = key.to_pkcs1_der().unwrap();
        let encoding = EncodingKey::from_rsa_der(der.as_bytes());
        let mut jwk = Jwk::from_encoding_key(&encoding, Algorithm::RS256).unwrap();
        jwk.common.key_id = Some(kid.to_owned());
        let mut jwk = serde_json::to_value(jwk).unwrap();
        jwk["use"] = json!("sig");
        Self {
            kid: kid.to_owned(),
            encoding,
            jwk,
        }
    }

    /// The key as the JWKS document lists it.
    pub(crate) fn jwk(&self) -> Value {
        self.jwk.clone()
    }

    /// A token with `claims`, signed by this key, under a header of `typ` (`None`: none) and
    /// Clerk's own extra header field.
    pub(crate) fn sign(&self, typ: Option<&str>, claims: &Value) -> String {
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some(self.kid.clone());
        header.typ = typ.map(str::to_owned);
        header.extras.insert("cat", "cl_test");
        jsonwebtoken::encode(&header, claims, &self.encoding).unwrap()
    }
}

/// The three test keys: Clerk's current key, the one it rotates to, and one it never publishes.
/// Each is made once per test process, when first asked for (an RSA key takes a moment).
pub(crate) fn key(which: usize) -> &'static TestKey {
    static KEYS: [OnceLock<TestKey>; 3] = [OnceLock::new(), OnceLock::new(), OnceLock::new()];
    KEYS[which].get_or_init(|| TestKey::generate(&format!("ins_test_{which}")))
}

/// Clerk's current key.
pub(crate) fn current() -> &'static TestKey {
    key(0)
}

/// The key Clerk rotates to.
pub(crate) fn next() -> &'static TestKey {
    key(1)
}

/// A key Clerk never publishes.
pub(crate) fn stranger() -> &'static TestKey {
    key(2)
}

/// A clock the test sets.
#[derive(Debug)]
pub(crate) struct TestClock(AtomicU64);

impl TestClock {
    /// Sets the time, in milliseconds.
    pub(crate) fn set(&self, millis: u64) {
        self.0.store(millis, Ordering::SeqCst);
    }

    /// Now, in whole seconds.
    pub(crate) fn seconds(&self) -> u64 {
        self.0.load(Ordering::SeqCst) / 1_000
    }
}

impl Clock for TestClock {
    fn now(&self) -> UnixMillis {
        UnixMillis::new(self.0.load(Ordering::SeqCst)).unwrap()
    }
}

/// The stand-in for Clerk's JWKS URL.
#[derive(Default)]
pub(crate) struct Jwks {
    keys: Mutex<Vec<Value>>,
    fetches: AtomicUsize,
    down: AtomicBool,
    delay: Mutex<Duration>,
}

impl Jwks {
    /// Publishes `keys`.
    pub(crate) fn publish(&self, keys: &[&TestKey]) {
        *self.keys.lock().unwrap() = keys.iter().map(|key| key.jwk()).collect();
    }

    /// How many times the set was asked for.
    pub(crate) fn fetches(&self) -> usize {
        self.fetches.load(Ordering::SeqCst)
    }

    /// Makes the URL answer 503 (`true`) or the set again.
    pub(crate) fn set_down(&self, down: bool) {
        self.down.store(down, Ordering::SeqCst);
    }

    /// Makes every answer wait `delay` first.
    pub(crate) fn set_delay(&self, delay: Duration) {
        *self.delay.lock().unwrap() = delay;
    }

    /// Waits up to 10 s for the set to have been asked for `count` times.
    pub(crate) async fn wait_for_fetches(&self, count: usize) {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        while self.fetches() < count {
            assert!(tokio::time::Instant::now() < deadline, "no fetch {count}");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    async fn serve(jwks: Arc<Self>) -> String {
        let app = Router::new().fallback(move || {
            let jwks = Arc::clone(&jwks);
            async move {
                // What it answers is decided as the request arrives.
                let down = jwks.down.load(Ordering::SeqCst);
                let keys = jwks.keys.lock().unwrap().clone();
                let delay = *jwks.delay.lock().unwrap();
                jwks.fetches.fetch_add(1, Ordering::SeqCst);
                tokio::time::sleep(delay).await;
                if down {
                    return StatusCode::SERVICE_UNAVAILABLE.into_response();
                }
                let body = serde_json::to_vec(&json!({ "keys": keys })).unwrap();
                ([(header::CONTENT_TYPE, "application/json")], body).into_response()
            }
        });
        let address = spawn(app).await;
        format!("http://{address}/.well-known/jwks.json")
    }
}

async fn spawn(app: Router) -> std::net::SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    address
}

/// How a test's directory is set up.
pub(crate) struct Setup {
    pub(crate) account_cap: u32,
    pub(crate) per_account: Rate,
    pub(crate) unknown_kid_wait: Duration,
    pub(crate) refresh: Duration,
}

impl Default for Setup {
    fn default() -> Self {
        Self {
            account_cap: 5,
            per_account: Rate {
                burst: 1_000,
                every: Duration::from_millis(1),
            },
            unknown_kid_wait: Duration::from_secs(60),
            refresh: Duration::from_secs(3_600),
        }
    }
}

/// A directory served on loopback, with its stand-in for Clerk.
pub(crate) struct Harness {
    pub(crate) base: String,
    pub(crate) http: reqwest::Client,
    pub(crate) clock: Arc<TestClock>,
    pub(crate) jwks: Arc<Jwks>,
    pub(crate) gateway: InProcessDirectory,
    _dir: TempDir,
}

impl Harness {
    /// A directory with the default setup, Clerk publishing its current key, at the vectors'
    /// time.
    pub(crate) async fn start() -> Self {
        Self::with(Setup::default()).await
    }

    /// A directory set up as `setup` says.
    pub(crate) async fn with(setup: Setup) -> Self {
        let jwks = Arc::new(Jwks::default());
        jwks.publish(&[current()]);
        let jwks_url = Jwks::serve(Arc::clone(&jwks)).await;
        let clock = Arc::new(TestClock(AtomicU64::new(VECTOR_TIME)));
        let settings = ClerkSettings {
            issuer: ISSUER.to_owned(),
            jwks_url: jwks_url.parse().unwrap(),
            audience: AUDIENCE.to_owned(),
            client_ids: vec![NATIVE.to_owned(), WEB.to_owned()],
            allowed_origins: vec![APP.to_owned()],
            access_token_claim: "client_id".to_owned(),
            key_timing: KeyTiming {
                refresh: setup.refresh,
                unknown_kid_wait: setup.unknown_kid_wait,
                fetch_timeout: Duration::from_secs(5),
            },
        };
        let verifier = ClerkVerifier::new(settings, clock.clone()).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let config = DirectoryConfig {
            relay_domain: RELAY.parse().unwrap(),
            database: dir.path().join("directory.db"),
            account_cap: NonZeroU32::new(setup.account_cap).unwrap(),
            limits: RequestLimits {
                per_account: setup.per_account,
                ..RequestLimits::default()
            },
        };
        let directory = Directory::open(config, verifier, clock.clone())
            .await
            .unwrap();
        let address = spawn(directory.router()).await;
        Self {
            base: format!("http://{address}"),
            http: http_client(),
            clock,
            jwks,
            gateway: directory.gateway(),
            _dir: dir,
        }
    }

    /// A session token for `account`, as the phone's JWT template makes it, with `extra`
    /// claims (a `null` removes one).
    pub(crate) fn session(&self, account: &str, extra: &Value) -> String {
        current().sign(Some("JWT"), &self.session_claims(account, extra))
    }

    /// The claims of a session token for `account`, with `extra` merged in.
    pub(crate) fn session_claims(&self, account: &str, extra: &Value) -> Value {
        let now = self.clock.seconds();
        let mut claims = json!({
            "aud": AUDIENCE,
            "exp": now + 60,
            "iat": now,
            "iss": ISSUER,
            "jti": "a1b2c3",
            "nbf": now - 5,
            "sub": account,
        });
        merge(&mut claims, extra);
        claims
    }

    /// The claims of an OAuth access token for `account` issued to `client`, with `extra`
    /// merged in.
    pub(crate) fn access_claims(&self, account: &str, client: &str, extra: &Value) -> Value {
        let now = self.clock.seconds();
        let mut claims = json!({
            "client_id": client,
            "exp": now + 600,
            "iat": now,
            "iss": ISSUER,
            "jti": "d4e5f6",
            "nbf": now,
            "scope": "openid offline_access",
            "sub": account,
        });
        merge(&mut claims, extra);
        claims
    }

    /// `path` on the directory.
    pub(crate) fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }

    /// `GET /v1/machines` with `token`.
    pub(crate) async fn list(&self, token: &str) -> Answer {
        Answer::of(
            self.http
                .get(self.url("/v1/machines"))
                .bearer_auth(token)
                .send()
                .await
                .unwrap(),
        )
        .await
    }

    /// `POST /v1/machines` with `token` and `body`.
    pub(crate) async fn link(&self, token: &str, body: &Value) -> Answer {
        Answer::of(
            self.http
                .post(self.url("/v1/machines"))
                .bearer_auth(token)
                .header(header::CONTENT_TYPE.as_str(), "application/json")
                .body(body.to_string())
                .send()
                .await
                .unwrap(),
        )
        .await
    }

    /// `POST path` with `token`, `body` as it is and `content_type`.
    pub(crate) async fn post_raw(
        &self,
        path: &str,
        token: &str,
        body: String,
        content_type: &str,
    ) -> Answer {
        Answer::of(
            self.http
                .post(self.url(path))
                .bearer_auth(token)
                .header("content-type", content_type)
                .body(body)
                .send()
                .await
                .unwrap(),
        )
        .await
    }

    /// `DELETE /v1/machines/{id}` with `token`.
    pub(crate) async fn delete(&self, token: &str, id: &str) -> Answer {
        Answer::of(
            self.http
                .delete(self.url(&format!("/v1/machines/{id}")))
                .bearer_auth(token)
                .send()
                .await
                .unwrap(),
        )
        .await
    }

    /// `POST /v1/machines/{id}/unlink` with `body` and no token.
    pub(crate) async fn unlink_itself(&self, id: &str, body: &Value) -> Answer {
        Answer::of(
            self.http
                .post(self.url(&format!("/v1/machines/{id}/unlink")))
                .header(header::CONTENT_TYPE.as_str(), "application/json")
                .body(body.to_string())
                .send()
                .await
                .unwrap(),
        )
        .await
    }
}

/// Merges `extra` into `claims`; a `null` removes the claim.
pub(crate) fn merge(claims: &mut Value, extra: &Value) {
    for (name, value) in extra.as_object().unwrap() {
        if value.is_null() {
            claims.as_object_mut().unwrap().remove(name);
        } else {
            claims[name] = value.clone();
        }
    }
}

/// An answer, read whole.
#[derive(Debug)]
pub(crate) struct Answer {
    pub(crate) status: u16,
    pub(crate) headers: header::HeaderMap,
    pub(crate) body: Value,
}

impl Answer {
    pub(crate) async fn of(response: reqwest::Response) -> Self {
        let status = response.status().as_u16();
        let headers = response.headers().clone();
        let bytes = response.bytes().await.unwrap();
        let body = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap()
        };
        Self {
            status,
            headers,
            body,
        }
    }

    /// The header's value, if any.
    pub(crate) fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).map(|value| value.to_str().unwrap())
    }

    /// Checks that this is a refusal with `status` and `code`, in directory.md's form.
    #[track_caller]
    pub(crate) fn refused(&self, status: u16, code: &str) {
        assert_eq!(
            (self.status, self.body["code"].as_str()),
            (status, Some(code)),
            "{self:?}"
        );
        assert!(
            self.body["error"]
                .as_str()
                .is_some_and(|error| !error.is_empty())
        );
        assert_eq!(
            self.header("content-type"),
            Some("application/json"),
            "{self:?}"
        );
        if status == 401 {
            assert_eq!(self.body["signIn"], json!(true));
            assert_eq!(self.header("www-authenticate"), Some("Bearer"));
        } else {
            assert!(self.body.get("signIn").is_none(), "{self:?}");
        }
        if matches!(status, 429 | 503) {
            let seconds: u64 = self.header("retry-after").unwrap().parse().unwrap();
            assert!(seconds >= 1, "{self:?}");
        }
    }
}

/// The shared connect vectors (`crates/emberhut-api/tests/fixtures/connect/`).
pub(crate) fn vectors(file: &str) -> Value {
    let path = format!(
        "{}/../emberhut-api/tests/fixtures/connect/{file}",
        env!("CARGO_MANIFEST_DIR")
    );
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

/// The vectors' signed link (the "machine" key, linking to [`ACCOUNT`] at [`RELAY`]).
pub(crate) fn vector_link() -> Value {
    vectors("connect.json")["messages"]["signedLink"].clone()
}

/// The vectors' machine id.
pub(crate) const VECTOR_MACHINE: &str = "3j6rajbbrd2yrrvdkoq2mouz5aqxwh5c";

/// A machine's key, from a one-byte seed.
pub(crate) fn machine(seed: u8) -> SecretKey {
    SecretKey::from_bytes(&[seed; 32])
}

/// The id of machine `seed`.
pub(crate) fn machine_id(seed: u8) -> String {
    machine(seed).machine_id().to_string()
}

/// Machine `seed`'s link statement for `account` at `relay`, under `name`, made at `time`.
pub(crate) fn link_at(seed: u8, name: &str, account: &str, relay: &str, time: u64) -> Value {
    let link = SignedLink::sign(
        &machine(seed),
        MachineName::cut(name).unwrap(),
        AccountId::new(account).unwrap(),
        relay.parse().unwrap(),
        UnixMillis::new(time).unwrap(),
    );
    serde_json::to_value(link).unwrap()
}

/// Machine `seed`'s link statement for `account` at the test relay, made at `time`.
pub(crate) fn link(seed: u8, account: &str, time: u64) -> Value {
    link_at(seed, &format!("Machine {seed}"), account, RELAY, time)
}

/// Machine `seed`'s own unlink at `relay`, made at `time`.
pub(crate) fn unlink_at(seed: u8, relay: &str, time: u64) -> Value {
    let request = UnlinkRequest::sign(
        &machine(seed),
        relay.parse().unwrap(),
        UnixMillis::new(time).unwrap(),
    );
    serde_json::to_value(request).unwrap()
}

/// Machine `seed`'s own unlink at the test relay, made at `time`.
pub(crate) fn unlink(seed: u8, time: u64) -> Value {
    unlink_at(seed, RELAY, time)
}

/// A plain-HTTP client for the directory on loopback (reqwest still wants a TLS setup made).
fn http_client() -> reqwest::Client {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let tls = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_root_certificates(rustls::RootCertStore::empty())
        .with_no_client_auth();
    reqwest::Client::builder()
        .tls_backend_preconfigured(tls)
        .no_proxy()
        .build()
        .unwrap()
}
