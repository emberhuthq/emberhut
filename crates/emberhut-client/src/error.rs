//! The client's errors.

use std::fmt;

use emberhut_api::auth::AuthRefusal;

use crate::config::InvalidBaseUrl;
use crate::session::StoreError;
use crate::transport::TransportError;

/// What can go wrong talking to the server.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// The base URL isn't a server address: [`crate::ClientConfig::new`]'s error, for `?` in
    /// code that returns this type.
    #[error(transparent)]
    InvalidBaseUrl(#[from] InvalidBaseUrl),
    /// The request didn't reach the server or its answer didn't arrive whole (no connection, a
    /// timeout, a broken stream).
    #[error("can't reach the server")]
    Network(#[source] NetworkError),
    /// The server refused the request or failed, with its own words when it gave them.
    #[error("the server answered {status}{}", detail(.message.as_deref()))]
    Status {
        /// The HTTP status.
        status: u16,
        /// The `error` of its body.
        message: Option<String>,
    },
    /// The server enforces sign-in and this device has no valid session: pair it (or sign in)
    /// first.
    #[error("this device has to sign in first")]
    SignInRequired,
    /// Pairing was refused (a wrong, used or expired code; too many wrong codes).
    #[error("sign-in refused")]
    Refused(#[source] AuthRefusal),
    /// The answer wasn't the JSON the contract describes.
    #[error("couldn't read the server's answer")]
    Decode(#[source] serde_json::Error),
    /// A request couldn't be written as JSON. The contract's types always can; a caller's own
    /// value might not.
    #[error("couldn't write the request")]
    Encode(#[source] serde_json::Error),
    /// The live stream broke the event-stream format (a line that isn't UTF-8, or longer than
    /// 4 MiB). Trying again won't help.
    #[error("the live stream isn't a valid event stream")]
    Protocol(#[source] std::io::Error),
    /// The session couldn't be loaded or saved.
    #[error("the session store failed")]
    Store(#[source] StoreError),
    /// The system's random number source failed (making this install's id).
    #[error("no randomness for an install id")]
    Random(#[source] std::io::Error),
    /// A path given to [`crate::Client::file`] isn't a path on this server (it names another
    /// host, a scheme, or isn't a path at all), or a name given to
    /// [`crate::Client::forget_runner`] isn't a runner's, so it was never requested.
    #[error("`{0}` isn't a path on the server")]
    InvalidPath(String),
    /// A file to upload couldn't be read: opened, sized, or read while it was being sent.
    #[error("couldn't read the file to upload")]
    Read(#[source] std::io::Error),
    /// A downloaded file couldn't be written where the caller said.
    #[error("couldn't write the downloaded file")]
    Write(#[source] std::io::Error),
    /// A file is larger than a transfer takes: an upload over the server's limit (never sent),
    /// or a download over [`crate::MAX_DOWNLOAD`] (stopped there).
    #[error("the file is {size} bytes, over the limit of {limit}")]
    TooLarge {
        /// Its size in bytes (for a download without a size given, the bytes it had reached).
        size: u64,
        /// The limit: [`emberhut_api::uploads::MAX_SIZE`], [`emberhut_api::transcribe::MAX_SIZE`] or
        /// [`crate::MAX_DOWNLOAD`].
        limit: u64,
    },
    /// The caller's progress callback stopped the transfer.
    #[error("the transfer was cancelled")]
    Cancelled,
    /// A pairing from a link was answered by another machine than the link names, or by a
    /// server that didn't say which machine it is. Nothing was kept.
    #[error("this server didn't prove it is the machine you meant")]
    WrongMachine,
    /// The client was stopped ([`crate::Client::stop`]): it sends nothing any more.
    #[error("this client was stopped")]
    Stopped,
}

fn detail(message: Option<&str>) -> String {
    message.map_or_default(|m| format!(": {m}"))
}

impl Error {
    /// The HTTP status, for an answer the server refused.
    #[must_use]
    pub fn status(&self) -> Option<u16> {
        match self {
            Self::Status { status, .. } => Some(*status),
            Self::Refused(refusal) => Some(refusal.status),
            _ => None,
        }
    }

    /// Whether trying again later may help: the server was out of reach or failed on its side.
    #[must_use]
    pub fn is_transient(&self) -> bool {
        match self {
            Self::Network(_) => true,
            Self::Status { status, .. } => *status >= 500,
            _ => false,
        }
    }
}

/// A request that didn't get through.
impl From<TransportError> for Error {
    fn from(error: TransportError) -> Self {
        Self::Network(NetworkError(error))
    }
}

impl From<StoreError> for Error {
    fn from(error: StoreError) -> Self {
        Self::Store(error)
    }
}

/// A request that didn't get through: the transport's own error, kept opaque so the HTTP
/// library can change without changing this API.
#[derive(Debug)]
pub struct NetworkError(TransportError);

impl NetworkError {
    /// Whether it timed out.
    #[must_use]
    pub fn is_timeout(&self) -> bool {
        self.0.is_timeout()
    }
}

impl fmt::Display for NetworkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

impl std::error::Error for NetworkError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.0.source()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn statuses_say_the_servers_words() {
        let error = Error::Status {
            status: 400,
            message: Some("no task T-9".to_owned()),
        };
        assert_eq!(error.to_string(), "the server answered 400: no task T-9");
        assert_eq!(error.status(), Some(400));
        assert!(!error.is_transient());
        let bare = Error::Status {
            status: 503,
            message: None,
        };
        assert_eq!(bare.to_string(), "the server answered 503");
        assert!(bare.is_transient());
    }

    #[test]
    fn refusals_keep_their_status() {
        let error = Error::Refused(AuthRefusal {
            status: 429,
            error: "Too many".to_owned(),
        });
        assert_eq!(error.status(), Some(429));
        assert_eq!(Error::SignInRequired.status(), None);
    }

    #[test]
    fn protocol_errors_are_not_transient() {
        let error = Error::Protocol(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "not UTF-8",
        ));
        assert!(!error.is_transient());
        assert_eq!(error.status(), None);
    }

    #[test]
    fn upload_errors_are_not_transient() {
        let large = Error::TooLarge {
            size: 600,
            limit: 500,
        };
        assert_eq!(
            large.to_string(),
            "the file is 600 bytes, over the limit of 500"
        );
        assert!(!large.is_transient());
        assert!(!Error::Read(std::io::Error::other("gone")).is_transient());
        assert!(!Error::Write(std::io::Error::other("full")).is_transient());
        assert!(!Error::Cancelled.is_transient());
        assert!(!Error::Stopped.is_transient());
    }

    #[test]
    fn network_errors_are_transient() {
        let error = Error::from(TransportError::new("connection failed"));
        assert!(error.is_transient());
        let Error::Network(inner) = error else {
            panic!("a network error")
        };
        assert!(!inner.is_timeout());
        assert_ne!(inner.to_string(), "");
    }
}
