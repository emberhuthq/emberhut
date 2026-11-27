//! The pairing link: `https://emberhut.com/pair#id=<machine id>&code=<code>&route=<url>…`.
//!
//! Everything is in the fragment, which browsers never send to a server. The fragment is
//! `application/x-www-form-urlencoded`: each value percent-encoded, so a route's `:`, `/`, `&`
//! and `#` are safe. `id` and `code` appear once each, `route` 0 to 8 times in the order of the
//! machine's route list, and other keys are ignored. The whole link is at most 2,000
//! characters, so its QR code stays scannable.

use std::fmt;

use zeroize::Zeroizing;

use crate::encoding::DecodeError;
use crate::machine_id::MachineId;

/// A pairing link: the machine's id, a one-time code, and routes to try.
///
/// Its `Debug` leaves the code out: until it is used, a code is a password.
#[derive(Clone, PartialEq, Eq)]
pub struct PairingLink {
    id: MachineId,
    code: Zeroizing<String>,
    routes: Vec<String>,
}

/// A key of the pairing link's fragment that must appear exactly once.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PairingLinkKey {
    /// `id`, the machine's id.
    Id,
    /// `code`, the one-time code.
    Code,
}

impl PairingLinkKey {
    /// The key as the fragment spells it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Id => "id",
            Self::Code => "code",
        }
    }
}

impl fmt::Display for PairingLinkKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Why text isn't a pairing link, or values don't make one.
#[derive(Debug, Clone, PartialEq, Eq, Hash, thiserror::Error)]
#[non_exhaustive]
pub enum PairingLinkError {
    /// The text doesn't start with [`PairingLink::BASE`] and `#`.
    #[error("not a pairing link: it starts with {}#", PairingLink::BASE)]
    NotAPairingLink,
    /// Longer than [`PairingLink::MAX_LEN`] characters.
    #[error("the pairing link is longer than {} characters", PairingLink::MAX_LEN)]
    TooLong,
    /// `id` or `code` is missing (or empty).
    #[error("the pairing link has no {0}")]
    Missing(PairingLinkKey),
    /// `id` or `code` appears more than once.
    #[error("the pairing link has {0} more than once")]
    Repeated(PairingLinkKey),
    /// More than [`PairingLink::MAX_ROUTES`] routes.
    #[error("the pairing link has more than {} routes", PairingLink::MAX_ROUTES)]
    TooManyRoutes,
    /// An empty route.
    #[error("the pairing link has an empty route")]
    EmptyRoute,
    /// The `id` isn't a machine id.
    #[error("the pairing link's id isn't a machine id")]
    Id(#[from] DecodeError),
}

impl PairingLink {
    /// Where every pairing link points, before its fragment.
    pub const BASE: &'static str = "https://emberhut.com/pair";
    /// The most routes a link carries.
    pub const MAX_ROUTES: usize = 8;
    /// The longest link, in characters.
    pub const MAX_LEN: usize = 2_000;

    /// A link to pair with machine `id`, using `code`, over `routes` in order.
    ///
    /// # Errors
    ///
    /// [`PairingLinkError`] for an empty code or route, more than 8 routes, or a link over
    /// 2,000 characters.
    pub fn new(
        id: MachineId,
        code: impl Into<String>,
        routes: Vec<String>,
    ) -> Result<Self, PairingLinkError> {
        let link = Self {
            id,
            code: Zeroizing::new(code.into()),
            routes,
        };
        link.check()?;
        if link.to_url().len() > Self::MAX_LEN {
            return Err(PairingLinkError::TooLong);
        }
        Ok(link)
    }

    fn check(&self) -> Result<(), PairingLinkError> {
        if self.code.is_empty() {
            return Err(PairingLinkError::Missing(PairingLinkKey::Code));
        }
        if self.routes.len() > Self::MAX_ROUTES {
            return Err(PairingLinkError::TooManyRoutes);
        }
        if self.routes.iter().any(String::is_empty) {
            return Err(PairingLinkError::EmptyRoute);
        }
        Ok(())
    }

