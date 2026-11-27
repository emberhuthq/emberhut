//! The routes the configuration gives, as a pairing link names them (ADR 0007, section 2;
//! connect.md, "What the machine advertises"), best first.
//!
//! This is the part a pairing link needs: the listen address, when it is one other devices
//! can reach, then the public URL. Listing every interface's address when the server listens
//! on all of them is the full advertisement's work (`GET /api/connect`), which this doesn't do.

use std::net::{IpAddr, SocketAddr};

use crate::config::PublicUrl;

/// The routes to this machine that `listen` and `public_url` give, best first, as base URLs.
///
/// A loopback address reaches only this computer and an unspecified one (`0.0.0.0`, `[::]`)
/// names no address, so neither is a route; nor is a link-local address, which needs a zone a
/// URL can't carry. The same goes for a public URL whose host is one of them (or `localhost`):
/// it is still used for the server's own links on this computer, but a pairing link leaves it
/// out.
#[must_use]
pub fn configured_routes(listen: SocketAddr, public_url: Option<&PublicUrl>) -> Vec<String> {
    let direct = reachable(listen.ip()).then(|| {
        let address = SocketAddr::new(listen.ip().to_canonical(), listen.port());
        format!("http://{address}")
    });
    let public = public_url
        .filter(|url| reachable_host(url.as_str()))
        .map(|url| url.as_str().to_owned());
    direct.into_iter().chain(public).collect()
}

/// Whether another device can reach this computer at `ip`.
fn reachable(ip: IpAddr) -> bool {
    match ip.to_canonical() {
        IpAddr::V4(ip) => !(ip.is_loopback() || ip.is_unspecified() || ip.is_link_local()),
        IpAddr::V6(ip) => !(ip.is_loopback() || ip.is_unspecified() || ip.is_unicast_link_local()),
    }
}

/// Whether another device can reach the host of `url`: not an address [`reachable`] refuses,
/// and not `localhost` or a name under it.
fn reachable_host(url: &str) -> bool {
    match url::Url::parse(url)
        .ok()
        .and_then(|url| url.host().map(|host| host.to_owned()))
    {
        Some(url::Host::Ipv4(ip)) => reachable(ip.into()),
        Some(url::Host::Ipv6(ip)) => reachable(ip.into()),
        Some(url::Host::Domain(name)) => {
            let name = name.trim_end_matches('.').to_ascii_lowercase();
            name != "localhost" && !name.ends_with(".localhost")
        }
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn routes(listen: &str, public: Option<&str>) -> Vec<String> {
        let public = public.map(|url| url.parse::<PublicUrl>().unwrap());
        configured_routes(listen.parse().unwrap(), public.as_ref())
    }

    #[test]
    fn a_reachable_listen_address_comes_before_the_public_url() {
        assert_eq!(
            routes("192.0.2.10:18990", Some("https://emberhut.example.com/")),
            ["http://192.0.2.10:18990", "https://emberhut.example.com"]
        );
        assert_eq!(
            routes("[2001:db8::10]:18990", None),
            ["http://[2001:db8::10]:18990"]
        );
        assert_eq!(
            routes("[::ffff:192.0.2.10]:18990", None),
            ["http://192.0.2.10:18990"]
        );
    }

    #[test]
    fn loopback_unspecified_and_link_local_addresses_are_no_route() {
        for listen in [
            "127.0.0.1:18990",
            "0.0.0.0:18990",
            "[::]:18990",
            "[::1]:18990",
            "169.254.1.1:18990",
            "[fe80::1]:18990",
        ] {
            assert_eq!(routes(listen, None), Vec::<String>::new(), "{listen}");
        }
        assert_eq!(
            routes("127.0.0.1:18990", Some("https://emberhut.example.com")),
            ["https://emberhut.example.com"]
        );
    }

    #[test]
    fn a_public_url_only_this_computer_reaches_is_no_route() {
        for public in [
            "http://127.0.0.1:18990",
            "http://localhost:18990",
            "https://app.localhost",
            "http://[::1]:18990",
            "http://0.0.0.0:18990",
            "http://169.254.1.1",
            "http://[fe80::1]:18990",
        ] {
            assert_eq!(
                routes("192.0.2.10:18990", Some(public)),
                ["http://192.0.2.10:18990"],
                "{public}"
            );
        }
        assert_eq!(
            routes("127.0.0.1:18990", Some("http://192.0.2.10:8080")),
            ["http://192.0.2.10:8080"]
        );
    }
}
