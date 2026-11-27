//! Whether a verified probe answer is bound to the route the client probed (`docs/api.md`,
//! `GET /api/connect/probe`).

use std::net::SocketAddr;

use emberhut_identity::{Host, LocalAddr, ProbeStatement, Scheme, Via};

use super::probe::SshForward;
use super::routes::{Route, RouteKind};

/// Whether `statement`, verified, binds the answer to `route`.
pub(super) fn matches(statement: &ProbeStatement, route: &Route) -> bool {
    if route.kind.as_str() != statement.kind().as_str() {
        return false;
    }
    let Some((scheme, authority)) = split_url(&route.url) else {
        return false;
    };
    let Ok(host) = Host::normalize(authority, scheme) else {
        return false;
    };
    match route.kind {
        RouteKind::Loopback | RouteKind::Lan => address_matches(statement, &host, scheme),
        RouteKind::Vpn if host.address().is_some() => address_matches(statement, &host, scheme),
        RouteKind::Vpn | RouteKind::Public => name_matches(statement, &host, scheme, Via::Direct),
        RouteKind::Relay => name_matches(statement, &host, scheme, Via::Relayed),
        _ => false,
    }
}

/// Whether `statement`, verified, binds the answer to the `ssh` route whose forward is
/// `forward`.
pub(super) fn forward_matches(statement: &ProbeStatement, forward: SshForward) -> bool {
    if statement.kind().as_str() != RouteKind::Ssh.as_str() {
        return false;
    }
    let on_loopback = statement.reached().local().is_some_and(|local| {
        local.get().ip().is_loopback() && local.get().port() == forward.machine_port
    });
    let host = Host::normalize(&format!("127.0.0.1:{}", forward.local_port), Scheme::Http);
    on_loopback && host.is_ok_and(|host| statement.host() == &host)
}

/// An address route: reached directly, on the route's address and port.
fn address_matches(statement: &ProbeStatement, host: &Host, scheme: Scheme) -> bool {
    let Some(address) = host.address() else {
        return false;
    };
    let port = host.port().unwrap_or(scheme.default_port());
    statement.reached().local() == Some(LocalAddr::new(SocketAddr::new(address, port)))
}

/// A name route: over `https`, reached as `via`, for the route's host.
fn name_matches(statement: &ProbeStatement, host: &Host, scheme: Scheme, via: Via) -> bool {
    scheme == Scheme::Https && statement.reached().via() == via && statement.host() == host
}

/// A URL's scheme and authority: `http` or `https`, and what comes before the path, query or
/// fragment. A URL with user information doesn't read.
fn split_url(url: &str) -> Option<(Scheme, &str)> {
    let (scheme, rest) = url.split_once("://")?;
    let scheme = if scheme.eq_ignore_ascii_case("http") {
        Scheme::Http
    } else if scheme.eq_ignore_ascii_case("https") {
        Scheme::Https
    } else {
        return None;
    };
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = &rest[..end];
    (!authority.contains('@')).then_some((scheme, authority))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_split_into_scheme_and_authority() {
        assert_eq!(
            split_url("HTTPS://example.com:8443/a?b#c"),
            Some((Scheme::Https, "example.com:8443"))
        );
        assert_eq!(
            split_url("http://[::1]:18990"),
            Some((Scheme::Http, "[::1]:18990"))
        );
        assert_eq!(split_url("ssh://example.com"), None);
        assert_eq!(split_url("https://user@example.com"), None);
        assert_eq!(split_url("example.com"), None);
    }
}
