//! Which release, if any, to install: a build newer than this one (never an equal or older
//! build, so the app never downgrades), not the build it rolled back from, made for this
//! computer's system and processor and fit for its system's version. The newest that fits wins.

use emberhut_api::releases::{Asset, Build, FORMAT, Release, ReleaseFeed};

use super::system::{OsVersion, ThisMachine};

/// The product the desktop's feed publishes.
pub(crate) const PRODUCT: &str = "emberhut-desktop";

/// What the feed offers this copy.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Choice<'a> {
    /// A newer release fits this computer.
    Newer(Offer<'a>),
    /// Nothing to install: no newer release, or none that fits (the newest newer one says why).
    Latest(Option<Unfit<'a>>),
    /// The feed is in a format newer than this app reads: Emberhut has to be updated by hand.
    ByHand,
    /// The feed is another product's or another platform's: the release is misconfigured.
    NotOurs,
}

/// The release to install and its payload for this computer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Offer<'a> {
    /// The release.
    pub(crate) release: &'a Release,
    /// Its payload for this computer's processor.
    pub(crate) asset: &'a Asset,
    /// The listed releases between this copy and that one, newest first, whose notes show as
    /// unverified.
    pub(crate) earlier: Vec<&'a Release>,
}

/// Why a newer release isn't offered.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Unfit<'a> {
    /// It needs a later version of the system.
    NeedsNewerSystem(&'a Release),
    /// It has no payload for this processor.
    NoPayload(&'a Release),
    /// The app rolled back from it lately.
    RolledBack(&'a Release),
}

/// What `feed` offers a copy of build `running` on `here`, the app having rolled back from
/// `refused` lately (if it did).
#[must_use]
pub(crate) fn choose<'a>(
    feed: &'a ReleaseFeed,
    running: Build,
    here: &ThisMachine,
    refused: Option<Build>,
) -> Choice<'a> {
    if feed.format > FORMAT {
        return Choice::ByHand;
    }
    if feed.platform != here.platform || feed.product != PRODUCT {
        return Choice::NotOurs;
    }
    let mut newer: Vec<&Release> = feed
        .releases
        .iter()
        .filter(|release| release.build > running)
        .collect();
    newer.sort_by_key(|release| std::cmp::Reverse(release.build));
    let mut first_unfit = None;
    for (at, release) in newer.iter().enumerate() {
        let unfit = if refused == Some(release.build) {
            Some(Unfit::RolledBack(release))
        } else if !fits(release, here.os) {
            Some(Unfit::NeedsNewerSystem(release))
        } else {
            match release.asset(&here.arch) {
                Some(asset) => {
                    return Choice::Newer(Offer {
                        release,
                        asset,
                        earlier: newer[at + 1..].to_vec(),
                    });
                }
                None => Some(Unfit::NoPayload(release)),
            }
        };
        first_unfit = first_unfit.or(unfit);
    }
    Choice::Latest(first_unfit)
}

/// Whether `release` runs on the system's version `os`: a minimum that doesn't read as a
/// version doesn't.
fn fits(release: &Release, os: OsVersion) -> bool {
    release
        .minimum_os
        .as_deref()
        .is_none_or(|minimum| OsVersion::parse(minimum).is_some_and(|minimum| minimum <= os))
}

#[cfg(test)]
mod tests {
    use emberhut_api::releases::{Arch, Platform};
    use serde_json::json;

    use super::*;

    fn release(build: u64, minimum: &str, arches: &[&str]) -> serde_json::Value {
        let assets: Vec<_> = arches
            .iter()
            .map(|arch| {
                json!({"arch": arch, "url": format!("/api/releases/files/{build}_{arch}.dmg"),
                    "size": 10, "sha256": "00", "signature": ""})
            })
            .collect();
        json!({"version": format!("0.{build}.0"), "build": build, "date": "2026-12-08",
            "notes": format!("Build {build}."), "minimumOs": minimum, "assets": assets})
    }

    fn feed(releases: &[serde_json::Value]) -> ReleaseFeed {
        serde_json::from_value(json!({"format": 1, "product": "emberhut-desktop",
            "platform": "macos", "published": "2026-12-08T10:00:00.000Z",
            "releases": releases}))
        .unwrap()
    }

    fn mac(os: &str) -> ThisMachine {
        ThisMachine {
            platform: Platform::Macos,
            arch: Arch::Arm64,
            os: OsVersion::parse(os).unwrap(),
        }
    }

