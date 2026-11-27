//! How to reach the server.

use std::fmt;
use std::time::Duration;

use http::Uri;

use crate::transport::Timeouts;

/// Where the server is and how long to wait for it. There is no default address: the caller
/// says which server to talk to.
///
/// ```
/// use std::time::Duration;
/// use emberhut_client::ClientConfig;
///
/// let config = ClientConfig::new("https://emberhut.example.com/")?
///     .with_device("Emberhut on a laptop", "linux")
///     .with_request_timeout(Duration::from_secs(30));
/// assert_eq!(config.base_url(), "https://emberhut.example.com");
/// # Ok::<(), emberhut_client::InvalidBaseUrl>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientConfig {
    base_url: String,
    connect_timeout: Duration,
    request_timeout: Duration,
    live_read_timeout: Duration,
    upload_timeout: Duration,
    download_limit: u64,
    device_name: String,
    device_model: String,
    built_dialect: bool,
}

impl ClientConfig {
    /// The default wait for a connection.
    pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(8);
    /// The default wait for a whole answer (the state is 1 to 2 MB).
    pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
    /// The default wait for the next line of the live stream: past two of the server's 15 s
    /// pings, so a quiet stream stays open and a dead one is noticed.
    pub const LIVE_READ_TIMEOUT: Duration = Duration::from_secs(40);
    /// The default wait, during an upload or a download, for the next bytes to move or the answer
    /// to come: 600 s, as the reference clients wait. A transfer as a whole may take as long as it
    /// needs.
    pub const UPLOAD_TIMEOUT: Duration = Duration::from_secs(600);

    /// A server at `base_url` (`http://` or `https://`, a path prefix allowed; a trailing `/` is
    /// dropped), with the default timeouts and device name.
    ///
    /// # Errors
    ///
    /// [`InvalidBaseUrl`] when it isn't such an address, or carries credentials
    /// (`user:password@`), which don't belong in it.
    pub fn new(base_url: &str) -> Result<Self, InvalidBaseUrl> {
        Ok(Self {
            base_url: checked_base_url(base_url)?,
            connect_timeout: Self::CONNECT_TIMEOUT,
            request_timeout: Self::REQUEST_TIMEOUT,
            live_read_timeout: Self::LIVE_READ_TIMEOUT,
            upload_timeout: Self::UPLOAD_TIMEOUT,
            download_limit: crate::MAX_DOWNLOAD,
            device_name: "Emberhut".to_owned(),
            device_model: std::env::consts::OS.to_owned(),
            built_dialect: true,
        })
    }

    /// Whether the server speaks the dialect this crate was built with (see
    /// [`crate::dialect`]): on by default, so every request and answer is rewritten as the
    /// build says; off for a server that speaks the contract's own words. A build without a
    /// dialect rewrites nothing either way.
    #[must_use]
    pub fn with_built_dialect(mut self, speaks_it: bool) -> Self {
        self.built_dialect = speaks_it;
        self
    }

    /// The name and kind this device gives when it pairs (the device list shows them). The
    /// server keeps 60 and 40 UTF-16 units of them. By default `Emberhut` and the OS (`macos`,
    /// `linux`, `windows`).
    #[must_use]
    pub fn with_device(mut self, name: impl Into<String>, model: impl Into<String>) -> Self {
        self.device_name = name.into();
        self.device_model = model.into();
        self
    }

    /// The wait for a connection.
    #[must_use]
    pub fn with_connect_timeout(mut self, timeout: Duration) -> Self {
        self.connect_timeout = timeout;
        self
    }

    /// The wait for a whole answer.
    #[must_use]
    pub fn with_request_timeout(mut self, timeout: Duration) -> Self {
        self.request_timeout = timeout;
        self
    }

    /// The wait for the next line of the live stream.
    #[must_use]
    pub fn with_live_read_timeout(mut self, timeout: Duration) -> Self {
        self.live_read_timeout = timeout;
        self
    }

    /// The wait, during an upload or a download, for the next bytes to move or the answer to
    /// come.
    #[must_use]
    pub fn with_upload_timeout(mut self, timeout: Duration) -> Self {
        self.upload_timeout = timeout;
        self
    }

