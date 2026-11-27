//! A device's session and where it is kept.
//!
//! A [`Session`] is this installation's id (made once, kept for good) and, while signed in, its
//! [`SessionTokens`]. A [`SessionStore`] keeps it between runs: [`crate::FileSessionStore`]
//! writes a file only its user can read; [`MemorySessionStore`] keeps it in memory. `Debug` on
//! these types never shows a token.

use std::path::PathBuf;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, SystemTime};

use emberhut_api::auth::{AccessToken, RefreshToken, SessionGrant};
use emberhut_api::ids::{DeviceId, InstallId};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

/// Renew the access token when it has less than this left.
pub(crate) const RENEW_MARGIN: Duration = Duration::from_secs(60);

/// How long an access token is taken to last when the server doesn't say: a little under the
/// reference server's hour.
const ASSUMED_LIFETIME: Duration = Duration::from_mins(50);

/// The longest lifetime taken as the server meant it.
const MAX_LIFETIME: Duration = Duration::from_hours(24);

const HEX: &[u8; 16] = b"0123456789abcdef";

/// This installation and, while it is signed in, its tokens.
///
/// Its serde form is the session file's: `{ "version": 1, "install": ..., "tokens": { "access",
/// "refresh", "device", "accessExpiresMs" } }`. A file of another version doesn't read (see
/// [`Session::FORMAT_VERSION`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(into = "SessionFile", try_from = "SessionFile")]
pub struct Session {
    /// This installation: the server keeps one device row per install.
    pub install: InstallId,
    /// The tokens, while signed in.
    pub tokens: Option<SessionTokens>,
}

impl Session {
    /// The version of the stored form this client writes and reads. A change to the form bumps
    /// it; a client then reads the versions before its own (migrating them) and refuses newer
    /// ones, so it never overwrites a newer client's session.
    pub const FORMAT_VERSION: u32 = 1;

    /// A new installation, not signed in, with a random id (`zr-` and 32 hex digits).
    ///
    /// # Errors
    ///
    /// The system's random number source failed.
    pub fn new_install() -> Result<Self, std::io::Error> {
        let mut bytes = [0_u8; 16];
        getrandom::getrandom(&mut bytes).map_err(std::io::Error::from)?;
        let id = bytes.iter().fold(String::from("zr-"), |mut id, byte| {
            id.push(char::from(HEX[usize::from(byte >> 4)]));
            id.push(char::from(HEX[usize::from(byte & 0xf)]));
            id
        });
        Ok(Self {
            install: InstallId::new(id),
            tokens: None,
        })
    }

    /// Whether it holds tokens.
    #[must_use]
    pub fn is_signed_in(&self) -> bool {
        self.tokens.is_some()
    }
}

/// The tokens of a signed-in device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionTokens {
    /// Sent with every request.
    pub access: AccessToken,
    /// Renews the access token; it turns over on every use.
    pub refresh: RefreshToken,
    /// This device's row on the server.
    pub device: Option<DeviceId>,
    /// When the access token runs out, by this device's clock.
    pub access_expires: SystemTime,
}

impl SessionTokens {
    /// The tokens of a grant received at `now`, in an answer the server dated `server_date`
    /// (its `Date` header).
    ///
    /// The expiry is the grant's lifetime (`expires` less `server_date`, both the server's
    /// clock) from `now`, so a device clock that differs from the server's doesn't shorten or
    /// stretch it. Without a date it is `expires` less `now`; without an expiry (or one that
    /// doesn't parse) the token is taken to last 50 minutes.
    ///
    /// A lifetime that can't be right (no longer than the renewal margin, as when `expires`
    /// isn't after `Date`, or longer than a day) is replaced by those 50 minutes and logged: a
    /// zero lifetime would renew on every request, turning the refresh token over each time.
    /// If the server's token is in fact shorter, its refusal renews it once (`Client`'s retry
    /// on a 401).
    #[must_use]
    pub fn from_grant(
        grant: SessionGrant,
        now: SystemTime,
        server_date: Option<SystemTime>,
    ) -> Self {
        let expires = grant
            .expires
            .as_ref()
            .and_then(|at| OffsetDateTime::parse(at.as_str(), &Rfc3339).ok())
            .map(SystemTime::from);
        let lifetime = match (expires, server_date) {
            (Some(expires), Some(date)) => expires.duration_since(date).unwrap_or_default(),
            (Some(expires), None) => expires.duration_since(now).unwrap_or_default(),
            (None, _) => ASSUMED_LIFETIME,
        };
        let lifetime = if lifetime <= RENEW_MARGIN || lifetime > MAX_LIFETIME {
            tracing::warn!(
                ?lifetime,
                "an access token lifetime that can't be right; taking it to be 50 minutes"
            );
            ASSUMED_LIFETIME
        } else {
            lifetime
        };
        let access_expires = now + lifetime;
        Self {
            access: grant.access,
            refresh: grant.refresh,
            device: grant.device,
            access_expires,
        }
    }

