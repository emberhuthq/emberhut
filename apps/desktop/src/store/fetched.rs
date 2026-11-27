//! A cache of things fetched from the server by path (pictures, reports): each is asked for
//! once and what came back is kept until it is removed; a failure is kept too. One that may
//! pass (a network blip) is asked for again once it is [`RETRY_AFTER`] old, so it doesn't leave
//! a hole for good; one that would come out the same (a file that isn't a picture) is not, so
//! it isn't downloaded again and again.
//! Which pictures are removed, and when, is [`super::retention`]'s decision.

use std::collections::HashMap;
use std::fmt;
use std::time::{Duration, Instant};

/// How long a failed fetch is remembered before it may be tried again.
pub(crate) const RETRY_AFTER: Duration = Duration::from_secs(30);

/// Where a fetch stands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Fetched<T> {
    /// On its way.
    Loading,
    /// Here.
    Ready(T),
    /// It failed, with the reason worded for the user.
    Failed(String),
}

/// Why a fetch failed, and whether asking again may help.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Failure {
    reason: String,
    lasting: bool,
}

impl Failure {
    /// A failure that may pass (the server couldn't be reached, or refused for now): asked for
    /// again once it is [`RETRY_AFTER`] old. `reason` is worded for the user.
    #[must_use]
    pub(crate) fn passing(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
            lasting: false,
        }
    }

    /// A failure that asking again would only repeat (what came can't be read): never asked
    /// for again while it is held. `reason` is worded for the user.
    #[must_use]
    pub(crate) fn lasting(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
            lasting: true,
        }
    }
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.reason)
    }
}

#[derive(Debug)]
struct Entry<T> {
    fetched: Fetched<T>,
    /// From when it may be asked for again: only for a failure that may pass.
    retry_from: Option<Instant>,
}

/// Fetched things by path.
#[derive(Debug)]
pub(crate) struct FetchCache<T> {
    entries: HashMap<String, Entry<T>>,
}

impl<T> Default for FetchCache<T> {
    fn default() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }
}

impl<T> FetchCache<T> {
    /// What is known of `path`; `None` if it was never asked for.
    #[must_use]
    pub(crate) fn get(&self, path: &str) -> Option<&Fetched<T>> {
        self.entries.get(path).map(|entry| &entry.fetched)
    }

    /// Whether to fetch `path` now (`now` being the time): it was never asked for, or it failed
    /// long enough ago. When it says yes, `path` is marked loading.
    pub(crate) fn begin(&mut self, path: &str, now: Instant) -> bool {
        let due = match self.entries.get(path) {
            None => true,
            Some(entry) => entry.retry_from.is_some_and(|from| now >= from),
        };
        if due {
            self.entries.insert(
                path.to_owned(),
                Entry {
                    fetched: Fetched::Loading,
                    retry_from: None,
                },
            );
        }
        due
    }

    /// Forgets `path`, handing back what was known of it.
    pub(crate) fn remove(&mut self, path: &str) -> Option<Fetched<T>> {
        self.entries.remove(path).map(|entry| entry.fetched)
    }

    /// How many paths are known.
    #[must_use]
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    /// What is here, by path.
    pub(crate) fn ready(&self) -> impl Iterator<Item = (&str, &T)> {
        self.entries
            .iter()
            .filter_map(|(path, entry)| match &entry.fetched {
                Fetched::Ready(value) => Some((path.as_str(), value)),
                Fetched::Loading | Fetched::Failed(_) => None,
            })
    }

    /// The fetch of `path` came back at `now`.
    pub(crate) fn finish(&mut self, path: String, outcome: Result<T, Failure>, now: Instant) {
        let entry = match outcome {
            Ok(value) => Entry {
                fetched: Fetched::Ready(value),
                retry_from: None,
            },
            Err(Failure { reason, lasting }) => Entry {
                fetched: Fetched::Failed(reason),
                // Past the end of time is never, too.
                retry_from: if lasting {
                    None
                } else {
                    now.checked_add(RETRY_AFTER)
                },
            },
        };
        self.entries.insert(path, entry);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_is_fetched_once() {
        let (mut cache, now) = (FetchCache::<u8>::default(), Instant::now());
        assert_eq!(cache.get("/a"), None);
        assert!(cache.begin("/a", now));
        assert_eq!(cache.get("/a"), Some(&Fetched::Loading));
        assert!(!cache.begin("/a", now), "already on its way");
        cache.finish("/a".into(), Ok(7), now);
        assert_eq!(cache.get("/a"), Some(&Fetched::Ready(7)));
        assert!(!cache.begin("/a", now + RETRY_AFTER * 10), "here for good");
    }

    #[test]
    fn a_removed_path_is_forgotten_and_fetched_again() {
        let (mut cache, now) = (FetchCache::<u8>::default(), Instant::now());
        assert!(cache.begin("/a", now));
        cache.finish("/a".into(), Ok(7), now);
        assert!(cache.begin("/b", now));
        assert_eq!(cache.len(), 2);
        assert_eq!(cache.ready().collect::<Vec<_>>(), [("/a", &7)]);
        assert_eq!(cache.remove("/a"), Some(Fetched::Ready(7)));
        assert_eq!((cache.len(), cache.get("/a")), (1, None));
        assert_eq!(cache.remove("/a"), None);
        assert!(cache.begin("/a", now), "asked for again");
    }

    #[test]
    fn a_failure_is_kept_with_its_reason_then_tried_again() {
        let (mut cache, now) = (FetchCache::<u8>::default(), Instant::now());
        assert!(cache.begin("/b", now));
        cache.finish("/b".into(), Err(Failure::passing("gone")), now);
        assert_eq!(cache.get("/b"), Some(&Fetched::Failed("gone".into())));
        assert!(!cache.begin("/b", now + RETRY_AFTER / 2), "too soon");
        assert!(cache.begin("/b", now + RETRY_AFTER));
        assert_eq!(cache.get("/b"), Some(&Fetched::Loading));
    }

    #[test]
    fn a_lasting_failure_is_never_tried_again() {
        let (mut cache, now) = (FetchCache::<u8>::default(), Instant::now());
        assert!(cache.begin("/c", now));
        cache.finish("/c".into(), Err(Failure::lasting("not a picture")), now);
        assert_eq!(
            cache.get("/c"),
            Some(&Fetched::Failed("not a picture".into()))
        );
        assert!(!cache.begin("/c", now + RETRY_AFTER * 10));
        assert_eq!(
            cache.remove("/c"),
            Some(Fetched::Failed("not a picture".into()))
        );
        assert!(cache.begin("/c", now), "once forgotten, asked for again");
    }
}
