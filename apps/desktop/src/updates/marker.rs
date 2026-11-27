//! The update's marker, `update.json` in the app's directory: the swap from one build to the
//! next, as the old copy (the watchdog) and the new one tell each other how it goes. Every
//! write goes to a temporary file renamed over the marker.
//!
//! `pending` from the swap until the new copy has drawn its first frame, then `healthy` (the
//! watchdog deletes the old bundle and the marker), or `rolled-back` with the reason. A
//! `rolled-back` marker is the refusal record: that build isn't offered again for 7 days, or
//! until the feed offers a newer one, whichever is first, and then the marker goes.

use std::io;
use std::path::Path;
use std::time::{Duration, SystemTime};

use emberhut_api::releases::Build;
use serde::{Deserialize, Serialize};

/// The marker's name in the app's directory.
pub(crate) const FILE: &str = "update.json";

/// How long a rolled-back build is refused.
const REFUSED_FOR: Duration = Duration::from_hours(7 * 24);

/// Where the swap is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum State {
    /// Swapped; the new copy hasn't drawn its first frame yet.
    Pending,
    /// The new copy drew its first frame.
    Healthy,
    /// The new copy didn't start, and the old bundle is back.
    RolledBack,
}

/// The marker.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Marker {
    /// The old build.
    pub(crate) from: Build,
    /// Its version.
    pub(crate) from_version: String,
    /// The new build.
    pub(crate) to: Build,
    /// Its version.
    pub(crate) to_version: String,
    /// Where the swap is.
    pub(crate) state: State,
    /// Why it rolled back.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) reason: Option<String>,
    /// When the state was set, in seconds since 1970.
    pub(crate) at: u64,
    /// The old copy's process, watching.
    pub(crate) watchdog: u32,
    /// The new copy's process, once started.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) new_copy: Option<u32>,
}

impl Marker {
    /// The marker in `path`, if there is one that reads.
    #[must_use]
    pub(crate) fn read(path: &Path) -> Option<Self> {
        let bytes = std::fs::read(path)
            .inspect_err(|error| {
                if error.kind() != io::ErrorKind::NotFound {
                    log::warn!("can't read the update marker: {error}");
                }
            })
            .ok()?;
        serde_json::from_slice(&bytes)
            .inspect_err(|error| log::warn!("the update marker doesn't read: {error}"))
            .ok()
    }

    /// Writes the marker to `path`, atomically.
    ///
    /// # Errors
    ///
    /// The file couldn't be written.
    pub(crate) fn write(&self, path: &Path) -> io::Result<()> {
        super::files::write_json(path, self)
    }

    /// The marker as `state` at `now`.
    pub(crate) fn set(&mut self, state: State, now: SystemTime) {
        self.state = state;
        self.at = now
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_or(0, |since| since.as_secs());
    }

    /// The build to refuse, while this marker is a refusal record that still holds at `now`
    /// with `newest` the newest build the feed lists (if known).
    #[must_use]
    pub(crate) fn refused(&self, now: SystemTime, newest: Option<Build>) -> Option<Build> {
        (self.state == State::RolledBack && !self.refusal_over(now, newest)).then_some(self.to)
    }

    /// Whether this refusal record has done its work: 7 days old, or the feed offers a build
    /// newer than the one refused.
    #[must_use]
    pub(crate) fn refusal_over(&self, now: SystemTime, newest: Option<Build>) -> bool {
        let since = SystemTime::UNIX_EPOCH + Duration::from_secs(self.at);
        let old = now
            .duration_since(since)
            .is_ok_and(|age| age >= REFUSED_FOR);
        old || newest.is_some_and(|newest| newest > self.to)
    }
}

/// What the copy of build `me` does once its first frame is drawn, given the marker it finds
/// and whether the watchdog named there still runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OnFirstFrame {
    /// Nothing: no swap of this build is waiting on it.
    Nothing,
    /// Report `healthy`; the watchdog keeps this build and tidies up.
    Report,
    /// Report `healthy` and tidy up itself (the old bundle, the marker): the watchdog is gone.
    ReportAndTidy,
    /// Tidy up: this build was kept, but the watchdog never finished.
    Tidy,
}

