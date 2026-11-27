//! Clerk's signing keys: fetched from its JWKS URL and cached by `kid` (ADR 0007, section 5).
//!
//! - The set is fetched again once it is [`KeyTiming::refresh`] old (an hour).
//! - A `kid` the set doesn't have makes a fetch, but no two fetches start within
//!   [`KeyTiming::unknown_kid_wait`] (a minute) of each other, so tokens with made-up `kid`s
//!   can't make the directory flood Clerk.
//! - The set is replaced only by a fetch that succeeds, so a rotation (a new key published
//!   before it signs) never fails a valid token, and an outage keeps the keys already known.
//! - A `kid` that is unknown while a fetch is held back, or while the URL can't be reached, is
//!   [`Lookup::Unavailable`] (503), never a refusal: a client doesn't sign out because of an
//!   outage or a burst of junk.
//!
//! Fetches are made one at a time; a request that waited for another's fetch takes its result if
//! that fetch started after the request asked. A request whose key is known never waits: when
//! only the hourly refresh is due, it is answered at once and the refresh runs in the
//! background (one at a time, and no sooner than the wait allows, so an outage doesn't make
//! every request try Clerk).

use std::collections::HashMap;
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use jsonwebtoken::DecodingKey;
use serde::Deserialize;
use tokio::time::Instant;
use url::Url;

/// How often the keys are fetched.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyTiming {
    /// How old the set may get before it is fetched again (an hour).
    pub refresh: Duration,
    /// The least time between two fetches, so an unknown `kid` makes at most one fetch in it
    /// (a minute).
    pub unknown_kid_wait: Duration,
    /// How long one fetch may take (5 seconds).
    pub fetch_timeout: Duration,
}

impl Default for KeyTiming {
    fn default() -> Self {
        Self {
            refresh: Duration::from_secs(3_600),
            unknown_kid_wait: Duration::from_secs(60),
            fetch_timeout: Duration::from_secs(5),
        }
    }
}

/// The most of a JWKS document that is read. Clerk's holds a key or two, about a kilobyte.
const MAX_SET_BYTES: usize = 256 * 1024;

/// The least `Retry-After` given while keys can't be had.
const MIN_RETRY: Duration = Duration::from_secs(1);

/// What the set has for a `kid`.
#[derive(Clone)]
pub(super) enum Lookup {
    /// The key.
    Found(Arc<DecodingKey>),
    /// No key, after a fetch that succeeded: the token wasn't signed by the account provider.
    Unknown,
    /// No key, and none can be had now: try again after `retry_after`.
    Unavailable {
        /// When a fetch may be made again.
        retry_after: Duration,
    },
}

/// Clerk's keys, by `kid`.
pub(super) struct KeySet {
    url: Url,
    http: reqwest::Client,
    timing: KeyTiming,
    state: Mutex<State>,
    /// Held while a fetch runs, so fetches are made one at a time.
    fetching: tokio::sync::Mutex<()>,
    /// Whether a background refresh is under way.
    refreshing: AtomicBool,
}

/// A background refresh under way: when it ends, even by a panic, another may start.
struct Refreshing(Arc<KeySet>);

impl Drop for Refreshing {
    fn drop(&mut self) {
        self.0.refreshing.store(false, Ordering::Release);
    }
}

/// The keys and when they were fetched.
#[derive(Default)]
struct State {
    keys: HashMap<String, Arc<DecodingKey>>,
    /// When the last fetch that succeeded started.
    fetched: Option<Instant>,
    /// When the last fetch, whatever its result, started.
    attempted: Option<Instant>,
}

/// How a request's call for a fetch went.
enum Fetched {
    /// A fetch made for it, or one that started after it asked, succeeded.
    Fresh,
    /// A fetch made for it, or one that started after it asked, failed.
    Failed,
    /// No fetch may start yet.
    HeldBack,
}

