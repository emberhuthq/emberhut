//! Report mode's notes: which requests came without a session, so the owner can see what
//! enforcing would refuse. Once an hour per address, so a polling client doesn't fill the log.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Arc, Mutex, PoisonError};

use emberhut_core::Clock;
use jiff::{SignedDuration, Timestamp};

/// How long an address stays noted before its next request without a session is logged again.
const NOTE_EVERY: SignedDuration = SignedDuration::from_hours(1);

/// When each address was last noted.
#[derive(Clone, Debug)]
pub struct ReportLog {
    clock: Arc<dyn Clock>,
    noted: Arc<Mutex<HashMap<Option<IpAddr>, Timestamp>>>,
}

impl ReportLog {
    /// A log that tells the time with `clock`.
    #[must_use]
    pub fn new(clock: Arc<dyn Clock>) -> Self {
        Self {
            clock,
            noted: Arc::default(),
        }
    }

    /// Notes that a request from `peer` (when known) to `path` had no session; `true` when it
    /// was logged, `false` when that address was logged within the hour.
    pub fn note(&self, peer: Option<IpAddr>, path: &str) -> bool {
        let now = self.clock.now();
        let mut noted = self.noted.lock().unwrap_or_else(PoisonError::into_inner);
        if noted
            .get(&peer)
            .is_some_and(|last| now - *last < NOTE_EVERY)
        {
            return false;
        }
        noted.insert(peer, now);
        let address = peer.map_or_else(|| "an unknown address".to_owned(), |ip| ip.to_string());
        tracing::info!(
            %address,
            path,
            "report mode: a request had no session; enforcing would refuse it"
        );
        true
    }
}

#[cfg(test)]
mod tests {
    use std::net::Ipv4Addr;

    use emberhut_core::ManualClock;

    use super::*;

    #[test]
    fn each_address_is_noted_once_an_hour() {
        let clock = Arc::new(ManualClock::new(
            Timestamp::from_second(1_800_000_000).unwrap(),
        ));
        let log = ReportLog::new(Arc::clone(&clock) as Arc<dyn Clock>);
        let here = Some(IpAddr::V4(Ipv4Addr::LOCALHOST));
        assert!(log.note(here, "/api/state"));
        assert!(!log.note(here, "/api/live"));
        assert!(log.note(None, "/api/state"), "another address");
        clock.advance(SignedDuration::from_mins(60));
        assert!(log.note(here, "/api/state"));
    }
}
