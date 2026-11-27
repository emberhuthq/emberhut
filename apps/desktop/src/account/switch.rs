//! The build-time switch: the account exists only in a build that sets all four of
//! `EMBERHUT_ACCOUNT_ISSUER`, `EMBERHUT_ACCOUNT_CLIENT_ID`, `EMBERHUT_DIRECTORY_URL` and
//! `EMBERHUT_RELAY_DOMAIN` (read here with `option_env!`, nowhere else). A build from source sets
//! none of them, so it has no account: no Account group in Settings, no request to the
//! account's provider or the directory, and no entity that could make one.
//!
//! `EMBERHUT_ASSOCIATED_DOMAIN` is set only by a build that is signed with the associated-domain
//! entitlement for `emberhut.com` (the release bundle, with its provisioning profile). Only that
//! build can receive the HTTPS callback, so only that build signs in through the system's web
//! authentication session; every other build uses the device grant.

use emberhut_identity::RelayDomain;

/// The account's provider: its issuer (OIDC discovery is at its
/// `/.well-known/openid-configuration`).
const ISSUER: Option<&str> = option_env!("EMBERHUT_ACCOUNT_ISSUER");
/// The native application's public OAuth client id (no secret).
const CLIENT_ID: Option<&str> = option_env!("EMBERHUT_ACCOUNT_CLIENT_ID");
/// Where the directory is: its origin, before `/v1`.
const DIRECTORY_URL: Option<&str> = option_env!("EMBERHUT_DIRECTORY_URL");
/// The relay's domain, which a link statement names.
const RELAY_DOMAIN: Option<&str> = option_env!("EMBERHUT_RELAY_DOMAIN");
/// The domain the build's associated-domain entitlement claims, when it has one.
const ASSOCIATED_DOMAIN: Option<&str> = option_env!("EMBERHUT_ASSOCIATED_DOMAIN");

/// The host of the one redirect the OAuth application has, and so the associated domain a
/// build must claim to receive it.
pub(crate) const CALLBACK_HOST: &str = "emberhut.com";

/// The account's settings, as the build gave them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AccountSettings {
    /// The issuer, without a trailing `/`.
    pub(crate) issuer: String,
    /// The native application's OAuth client id.
    pub(crate) client_id: String,
    /// The directory's origin, without a trailing `/`.
    pub(crate) directory: String,
    /// The relay's domain.
    pub(crate) relay_domain: RelayDomain,
    /// Whether this build may sign in through the web authentication session: it claims the
    /// callback's associated domain.
    pub(crate) web_session: bool,
}

/// The build's values, each as `option_env!` gave it.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct BuildValues<'a> {
    /// `EMBERHUT_ACCOUNT_ISSUER`.
    pub(crate) issuer: Option<&'a str>,
    /// `EMBERHUT_ACCOUNT_CLIENT_ID`.
    pub(crate) client_id: Option<&'a str>,
    /// `EMBERHUT_DIRECTORY_URL`.
    pub(crate) directory: Option<&'a str>,
    /// `EMBERHUT_RELAY_DOMAIN`.
    pub(crate) relay_domain: Option<&'a str>,
    /// `EMBERHUT_ASSOCIATED_DOMAIN`.
    pub(crate) associated_domain: Option<&'a str>,
}

impl AccountSettings {
    /// This build's account, or `None` when the build doesn't turn it on.
    #[must_use]
    pub(crate) fn built_in() -> Option<Self> {
        Self::from_build(BuildValues {
            issuer: ISSUER,
            client_id: CLIENT_ID,
            directory: DIRECTORY_URL,
            relay_domain: RELAY_DOMAIN,
            associated_domain: ASSOCIATED_DOMAIN,
        })
    }

    /// The account `values` turn on: all four set and well formed. None set is a build without
    /// the account; some set, or one that doesn't read, is a mistake in the build, said in the
    /// log, and the account stays off.
    #[must_use]
    pub(crate) fn from_build(values: BuildValues<'_>) -> Option<Self> {
        let given = [
            values.issuer,
            values.client_id,
            values.directory,
            values.relay_domain,
        ]
        .map(|value| value.map(str::trim).filter(|value| !value.is_empty()));
        let [
            Some(issuer),
            Some(client_id),
            Some(directory),
            Some(relay_domain),
        ] = given
        else {
            if given.iter().any(Option::is_some) {
                log::warn!(
                    "the account is off: this build sets only some of EMBERHUT_ACCOUNT_ISSUER, \
                     EMBERHUT_ACCOUNT_CLIENT_ID, EMBERHUT_DIRECTORY_URL and EMBERHUT_RELAY_DOMAIN"
                );
            }
            return None;
        };
        let checked = web_origin(issuer)
            .zip(web_origin(directory))
            .zip(RelayDomain::normalize(relay_domain).ok())
            .map(|((issuer, directory), relay_domain)| Self {
                issuer,
                client_id: client_id.to_owned(),
                directory,
                relay_domain,
                web_session: values.associated_domain.map(str::trim) == Some(CALLBACK_HOST),
            });
        if checked.is_none() {
            log::warn!(
                "the account is off: this build's issuer or directory isn't an https address, \
                 or its relay domain isn't a domain"
            );
        }
        checked
    }
}

