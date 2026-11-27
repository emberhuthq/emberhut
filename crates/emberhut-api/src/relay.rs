//! The relay protocol, version 1 (`docs/relay.md`): the control messages between a machine and
//! the relay, the close codes and reasons, and the relay's own error header.
//!
//! A machine opens a WebSocket to `wss://connect.<relay domain>/v1`. Text messages carry the
//! JSON control messages ([`Control`]) for the connection's whole life; binary messages carry
//! yamux. Unknown fields and unknown message types are ignored, so a later addition doesn't
//! break older peers.

use std::fmt;
use std::str::FromStr;

use emberhut_identity::{
    MachineId, MachineName, Nonce, PublicKey, RelayAuthStatement, RelayDomain, SecretKey,
    Signature, SignatureError, UnixMillis,
};
use serde::{Deserialize, Serialize};

use crate::open_enum::open_enum;

/// The protocol's version, sent in [`Hello`].
pub const VERSION: u32 = 1;
/// The path machines dial at `connect.<relay domain>`. The directory's API is under the same
/// version prefix, on the relay domain itself (`docs/directory.md`).
pub const PATH: &str = "/v1";
/// The label machines dial: `connect.<relay domain>`.
pub const CONNECT_LABEL: &str = "connect";
/// The longest message, in bytes; a longer one closes the connection with
/// [`CloseCode::TooLarge`].
pub const MAX_MESSAGE: usize = 64 * 1024;

/// A control message, a JSON text message on the machine's connection, told apart by `type`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Control {
    /// Relay to machine, first.
    Hello(Hello),
    /// Machine to relay, in answer to `hello`. Boxed: it is far larger than the others.
    Auth(Box<Auth>),
    /// Relay to machine: accepted; yamux starts with the next binary message.
    Welcome(Welcome),
    /// Relay to machine: move to a new connection.
    Drain(Drain),
    /// A message type this version doesn't know: ignored.
    #[serde(other)]
    Unknown,
}

/// `hello`: the relay's greeting, with a fresh nonce for the machine to sign.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hello {
    /// The protocol version, [`VERSION`].
    pub version: u32,
    /// 32 fresh random bytes.
    pub nonce: Nonce,
    /// The relay's clock.
    pub time: UnixMillis,
}

/// `auth`: the machine's key and its signature over the relay's domain, its nonce, its name and
/// `time`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Auth {
    /// The machine's public key; the relay derives the id from it.
    pub public_key: PublicKey,
    /// The machine's name: 1 to 60 characters, trimmed. The machine cuts a longer one before it
    /// signs; the relay refuses one that doesn't read (4400).
    pub name: MachineName,
    /// The machine's clock, which must be within 5 minutes of the relay's.
    pub time: UnixMillis,
    /// The signature over the `emberhut relay auth v1` statement.
    pub signature: Signature,
}

impl Auth {
    /// The machine's answer to `hello` from the relay at `relay_domain`.
    #[must_use]
    pub fn sign(
        key: &SecretKey,
        name: MachineName,
        relay_domain: RelayDomain,
        hello: &Hello,
        time: UnixMillis,
    ) -> Self {
        let statement = RelayAuthStatement::new(relay_domain, hello.nonce, name, time);
        Self {
            public_key: key.public_key(),
            name: statement.name().clone(),
            time,
            signature: key.sign(&statement),
        }
    }

    /// The statement the signature covers, for the relay at `relay_domain` that sent `nonce`.
    #[must_use]
    pub fn statement(&self, relay_domain: RelayDomain, nonce: Nonce) -> RelayAuthStatement {
        RelayAuthStatement::new(relay_domain, nonce, self.name.clone(), self.time)
    }

    /// Checks the signature for this relay and its nonce, and returns the machine's id.
    ///
    /// # Errors
    ///
    /// [`SignatureError`] when it doesn't verify ([`CloseCode::BadSignature`]).
    pub fn verify(
        &self,
        relay_domain: RelayDomain,
        nonce: Nonce,
    ) -> Result<MachineId, SignatureError> {
        self.public_key
            .verify(&self.statement(relay_domain, nonce), &self.signature)?;
        Ok(self.public_key.machine_id())
    }
}

/// `welcome`: the machine is connected.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Welcome {
    /// The machine's own origin, `https://<id>.<relay domain>`.
    pub url: String,
    /// The connection's limits.
    pub limits: Limits,
    /// How often the machine pings, in seconds.
    pub ping: u32,
}

/// The limits in `welcome`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Limits {
    /// The most yamux streams at once.
    pub streams: u32,
    /// Each stream's window, in bytes.
    pub window: u32,
}

/// `drain`: open a new connection at a random moment within `within` seconds, then send yamux
/// `GoAway` on this one and let its streams finish.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Drain {
    /// The window, in seconds.
    pub within: u32,
}

