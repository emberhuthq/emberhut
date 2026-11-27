//! The live stream of the assistant's steps in the open room. Each room is followed on a thread
//! of its own (a long-lived blocking stream): a room switch starts the new room's stream at
//! once, and the old one ends at its next line, an event or the server's ping, so within
//! [`PING_INTERVAL_SECS`] (or the client's live read timeout if the pings stop). Until then it
//! holds its thread and its connection, and anything it reads is dropped. emberhut-client offers no
//! way to close a stream from another thread; that bound is the cost, and flicking through
//! rooms leaves one winding-down stream per switch for those seconds.
//!
//! The stream also says when an approval request changes, whichever room it follows: what
//! shows approvals is fetched again then (`approvals`).

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use emberhut_api::ids::RoomId;
use emberhut_api::live::LiveEvent;
#[cfg(doc)]
use emberhut_api::live::PING_INTERVAL_SECS;
use gpui_kit::Context;

use super::{Store, update_or_log};
use crate::backend::{Backend, RoomEvent};

/// How long to wait before following the stream again after it ended or failed.
const RECONNECT_AFTER: Duration = Duration::from_secs(2);

/// An event of a room's stream, sent from its thread to the store.
type Sent = (RoomId, RoomEvent);

/// The live stream's side of the store.
pub(crate) struct LiveFeed {
    backend: Arc<dyn Backend>,
    /// Bumped on every [`LiveFeed::follow`]: a thread whose number is no longer current stops.
    generation: Arc<AtomicU64>,
    /// Where the threads send; `None` until [`LiveFeed::start`].
    events: Option<smol::channel::Sender<Sent>>,
}

impl LiveFeed {
    /// The feed, not following anything yet.
    pub(crate) fn new(backend: Arc<dyn Backend>) -> Self {
        Self {
            backend,
            generation: Arc::new(AtomicU64::new(0)),
            events: None,
        }
    }

    /// Starts taking the threads' events into the store in `cx`.
    pub(crate) fn start(&mut self, cx: &mut Context<'_, Store>) {
        let (events, received) = smol::channel::unbounded::<Sent>();
        self.events = Some(events);
        cx.spawn(async move |this, cx| {
            while let Ok((room, event)) = received.recv().await {
                let taken = update_or_log(&this, cx, |store, cx| match event {
                    RoomEvent::Live(live) => store.take_live(room, live, cx),
                    RoomEvent::Approvals(changed) => store.approvals_changed(&changed, cx),
                });
                if taken.is_none() {
                    return;
                }
            }
        })
        .detach();
    }

    /// Follows `room` from now on (once started); any earlier room's thread stops.
    pub(crate) fn follow(&self, room: RoomId) {
        let mine = self.generation.fetch_add(1, Ordering::Relaxed) + 1;
        let Some(events) = self.events.clone() else {
            return;
        };
        let (backend, generation) = (Arc::clone(&self.backend), Arc::clone(&self.generation));
        let thread = std::thread::Builder::new()
            .name(format!("live-{room}"))
            .spawn(move || follow(&*backend, room, &generation, mine, &events));
        if let Err(error) = thread {
            log::error!("couldn't start the live stream's thread: {error}");
        }
    }
}

impl LiveFeed {
    /// Follows `room` through `backend` from now on (another server's): the thread on the old
    /// one stops at its next line. Nothing is followed until the feed is started.
    pub(crate) fn switch(&mut self, backend: Arc<dyn Backend>, room: RoomId) {
        self.backend = backend;
        self.follow(room);
    }

    /// Follows nothing any more: the room's thread stops at its next line, and none starts
    /// again until [`LiveFeed::follow`].
    pub(crate) fn halt(&self) {
        self.generation.fetch_add(1, Ordering::Relaxed);
    }
}

impl Drop for LiveFeed {
    /// A store that goes away stops its room's thread too (at the thread's next line).
    fn drop(&mut self) {
        self.generation.fetch_add(1, Ordering::Relaxed);
    }
}

/// A room's thread: follows its stream, again after each end, while `mine` is current.
fn follow(
    backend: &dyn Backend,
    room: RoomId,
    generation: &AtomicU64,
    mine: u64,
    events: &smol::channel::Sender<Sent>,
) {
    let current = || generation.load(Ordering::Relaxed) == mine;
    while current() {
        let mut open = true;
        let outcome = backend.follow_live(room, &|| !current(), &mut |event| {
            open = events.send_blocking((room, event)).is_ok();
        });
        if let Err(error) = outcome {
            log::debug!("room {room}'s live stream broke off: {error}");
        }
        if !current() {
            return;
        }
        // However it ended, the turn shows as over until the stream says otherwise.
        let over = RoomEvent::Live(LiveEvent::Idle);
        if !open || events.send_blocking((room, over)).is_err() {
            return; // the store is gone
        }
        std::thread::sleep(RECONNECT_AFTER);
    }
}

impl Store {
    pub(super) fn take_live(&mut self, room: RoomId, event: LiveEvent, cx: &mut Context<'_, Self>) {
        if room != self.room {
            return;
        }
        let live = match event {
            LiveEvent::Working(live) => Some(live),
            LiveEvent::Idle => {
                // The turn is over: the answer is in the server's chat already, so fetch it now.
                if self.live.is_some() {
                    self.kicker.kick();
                }
                None
            }
        };
        if self.live != live {
            self.live = live;
            cx.notify();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use emberhut_api::live::Live;

    use super::*;
    use crate::backend::fake::FakeBackend;

    /// Waits (up to a second) for `thread` to end; whether it did.
    fn ends(thread: &std::thread::JoinHandle<()>) -> bool {
        let deadline = Instant::now() + Duration::from_secs(1);
        while !thread.is_finished() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        thread.is_finished()
    }

    #[test]
    fn a_switch_ends_the_old_rooms_thread_and_drops_its_end() {
        let step = Live {
            text: String::new(),
            doing: Some("Reading".into()),
        };
        let backend =
            Arc::new(FakeBackend::new([]).with_live(vec![LiveEvent::Working(step.clone())]));
        let generation = Arc::new(AtomicU64::new(1));
        let (events, received) = smol::channel::unbounded();
        let thread = {
            let (backend, generation) = (Arc::clone(&backend), Arc::clone(&generation));
            std::thread::spawn(move || follow(&*backend, RoomId::new(2), &generation, 1, &events))
        };
        assert_eq!(
            received.recv_blocking().unwrap(),
            (RoomId::new(2), RoomEvent::Live(LiveEvent::Working(step)))
        );
        generation.fetch_add(1, Ordering::Relaxed); // the user switched rooms
        assert!(ends(&thread), "the old room's thread ended");
        assert!(
            received.try_recv().is_err(),
            "no \"turn over\" for a room no longer open"
        );
    }

    #[test]
    fn dropping_the_feed_stops_its_thread() {
        let backend: Arc<dyn Backend> = Arc::new(FakeBackend::new([]));
        let feed = LiveFeed::new(Arc::clone(&backend));
        let generation = Arc::clone(&feed.generation);
        let before = generation.load(Ordering::Relaxed);
        drop(feed);
        assert_ne!(generation.load(Ordering::Relaxed), before);
    }
}