    /// Reads a whole link, `https://emberhut.com/pair#…`.
    ///
    /// # Errors
    ///
    /// [`PairingLinkError`] when it isn't a valid pairing link.
    pub fn parse(link: &str) -> Result<Self, PairingLinkError> {
        if link.chars().count() > Self::MAX_LEN {
            return Err(PairingLinkError::TooLong);
        }
        let fragment = link
            .strip_prefix(Self::BASE)
            .and_then(|rest| rest.strip_prefix('#'))
            .ok_or(PairingLinkError::NotAPairingLink)?;
        Self::from_fragment(fragment)
    }

    /// Reads a link's fragment, the part after `#`.
    ///
    /// # Errors
    ///
    /// [`PairingLinkError`] when it isn't a valid pairing link's fragment.
    pub fn from_fragment(fragment: &str) -> Result<Self, PairingLinkError> {
        if Self::BASE.len() + 1 + fragment.chars().count() > Self::MAX_LEN {
            return Err(PairingLinkError::TooLong);
        }
        let mut id = None;
        let mut code = None;
        let mut routes = Vec::new();
        for (key, value) in form_urlencoded::parse(fragment.as_bytes()) {
            match &*key {
                "id" => once(&mut id, value.into_owned(), PairingLinkKey::Id)?,
                "code" => once(
                    &mut code,
                    Zeroizing::new(value.into_owned()),
                    PairingLinkKey::Code,
                )?,
                "route" => routes.push(value.into_owned()),
                _ => {}
            }
        }
        let id = id
            .filter(|id: &String| !id.is_empty())
            .ok_or(PairingLinkError::Missing(PairingLinkKey::Id))?
            .parse()?;
        let code = code.ok_or(PairingLinkError::Missing(PairingLinkKey::Code))?;
        let link = Self { id, code, routes };
        link.check()?;
        Ok(link)
    }

    /// The link, to show as text and as a QR code.
    #[must_use]
    pub fn to_url(&self) -> String {
        let base = format!("{}#", Self::BASE);
        let start = base.len();
        let mut fragment = form_urlencoded::Serializer::for_suffix(base, start);
        fragment.append_pair(PairingLinkKey::Id.as_str(), &self.id.to_string());
        fragment.append_pair(PairingLinkKey::Code.as_str(), &self.code);
        for route in &self.routes {
            fragment.append_pair("route", route);
        }
        fragment.finish()
    }

    /// The machine's id: the key in the pairing answer must have it.
    #[must_use]
    pub const fn id(&self) -> &MachineId {
        &self.id
    }

    /// The one-time code, as shown (`ABCD-EFGH`). The server reads it like a typed code.
    #[must_use]
    pub fn code(&self) -> &str {
        &self.code
    }

    /// The routes to pair over, in the machine's order.
    #[must_use]
    pub fn routes(&self) -> &[String] {
        &self.routes
    }
}

/// Sets `slot` to `value`, refusing a second value for `key`.
fn once<T>(slot: &mut Option<T>, value: T, key: PairingLinkKey) -> Result<(), PairingLinkError> {
    if slot.is_some() {
        return Err(PairingLinkError::Repeated(key));
    }
    *slot = Some(value);
    Ok(())
}

impl fmt::Debug for PairingLink {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PairingLink")
            .field("id", &self.id)
            .field("code", &format_args!("<redacted>"))
            .field("routes", &self.routes)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;
    use crate::SecretKey;

    fn id() -> MachineId {
        SecretKey::from_bytes(&[1; 32]).machine_id()
    }

    #[test]
    fn a_link_round_trips_with_its_routes_encoded() {
        let link = PairingLink::new(
            id(),
            "ABCD-EFGH",
            vec![
                "http://192.0.2.1:18990".to_owned(),
                "https://example.com/a?b=c&d#e".to_owned(),
            ],
        )
        .unwrap();
        let url = link.to_url();
        assert_eq!(
            url,
            format!(
                "https://emberhut.com/pair#id={}&code=ABCD-EFGH&route=http%3A%2F%2F192.0.2.1%3A18990\
                 &route=https%3A%2F%2Fexample.com%2Fa%3Fb%3Dc%26d%23e",
                id()
            )
        );
        assert_eq!(PairingLink::parse(&url), Ok(link));
    }