/// How the relay closes a machine's connection, and what the machine does then.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum CloseCode {
    /// 4400: a bad handshake or an unknown version. Try again after an hour.
    BadHandshake,
    /// 4401: the signature doesn't verify. Try again after an hour.
    BadSignature,
    /// 4403: not allowed, or revoked. Try again every 15 minutes, and at once after linking.
    NotAllowed,
    /// 4408: the machine's clock is more than 5 minutes off (`time=` in the reason). Try again
    /// after 60 s.
    ClockSkew,
    /// 4409: replaced by a newer connection with the same id. Try again after 5 minutes.
    Replaced,
    /// 4410: a handshake deadline was missed (`reason=deadline`). Reconnect as after a drop.
    Deadline,
    /// 4429: too many connections for this id or account (`retry=` in the reason).
    TooMany,
    /// 4502: the authoriser can't be reached. Try again after a random 5 to 30 s.
    AuthoriserUnreachable,
    /// 1009: a message over 64 KiB. Reconnect as after a drop.
    TooLarge,
}

/// A close code outside the table: reconnect as after a drop.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
#[error("close code {0} isn't one of the relay's")]
pub struct UnknownCloseCode(pub u16);

impl From<CloseCode> for u16 {
    /// The WebSocket close code.
    fn from(code: CloseCode) -> Self {
        match code {
            CloseCode::BadHandshake => 4400,
            CloseCode::BadSignature => 4401,
            CloseCode::NotAllowed => 4403,
            CloseCode::ClockSkew => 4408,
            CloseCode::Replaced => 4409,
            CloseCode::Deadline => 4410,
            CloseCode::TooMany => 4429,
            CloseCode::AuthoriserUnreachable => 4502,
            CloseCode::TooLarge => 1009,
        }
    }
}

impl TryFrom<u16> for CloseCode {
    type Error = UnknownCloseCode;

    fn try_from(code: u16) -> Result<Self, Self::Error> {
        Ok(match code {
            4400 => Self::BadHandshake,
            4401 => Self::BadSignature,
            4403 => Self::NotAllowed,
            4408 => Self::ClockSkew,
            4409 => Self::Replaced,
            4410 => Self::Deadline,
            4429 => Self::TooMany,
            4502 => Self::AuthoriserUnreachable,
            1009 => Self::TooLarge,
            _ => return Err(UnknownCloseCode(code)),
        })
    }
}

/// A close reason: ASCII `key=value` pairs separated by `;`, at most 123 bytes, with
/// `reason=<word>` always, plus `time=` (4408) or `retry=` (4429). Unknown keys are ignored.
///
/// The word is 1 or more visible ASCII characters other than `;` and `=`. Every reason this
/// type holds fits the 123 bytes.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CloseReason {
    reason: String,
    time: Option<UnixMillis>,
    retry: Option<u32>,
}

/// Why a close reason can't be made or read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
#[non_exhaustive]
pub enum CloseReasonError {
    /// Over 123 bytes, WebSocket's limit.
    #[error("a close reason is at most 123 bytes")]
    TooLong,
    /// The word is empty, or has a character other than visible ASCII, or a `;` or `=`.
    #[error("a close reason's word is visible ASCII without ; or =")]
    Word,
    /// Not `key=value` pairs separated by `;`, or a `time` or `retry` that isn't a number.
    #[error("a close reason is ASCII key=value pairs separated by ;")]
    Malformed,
    /// No `reason=`.
    #[error("a close reason has a reason=")]
    NoReason,
}

impl CloseReason {
    /// WebSocket's limit on a close reason, in bytes.
    pub const MAX_LEN: usize = 123;

    /// `reason=<word>`, with `time=` and `retry=` when given.
    ///
    /// # Errors
    ///
    /// [`CloseReasonError::Word`] for a word that isn't one, and
    /// [`CloseReasonError::TooLong`] when the whole is over 123 bytes.
    pub fn new(
        reason: &str,
        time: Option<UnixMillis>,
        retry: Option<u32>,
    ) -> Result<Self, CloseReasonError> {
        let is_word = !reason.is_empty()
            && reason
                .bytes()
                .all(|byte| byte.is_ascii_graphic() && byte != b';' && byte != b'=');
        if !is_word {
            return Err(CloseReasonError::Word);
        }
        let made = Self {
            reason: reason.to_owned(),
            time,
            retry,
        };
        if made.to_string().len() > Self::MAX_LEN {
            return Err(CloseReasonError::TooLong);
        }
        Ok(made)
    }

    /// The word for why (`deadline`, ...).
    #[must_use]
    pub fn reason(&self) -> &str {
        &self.reason
    }

    /// The relay's clock, on 4408.
    #[must_use]
    pub const fn time(&self) -> Option<UnixMillis> {
        self.time
    }

    /// How many seconds to wait, on 4429.
    #[must_use]
    pub const fn retry(&self) -> Option<u32> {
        self.retry
    }
}

impl fmt::Display for CloseReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "reason={}", self.reason)?;
        if let Some(time) = self.time {
            write!(f, ";time={time}")?;
        }
        if let Some(retry) = self.retry {
            write!(f, ";retry={retry}")?;
        }
        Ok(())
    }
}