    fn chosen(choice: &Choice<'_>) -> Option<u64> {
        match choice {
            Choice::Newer(offer) => Some(offer.release.build.get()),
            _ => None,
        }
    }

    #[test]
    fn the_newest_newer_build_that_fits_is_offered_with_the_ones_it_skips() {
        let feed = feed(&[
            release(15, "13.0", &["arm64"]),
            release(14, "13.0", &["arm64"]),
            release(12, "13.0", &["arm64"]),
        ]);
        let choice = choose(&feed, Build::new(12), &mac("14.5"), None);
        let Choice::Newer(offer) = &choice else {
            panic!("{choice:?}")
        };
        assert_eq!(offer.release.build, Build::new(15));
        assert_eq!(offer.asset.url, "/api/releases/files/15_arm64.dmg");
        let earlier: Vec<_> = offer.earlier.iter().map(|r| r.build.get()).collect();
        assert_eq!(earlier, [14]);
    }

    #[test]
    fn an_equal_or_older_build_is_never_offered() {
        let feed = feed(&[
            release(12, "13.0", &["arm64"]),
            release(11, "13.0", &["arm64"]),
        ]);
        assert_eq!(
            choose(&feed, Build::new(12), &mac("14.0"), None),
            Choice::Latest(None)
        );
        assert_eq!(
            choose(&feed, Build::new(13), &mac("14.0"), None),
            Choice::Latest(None)
        );
    }

    #[test]
    fn a_build_for_a_later_macos_or_another_processor_is_passed_over() {
        let feed = feed(&[
            release(16, "15.0", &["arm64"]),
            release(15, "13.0", &["x86_64"]),
            release(14, "13.0", &["arm64"]),
        ]);
        assert_eq!(
            chosen(&choose(&feed, Build::new(12), &mac("14.0"), None)),
            Some(14)
        );
        let Choice::Latest(Some(Unfit::NeedsNewerSystem(release))) =
            choose(&feed, Build::new(14), &mac("14.0"), None)
        else {
            panic!("the newest needs a later macOS")
        };
        assert_eq!(release.build, Build::new(16));
        assert_eq!(
            chosen(&choose(&feed, Build::new(12), &mac("15.0"), None)),
            Some(16)
        );
    }

    #[test]
    fn a_minimum_that_isnt_a_version_doesnt_fit_and_none_fits_all() {
        let odd = release(15, "soon", &["arm64"]);
        let mut open = release(14, "13.0", &["arm64"]);
        open.as_object_mut().unwrap().remove("minimumOs");
        let feed = feed(&[odd, open]);
        assert_eq!(
            chosen(&choose(&feed, Build::new(12), &mac("26.0"), None)),
            Some(14)
        );
    }

    #[test]
    fn the_build_rolled_back_from_is_not_offered_again() {
        let feed = feed(&[
            release(15, "13.0", &["arm64"]),
            release(14, "13.0", &["arm64"]),
        ]);
        let refused = Some(Build::new(15));
        assert_eq!(
            chosen(&choose(&feed, Build::new(12), &mac("14.0"), refused)),
            Some(14)
        );
        assert!(matches!(
            choose(&feed, Build::new(14), &mac("14.0"), refused),
            Choice::Latest(Some(Unfit::RolledBack(_)))
        ));
    }

    #[test]
    fn a_newer_format_or_another_feed_is_not_read() {
        let mut newer = feed(&[release(15, "13.0", &["arm64"])]);
        newer.format = FORMAT + 1;
        assert_eq!(
            choose(&newer, Build::new(12), &mac("14.0"), None),
            Choice::ByHand
        );
        let mut phone = feed(&[release(15, "13.0", &["arm64"])]);
        phone.platform = Platform::Ios;
        assert_eq!(
            choose(&phone, Build::new(12), &mac("14.0"), None),
            Choice::NotOurs
        );
        let mut windows = feed(&[release(15, "13.0", &["arm64"])]);
        windows.platform = Platform::Windows;
        assert_eq!(
            choose(&windows, Build::new(12), &mac("14.0"), None),
            Choice::NotOurs,
            "a Windows feed on a Mac"
        );
        let mut other = feed(&[release(15, "13.0", &["arm64"])]);
        other.product = "another-app".into();
        assert_eq!(
            choose(&other, Build::new(12), &mac("14.0"), None),
            Choice::NotOurs
        );
    }
}
