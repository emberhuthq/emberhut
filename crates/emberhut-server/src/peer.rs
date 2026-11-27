//! How a request reached the server.
//!
//! [`run`](crate::run), which serves the API on a TCP listener, marks every request it accepts
//! with [`DirectTcp`]: the peer in its connection info is the TCP socket's own. Any other way of
//! serving (a relayed or sealed listener) doesn't set the mark, so a route that must know its
//! peer is on this computer (`POST /api/connect/link-statement`) can't be reached through one,
//! whatever connection info it carries.

/// The request came over a TCP connection the server accepted itself: its
/// `ConnectInfo<SocketAddr>` is that socket's peer. Set only by [`run`](crate::run); tests set
/// it to stand for such a connection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DirectTcp;