impl FromStr for CloseReason {
    type Err = CloseReasonError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        if text.len() > Self::MAX_LEN {
            return Err(CloseReasonError::TooLong);
        }
        if !text.is_ascii() {
            return Err(CloseReasonError::Malformed);
        }
        let mut reason = None;
        let mut time = None;
        let mut retry = None;
        for pair in text.split(';') {
            let (key, value) = pair.split_once('=').ok_or(CloseReasonError::Malformed)?;
            match key {
                "reason" => reason = Some(value),
                "time" => {
                    let millis: u64 = value.parse().map_err(|_| CloseReasonError::Malformed)?;
                    time = Some(UnixMillis::new(millis).map_err(|_| CloseReasonError::Malformed)?);
                }
                "retry" => retry = Some(value.parse().map_err(|_| CloseReasonError::Malformed)?),
                _ => {}
            }
        }
        Self::new(reason.ok_or(CloseReasonError::NoReason)?, time, retry)
    }
}

open_enum! {
    /// The code in [`EMBERHUT_RELAY_ERROR`](crate::headers::EMBERHUT_RELAY_ERROR) on an answer the relay made itself. Clients act on it,
    /// never on the status or the sentence alone.
    pub enum RelayError {
        /// 404: an unknown label.
        UnknownHost = "unknown-host",
        /// 421: a reused connection can't reach that machine; retry on a new connection.
        Misdirected = "misdirected",
        /// 429: over a limit; wait out `Retry-After`.
        Limit = "limit",
        /// 431: the request's head is over 32 KiB.
        TooLarge = "too-large",
        /// 503: the machine has no uplink. The relay route has failed: start a race.
        Offline = "offline",
        /// 503: the machine's streams are all in use; `Retry-After: 1`.
        Busy = "busy",
        /// 503: the relay's buffers stayed full; `Retry-After: 5`.
        Full = "full",
        /// 504: no response head from the machine within 100 s.
        Timeout = "timeout",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn close_codes_round_trip() {
        for code in [4400, 4401, 4403, 4408, 4409, 4410, 4429, 4502, 1009] {
            assert_eq!(CloseCode::try_from(code).map(u16::from), Ok(code));
        }
        assert_eq!(CloseCode::try_from(1000), Err(UnknownCloseCode(1000)));
    }

    #[test]
    fn close_reasons_are_checked_when_made() {
        let time = UnixMillis::new(1_795_561_073_000).unwrap();
        let clock = CloseReason::new("clock", Some(time), None).unwrap();
        assert_eq!(clock.to_string(), "reason=clock;time=1795561073000");
        assert_eq!(
            (clock.reason(), clock.time(), clock.retry()),
            ("clock", Some(time), None)
        );
        for word in ["", "a;b", "a=b", "a b", "é"] {
            assert_eq!(
                CloseReason::new(word, None, None),
                Err(CloseReasonError::Word),
                "{word:?}"
            );
        }
        assert!(CloseReason::new(&"x".repeat(116), None, None).is_ok());
        assert_eq!(
            CloseReason::new(&"x".repeat(117), None, None),
            Err(CloseReasonError::TooLong)
        );
        assert_eq!(
            CloseReason::new(&"x".repeat(100), Some(time), None),
            Err(CloseReasonError::TooLong)
        );
    }

    #[test]
    fn close_reasons_read_and_write() {
        let reason: CloseReason = "reason=clock;time=1795561073000;future=1".parse().unwrap();
        assert_eq!(reason.to_string(), "reason=clock;time=1795561073000");
        assert_eq!(
            "time=1".parse::<CloseReason>(),
            Err(CloseReasonError::NoReason)
        );
        assert_eq!(
            "reason".parse::<CloseReason>(),
            Err(CloseReasonError::Malformed)
        );
        assert_eq!(
            "reason=a=b".parse::<CloseReason>(),
            Err(CloseReasonError::Word)
        );
        assert_eq!(
            "reason=x;retry=soon".parse::<CloseReason>(),
            Err(CloseReasonError::Malformed)
        );
    }

    #[test]
    fn an_auth_verifies_for_its_relay_and_nonce_only() {
        let key = SecretKey::from_bytes(&[4; 32]);
        let domain: RelayDomain = "relay.example".parse().unwrap();
        let hello = Hello {
            version: VERSION,
            nonce: Nonce::from_bytes([8; 32]),
            time: UnixMillis::new(1_795_561_073_000).unwrap(),
        };
        let name = MachineName::cut("Main computer").unwrap();
        let auth = Auth::sign(&key, name, domain.clone(), &hello, hello.time);
        assert_eq!(
            auth.verify(domain.clone(), hello.nonce),
            Ok(key.machine_id())
        );
        let other: RelayDomain = "other.example".parse().unwrap();
        assert_eq!(auth.verify(other, hello.nonce), Err(SignatureError));
        assert_eq!(
            auth.verify(domain.clone(), Nonce::from_bytes([9; 32])),
            Err(SignatureError)
        );
        let mut renamed = auth;
        renamed.name = MachineName::cut("Another").unwrap();
        assert_eq!(
            renamed.verify(domain, hello.nonce),
            Err(SignatureError),
            "the name is signed"
        );
    }
}