/// `address` without its trailing `/`, when it is `https://…`, or `http://` to this computer
/// (a provider or directory run on a developer's machine).
fn web_origin(address: &str) -> Option<String> {
    let address = address.trim_end_matches('/');
    let rest = address.strip_prefix("https://").or_else(|| {
        address
            .strip_prefix("http://")
            .filter(|rest| is_loopback_host(rest))
    })?;
    (!rest.is_empty() && !rest.contains(char::is_whitespace)).then(|| address.to_owned())
}

/// Whether `authority` (with whatever path follows it) names this computer: `localhost`,
/// `127.0.0.1` or `[::1]`, with or without a port.
#[must_use]
pub(crate) fn is_loopback_host(authority: &str) -> bool {
    let authority = authority.split(['/', '?', '#']).next().unwrap_or_default();
    let host = match authority.strip_prefix('[') {
        Some(v6) => v6.split(']').next().unwrap_or_default(),
        None => authority.split(':').next().unwrap_or_default(),
    };
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all() -> BuildValues<'static> {
        BuildValues {
            issuer: Some("https://accounts.example/"),
            client_id: Some("client-example"),
            directory: Some("https://relay.example"),
            relay_domain: Some("Relay.Example"),
            associated_domain: None,
        }
    }

    #[test]
    fn a_build_that_sets_nothing_has_no_account() {
        assert_eq!(AccountSettings::from_build(BuildValues::default()), None);
    }

    #[test]
    fn a_source_build_has_no_account() {
        // The tests are built as a source build is: with none of the settings.
        if ISSUER.is_none()
            && CLIENT_ID.is_none()
            && DIRECTORY_URL.is_none()
            && RELAY_DOMAIN.is_none()
        {
            assert_eq!(AccountSettings::built_in(), None);
        }
    }

    #[test]
    fn all_four_turn_the_account_on() {
        let settings = AccountSettings::from_build(all()).expect("an account");
        assert_eq!(settings.issuer, "https://accounts.example");
        assert_eq!(settings.client_id, "client-example");
        assert_eq!(settings.directory, "https://relay.example");
        assert_eq!(settings.relay_domain.as_str(), "relay.example");
        assert!(
            !settings.web_session,
            "the device grant without the entitlement"
        );
    }

    #[test]
    fn any_one_missing_keeps_the_account_off() {
        let without = [
            BuildValues {
                issuer: None,
                ..all()
            },
            BuildValues {
                client_id: Some("  "),
                ..all()
            },
            BuildValues {
                directory: None,
                ..all()
            },
            BuildValues {
                relay_domain: None,
                ..all()
            },
        ];
        for values in without {
            assert_eq!(AccountSettings::from_build(values), None, "{values:?}");
        }
    }

    #[test]
    fn plain_http_is_only_for_this_computer() {
        let local = BuildValues {
            directory: Some("http://127.0.0.1:8443"),
            ..all()
        };
        assert!(AccountSettings::from_build(local).is_some());
        let remote = BuildValues {
            issuer: Some("http://accounts.example"),
            ..all()
        };
        assert_eq!(AccountSettings::from_build(remote), None);
        let not_a_domain = BuildValues {
            relay_domain: Some("127.0.0.1"),
            ..all()
        };
        assert_eq!(AccountSettings::from_build(not_a_domain), None);
    }

    #[test]
    fn only_the_callbacks_associated_domain_turns_the_web_session_on() {
        let claimed = BuildValues {
            associated_domain: Some("emberhut.com"),
            ..all()
        };
        assert!(AccountSettings::from_build(claimed).unwrap().web_session);
        let other = BuildValues {
            associated_domain: Some("example.com"),
            ..all()
        };
        assert!(!AccountSettings::from_build(other).unwrap().web_session);
    }

    #[test]
    fn loopback_hosts_are_this_computer() {
        for host in [
            "localhost:8791",
            "127.0.0.1",
            "127.0.0.1:8791/x",
            "[::1]:8791",
            "LOCALHOST",
        ] {
            assert!(is_loopback_host(host), "{host}");
        }
        for host in ["example.com", "192.0.2.2:8791", "localhost.example", ""] {
            assert!(!is_loopback_host(host), "{host}");
        }
    }
}
