//! What the app measures about its own polling, for Settings and the benchmarks.

use std::time::{Duration, Instant};

/// The poll's numbers since launch.
#[derive(Clone, Debug)]
pub(crate) struct Metrics {
    started: Instant,
    first_state: Option<Duration>,
    last_fetch: Option<Duration>,
    polls: u64,
    unchanged: u64,
}

impl Default for Metrics {
    fn default() -> Self {
        Self {
            started: Instant::now(),
            first_state: None,
            last_fetch: None,
            polls: 0,
            unchanged: 0,
        }
    }
}

impl Metrics {
    /// A poll came back.
    pub(crate) fn polled(&mut self) {
        self.polls += 1;
    }

    /// It came back unchanged (a 304).
    pub(crate) fn unchanged(&mut self) {
        self.unchanged += 1;
    }

    /// It came back with a new state, fetched and decoded in `took`.
    pub(crate) fn fresh(&mut self, took: Duration) {
        self.last_fetch = Some(took);
        self.first_state
            .get_or_insert_with(|| self.started.elapsed());
    }

    /// From launch to the first state.
    #[must_use]
    pub(crate) fn first_state(&self) -> Option<Duration> {
        self.first_state
    }

    /// How long the last new state took to fetch and decode.
    #[must_use]
    pub(crate) fn last_fetch(&self) -> Option<Duration> {
        self.last_fetch
    }

    /// Polls so far.
    #[must_use]
    pub(crate) fn polls(&self) -> u64 {
        self.polls
    }

    /// Polls that came back unchanged.
    #[must_use]
    pub(crate) fn unchanged_polls(&self) -> u64 {
        self.unchanged
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_state_is_timed_once() {
        let mut metrics = Metrics::default();
        metrics.polled();
        metrics.fresh(Duration::from_millis(5));
        let first = metrics.first_state();
        metrics.polled();
        metrics.unchanged();
        metrics.fresh(Duration::from_millis(7));
        assert_eq!(metrics.first_state(), first);
        assert_eq!((metrics.polls(), metrics.unchanged_polls()), (2, 1));
        assert_eq!(metrics.last_fetch(), Some(Duration::from_millis(7)));
    }
}
