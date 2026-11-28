//! The runner's HTTP client for its server: rustls with ring and the Mozilla roots, redirects
//! not followed, no proxy, HTTP/1.1. Plain `http://` is taken too: a server on loopback or a
//! private network.

use std::sync::Arc;

/// The client could not be made.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ClientError {
    /// TLS could not be set up.
    #[error("TLS could not be set up: {0}")]
    Tls(#[from] rustls::Error),
    /// reqwest refused the configuration.
    #[error("the HTTP client could not be built: {0}")]
    Build(#[from] reqwest::Error),
}

/// A pooled client for the server.
///
/// # Errors
/// TLS or the client cannot be set up.
pub fn client() -> Result<reqwest::Client, ClientError> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let roots = rustls::RootCertStore {
        roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
    };
    let mut tls = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()?
        .with_root_certificates(roots)
        .with_no_client_auth();
    tls.alpn_protocols = vec![b"http/1.1".to_vec()];
    Ok(reqwest::Client::builder()
        .tls_backend_preconfigured(tls)
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .build()?)
}