/// See [`OnFirstFrame`].
#[must_use]
pub(crate) fn on_first_frame(
    marker: Option<&Marker>,
    me: Build,
    watchdog_runs: bool,
) -> OnFirstFrame {
    match marker {
        Some(marker) if marker.to == me => match (marker.state, watchdog_runs) {
            (State::Pending, true) => OnFirstFrame::Report,
            (State::Pending, false) => OnFirstFrame::ReportAndTidy,
            (State::Healthy, false) => OnFirstFrame::Tidy,
            (State::Healthy, true) | (State::RolledBack, _) => OnFirstFrame::Nothing,
        },
        _ => OnFirstFrame::Nothing,
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    const DAY: Duration = Duration::from_hours(24);

    /// A pending swap from build 12 to 15, at day 1000.
    pub(crate) fn pending() -> Marker {
        Marker {
            from: Build::new(12),
            from_version: "0.2.0".into(),
            to: Build::new(15),
            to_version: "0.3.0".into(),
            state: State::Pending,
            reason: None,
            at: (1000 * DAY).as_secs(),
            watchdog: 41,
            new_copy: Some(42),
        }
    }

    #[test]
    fn the_marker_is_written_and_read_back_in_its_wire_words() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE);
        assert_eq!(Marker::read(&path), None);
        let mut marker = pending();
        marker.set(State::RolledBack, SystemTime::UNIX_EPOCH + 1001 * DAY);
        marker.reason = Some("it quit before its first frame".into());
        marker.write(&path).unwrap();
        assert_eq!(Marker::read(&path), Some(marker));
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains(r#""state": "rolled-back""#), "{text}");
        assert!(text.contains(r#""newCopy": 42"#), "{text}");
        std::fs::write(&path, "{").unwrap();
        assert_eq!(Marker::read(&path), None);
    }

    #[test]
    fn a_rolled_back_build_is_refused_for_a_week_or_until_a_newer_one() {
        let mut marker = pending();
        let then = SystemTime::UNIX_EPOCH + 1000 * DAY;
        assert_eq!(marker.refused(then, None), None, "pending is no refusal");
        marker.set(State::RolledBack, then);
        assert_eq!(marker.refused(then + DAY, None), Some(Build::new(15)));
        assert_eq!(
            marker.refused(then + DAY, Some(Build::new(15))),
            Some(Build::new(15))
        );
        assert_eq!(marker.refused(then + DAY, Some(Build::new(16))), None);
        assert!(marker.refusal_over(then + 7 * DAY, None));
        assert_eq!(marker.refused(then + 7 * DAY, None), None);
    }

    #[test]
    fn the_new_copy_reports_and_tidies_as_the_watchdog_needs() {
        let me = Build::new(15);
        let mut marker = pending();
        assert_eq!(on_first_frame(None, me, true), OnFirstFrame::Nothing);
        assert_eq!(
            on_first_frame(Some(&marker), me, true),
            OnFirstFrame::Report
        );
        assert_eq!(
            on_first_frame(Some(&marker), me, false),
            OnFirstFrame::ReportAndTidy
        );
        assert_eq!(
            on_first_frame(Some(&marker), Build::new(12), false),
            OnFirstFrame::Nothing,
            "the old build, restored"
        );
        marker.state = State::Healthy;
        assert_eq!(on_first_frame(Some(&marker), me, false), OnFirstFrame::Tidy);
        assert_eq!(
            on_first_frame(Some(&marker), me, true),
            OnFirstFrame::Nothing
        );
        marker.state = State::RolledBack;
        assert_eq!(
            on_first_frame(Some(&marker), me, false),
            OnFirstFrame::Nothing
        );
    }
}
