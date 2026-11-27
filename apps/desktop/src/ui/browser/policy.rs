//! Where the live view's web view may go: the stream's own address, nowhere else.

/// The origin of an `http` or `https` address: its scheme, host and port, lowercased, without
/// a user or password, and without the scheme's own port (`:443` for `https`, `:80` for
/// `http`), as the web view writes the addresses it goes to.
#[must_use]
pub(crate) fn origin(url: &str) -> Option<String> {
    let (scheme, rest) = url.split_once("://")?;
    let scheme = scheme.to_ascii_lowercase();
    let default_port = match scheme.as_str() {
        "https" => ":443",
        "http" => ":80",
        _ => return None,
    };
    let authority = rest.split(['/', '?', '#']).next()?;
    let host = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host)
        .to_ascii_lowercase();
    // A bracketed IPv6 host without a port ends in `]`, so only a port can be cut here.
    let host = host.strip_suffix(default_port).unwrap_or(&host);
    (!host.is_empty()).then(|| format!("{scheme}://{host}"))
}

/// Whether a navigation to `url` stays on the stream at `home` (an origin): its own pages, and
/// the blank frames a page makes.
#[must_use]
pub(crate) fn stays_on(home: &str, url: &str) -> bool {
    url.starts_with("about:") || origin(url).as_deref() == Some(home)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_origin_drops_the_sign_in_and_the_path() {
        assert_eq!(
            origin("https://viewer:p%40ss@example.org:8447/a?b#c").as_deref(),
            Some("https://example.org:8447")
        );
        assert_eq!(
            origin("http://example.org").as_deref(),
            Some("http://example.org")
        );
        assert_eq!(origin("file:///etc/passwd"), None);
        assert_eq!(
            origin("https://Example.ORG:443/").as_deref(),
            Some("https://example.org"),
            "the scheme's own port is the address without one"
        );
        assert_eq!(
            origin("http://example.org:80").as_deref(),
            Some("http://example.org")
        );
        assert_eq!(
            origin("http://example.org:443").as_deref(),
            Some("http://example.org:443"),
            "another scheme's port stays"
        );
        assert_eq!(
            origin("https://[::1]:443/").as_deref(),
            Some("https://[::1]")
        );
        assert_eq!(origin("https://[::1]").as_deref(), Some("https://[::1]"));
        assert_eq!(origin("https://"), None);
    }

    #[test]
    fn only_the_streams_own_pages_load() {
        let home = origin("https://example.org:8447/").unwrap();
        assert!(stays_on(&home, "https://viewer:x@example.org:8447/"));
        assert!(stays_on(&home, "https://example.org:8447/settings"));
        assert!(stays_on(&home, "about:blank"));
        assert!(!stays_on(&home, "https://example.org/"), "another port");
        let plain = origin("https://example.org:443/").unwrap();
        assert!(stays_on(&plain, "https://example.org/"), "its first load");
        assert!(!stays_on(&home, "https://example.com/"));
        assert!(!stays_on(&home, "javascript:alert(1)"));
    }
}
