//! Release tags and the numbers derived from them: `vX.Y.Z-rc.N` is a release candidate (a
//! GitHub pre-release), `vX.Y.Z` a final release. The workspace's own version stays plain
//! `X.Y.Z`; the tag says which build of it this is, so nothing in `Cargo.toml` changes per tag.

use std::cmp::Ordering;
use std::fmt;

/// The highest release candidate number: 99 stands for the final release in the build number.
pub(crate) const MAX_RC: u8 = 98;
/// The build number's last two digits for a final release.
const FINAL: u64 = 99;

/// Which updater channel a release belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Channel {
    /// Release candidates: pre-releases, which the "Release candidates" channel installs.
    Rc,
    /// Final releases, which every channel installs.
    Stable,
}

impl Channel {
    /// The channel's name, as `EMBERHUT_CHANNEL` carries it.
    #[must_use]
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Rc => "rc",
            Self::Stable => "stable",
        }
    }
}

/// A valid release tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Tag {
    /// `X`.
    pub(crate) major: u64,
    /// `Y`, below 100 so the build number stays ordered.
    pub(crate) minor: u64,
    /// `Z`, below 100 for the same reason.
    pub(crate) patch: u64,
    /// `N` of `-rc.N` (1 to [`MAX_RC`]), or `None` for a final release.
    pub(crate) rc: Option<u8>,
}

/// Why a tag isn't a release tag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TagError {
    /// It doesn't read as `vX.Y.Z` or `vX.Y.Z-rc.N`.
    Format(String),
    /// A number is out of the range the build and MSI numbers can hold.
    Range(String),
}

impl fmt::Display for TagError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Format(tag) => write!(
                f,
                "`{tag}` is not a release tag: use vX.Y.Z-rc.N (N from 1 to {MAX_RC}) or vX.Y.Z, \
                 with no leading zeros"
            ),
            Self::Range(why) => f.write_str(why),
        }
    }
}

impl std::error::Error for TagError {}

/// A version number without leading zeros.
fn number(text: &str) -> Option<u64> {
    let plain = !text.is_empty()
        && text.bytes().all(|b| b.is_ascii_digit())
        && (text == "0" || !text.starts_with('0'));
    plain.then(|| text.parse().ok()).flatten()
}

/// `X.Y.Z` as three numbers.
fn base(text: &str) -> Option<(u64, u64, u64)> {
    let mut parts = text.split('.');
    let parsed = (
        number(parts.next()?)?,
        number(parts.next()?)?,
        number(parts.next()?)?,
    );
    parts.next().is_none().then_some(parsed)
}

impl Tag {
    /// Reads `tag`.
    ///
    /// # Errors
    ///
    /// [`TagError::Format`] when it isn't `vX.Y.Z` or `vX.Y.Z-rc.N`, and [`TagError::Range`] when
    /// N isn't 1 to 98, Y or Z is 100 or more, or X is over 255 (the MSI version's limit).
    pub(crate) fn parse(tag: &str) -> Result<Self, TagError> {
        let format = || TagError::Format(tag.to_owned());
        let rest = tag.strip_prefix('v').ok_or_else(format)?;
        let (version, rc) = match rest.split_once('-') {
            None => (rest, None),
            Some((version, pre)) => {
                let n = pre
                    .strip_prefix("rc.")
                    .and_then(number)
                    .ok_or_else(format)?;
                (version, Some(n))
            }
        };
        let (major, minor, patch) = base(version).ok_or_else(format)?;
        let rc = match rc {
            None => None,
            Some(n) => match u8::try_from(n) {
                Ok(n) if (1..=MAX_RC).contains(&n) => Some(n),
                _ => {
                    return Err(TagError::Range(format!(
                        "`{tag}`: a release candidate is numbered 1 to {MAX_RC}"
                    )));
                }
            },
        };
        if minor > 99 || patch > 99 {
            return Err(TagError::Range(format!(
                "`{tag}`: the minor and patch numbers stay below 100, so the build number keeps \
                 their order"
            )));
        }
        if major > 255 {
            return Err(TagError::Range(format!(
                "`{tag}`: the major number stays at 255 or below (the MSI version's limit)"
            )));
        }
        Ok(Self {
            major,
            minor,
            patch,
            rc,
        })
    }

