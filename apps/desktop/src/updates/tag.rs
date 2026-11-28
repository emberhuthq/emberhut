//! A release's tag read as the release pipeline writes it: `vX.Y.Z-rc.N` (a release candidate,
//! N from 1 to 98) or `vX.Y.Z` (final). Its build number is
//! `X·1 000 000 + Y·10 000 + Z·100 + N` (99 for a final), so it rises with the version, a
//! rebuild of a tag gets the same number, and no counter is kept: `v0.1.0-rc.1` is 10001,
//! `v0.1.0` 10099, `v0.1.1-rc.1` 10101. The same rule as `cargo xtask release version`.

use emberhut_api::releases::Build;

/// A release tag, read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Tag {
    /// The version: the tag without its `v` (`0.1.0-rc.1`).
    pub(crate) version: String,
    /// The build number it gives.
    pub(crate) build: Build,
    /// Whether it is a release candidate.
    pub(crate) candidate: bool,
}

/// The candidate number a final release counts as: above every candidate.
const FINAL: u64 = 99;

/// `tag` read; `None` unless it is `vX.Y.Z` or `vX.Y.Z-rc.N`, with Y and Z below 100 and N from
/// 1 to 98, written without leading zeros.
#[must_use]
pub(crate) fn parse(tag: &str) -> Option<Tag> {
    let version = tag.strip_prefix('v')?;
    let (base, candidate) = match version.split_once("-rc.") {
        Some((base, number)) => (base, Some(number)),
        None => (version, None),
    };
    let mut parts = base.split('.');
    let (major, minor, patch) = (
        number(parts.next()?)?,
        number(parts.next()?)?,
        number(parts.next()?)?,
    );
    if parts.next().is_some() || minor >= 100 || patch >= 100 {
        return None;
    }
    let n = match candidate {
        Some(text) => Some(number(text)?).filter(|n| (1..FINAL).contains(n))?,
        None => FINAL,
    };
    let build = major
        .checked_mul(1_000_000)?
        .checked_add(minor * 10_000 + patch * 100 + n)?;
    Some(Tag {
        version: version.to_owned(),
        build: Build::new(build),
        candidate: candidate.is_some(),
    })
}

/// A whole number in plain digits, no sign and no leading zero.
fn number(text: &str) -> Option<u64> {
    let plain = !text.is_empty()
        && text.bytes().all(|byte| byte.is_ascii_digit())
        && (text == "0" || !text.starts_with('0'));
    plain.then(|| text.parse().ok()).flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build(tag: &str) -> Option<u64> {
        parse(tag).map(|tag| tag.build.get())
    }

    #[test]
    fn builds_follow_the_pipelines_rule() {
        assert_eq!(build("v0.1.0-rc.1"), Some(10001));
        assert_eq!(build("v0.1.0-rc.2"), Some(10002));
        assert_eq!(build("v0.1.0"), Some(10099));
        assert_eq!(build("v0.1.1-rc.1"), Some(10101));
        assert_eq!(build("v1.2.3"), Some(1_020_399));
        let rc = parse("v0.1.0-rc.1").unwrap();
        assert!(rc.candidate);
        assert_eq!(rc.version, "0.1.0-rc.1");
        assert!(!parse("v0.1.0").unwrap().candidate);
    }

    #[test]
    fn a_final_is_above_its_candidates_and_below_the_next_ones() {
        let order = [
            "v0.1.0-rc.1",
            "v0.1.0-rc.98",
            "v0.1.0",
            "v0.1.1-rc.1",
            "v0.2.0",
        ];
        let builds: Vec<_> = order.iter().map(|tag| build(tag).unwrap()).collect();
        assert!(builds.is_sorted(), "{builds:?}");
    }

    #[test]
    fn anything_else_isnt_a_release_tag() {
        for tag in [
            "",
            "0.1.0",
            "v0.1",
            "v0.1.0.1",
            "v0.100.0",
            "v0.1.100",
            "v0.1.0-rc.0",
            "v0.1.0-rc.99",
            "v0.1.0-rc.",
            "v0.1.0-beta.1",
            "v01.1.0",
            "v0.1.0-rc.01",
            "v+1.0.0",
            "server-v0.1.0",
        ] {
            assert_eq!(parse(tag), None, "{tag:?}");
        }
    }
}
