//! A transport's failure.

use std::error::Error as StdError;
use std::{fmt, io};

/// A request that didn't get through, or an answer that didn't arrive (no connection, a
/// timeout, a broken stream), with the transport's own error inside.
///
/// Its [`kind`](Self::kind) says how far the request got, which tells whether it may be sent
/// again elsewhere; [`is_timeout`](Self::is_timeout) says whether a wait ran out. Together they
/// tell a connection that took too long ([`TransportErrorKind::NotSent`]) from an answer that
/// did ([`TransportErrorKind::Sent`]). It shows as the transport's error and has its source, so
/// a transport's words reach the caller as they are.
#[derive(Debug)]
pub struct TransportError {
    kind: TransportErrorKind,
    timeout: bool,
    error: Box<dyn StdError + Send + Sync>,
}

/// How far a failed request got.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum TransportErrorKind {
    /// Nothing was sent: the name didn't resolve, or the connection or its handshake (TLS, or
    /// a wrapping transport's own) failed before the first byte of the request went out. The
    /// request may be sent on another route.
    NotSent,
    /// The request may have gone out, at least in part, and then writing it or reading the
    /// answer failed (or the transport can't tell how far it got): the machine may have acted
    /// on it.
    Sent,
    /// The caller's own request body failed (its reader, or a progress callback reading
    /// through it), not the way to the machine.
    Body,
}

impl TransportError {
    /// A failure after the request may have reached the machine
    /// ([`TransportErrorKind::Sent`]), not a timeout: the kind that promises least. A transport
    /// that knows more says so with [`with_kind`](Self::with_kind); a kind added later is only
    /// ever given that way, so what `new` means doesn't change.
    #[must_use]
    pub fn new(error: impl Into<Box<dyn StdError + Send + Sync>>) -> Self {
        Self {
            kind: TransportErrorKind::Sent,
            timeout: false,
            error: error.into(),
        }
    }

    /// A wait that ran out, after the request may have reached the machine
    /// ([`TransportErrorKind::Sent`]); a connection that took too long is
    /// `TransportError::timeout(error).with_kind(TransportErrorKind::NotSent)`.
    #[must_use]
    pub fn timeout(error: impl Into<Box<dyn StdError + Send + Sync>>) -> Self {
        Self {
            timeout: true,
            ..Self::new(error)
        }
    }

    /// The same failure, of this kind.
    #[must_use]
    pub fn with_kind(mut self, kind: TransportErrorKind) -> Self {
        self.kind = kind;
        self
    }

    /// How far the request got.
    #[must_use]
    pub fn kind(&self) -> TransportErrorKind {
        self.kind
    }

    /// Whether a wait ran out.
    #[must_use]
    pub fn is_timeout(&self) -> bool {
        self.timeout
    }
}

/// A body read that failed: the transport's own error when the [`io::Error`] carries one (as
/// an [`Answer`](super::Answer)'s body may), else a failure after the request went out
/// ([`TransportErrorKind::Sent`]), a timeout when its kind is [`io::ErrorKind::TimedOut`].
impl From<io::Error> for TransportError {
    fn from(error: io::Error) -> Self {
        match error.downcast::<Self>() {
            Ok(failure) => failure,
            Err(error) if error.kind() == io::ErrorKind::TimedOut => Self::timeout(error),
            Err(error) => Self::new(error),
        }
    }
}

impl fmt::Display for TransportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.error, f)
    }
}

impl StdError for TransportError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        self.error.source()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shows_the_transports_own_words() {
        let error = TransportError::new("no route to the machine");
        assert_eq!(error.to_string(), "no route to the machine");
        assert!(!error.is_timeout());
        assert_eq!(error.kind(), TransportErrorKind::Sent);
        let slow = TransportError::timeout("waited");
        assert!(slow.is_timeout());
        assert_eq!(slow.kind(), TransportErrorKind::Sent);
    }

    #[test]
    fn a_kind_says_how_far_it_got() {
        let connect =
            TransportError::timeout("no connection").with_kind(TransportErrorKind::NotSent);
        assert!(connect.is_timeout());
        assert_eq!(connect.kind(), TransportErrorKind::NotSent);
        let body = TransportError::new("disk gone").with_kind(TransportErrorKind::Body);
        assert_eq!(body.kind(), TransportErrorKind::Body);
        assert!(!body.is_timeout());
    }

    #[test]
    fn a_read_that_timed_out_is_a_timeout() {
        let timed_out = io::Error::new(io::ErrorKind::TimedOut, "slow");
        assert!(TransportError::from(timed_out).is_timeout());
        let reset = io::Error::new(io::ErrorKind::ConnectionReset, "reset");
        let error = TransportError::from(reset);
        assert!(!error.is_timeout());
        assert_eq!(error.kind(), TransportErrorKind::Sent);
        assert_eq!(error.to_string(), "reset");
    }

    #[test]
    fn a_transports_error_inside_a_read_comes_back_whole() {
        let connect = TransportError::timeout("dial").with_kind(TransportErrorKind::NotSent);
        let read = io::Error::new(io::ErrorKind::TimedOut, connect);
        let back = TransportError::from(read);
        assert_eq!(
            (back.kind(), back.is_timeout()),
            (TransportErrorKind::NotSent, true)
        );
        assert_eq!(back.to_string(), "dial");
    }
}