    /// The version without the `v`: `0.1.0-rc.1` (`EMBERHUT_VERSION`).
    #[must_use]
    pub(crate) fn version(&self) -> String {
        match self.rc {
            Some(n) => format!("{}-rc.{n}", self.base()),
            None => self.base(),
        }
    }

    /// The plain `X.Y.Z`, which the workspace's version must equal.
    #[must_use]
    pub(crate) fn base(&self) -> String {
        format!("{}.{}.{}", self.major, self.minor, self.patch)
    }

    /// The tag itself, `v` and the version.
    #[must_use]
    pub(crate) fn name(&self) -> String {
        format!("v{}", self.version())
    }

    /// The build number (`EMBERHUT_BUILD`): `X*1_000_000 + Y*10_000 + Z*100` plus N for a release
    /// candidate or 99 for the final release. It rises with every later tag and is the same on
    /// a rebuild of the same tag.
    #[must_use]
    pub(crate) fn build(&self) -> u64 {
        self.major * 1_000_000 + self.minor * 10_000 + self.patch * 100 + self.last_two()
    }

    /// N, or 99 for a final release.
    fn last_two(&self) -> u64 {
        self.rc.map_or(FINAL, u64::from)
    }

    /// The updater channel.
    #[must_use]
    pub(crate) fn channel(&self) -> Channel {
        if self.rc.is_some() {
            Channel::Rc
        } else {
            Channel::Stable
        }
    }

    /// Whether the GitHub release is a pre-release.
    #[must_use]
    pub(crate) fn prerelease(&self) -> bool {
        self.rc.is_some()
    }

    /// The Windows installer's `ProductVersion`, `X.Y.(Z*100 + N or 99)`: MSI compares only the
    /// first three fields, so every tag needs its own third field for upgrades to work.
    #[must_use]
    pub(crate) fn msi_version(&self) -> String {
        format!(
            "{}.{}.{}",
            self.major,
            self.minor,
            self.patch * 100 + self.last_two()
        )
    }
}

impl PartialOrd for Tag {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Tag {
    /// Tags order by their build number, which is semver's order for the tags this accepts.
    fn cmp(&self, other: &Self) -> Ordering {
        self.build().cmp(&other.build())
    }
}

/// The latest release tag before `tag` among `tags` (any text; what isn't a release tag is
/// skipped), by build number.
#[must_use]
pub(crate) fn previous<'a>(tag: &Tag, tags: impl IntoIterator<Item = &'a str>) -> Option<Tag> {
    tags.into_iter()
        .filter_map(|name| Tag::parse(name.trim()).ok())
        .filter(|candidate| candidate < tag)
        .max()
}