    /// Whether the access token should be renewed before it is used at `now`: it has less than a
    /// minute left.
    #[must_use]
    pub fn needs_renewal(&self, now: SystemTime) -> bool {
        now + RENEW_MARGIN >= self.access_expires
    }

    /// Whether the access token has run out at `now`.
    #[must_use]
    pub fn is_expired(&self, now: SystemTime) -> bool {
        now >= self.access_expires
    }
}

/// The stored form of a [`Session`].
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SessionFile {
    version: u32,
    install: InstallId,
    #[serde(default)]
    tokens: Option<TokensFile>,
}

/// The stored form of [`SessionTokens`]: the expiry in Unix milliseconds.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TokensFile {
    access: AccessToken,
    refresh: RefreshToken,
    device: Option<DeviceId>,
    access_expires_ms: u64,
}

impl From<Session> for SessionFile {
    fn from(session: Session) -> Self {
        Self {
            version: Session::FORMAT_VERSION,
            install: session.install,
            tokens: session.tokens.map(|t| TokensFile {
                access: t.access,
                refresh: t.refresh,
                device: t.device,
                access_expires_ms: t
                    .access_expires
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX)),
            }),
        }
    }
}

/// A stored session of a version this client doesn't read.
#[derive(Debug)]
struct OtherVersion(u32);

impl std::fmt::Display for OtherVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "session format {}, not {}",
            self.0,
            Session::FORMAT_VERSION
        )
    }
}

impl TryFrom<SessionFile> for Session {
    type Error = OtherVersion;

    fn try_from(file: SessionFile) -> Result<Self, Self::Error> {
        if file.version != Self::FORMAT_VERSION {
            return Err(OtherVersion(file.version));
        }
        Ok(Self {
            install: file.install,
            tokens: file.tokens.map(|t| SessionTokens {
                access: t.access,
                refresh: t.refresh,
                device: t.device,
                access_expires: SystemTime::UNIX_EPOCH + Duration::from_millis(t.access_expires_ms),
            }),
        })
    }
}

/// The format version of a stored session (its JSON form), read before the rest: `None` when
/// there isn't one. A [`SessionStore`] that isn't a file checks it as [`crate::FileSessionStore`]
/// does: a version above [`Session::FORMAT_VERSION`] is a newer client's, neither read nor
/// overwritten ([`StoreError::NewerFormatInStore`]).
///
/// ```
/// use emberhut_client::{Session, stored_version};
///
/// assert_eq!(stored_version(br#"{"version": 9, "install": "zr-1"}"#), Some(9));
/// assert_eq!(stored_version(b"not a session"), None);
/// assert!(Session::FORMAT_VERSION < 9);
/// ```
#[must_use]
pub fn stored_version(bytes: &[u8]) -> Option<u32> {
    #[derive(Deserialize)]
    struct Probe {
        version: Option<u32>,
    }
    serde_json::from_slice::<Probe>(bytes).ok()?.version
}

/// Where a session is kept between runs.
///
/// Implementations are called from several threads; each call should be atomic (a reader never
/// sees half a save).
pub trait SessionStore: Send + Sync + std::fmt::Debug {
    /// The saved session, or none when nothing was saved yet.
    ///
    /// # Errors
    ///
    /// The store couldn't be read, or what it holds isn't a session.
    fn load(&self) -> Result<Option<Session>, StoreError>;

    /// Saves the session in place of the one before, unless that one is a newer client's.
    ///
    /// # Errors
    ///
    /// The store couldn't be written, or holds a newer client's session
    /// ([`StoreError::NewerFormat`], [`StoreError::NewerFormatInStore`]); the session before
    /// stays.
    fn save(&self, session: &Session) -> Result<(), StoreError>;

