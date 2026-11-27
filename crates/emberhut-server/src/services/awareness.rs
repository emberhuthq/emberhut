//! How fresh the assistant's picture is: each source it reads, when it last brought something,
//! and whether it is `ok`, `stale` or `down` (the state's `awareness`).
//!
//! The sources are the ones this server reads: the coding-thread app on each configured
//! machine (its threads, polled in the background). A machine whose app could not be reached
//! at the last look is down, with why; one whose app has not answered a poll for
//! [`STALE_AFTER`] (or never has) is stale, as its `threadsAt` says. A row's
//! time is that last answered poll.

use std::collections::BTreeMap;

use emberhut_api::wake::{AwarenessSource, SourceState};
use jiff::{SignedDuration, Timestamp};

use crate::convert::timestamp_to_api;
use crate::services::threads::{ThreadError, ThreadService};

/// A source whose last successful look is older than this is stale.
pub const STALE_AFTER: SignedDuration = SignedDuration::from_mins(5);

/// Says how fresh each source is.
#[derive(Clone, Debug)]
pub struct AwarenessService {
    threads: ThreadService,
}

impl AwarenessService {
    /// The sources behind `threads`.
    #[must_use]
    pub fn new(threads: ThreadService) -> Self {
        Self { threads }
    }

    /// Each source, as of `now`.
    ///
    /// # Errors
    ///
    /// [`ThreadError`] when the threads can't be read.
    pub async fn sources(&self, now: Timestamp) -> Result<Vec<AwarenessSource>, ThreadError> {
        let errors = self.threads.machine_errors();
        let polled = self.threads.polled_at();
        let threads = self.threads.recent().await?;
        let mut counts: BTreeMap<String, usize> = BTreeMap::new();
        for thread in &threads {
            *counts.entry(thread.machine.to_string()).or_default() += 1;
        }
        Ok(errors
            .into_iter()
            .map(|(machine, error)| {
                let count = counts.get(&machine).copied().unwrap_or_default();
                let at = polled.get(&machine).copied();
                source(&machine, count, at, error, now)
            })
            .collect())
    }
}

/// One coding-thread machine's row: `at` is when its app last answered a poll.
fn source(
    machine: &str,
    threads: usize,
    at: Option<Timestamp>,
    error: Option<String>,
    now: Timestamp,
) -> AwarenessSource {
    let fresh = at.is_some_and(|at| now.duration_since(at) <= STALE_AFTER);
    let state = if error.is_some() {
        SourceState::Down
    } else if fresh {
        SourceState::Ok
    } else {
        SourceState::Stale
    };
    let plural = if threads == 1 { "" } else { "s" };
    AwarenessSource {
        id: machine.to_owned(),
        name: format!("Coding threads on {machine}"),
        detail: format!("{threads} thread{plural}"),
        at: at.map(timestamp_to_api),
        error,
        state,
    }
}

#[cfg(test)]
mod tests {
    use emberhut_api::time::Timestamp as ApiTimestamp;

    use super::*;

    #[test]
    fn a_machine_is_down_with_an_error_stale_when_not_polled_lately_and_ok_otherwise() {
        let now: Timestamp = "2026-11-26T10:00:00Z".parse().unwrap();
        let polled: Timestamp = "2026-11-26T09:56:00Z".parse().unwrap();
        let earlier: Timestamp = "2026-11-26T09:54:00Z".parse().unwrap();
        let ok = source("pc", 2, Some(polled), None, now);
        assert_eq!(ok.state, SourceState::Ok);
        assert_eq!(
            ok.at.as_ref().map(ApiTimestamp::as_str),
            Some("2026-11-26T09:56:00.000Z")
        );
        assert_eq!(
            source("pc", 2, Some(earlier), None, now).state,
            SourceState::Stale
        );
        assert_eq!(source("pc", 0, None, None, now).state, SourceState::Stale);
        let down = source("pc", 1, Some(polled), Some("refused".to_owned()), now);
        assert_eq!(down.state, SourceState::Down);
        assert_eq!(down.detail, "1 thread");
        assert_eq!(down.name, "Coding threads on pc");
    }
}
