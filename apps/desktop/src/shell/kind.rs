//! The windows of their own, besides the main ones: each opens once (opening it again brings
//! it forward), at its own size the first time.

/// A window of its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum WindowKind {
    /// The assistant's browser, live.
    Browser,
    /// The calendar, wide enough for a week.
    Calendar,
    /// The user's own coding sessions.
    Sessions,
    /// The Dark room: off the record.
    Dark,
    /// The diary: quick notes by day.
    Diary,
    /// Party: personas talking to each other.
    Party,
    /// A quick note.
    QuickNote,
}

impl WindowKind {
    /// Every kind.
    pub(crate) const ALL: [Self; 7] = [
        Self::Browser,
        Self::Calendar,
        Self::Sessions,
        Self::Dark,
        Self::Diary,
        Self::Party,
        Self::QuickNote,
    ];

    /// The window's title.
    #[must_use]
    pub(crate) fn title(self) -> &'static str {
        match self {
            Self::Browser => "Browser",
            Self::Calendar => "Calendar",
            Self::Sessions => "Sessions",
            Self::Dark => "Dark",
            Self::Diary => "Diary",
            Self::Party => "Party",
            Self::QuickNote => "Quick Note",
        }
    }

    /// Its size the first time it opens, in logical pixels (the reference client's).
    #[must_use]
    pub(crate) fn default_size(self) -> (f32, f32) {
        match self {
            Self::Browser => (1280., 860.),
            Self::Calendar => (1180., 820.),
            Self::Sessions => (780., 900.),
            Self::Dark => (720., 760.),
            Self::Diary => (560., 760.),
            Self::Party => (660., 880.),
            Self::QuickNote => (460., 280.),
        }
    }

    /// The name links and the command line use for it (`emberhut://diary`, `--window diary`).
    #[must_use]
    pub(crate) fn link_name(self) -> &'static str {
        match self {
            Self::Browser => "browser",
            Self::Calendar => "calendar",
            Self::Sessions => "sessions",
            Self::Dark => "dark",
            Self::Diary => "diary",
            Self::Party => "party",
            Self::QuickNote => "note",
        }
    }

    /// The kind a link or the command line names.
    #[must_use]
    pub(crate) fn from_link_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.link_name() == name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_kind_is_named_once_and_found_by_its_name() {
        for kind in WindowKind::ALL {
            assert_eq!(WindowKind::from_link_name(kind.link_name()), Some(kind));
            let (width, height) = kind.default_size();
            assert!(width > 0. && height > 0.);
        }
        assert_eq!(WindowKind::from_link_name("chat"), None);
    }
}
