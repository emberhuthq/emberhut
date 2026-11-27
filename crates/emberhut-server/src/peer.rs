//! How a request reached the server.
//!
//! [`run`](crate::run), which serves the API on a TCP listener, marks every request it accepts
//! with [`DirectTcp`]: the peer in its connection info is the TCP socket's own, and the mark
//! holds the local address that accepted the connection (what the probe signs). Any other way
//! of serving (a relayed or sealed listener) doesn't set the mark, so a route that must know
//! its peer is on this computer (`POST /api/connect/link-statement`) can't be reached through
//! one, whatever connection info it carries.

use std::net::SocketAddr;

use axum::extract::ConnectInfo;
use axum::extract::connect_info::Connected;
use axum::http::Request;
use axum::serve::IncomingStream;
use tokio::net::TcpListener;

/// The request came over a TCP connection the server accepted itself: its
/// `ConnectInfo<SocketAddr>` is that socket's peer. Set only by [`run`](crate::run); tests set
/// it to stand for such a connection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DirectTcp {
    /// The local socket address that accepted the connection: the listener's own, or, on one
    /// listening on every interface, the address the peer connected to. None only when the
    /// system wouldn't name it.
    pub local: Option<SocketAddr>,
}

/// A TCP connection as the server's listener accepted it: the connection info
/// [`run`](crate::run) serves with, which [`mark_direct`] turns into each request's peer and
/// [`DirectTcp`] mark.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Accepted {
    peer: SocketAddr,
    local: Option<SocketAddr>,
}

impl Connected<IncomingStream<'_, TcpListener>> for Accepted {
    fn connect_info(stream: IncomingStream<'_, TcpListener>) -> Self {
        Self {
            peer: *stream.remote_addr(),
            local: stream.io().local_addr().ok(),
        }
    }
}

/// Gives a request on a connection the listener accepted its peer, as the
/// `ConnectInfo<SocketAddr>` the routes read, and the [`DirectTcp`] mark with the connection's
/// local address. A request without [`Accepted`] connection info is left as it is.
pub(crate) async fn mark_direct<B>(mut request: Request<B>) -> Request<B> {
    let accepted = request
        .extensions()
        .get::<ConnectInfo<Accepted>>()
        .map(|ConnectInfo(accepted)| *accepted);
    if let Some(Accepted { peer, local }) = accepted {
        let extensions = request.extensions_mut();
        extensions.insert(ConnectInfo(peer));
        extensions.insert(DirectTcp { local });
    }
    request
}