/// Why a fetch failed. Logged; the old set stays.
#[derive(Debug, thiserror::Error)]
enum FetchError {
    #[error("the keys couldn't be fetched")]
    Request(#[from] reqwest::Error),
    #[error("the key set is larger than {MAX_SET_BYTES} bytes")]
    TooLarge,
    #[error("the key set isn't a JWKS document")]
    Json(#[from] serde_json::Error),
    #[error("the key set has no RS256 signing key")]
    NoKeys,
}

/// Why the keys' HTTP client couldn't be made.
#[derive(Debug, thiserror::Error)]
pub(super) enum ClientError {
    /// TLS couldn't be set up.
    #[error("TLS couldn't be set up")]
    Tls(#[source] rustls::Error),
    /// reqwest refused the configuration.
    #[error("the HTTP client couldn't be built")]
    Build(#[source] reqwest::Error),
}

impl KeySet {
    /// The keys at `url`, none fetched yet.
    pub(super) fn new(url: Url, timing: KeyTiming) -> Result<Self, ClientError> {
        Ok(Self {
            url,
            http: client()?,
            timing,
            state: Mutex::default(),
            fetching: tokio::sync::Mutex::new(()),
            refreshing: AtomicBool::new(false),
        })
    }

    /// The key for `kid`. A known key is answered at once, starting a refresh in the
    /// background when one is due; an unknown `kid` fetches the set, within the limits.
    pub(super) async fn key(self: &Arc<Self>, kid: &str) -> Lookup {
        let asked = Instant::now();
        if let Some(key) = self.known(kid) {
            if self.refresh_due(asked) {
                self.refresh_in_background();
            }
            return Lookup::Found(key);
        }
        let fetched = self.fetch(asked).await;
        match (self.known(kid), fetched) {
            (Some(key), _) => Lookup::Found(key),
            (None, Fetched::Fresh) => Lookup::Unknown,
            (None, Fetched::Failed | Fetched::HeldBack) => Lookup::Unavailable {
                retry_after: self.retry_after(),
            },
        }
    }

    /// Fetches the set again on a task of its own, unless one is already doing so or the last
    /// fetch started too recently (then nothing is spawned). A failed refresh keeps the old set.
    fn refresh_in_background(self: &Arc<Self>) {
        if self.held_back(Instant::now()) || self.refreshing.swap(true, Ordering::AcqRel) {
            return;
        }
        let refreshing = Refreshing(Arc::clone(self));
        tokio::spawn(async move {
            refreshing.0.fetch(Instant::now()).await;
            // `refreshing` is dropped here, or while unwinding, and lets the next refresh run.
        });
    }

    /// Whether a fetch started within the wait before `now`.
    fn held_back(&self, now: Instant) -> bool {
        self.state()
            .attempted
            .is_some_and(|attempted| now < attempted + self.timing.unknown_kid_wait)
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn known(&self, kid: &str) -> Option<Arc<DecodingKey>> {
        self.state().keys.get(kid).cloned()
    }

    fn refresh_due(&self, now: Instant) -> bool {
        self.state()
            .fetched
            .is_none_or(|fetched| now >= fetched + self.timing.refresh)
    }

    /// How long until a fetch may start again.
    fn retry_after(&self) -> Duration {
        let next = self
            .state()
            .attempted
            .map(|attempted| attempted + self.timing.unknown_kid_wait);
        next.map_or(MIN_RETRY, |next| {
            next.saturating_duration_since(Instant::now())
                .max(MIN_RETRY)
        })
    }

    /// Fetches the set for a request made at `asked`, unless a fetch that started since then
    /// already answered it, or the last fetch started too recently.
    async fn fetch(&self, asked: Instant) -> Fetched {
        let _one_at_a_time = self.fetching.lock().await;
        let started = Instant::now();
        {
            let mut state = self.state();
            if state.fetched.is_some_and(|fetched| fetched >= asked) {
                return Fetched::Fresh;
            }
            if state.attempted.is_some_and(|attempted| attempted >= asked) {
                return Fetched::Failed;
            }
            if state
                .attempted
                .is_some_and(|attempted| started < attempted + self.timing.unknown_kid_wait)
            {
                return Fetched::HeldBack;
            }
            state.attempted = Some(started);
        }
        match self.download().await {
            Ok(keys) => {
                tracing::info!(keys = keys.len(), "fetched the account provider's keys");
                let mut state = self.state();
                state.keys = keys;
                state.fetched = Some(started);
                Fetched::Fresh
            }
            Err(error) => {
                tracing::warn!(%error, "the account provider's keys couldn't be fetched; keeping the old ones");
                Fetched::Failed
            }
        }
    }

    async fn download(&self) -> Result<HashMap<String, Arc<DecodingKey>>, FetchError> {
        let mut response = self
            .http
            .get(self.url.clone())
            .timeout(self.timing.fetch_timeout)
            .send()
            .await?
            .error_for_status()?;
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            if body.len() + chunk.len() > MAX_SET_BYTES {
                return Err(FetchError::TooLarge);
            }
            body.extend_from_slice(&chunk);
        }
        let keys = read_set(&body)?;
        if keys.is_empty() {
            return Err(FetchError::NoKeys);
        }
        Ok(keys)
    }
}

impl fmt::Debug for KeySet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let state = self.state();
        let mut kids: Vec<&String> = state.keys.keys().collect();
        kids.sort();
        f.debug_struct("KeySet")
            .field("url", &self.url.as_str())
            .field("kids", &kids)
            .field("timing", &self.timing)
            .finish_non_exhaustive()
    }
}

/// A JWKS document. Readers ignore fields they don't know, and keys they can't use.
#[derive(Deserialize)]
struct JwkSet {
    keys: Vec<serde_json::Value>,
}

/// The fields of an RSA key that matter here.
#[derive(Deserialize)]
struct RsaJwk {
    kty: String,
    kid: String,
    #[serde(default)]
    alg: Option<String>,
    #[serde(default, rename = "use")]
    usage: Option<String>,
    n: String,
    e: String,
}

/// The RS256 signing keys of a JWKS document, by `kid`. Keys of other types, for other
/// algorithms or uses, or without a `kid`, are skipped.
fn read_set(body: &[u8]) -> Result<HashMap<String, Arc<DecodingKey>>, serde_json::Error> {
    let set: JwkSet = serde_json::from_slice(body)?;
    Ok(set
        .keys
        .into_iter()
        .filter_map(|key| serde_json::from_value::<RsaJwk>(key).ok())
        .filter(|key| {
            key.kty == "RSA"
                && key.alg.as_deref().is_none_or(|alg| alg == "RS256")
                && key.usage.as_deref().is_none_or(|usage| usage == "sig")
        })
        .filter_map(|key| {
            let decoding = DecodingKey::from_rsa_components(&key.n, &key.e).ok()?;
            Some((key.kid, Arc::new(decoding)))
        })
        .collect())
}

/// The client the keys are fetched with: reqwest on rustls with ring and the Mozilla root
/// store, so a container needs no system certificates. Redirects are never followed and no
/// proxy is read from the environment.
fn client() -> Result<reqwest::Client, ClientError> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let roots = rustls::RootCertStore {
        roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
    };
    let mut tls = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(ClientError::Tls)?
        .with_root_certificates(roots)
        .with_no_client_auth();
    tls.alpn_protocols = vec![b"http/1.1".to_vec()];
    reqwest::Client::builder()
        .tls_backend_preconfigured(tls)
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .build()
        .map_err(ClientError::Build)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The example RSA key's modulus of RFC 7517, appendix A.1.
    const N: &str = "0vx7agoebGcQSuuPiLJXZptN9nndrQmbXEps2aiAFbWhM78LhWx4cbbfAAtVT86zwu1RK7aPFFxuhDR1L6tSoc_BJECPebWKRXjBZCiFV4n3oknjhMstn64tZ_2W-5JsGY4Hc5n9yBXArwl93lqt7_RN5w6Cf0h4QyQ5v-65YGjQR0_FDW2QvzqY368QQMicAtaSqzs8KJZgnYb9c7d0zgdAZHzu6qMQvRL5hajrn1n91CbOpbISD08qNLyrdkt-bFTWhAI4vMQFh6WeZu0fM4lFd2NcRwr3XPksINHaQ-G_xBniIqbw0Ls1jF44-csFCur-kEgU8awapJzKnqDKgw";

    #[test]
    fn only_rs256_signing_keys_with_a_kid_are_kept() {
        let set = serde_json::json!({
            "keys": [
                { "kty": "RSA", "kid": "plain", "n": N, "e": "AQAB" },
                { "kty": "RSA", "kid": "rs256", "alg": "RS256", "use": "sig", "n": N, "e": "AQAB", "x5c": [] },
                { "kty": "RSA", "kid": "ps256", "alg": "PS256", "n": N, "e": "AQAB" },
                { "kty": "RSA", "kid": "enc", "use": "enc", "n": N, "e": "AQAB" },
                { "kty": "RSA", "n": N, "e": "AQAB" },
                { "kty": "EC", "kid": "ec", "crv": "P-256", "x": "AA", "y": "AA" },
                "not a key"
            ],
            "fromALaterVersion": true
        });
        let keys = read_set(&serde_json::to_vec(&set).unwrap()).unwrap();
        let mut kids: Vec<&str> = keys.keys().map(String::as_str).collect();
        kids.sort_unstable();
        assert_eq!(kids, ["plain", "rs256"]);
    }

    #[test]
    fn a_document_without_keys_is_refused() {
        assert!(read_set(b"{}").is_err());
        assert!(read_set(b"not json").is_err());
        assert!(read_set(br#"{"keys": []}"#).unwrap().is_empty());
    }

    #[test]
    fn the_default_timing_is_the_adrs() {
        let timing = KeyTiming::default();
        assert_eq!(timing.refresh, Duration::from_secs(3_600));
        assert_eq!(timing.unknown_kid_wait, Duration::from_secs(60));
    }

    #[test]
    fn the_client_is_made_without_a_default_crypto_provider() {
        assert!(client().is_ok());
    }

    fn key_set() -> Arc<KeySet> {
        let url = "https://clerk.example/.well-known/jwks.json"
            .parse()
            .unwrap();
        Arc::new(KeySet::new(url, KeyTiming::default()).unwrap())
    }

    #[test]
    fn a_refresh_that_panics_still_lets_the_next_one_run() {
        let keys = key_set();
        keys.refreshing.store(true, Ordering::Release);
        let guard = Refreshing(Arc::clone(&keys));
        let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
            let _held = guard;
            panic!("a refresh failed badly");
        }));
        assert!(unwound.is_err());
        assert!(!keys.refreshing.load(Ordering::Acquire));
    }

    #[tokio::test]
    async fn a_held_back_refresh_spawns_nothing() {
        let keys = key_set();
        {
            let mut state = keys.state();
            state.attempted = Some(Instant::now());
        }
        keys.refresh_in_background();
        assert!(
            !keys.refreshing.load(Ordering::Acquire),
            "no refresh was started"
        );
    }
}
