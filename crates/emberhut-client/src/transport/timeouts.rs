//! How long a transport waits.

use std::time::Duration;

/// How long a transport waits for each part of an exchange; none set means no limit.
///
/// ```
/// use std::time::Duration;
/// use emberhut_client::transport::Timeouts;
///
/// let live = Timeouts::default()
///     .with_connect(Duration::from_secs(8))
///     .with_read(Duration::from_secs(40));
/// assert_eq!(live.total(), None);
/// ```
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Timeouts {
    connect: Option<Duration>,
    total: Option<Duration>,
    read: Option<Duration>,
    write: Option<Duration>,
}

impl Timeouts {
    /// The wait for a connection to the machine: the dial and its handshakes (TLS, and for a
    /// wrapping transport its own, such as an encrypted session's), before the request goes out.
    #[must_use]
    pub fn with_connect(mut self, timeout: Duration) -> Self {
        self.connect = Some(timeout);
        self
    }

    /// The wait for the whole exchange: from sending the request to the end of the answer's
    /// body.
    #[must_use]
    pub fn with_total(mut self, timeout: Duration) -> Self {
        self.total = Some(timeout);
        self
    }

    /// The longest wait for the next bytes of the answer, its head or its body.
    #[must_use]
    pub fn with_read(mut self, timeout: Duration) -> Self {
        self.read = Some(timeout);
        self
    }

    /// The longest wait for the next bytes of the request to go out.
    #[must_use]
    pub fn with_write(mut self, timeout: Duration) -> Self {
        self.write = Some(timeout);
        self
    }

    /// The wait for a connection.
    #[must_use]
    pub fn connect(&self) -> Option<Duration> {
        self.connect
    }

    /// The wait for the whole exchange.
    #[must_use]
    pub fn total(&self) -> Option<Duration> {
        self.total
    }

    /// The longest wait for the next bytes of the answer.
    #[must_use]
    pub fn read(&self) -> Option<Duration> {
        self.read
    }

    /// The longest wait for the next bytes of the request.
    #[must_use]
    pub fn write(&self) -> Option<Duration> {
        self.write
    }
}