    /// The largest file [`crate::Client::download`] takes, at most [`crate::MAX_DOWNLOAD`]
    /// (the default; a larger value is held to it).
    #[must_use]
    pub fn with_download_limit(mut self, limit: u64) -> Self {
        self.download_limit = limit.min(crate::MAX_DOWNLOAD);
        self
    }

    /// The server's address, without a trailing `/`.
    #[must_use]
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// The wait for a connection.
    #[must_use]
    pub fn connect_timeout(&self) -> Duration {
        self.connect_timeout
    }

    /// The wait for a whole answer.
    #[must_use]
    pub fn request_timeout(&self) -> Duration {
        self.request_timeout
    }

    /// The wait for the next line of the live stream.
    #[must_use]
    pub fn live_read_timeout(&self) -> Duration {
        self.live_read_timeout
    }

    /// The wait, during an upload or a download, for the next bytes to move or the answer to
    /// come.
    #[must_use]
    pub fn upload_timeout(&self) -> Duration {
        self.upload_timeout
    }

    /// The largest file [`crate::Client::download`] takes.
    #[must_use]
    pub fn download_limit(&self) -> u64 {
        self.download_limit
    }

    /// The name this device pairs with.
    #[must_use]
    pub fn device_name(&self) -> &str {
        &self.device_name
    }

    /// The kind of device it pairs as.
    #[must_use]
    pub fn device_model(&self) -> &str {
        &self.device_model
    }

    /// The dialect the server is spoken to in: the built one, unless
    /// [`ClientConfig::with_built_dialect`] said the server doesn't speak it.
    #[must_use]
    pub fn dialect(&self) -> crate::dialect::Dialect {
        crate::dialect::Dialect::for_server(self.built_dialect)
    }

    /// The waits of a request: a connection, then the whole answer.
    pub(crate) fn request_timeouts(&self) -> Timeouts {
        Timeouts::default()
            .with_connect(self.connect_timeout)
            .with_total(self.request_timeout)
    }

    /// The waits of the live stream: a connection, then each read, with no overall limit.
    pub(crate) fn live_timeouts(&self) -> Timeouts {
        Timeouts::default()
            .with_connect(self.connect_timeout)
            .with_read(self.live_read_timeout)
    }

    /// The waits of an upload or a download: a connection, then each write and each read,
    /// with no overall limit.
    pub(crate) fn transfer_timeouts(&self) -> Timeouts {
        Timeouts::default()
            .with_connect(self.connect_timeout)
            .with_read(self.upload_timeout)
            .with_write(self.upload_timeout)
    }
}

/// `base_url` without its trailing `/`, when it is a server address: `http://` or `https://`,
/// with a host, a path prefix allowed, and no credentials or query.
pub(crate) fn checked_base_url(base_url: &str) -> Result<String, InvalidBaseUrl> {
    let invalid = |reason| InvalidBaseUrl::new(base_url, reason);
    let uri: Uri = base_url.parse().map_err(|_| invalid("not a URL"))?;
    if uri.authority().is_some_and(|a| a.as_str().contains('@')) {
        return Err(InvalidBaseUrl::credentials(&uri));
    }
    if !matches!(uri.scheme_str(), Some("http" | "https")) {
        return Err(invalid("not http or https"));
    }
    if uri.host().is_none_or(str::is_empty) {
        return Err(invalid("no host"));
    }
    if uri.query().is_some() {
        return Err(invalid("a query has no place in a base URL"));
    }
    Ok(base_url.trim_end_matches('/').to_owned())
}

/// A base URL that [`ClientConfig::new`] refuses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidBaseUrl {
    url: String,
    reason: &'static str,
}

impl InvalidBaseUrl {
    fn new(url: &str, reason: &'static str) -> Self {
        Self {
            url: url.to_owned(),
            reason,
        }
    }

    /// For a URL with credentials, which are kept out of the error.
    fn credentials(uri: &Uri) -> Self {
        let host = uri.host().unwrap_or_default();
        let scheme = uri.scheme_str().unwrap_or("http");
        Self {
            url: format!("{scheme}://…@{host}"),
            reason: "credentials don't belong in the base URL",
        }
    }

