//! The one HTTP client stack the server reaches other services with (Apple's and Google's push
//! services, the Mac helper, the coding-thread app): reqwest on rustls with **ring** and the
//! Mozilla root store, so a container needs no system certificates and no other TLS library.
//!
//! reqwest is given its TLS configuration ready-made, with ring as its crypto provider, rather
//! than a process-wide default provider: nothing has to be installed before a client is made,
//! in the server or in its tests. Redirects are never followed (a redirect would carry a bearer
//! token elsewhere) and no proxy is read from the environment.

use std::sync::Arc;

use reqwest::{Client, Response};

/// How a client speaks HTTP.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Protocol {
    /// HTTP/2 only, as Apple's push service requires.
    Http2Only,
    /// HTTP/2 or HTTP/1.1, whichever the other end offers.
    Any,
}

/// Why a client could not be made.
#[derive(Debug, thiserror::Error)]
pub(crate) enum ClientError {
    /// TLS could not be set up.
    #[error("TLS could not be set up")]
    Tls(#[source] rustls::Error),
    /// reqwest refused the configuration.
    #[error("the HTTP client could not be built")]
    Build(#[source] reqwest::Error),
}

/// The server's HTTP client could not be made.
#[derive(Debug, thiserror::Error)]
#[error("the server's HTTP client could not be made")]
pub struct HttpSetupError(#[source] ClientError);

/// The client the server's services share for HTTP/2 or HTTP/1.1 (a pool clones cheaply): the
/// one [`serve`](crate::serve) makes and hands to each service it builds.
///
/// # Errors
///
/// [`HttpSetupError`] when TLS or the client cannot be set up.
pub(crate) fn shared_client() -> Result<Client, HttpSetupError> {
    client(Protocol::Any).map_err(HttpSetupError)
}

/// A pooled client speaking `protocol`. Plain `http://` is taken too: the Mac helper and the
/// coding-thread app are on the person's own network, and tests use stand-ins on loopback.
pub(crate) fn client(protocol: Protocol) -> Result<Client, ClientError> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let roots = rustls::RootCertStore {
        roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
    };
    let mut tls = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(ClientError::Tls)?
        .with_root_certificates(roots)
        .with_no_client_auth();
    // A configuration given ready-made is used as it is, so it names the protocols itself.
    tls.alpn_protocols = match protocol {
        Protocol::Http2Only => vec![b"h2".to_vec()],
        Protocol::Any => vec![b"h2".to_vec(), b"http/1.1".to_vec()],
    };
    let builder = Client::builder()
        .tls_backend_preconfigured(tls)
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy();
    let builder = match protocol {
        Protocol::Http2Only => builder.http2_prior_knowledge(),
        Protocol::Any => builder,
    };
    builder.build().map_err(ClientError::Build)
}

/// An answer's body, read up to `limit` bytes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Body {
    /// What was read: all of it, or the first `limit` bytes.
    pub(crate) bytes: Vec<u8>,
    /// The body went on past `limit`; the rest was not read.
    pub(crate) cut: bool,
}

impl Body {
    /// The body as text, any bytes that aren't UTF-8 replaced.
    pub(crate) fn text(&self) -> String {
        String::from_utf8_lossy(&self.bytes).into_owned()
    }
}

/// Reads `response`'s body, stopping after `limit` bytes, so a misbehaving service cannot make
/// the server hold an answer of any size.
pub(crate) async fn read_body(
    mut response: Response,
    limit: usize,
) -> Result<Body, reqwest::Error> {
    let mut body = Body::default();
    while let Some(chunk) = response.chunk().await? {
        let room = limit - body.bytes.len();
        if chunk.len() > room {
            body.bytes.extend_from_slice(&chunk[..room]);
            body.cut = true;
            break;
        }
        body.bytes.extend_from_slice(&chunk);
    }
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_kinds_of_client_are_made_without_a_default_crypto_provider() {
        assert!(client(Protocol::Http2Only).is_ok());
        assert!(client(Protocol::Any).is_ok());
    }
}
