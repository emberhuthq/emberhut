//! What the updater keeps across launches (`update-check.json` in the app's directory): when it
//! last checked, how old the feed was, the build whose bar the user put away, and the payloads
//! that failed verification, with the wait before the next download.
//!
//! After a payload fails verification it isn't fetched again until the feed offers a different
//! `sha256`, and no payload is downloaded for 24 hours, doubling with each failure after that,
//! so a hostile server can make the app download at most one capped payload a day.

use std::io;
use std::path::Path;
use std::time::{Duration, SystemTime};

use emberhut_api::releases::Build;
use serde::{Deserialize, Serialize};

/// The file's name in the app's directory.
pub(crate) const FILE: &str = "update-check.json";

/// The wait after the first failed verification.
const FIRST_WAIT: Duration = Duration::from_hours(24);

/// The most the wait doubles: 2⁵ days.
const MAX_DOUBLINGS: u32 = 5;

/// The record.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Record {
    /// When the feed was last read, in seconds since 1970.
    #[serde(default)]
    last_checked: Option<u64>,
    /// When the feed last read was published, as the server wrote it.
    #[serde(default)]
    pub(crate) published: Option<String>,
    /// The build whose bar the user closed: it shows again for a newer one.
    #[serde(default)]
    pub(crate) dismissed: Option<Build>,
    /// The SHA-256 of the last payload that failed verification.
    #[serde(default)]
    refused_payload: Option<String>,
    /// How many verifications have failed in a row.
    #[serde(default)]
    failures: u32,
    /// When the last one failed, in seconds since 1970.
    #[serde(default)]
    last_failure: Option<u64>,
}

/// Why no payload is downloaded now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Held {
    /// This very payload failed verification before.
    SamePayload,
    /// A payload failed verification lately: the next download waits until then.
    Until(SystemTime),
}

impl Record {
    /// The record in `path`; a fresh one when there is none or it doesn't read.
    #[must_use]
    pub(crate) fn read(path: &Path) -> Self {
        match std::fs::read(path) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|error| {
                log::warn!("the update record doesn't read ({error}); starting a new one");
                Self::default()
            }),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Self::default(),
            Err(error) => {
                log::warn!("can't read the update record: {error}");
                Self::default()
            }
        }
    }

    /// Writes the record to `path`, through a temporary file renamed over it.
    ///
    /// # Errors
    ///
    /// The file couldn't be written.
    pub(crate) fn write(&self, path: &Path) -> io::Result<()> {
        super::files::write_json(path, self)
    }

    /// When the feed was last read.
    #[must_use]
    pub(crate) fn last_checked(&self) -> Option<SystemTime> {
        self.last_checked.map(from_secs)
    }

    /// The feed was read at `now`, published at `published`.
    pub(crate) fn checked(&mut self, now: SystemTime, published: Option<String>) {
        self.last_checked = Some(secs(now));
        if published.is_some() {
            self.published = published;
        }
    }

    /// Whether the payload with SHA-256 `sha256` may be downloaded at `now`.
    ///
    /// # Errors
    ///
    /// Why not, as [`Held`].
    pub(crate) fn may_download(&self, sha256: &str, now: SystemTime) -> Result<(), Held> {
        if self
            .refused_payload
            .as_deref()
            .is_some_and(|refused| refused.eq_ignore_ascii_case(sha256))
        {
            return Err(Held::SamePayload);
        }
        match (self.failures, self.last_failure) {
            (0, _) | (_, None) => Ok(()),
            (failures, Some(last)) => {
                let doublings = (failures - 1).min(MAX_DOUBLINGS);
                let until = from_secs(last) + FIRST_WAIT * 2_u32.pow(doublings);
                if now < until {
                    Err(Held::Until(until))
                } else {
                    Ok(())
                }
            }
        }
    }

    /// The payload with SHA-256 `sha256` failed verification at `now`.
    pub(crate) fn failed_verification(&mut self, sha256: &str, now: SystemTime) {
        self.refused_payload = Some(sha256.to_ascii_lowercase());
        self.failures = self.failures.saturating_add(1);
        self.last_failure = Some(secs(now));
    }

    /// A payload verified: the waits start over.
    pub(crate) fn verified(&mut self) {
        self.refused_payload = None;
        self.failures = 0;
        self.last_failure = None;
    }
}

fn secs(time: SystemTime) -> u64 {
    time.duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |since| since.as_secs())
}

fn from_secs(secs: u64) -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_secs(secs)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: Duration = Duration::from_hours(24);

    fn at(days: u32) -> SystemTime {
        SystemTime::UNIX_EPOCH + DAY * (1000 + days)
    }

    #[test]
    fn a_failed_payload_is_not_fetched_again_and_the_next_waits_a_day_then_longer() {
        let mut record = Record::default();
        assert_eq!(record.may_download("aa", at(0)), Ok(()));
        record.failed_verification("AA", at(0));
        assert_eq!(record.may_download("aa", at(9)), Err(Held::SamePayload));
        assert_eq!(record.may_download("bb", at(0)), Err(Held::Until(at(1))));
        assert_eq!(record.may_download("bb", at(1)), Ok(()));
        record.failed_verification("bb", at(1));
        assert_eq!(record.may_download("cc", at(2)), Err(Held::Until(at(3))));
        record.failed_verification("cc", at(3));
        assert_eq!(record.may_download("dd", at(6)), Err(Held::Until(at(7))));
        record.verified();
        assert_eq!(
            record.may_download("cc", at(3)),
            Ok(()),
            "verified: all clear"
        );
    }

    #[test]
    fn the_wait_stops_doubling_at_a_month() {
        let mut record = Record::default();
        for _ in 0..20 {
            record.failed_verification("aa", at(0));
        }
        assert_eq!(record.may_download("bb", at(0)), Err(Held::Until(at(32))));
    }

    #[test]
    fn the_record_is_kept_across_launches() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE);
        assert_eq!(Record::read(&path), Record::default());
        let mut record = Record {
            dismissed: Some(Build::new(15)),
            ..Record::default()
        };
        record.checked(at(2), Some("2026-12-08T10:00:00.000Z".into()));
        record.failed_verification("aa", at(1));
        record.write(&path).unwrap();
        let read = Record::read(&path);
        assert_eq!(read, record);
        assert_eq!(read.last_checked(), Some(at(2)));
        std::fs::write(&path, "not json").unwrap();
        assert_eq!(Record::read(&path), Record::default());
    }
}
