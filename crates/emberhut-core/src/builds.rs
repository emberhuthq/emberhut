//! The newest build of the app per platform, as the release step reports it once a store has
//! it ready: an app older than that shows "Build N is ready" with Update.
//!
//! A report only ever moves its platform's number up, so a late or repeated report of an older
//! build changes nothing.

use std::fmt;

/// A platform the release step reports builds for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Platform {
    /// The phone app, on `TestFlight`.
    Ios,
    /// The desktop app.
    Mac,
}

impl Platform {
    /// Every platform.
    pub const ALL: [Self; 2] = [Self::Ios, Self::Mac];

    /// Its name on the wire and in the database: `ios` or `mac`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ios => "ios",
            Self::Mac => "mac",
        }
    }

    /// The platform named `name` exactly (`ios`, `mac`).
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|platform| platform.as_str() == name)
    }
}

impl fmt::Display for Platform {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Why a build report was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("build: {{ platform: \"ios\" or \"mac\", number: a whole build number }}")]
pub struct InvalidBuild;

/// The highest build number taken: builds are kept as 32-bit integers.
pub const MAX_BUILD: u32 = i32::MAX.unsigned_abs();

/// One report: build `number` of the app is ready on `platform`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BuildReport {
    /// Where.
    pub platform: Platform,
    /// Which build: a whole number from 1.
    pub number: u32,
}

impl BuildReport {
    /// A report of build `number` on the platform named `platform`.
    ///
    /// # Errors
    ///
    /// [`InvalidBuild`] when `platform` is not `ios` or `mac`, or `number` is not a whole
    /// number from 1 to [`MAX_BUILD`].
    pub fn new(platform: &str, number: f64) -> Result<Self, InvalidBuild> {
        let platform = Platform::from_name(platform).ok_or(InvalidBuild)?;
        if !(number >= 1.0 && number.fract() == 0.0 && number <= f64::from(MAX_BUILD)) {
            return Err(InvalidBuild);
        }
        // A whole number in 1..=MAX_BUILD, so the cast is exact.
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "checked above: a whole number from 1 to MAX_BUILD"
        )]
        let number = number as u32;
        Ok(Self { platform, number })
    }
}

/// The newest build reported per platform; `None` where none was.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LatestBuilds {
    /// The phone app's.
    pub ios: Option<u32>,
    /// The desktop app's.
    pub mac: Option<u32>,
}

impl LatestBuilds {
    /// The platform's newest build.
    #[must_use]
    pub const fn get(&self, platform: Platform) -> Option<u32> {
        match platform {
            Platform::Ios => self.ios,
            Platform::Mac => self.mac,
        }
    }

    /// These builds after `report`: its platform's number moves up to it, never down.
    #[must_use]
    pub fn with(mut self, report: BuildReport) -> Self {
        let raised = self
            .get(report.platform)
            .map_or(report.number, |known| known.max(report.number));
        match report.platform {
            Platform::Ios => self.ios = Some(raised),
            Platform::Mac => self.mac = Some(raised),
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_report_names_ios_or_mac_and_a_whole_number_from_one() {
        assert_eq!(
            BuildReport::new("ios", 12.0),
            Ok(BuildReport {
                platform: Platform::Ios,
                number: 12
            })
        );
        assert_eq!(
            BuildReport::new("mac", f64::from(MAX_BUILD)).map(|r| r.number),
            Ok(MAX_BUILD)
        );
        for (platform, number) in [
            ("android", 3.0),
            ("IOS", 3.0),
            ("", 3.0),
            ("ios", 0.0),
            ("ios", -4.0),
            ("ios", 2.5),
            ("ios", f64::NAN),
            ("ios", f64::INFINITY),
            ("mac", f64::from(MAX_BUILD) + 1.0),
        ] {
            assert_eq!(
                BuildReport::new(platform, number),
                Err(InvalidBuild),
                "{platform} {number}"
            );
        }
    }

    #[test]
    fn the_refusal_says_what_a_report_is() {
        assert_eq!(
            InvalidBuild.to_string(),
            "build: { platform: \"ios\" or \"mac\", number: a whole build number }"
        );
    }

    #[test]
    fn a_platform_s_number_only_moves_up() {
        let report = |platform: &str, number: f64| BuildReport::new(platform, number).unwrap();
        let builds = LatestBuilds::default()
            .with(report("ios", 12.0))
            .with(report("ios", 9.0))
            .with(report("mac", 4.0));
        assert_eq!(
            builds,
            LatestBuilds {
                ios: Some(12),
                mac: Some(4)
            }
        );
        assert_eq!(builds.with(report("mac", 7.0)).mac, Some(7));
    }

    #[test]
    fn platforms_have_their_wire_names() {
        for platform in Platform::ALL {
            assert_eq!(Platform::from_name(platform.as_str()), Some(platform));
            assert_eq!(platform.to_string(), platform.as_str());
        }
    }
}
