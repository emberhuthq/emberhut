//! What the assistant is doing in each room, for the state's `live` and the live stream.
//!
//! Each room has a [`watch`] channel holding its current value: a subscriber always sees the
//! latest value, and one that falls behind skips straight to it rather than replaying stale
//! steps. `None` is "nothing is being written" (a turn ended, or none runs).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use emberhut_api::live::Live;
use emberhut_core::RoomId;
use tokio::sync::watch;

/// Every room's live value.
#[derive(Clone, Debug, Default)]
pub struct LiveHub {
    rooms: Arc<Mutex<HashMap<RoomId, watch::Sender<Option<Live>>>>>,
}

impl LiveHub {
    /// Sets `room`'s live value; subscribers hear of it only if it changed.
    pub fn set(&self, room: RoomId, live: Option<Live>) {
        self.channels()
            .entry(room)
            .or_insert_with(|| watch::Sender::new(None))
            .send_if_modified(|current| {
                if *current == live {
                    return false;
                }
                *current = live;
                true
            });
    }

    /// Changes `room`'s live value with `change` (starting from an empty one when nothing
    /// runs); subscribers hear of it only if it changed.
    pub fn update(&self, room: RoomId, change: impl FnOnce(&mut Live)) {
        self.channels()
            .entry(room)
            .or_insert_with(|| watch::Sender::new(None))
            .send_if_modified(|current| {
                let mut live = current.clone().unwrap_or_default();
                change(&mut live);
                if current.as_ref() == Some(&live) {
                    return false;
                }
                *current = Some(live);
                true
            });
    }

    /// `room`'s live value now.
    #[must_use]
    pub fn now(&self, room: RoomId) -> Option<Live> {
        self.channels()
            .get(&room)
            .and_then(|channel| channel.borrow().clone())
    }

    /// A receiver of `room`'s live value: its current value, then each change.
    #[must_use]
    pub fn subscribe(&self, room: RoomId) -> watch::Receiver<Option<Live>> {
        self.channels()
            .entry(room)
            .or_insert_with(|| watch::Sender::new(None))
            .subscribe()
    }

    fn channels(&self) -> MutexGuard<'_, HashMap<RoomId, watch::Sender<Option<Live>>>> {
        // A map of channels: a panic elsewhere cannot leave it half-written.
        self.rooms.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn working(step: &str) -> Live {
        Live {
            text: String::new(),
            doing: Some(step.to_owned()),
        }
    }

    #[tokio::test]
    async fn subscribers_see_the_current_value_then_changes() {
        let hub = LiveHub::default();
        let room = RoomId::MAIN;
        assert_eq!(hub.now(room), None);
        hub.set(room, Some(working("Reading")));
        let mut live = hub.subscribe(room);
        assert_eq!(*live.borrow_and_update(), Some(working("Reading")));
        hub.set(room, None);
        live.changed().await.unwrap();
        assert_eq!(*live.borrow_and_update(), None);
        assert_eq!(hub.now(room), None);
    }

    #[tokio::test]
    async fn setting_the_same_value_again_is_not_a_change() {
        let hub = LiveHub::default();
        let mut live = hub.subscribe(RoomId::MAIN);
        live.borrow_and_update();
        hub.set(RoomId::MAIN, None);
        assert!(!live.has_changed().unwrap());
        hub.set(RoomId::MAIN, Some(working("Reading")));
        assert!(live.has_changed().unwrap());
    }

    #[test]
    fn rooms_are_apart() {
        let hub = LiveHub::default();
        hub.set(RoomId::new(2), Some(working("Elsewhere")));
        assert_eq!(hub.now(RoomId::MAIN), None);
        assert_eq!(hub.now(RoomId::new(2)), Some(working("Elsewhere")));
    }
}