    /// Forgets the session, install id included.
    ///
    /// # Errors
    ///
    /// The store couldn't be cleared.
    fn clear(&self) -> Result<(), StoreError>;
}

/// Why a [`SessionStore`] failed.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum StoreError {
    /// A file couldn't be read or written.
    #[error("couldn't read or write {}", path.display())]
    Io {
        /// The file.
        path: PathBuf,
        /// Why.
        #[source]
        source: std::io::Error,
    },
    /// What is stored isn't a session.
    #[error("{} isn't a session file", path.display())]
    Corrupt {
        /// The file.
        path: PathBuf,
        /// Why it doesn't read.
        #[source]
        source: serde_json::Error,
    },
    /// The session couldn't be written as JSON.
    #[error("couldn't write the session")]
    Encode(#[source] serde_json::Error),
    /// The store holds a session of a newer client: it isn't read, and it isn't overwritten.
    #[error("{} holds a session of a newer client (format {version})", path.display())]
    NewerFormat {
        /// The file.
        path: PathBuf,
        /// Its format version.
        version: u32,
    },
    /// A store that isn't a file (a keychain, say) holds a session of a newer client: it isn't
    /// read, and it isn't overwritten.
    #[error("the session store holds a session of a newer client (format {version})")]
    NewerFormatInStore {
        /// Its format version.
        version: u32,
    },
    /// Another store's own failure (a keychain's, say).
    #[error("the session store failed")]
    Backend(#[source] Box<dyn std::error::Error + Send + Sync>),
}

/// A session kept in memory only: gone when the process ends. For tests, and for a client that
/// shouldn't remember its sign-in.
#[derive(Debug, Default)]
pub struct MemorySessionStore {
    session: Mutex<Option<Session>>,
}

impl MemorySessionStore {
    /// An empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// A store that holds this session.
    #[must_use]
    pub fn with_session(session: Session) -> Self {
        Self {
            session: Mutex::new(Some(session)),
        }
    }
}

impl SessionStore for MemorySessionStore {
    fn load(&self) -> Result<Option<Session>, StoreError> {
        Ok(self
            .session
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone())
    }

    fn save(&self, session: &Session) -> Result<(), StoreError> {
        *self.session.lock().unwrap_or_else(PoisonError::into_inner) = Some(session.clone());
        Ok(())
    }

    fn clear(&self) -> Result<(), StoreError> {
        *self.session.lock().unwrap_or_else(PoisonError::into_inner) = None;
        Ok(())
    }
}

impl<S: SessionStore + ?Sized> SessionStore for std::sync::Arc<S> {
    fn load(&self) -> Result<Option<Session>, StoreError> {
        (**self).load()
    }

    fn save(&self, session: &Session) -> Result<(), StoreError> {
        (**self).save(session)
    }

    fn clear(&self) -> Result<(), StoreError> {
        (**self).clear()
    }
}

#[cfg(test)]
mod tests {
    use emberhut_api::time::Timestamp;

    use super::*;

    fn grant(expires: Option<&str>) -> SessionGrant {
        SessionGrant {
            access: AccessToken::new("access-token-for-tests"),
            refresh: RefreshToken::new("refresh-token-for-tests"),
            device: Some(DeviceId::new(3)),
            expires: expires.map(Timestamp::new),
        }
    }

    #[test]
    fn install_ids_are_random_and_short() {
        let a = Session::new_install().unwrap();
        let b = Session::new_install().unwrap();
        assert_ne!(a.install, b.install);
        assert_eq!(a.install.as_str().len(), 35);
        assert!(a.install.as_str().len() <= InstallId::MAX_LEN);
        assert!(!a.is_signed_in());
    }

    #[test]
    fn the_servers_expiry_is_kept() {
        let expires = SystemTime::UNIX_EPOCH + Duration::from_millis(1_795_561_073_410);
        let now = expires - Duration::from_hours(1);
        let tokens = SessionTokens::from_grant(grant(Some("2026-11-24T22:57:53.410Z")), now, None);
        assert_eq!(tokens.access_expires, expires);
        assert_eq!(tokens.device, Some(DeviceId::new(3)));
    }

    #[test]
    fn a_lifetime_that_cant_be_right_is_taken_as_fifty_minutes() {
        let date = SystemTime::UNIX_EPOCH + Duration::from_millis(1_795_561_073_410);
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1000);
        for (expires, why) in [
            ("2026-11-24T22:57:53.410Z", "expires at the Date"),
            ("2026-11-24T21:57:53.410Z", "expires before the Date"),
            ("2026-11-24T22:58:30.000Z", "within the renewal margin"),
            ("2026-11-27T22:57:53.410Z", "longer than a day"),
        ] {
            let tokens = SessionTokens::from_grant(grant(Some(expires)), now, Some(date));
            assert_eq!(
                tokens.access_expires,
                now + Duration::from_mins(50),
                "{why}"
            );
            assert!(
                !tokens.needs_renewal(now),
                "{why}: not renewed on every request"
            );
        }
    }

    #[test]
    fn a_clock_apart_from_the_servers_keeps_the_lifetime() {
        // The server's clock says 22:57:53.410 and the token lasts an hour; this device's clock
        // is five minutes behind.
        let server_now = SystemTime::UNIX_EPOCH + Duration::from_millis(1_795_561_073_410);
        let device_now = server_now - Duration::from_mins(5);
        let grant = grant(Some("2026-11-24T23:57:53.410Z"));
        let tokens = SessionTokens::from_grant(grant, device_now, Some(server_now));
        assert_eq!(tokens.access_expires, device_now + Duration::from_hours(1));
    }

    #[test]
    fn the_stored_form_is_versioned_with_the_expiry_in_milliseconds() {
        let mut session = Session::new_install().unwrap();
        let expires = SystemTime::UNIX_EPOCH + Duration::from_millis(1_795_561_073_410);
        session.tokens = Some(SessionTokens::from_grant(
            grant(Some("2026-11-24T22:57:53.410Z")),
            expires - Duration::from_hours(1),
            None,
        ));
        let stored = serde_json::to_value(&session).unwrap();
        assert_eq!(stored["version"], 1);
        assert_eq!(stored["tokens"]["accessExpiresMs"], 1_795_561_073_410_u64);
        assert_eq!(stored["tokens"]["refresh"], "refresh-token-for-tests");
        assert_eq!(
            serde_json::from_value::<Session>(stored.clone()).unwrap(),
            session
        );
        assert_eq!(stored_version(stored.to_string().as_bytes()), Some(1));

        let mut newer = stored;
        newer["version"] = 2.into();
        let error = serde_json::from_value::<Session>(newer).unwrap_err();
        assert!(
            error.to_string().contains("session format 2, not 1"),
            "{error}"
        );
        assert_eq!(stored_version(b"{}"), None);
        assert_eq!(stored_version(b"not json"), None);
    }

    #[test]
    fn without_an_expiry_a_token_lasts_fifty_minutes() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1000);
        for expires in [None, Some("not a time")] {
            let tokens = SessionTokens::from_grant(grant(expires), now, None);
            assert_eq!(tokens.access_expires, now + Duration::from_mins(50));
        }
    }

    #[test]
    fn renewal_starts_a_minute_early() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(10_000);
        let mut tokens = SessionTokens::from_grant(grant(None), now, None);
        tokens.access_expires = now + Duration::from_secs(61);
        assert!(!tokens.needs_renewal(now));
        tokens.access_expires = now + Duration::from_secs(60);
        assert!(tokens.needs_renewal(now));
        assert!(!tokens.is_expired(now));
        assert!(tokens.is_expired(now + Duration::from_secs(60)));
    }

    #[test]
    fn debug_never_shows_a_token() {
        let session = Session {
            install: InstallId::new("zr-1"),
            tokens: Some(SessionTokens::from_grant(
                grant(None),
                SystemTime::UNIX_EPOCH,
                None,
            )),
        };
        let shown = format!("{session:?}");
        assert!(!shown.contains("token-for-tests"), "{shown}");
        assert!(shown.contains("zr-1"));
    }

    #[test]
    fn memory_store() {
        let store = MemorySessionStore::new();
        assert_eq!(store.load().unwrap(), None);
        let session = Session::new_install().unwrap();
        store.save(&session).unwrap();
        assert_eq!(store.load().unwrap(), Some(session.clone()));
        let shared = std::sync::Arc::new(MemorySessionStore::with_session(session.clone()));
        assert_eq!(shared.load().unwrap(), Some(session));
        shared.clear().unwrap();
        assert_eq!(shared.load().unwrap(), None);
    }
}