    /// The URL given (credentials left out).
    #[must_use]
    pub fn url(&self) -> &str {
        &self.url
    }

    /// What is wrong with it.
    #[must_use]
    pub fn reason(&self) -> &str {
        self.reason
    }
}

impl fmt::Display for InvalidBaseUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "not a server address: {:?} ({})", self.url, self.reason)
    }
}

impl std::error::Error for InvalidBaseUrl {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn takes_http_and_https_addresses() {
        assert_eq!(
            ClientConfig::new("http://127.0.0.1:8080")
                .unwrap()
                .base_url(),
            "http://127.0.0.1:8080"
        );
        assert_eq!(
            ClientConfig::new("https://emberhut.example.com/")
                .unwrap()
                .base_url(),
            "https://emberhut.example.com"
        );
        assert_eq!(
            ClientConfig::new("https://example.com/emberhut/")
                .unwrap()
                .base_url(),
            "https://example.com/emberhut"
        );
    }

    #[test]
    fn refuses_anything_else() {
        for bad in [
            "",
            "emberhut.example.com",
            "ftp://example.com",
            "http://",
            "https://example.com/?x=1",
            "not a url",
        ] {
            let error = ClientConfig::new(bad).unwrap_err();
            assert_eq!(error.url(), bad);
            assert!(
                error.to_string().starts_with("not a server address"),
                "{bad}: {error}"
            );
        }
    }

    #[test]
    fn refuses_credentials_and_keeps_them_out_of_the_error() {
        let error = ClientConfig::new("https://user:secret@example.com").unwrap_err();
        assert_eq!(error.reason(), "credentials don't belong in the base URL");
        assert!(!error.to_string().contains("secret"), "{error}");
        assert!(!format!("{error:?}").contains("user"), "{error:?}");
        let into: crate::Error = error.into();
        assert!(matches!(into, crate::Error::InvalidBaseUrl(_)));
    }

    #[test]
    fn options() {
        let config = ClientConfig::new("http://localhost")
            .unwrap()
            .with_device("Laptop", "linux")
            .with_connect_timeout(Duration::from_secs(1))
            .with_request_timeout(Duration::from_secs(2))
            .with_live_read_timeout(Duration::from_secs(3))
            .with_upload_timeout(Duration::from_secs(4))
            .with_download_limit(5);
        assert_eq!(
            (config.device_name(), config.device_model()),
            ("Laptop", "linux")
        );
        assert_eq!(config.connect_timeout(), Duration::from_secs(1));
        assert_eq!(config.request_timeout(), Duration::from_secs(2));
        assert_eq!(config.live_read_timeout(), Duration::from_secs(3));
        assert_eq!(config.upload_timeout(), Duration::from_secs(4));
        assert_eq!(config.download_limit(), 5);
        let defaults = ClientConfig::new("http://localhost").unwrap();
        assert_eq!(defaults.device_name(), "Emberhut");
        assert_eq!(defaults.device_model(), std::env::consts::OS);
        assert_eq!(defaults.download_limit(), crate::MAX_DOWNLOAD);
        let held = defaults.with_download_limit(u64::MAX).download_limit();
        assert_eq!(held, crate::MAX_DOWNLOAD);
    }

    #[test]
    fn each_kind_of_request_has_its_waits() {
        let second = Duration::from_secs;
        let config = ClientConfig::new("http://localhost")
            .unwrap()
            .with_connect_timeout(second(1))
            .with_request_timeout(second(2))
            .with_live_read_timeout(second(3))
            .with_upload_timeout(second(4));
        let request = config.request_timeouts();
        assert_eq!(
            (
                request.connect(),
                request.total(),
                request.read(),
                request.write()
            ),
            (Some(second(1)), Some(second(2)), None, None)
        );
        let live = config.live_timeouts();
        assert_eq!(
            (live.connect(), live.total(), live.read(), live.write()),
            (Some(second(1)), None, Some(second(3)), None)
        );
        let transfer = config.transfer_timeouts();
        assert_eq!(
            (
                transfer.connect(),
                transfer.total(),
                transfer.read(),
                transfer.write()
            ),
            (Some(second(1)), None, Some(second(4)), Some(second(4)))
        );
    }
}