    #[test]
    fn debug_leaves_the_code_out() {
        let link = PairingLink::new(id(), "WXYZ-2345", Vec::new()).unwrap();
        let shown = format!("{link:?}");
        assert!(!shown.contains("WXYZ"), "{shown}");
        assert!(shown.contains("<redacted>"), "{shown}");
    }

    #[test]
    fn bad_links_are_refused() {
        let id = id();
        let cases = [
            (
                "https://emberhut.com/pair?id=x",
                PairingLinkError::NotAPairingLink,
            ),
            (
                "https://example.com/pair#id=x",
                PairingLinkError::NotAPairingLink,
            ),
            (
                "https://emberhut.com/pair#code=A",
                PairingLinkError::Missing(PairingLinkKey::Id),
            ),
            (
                &*format!("https://emberhut.com/pair#id={id}"),
                PairingLinkError::Missing(PairingLinkKey::Code),
            ),
            (
                &*format!("https://emberhut.com/pair#id={id}&id={id}&code=A"),
                PairingLinkError::Repeated(PairingLinkKey::Id),
            ),
            (
                &*format!("https://emberhut.com/pair#id={id}&code=A&code=B"),
                PairingLinkError::Repeated(PairingLinkKey::Code),
            ),
            (
                &*format!("https://emberhut.com/pair#id={id}&code=A&route="),
                PairingLinkError::EmptyRoute,
            ),
            (
                &*format!(
                    "https://emberhut.com/pair#id={id}&code=A{}",
                    "&route=r".repeat(9)
                ),
                PairingLinkError::TooManyRoutes,
            ),
            (
                &*format!(
                    "https://emberhut.com/pair#id={id}&code=A&x={}",
                    "a".repeat(2_000)
                ),
                PairingLinkError::TooLong,
            ),
        ];
        for (link, error) in cases {
            assert_eq!(PairingLink::parse(link), Err(error), "{link}");
        }
        assert!(matches!(
            PairingLink::parse("https://emberhut.com/pair#id=NOTANID&code=A"),
            Err(PairingLinkError::Id(_))
        ));
    }

    #[test]
    fn unknown_keys_are_ignored_and_plus_is_a_space() {
        let link = PairingLink::parse(&format!(
            "https://emberhut.com/pair#v=2&id={}&code=abcd+efgh&route=r1&other=1&route=r2",
            id()
        ))
        .unwrap();
        assert_eq!(link.code(), "abcd efgh");
        assert_eq!(link.routes(), ["r1", "r2"]);
        assert_eq!(link.id(), &id());
    }

    #[test]
    fn values_that_make_a_long_link_are_refused() {
        let route = format!("https://{}.example", "a".repeat(240));
        assert_eq!(
            PairingLink::new(id(), "ABCD-EFGH", vec![route; 8]),
            Err(PairingLinkError::TooLong)
        );
        assert_eq!(
            PairingLink::new(id(), "", Vec::new()),
            Err(PairingLinkError::Missing(PairingLinkKey::Code))
        );
    }

    proptest! {
        #[test]
        fn any_values_round_trip(code in "\\PC{1,12}", routes in prop::collection::vec("\\PC{1,8}", 0..=8)) {
            let link = PairingLink::new(id(), code, routes).unwrap();
            let url = link.to_url();
            prop_assert!(url.starts_with("https://emberhut.com/pair#id="), "{}", url);
            prop_assert_eq!(PairingLink::parse(&url), Ok(link));
        }

        #[test]
        fn parsing_never_panics(fragment in "\\PC{0,80}") {
            let _link = PairingLink::from_fragment(&fragment);
        }
    }
}