/// The `KEY=value` lines `release version` prints for `$GITHUB_ENV`.
#[must_use]
pub(crate) fn env_lines(tag: &Tag, previous: Option<&Tag>) -> Vec<String> {
    vec![
        format!("EMBERHUT_VERSION={}", tag.version()),
        format!("EMBERHUT_BUILD={}", tag.build()),
        format!("EMBERHUT_CHANNEL={}", tag.channel().name()),
        format!("EMBERHUT_PRERELEASE={}", tag.prerelease()),
        format!("EMBERHUT_MSI_VERSION={}", tag.msi_version()),
        format!(
            "EMBERHUT_PREVIOUS_TAG={}",
            previous.map_or_else(String::new, Tag::name)
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::{Channel, Tag, TagError, env_lines, previous};

    fn tag(text: &str) -> Tag {
        Tag::parse(text).unwrap()
    }

    #[test]
    fn the_design_examples_have_their_numbers() {
        assert_eq!(tag("v0.1.0-rc.1").build(), 10001);
        assert_eq!(tag("v0.1.0-rc.2").build(), 10002);
        assert_eq!(tag("v0.1.0").build(), 10099);
        assert_eq!(tag("v0.1.1-rc.1").build(), 10101);
        assert_eq!(tag("v1.2.3").build(), 1_020_399);
        assert_eq!(tag("v0.1.0-rc.1").msi_version(), "0.1.1");
        assert_eq!(tag("v0.1.0").msi_version(), "0.1.99");
        assert_eq!(tag("v2.3.4-rc.98").msi_version(), "2.3.498");
    }

    #[test]
    fn a_candidate_and_a_final() {
        let rc = tag("v0.1.0-rc.1");
        assert_eq!(rc.version(), "0.1.0-rc.1");
        assert_eq!(rc.base(), "0.1.0");
        assert_eq!(rc.name(), "v0.1.0-rc.1");
        assert_eq!(rc.channel(), Channel::Rc);
        assert!(rc.prerelease());
        let stable = tag("v0.1.0");
        assert_eq!(stable.version(), "0.1.0");
        assert_eq!(stable.channel().name(), "stable");
        assert!(!stable.prerelease());
    }

    #[test]
    fn malformed_tags_are_refused() {
        for bad in [
            "0.1.0",
            "v0.1",
            "v0.1.0.0",
            "v01.1.0",
            "v0.01.0",
            "v0.1.00",
            "v0.1.0-rc",
            "v0.1.0-rc.",
            "v0.1.0-rc.01",
            "v0.1.0-beta.1",
            "v0.1.0-rc.1.2",
            "v0.1.0+build",
            "v0.1.0 ",
            "v-1.0.0",
            "",
            "v",
        ] {
            assert!(
                matches!(Tag::parse(bad), Err(TagError::Format(_))),
                "{bad:?} should not parse"
            );
        }
    }

    #[test]
    fn numbers_out_of_range_are_refused() {
        for bad in [
            "v0.1.0-rc.0",
            "v0.1.0-rc.99",
            "v0.1.0-rc.300",
            "v0.100.0",
            "v0.1.100",
            "v256.0.0",
        ] {
            assert!(
                matches!(Tag::parse(bad), Err(TagError::Range(_))),
                "{bad:?} should be out of range"
            );
        }
        assert_eq!(tag("v0.1.0-rc.98").rc, Some(98));
        assert_eq!(tag("v255.99.99").build(), 255_999_999);
    }

    #[test]
    fn builds_follow_semver() {
        let ordered = [
            "v0.1.0-rc.1",
            "v0.1.0-rc.2",
            "v0.1.0-rc.10",
            "v0.1.0-rc.98",
            "v0.1.0",
            "v0.1.1-rc.1",
            "v0.1.1",
            "v0.2.0-rc.1",
            "v0.99.99",
            "v1.0.0-rc.1",
            "v1.0.0",
        ];
        for pair in ordered.windows(2) {
            let (a, b) = (tag(pair[0]), tag(pair[1]));
            assert!(a < b && a.build() < b.build(), "{} < {}", pair[0], pair[1]);
            assert_ne!(a.msi_version(), b.msi_version());
        }
    }

    #[test]
    fn the_previous_tag_is_the_latest_earlier_release_tag() {
        let tags = [
            "v0.1.0-rc.1",
            "v0.1.0-rc.10",
            "v0.1.0-rc.2",
            "not-a-release",
            "v0.1.0",
            "v0.2.0-rc.1",
        ];
        let before = |name: &str| previous(&tag(name), tags).map(|t| t.name());
        assert_eq!(before("v0.1.0").as_deref(), Some("v0.1.0-rc.10"));
        assert_eq!(before("v0.1.1-rc.1").as_deref(), Some("v0.1.0"));
        assert_eq!(before("v0.2.0-rc.1").as_deref(), Some("v0.1.0"));
        assert_eq!(before("v0.1.0-rc.1"), None);
        assert_eq!(previous(&tag("v1.0.0"), []), None);
    }

    #[test]
    fn the_env_lines_carry_every_value() {
        let lines = env_lines(&tag("v0.1.0-rc.2"), Some(&tag("v0.1.0-rc.1")));
        assert_eq!(
            lines,
            [
                "EMBERHUT_VERSION=0.1.0-rc.2",
                "EMBERHUT_BUILD=10002",
                "EMBERHUT_CHANNEL=rc",
                "EMBERHUT_PRERELEASE=true",
                "EMBERHUT_MSI_VERSION=0.1.2",
                "EMBERHUT_PREVIOUS_TAG=v0.1.0-rc.1",
            ]
        );
        let first = env_lines(&tag("v0.1.0"), None);
        assert_eq!(first[3], "EMBERHUT_PRERELEASE=false");
        assert_eq!(first[5], "EMBERHUT_PREVIOUS_TAG=");
    }
}
