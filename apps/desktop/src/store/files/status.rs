//! The status line at the top of the window about files leaving the app: `Downloading name… 42%`
//! while a file comes down (only once it has taken a moment, [`BUSY_AFTER`]: a file kept
//! already shows nothing), then how its use went for a moment ([`SHOWN_FOR`]): `Saved to
//! Downloads as name (1).pdf`, `Copied`, `Couldn't download name`.

use std::time::Duration;

/// How long a download runs before the line says so.
pub(crate) const BUSY_AFTER: Duration = Duration::from_millis(250);

/// How long a result shows.
pub(crate) const SHOWN_FOR: Duration = Duration::from_secs(2);

/// What the line says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Note {
    /// A file coming down.
    Busy {
        /// The server address of the file.
        path: String,
        /// Its name.
        name: String,
        /// How far it is, when the server gave its size.
        percent: Option<u8>,
    },
    /// How a use went.
    Done {
        /// What it says.
        text: String,
        /// Whether it went wrong.
        failed: bool,
    },
}

impl Note {
    /// What the line reads.
    #[must_use]
    pub(crate) fn text(&self) -> String {
        match self {
            Self::Busy {
                name,
                percent: Some(percent),
                ..
            } => format!("Downloading {name}… {percent}%"),
            Self::Busy { name, .. } => format!("Downloading {name}…"),
            Self::Done { text, .. } => text.clone(),
        }
    }

    /// What a screen reader says of the line: its state without the percent, so a download
    /// is announced once as it starts and once as it ends, not at every percent.
    #[must_use]
    pub(crate) fn spoken(&self) -> String {
        match self {
            Self::Busy { name, .. } => format!("Downloading {name}…"),
            Self::Done { text, .. } => text.clone(),
        }
    }
}

/// The line's note, and the number of the latest one (a result's timer clears only its own).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct FileStatus {
    note: Option<Note>,
    shown: u64,
}

impl FileStatus {
    /// What the line says now.
    #[must_use]
    pub(crate) fn note(&self) -> Option<&Note> {
        self.note.as_ref()
    }

    /// The file at `path`, called `name`, is still coming down after [`BUSY_AFTER`]: the line
    /// says so.
    pub(crate) fn busy(&mut self, path: &str, name: &str, percent: Option<u8>) {
        self.shown += 1;
        self.note = Some(Note::Busy {
            path: path.to_owned(),
            name: name.to_owned(),
            percent,
        });
    }

    /// The download of `path` is at `percent`; whether the line changed (only while it says
    /// that download).
    pub(crate) fn progress(&mut self, path: &str, percent: Option<u8>) -> bool {
        match &mut self.note {
            Some(Note::Busy {
                path: shown,
                percent: was,
                ..
            }) if shown == path && *was != percent => {
                *was = percent;
                true
            }
            _ => false,
        }
    }

    /// The download of `path` ended; whether the line, which said it, is clear now.
    pub(crate) fn ended(&mut self, path: &str) -> bool {
        let said = matches!(&self.note, Some(Note::Busy { path: shown, .. }) if shown == path);
        if said {
            self.note = None;
        }
        said
    }

    /// Shows how a use went; gives its number, for [`Self::expire`] after [`SHOWN_FOR`].
    pub(crate) fn show(&mut self, text: impl Into<String>, failed: bool) -> u64 {
        self.shown += 1;
        self.note = Some(Note::Done {
            text: text.into(),
            failed,
        });
        self.shown
    }

    /// Note `number`'s time is up: it goes, unless another took its place; whether it went.
    pub(crate) fn expire(&mut self, number: u64) -> bool {
        let current = self.shown == number && matches!(self.note, Some(Note::Done { .. }));
        if current {
            self.note = None;
        }
        current
    }
}

/// How far `done` bytes of `total` are, in whole percent.
#[must_use]
pub(crate) fn percent(done: u64, total: Option<u64>) -> Option<u8> {
    let total = total.filter(|total| *total > 0)?;
    u8::try_from(done.min(total).saturating_mul(100) / total).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_download_is_announced_without_its_percent() {
        let mut status = FileStatus::default();
        status.busy("/files/plan.pdf", "plan.pdf", Some(10));
        let said = status.note().map(Note::spoken);
        assert_eq!(said.as_deref(), Some("Downloading plan.pdf…"));
        assert!(status.progress("/files/plan.pdf", Some(11)));
        assert_eq!(
            status.note().map(Note::text).as_deref(),
            Some("Downloading plan.pdf… 11%"),
            "the line shows the percent"
        );
        assert_eq!(
            status.note().map(Note::spoken),
            said,
            "the announced words stay the same"
        );
        status.show("Couldn't download plan.pdf", true);
        assert_eq!(
            status.note().map(Note::spoken).as_deref(),
            Some("Couldn't download plan.pdf"),
            "then how it went"
        );
    }

    #[test]
    fn a_download_reads_with_how_far_it_is_then_clears() {
        let mut status = FileStatus::default();
        assert_eq!(status.note(), None);
        status.busy("/a", "review-pack.pdf", None);
        assert_eq!(
            status.note().map(Note::text).as_deref(),
            Some("Downloading review-pack.pdf…")
        );
        assert!(status.progress("/a", Some(42)));
        assert!(!status.progress("/a", Some(42)), "nothing new");
        assert!(!status.progress("/b", Some(10)), "another file's");
        assert_eq!(
            status.note().map(Note::text).as_deref(),
            Some("Downloading review-pack.pdf… 42%")
        );
        assert!(!status.ended("/b"));
        assert!(status.ended("/a"));
        assert_eq!(status.note(), None);
    }

    #[test]
    fn a_result_goes_after_its_time_unless_another_took_its_place() {
        let mut status = FileStatus::default();
        let first = status.show("Copied", false);
        let second = status.show("Couldn't download a.pdf", true);
        assert!(!status.expire(first), "the newer one stays");
        assert_eq!(
            status.note(),
            Some(&Note::Done {
                text: "Couldn't download a.pdf".into(),
                failed: true
            })
        );
        assert!(status.expire(second));
        assert_eq!(status.note(), None);
        let third = status.show("Copied", false);
        status.busy("/a", "a.pdf", None);
        assert!(!status.expire(third), "a download took its place");
        assert!(status.note().is_some());
    }

    #[test]
    fn percent_is_whole_and_needs_a_size() {
        assert_eq!(percent(21, Some(50)), Some(42));
        assert_eq!(percent(70, Some(50)), Some(100));
        assert_eq!(percent(9, None), None);
        assert_eq!(percent(0, Some(0)), None);
    }
}
