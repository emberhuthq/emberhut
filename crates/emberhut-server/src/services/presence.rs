//! Which rooms the person has on screen right now ([`emberhut_api::headers::LOOKING`]), so that replies
//! in a room being read are not pushed to the phones
//! ([`PushService::reply`](crate::services::push::PushService::reply)).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use emberhut_core::{Clock, RoomId};
use jiff::{SignedDuration, Timestamp};

/// How long a room counts as looked at after a poll said so.
pub const LOOKING_FOR: SignedDuration = SignedDuration::from_secs(10);

/// When each room was last said to be on screen.
#[derive(Clone, Debug)]
pub struct Presence {
    clock: Arc<dyn Clock>,
    looked: Arc<Mutex<HashMap<RoomId, Timestamp>>>,
}

impl Presence {
    /// Presence that tells the time with `clock`.
    #[must_use]
    pub fn new(clock: Arc<dyn Clock>) -> Self {
        Self {
            clock,
            looked: Arc::default(),
        }
    }

    /// Notes that `room` is on screen in front of the person now.
    pub fn looking(&self, room: RoomId) {
        let now = self.clock.now();
        self.looked
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(room, now);
    }

    /// Whether `room` was on screen within the last [`LOOKING_FOR`].
    #[must_use]
    pub fn is_looked_at(&self, room: RoomId) -> bool {
        let now = self.clock.now();
        self.looked
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&room)
            .is_some_and(|at| now - *at < LOOKING_FOR)
    }
}

#[cfg(test)]
mod tests {
    use emberhut_core::ManualClock;

    use super::*;

    #[test]
    fn a_room_is_looked_at_for_ten_seconds() {
        let clock = Arc::new(ManualClock::new(
            Timestamp::from_second(1_800_000_000).unwrap(),
        ));
        let presence = Presence::new(Arc::clone(&clock) as Arc<dyn Clock>);
        assert!(!presence.is_looked_at(RoomId::MAIN));
        presence.looking(RoomId::MAIN);
        assert!(presence.is_looked_at(RoomId::MAIN));
        assert!(!presence.is_looked_at(RoomId::new(2)));
        clock.advance(SignedDuration::from_secs(10));
        assert!(!presence.is_looked_at(RoomId::MAIN));
    }
}
