//! Which fetched pictures stay in memory. The open room's are always kept: they are what the
//! chat shows. Of the other rooms, the ones open last are kept too, newest first, while they
//! fit [`Limits`] (so going back to a room is instant); every other room's pictures are
//! dropped, and fetched again if that room is opened again. Kept for good instead, pictures
//! grew the app to 703 MB in 80 minutes.
//!
//! A room's pictures stay or go together. A picture several rooms show belongs to the newest
//! of them, so one the open room shows is never dropped.
//!
//! The limit this leaves: the open room's pictures are not bounded. Every picture in its held
//! chat is kept, decoded, since that is what the room can show: pages fetched with "Show
//! earlier" and rows never scrolled into view included. A decoded picture costs its pixels,
//! width × height × 4 bytes a frame (about 8 MB for a 1920 × 1080 screenshot, against a few
//! hundred KB encoded), so a long session in one room grows with the chat it holds. Bounding
//! that needs to know which rows are drawn (decode on first draw, or keep only the pictures
//! drawn last), which is a follow-up; what this module bounds is the other rooms' pictures.

use std::collections::{BTreeSet, HashMap};
use std::time::Instant;

use emberhut_api::ids::RoomId;

use super::fetched::{Failure, FetchCache, Fetched};

/// How much of the other rooms' pictures is kept.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Limits {
    /// How many rooms besides the open one.
    pub(crate) rooms: usize,
    /// How many bytes those rooms' pictures may take together.
    pub(crate) bytes: usize,
}

impl Limits {
    /// The chat's: the last three rooms besides the open one, within 64 MiB of decoded pixels
    /// between them (eight 1920 × 1080 screenshots, or one 5K one).
    pub(crate) const PICTURES: Self = Self {
        rooms: 3,
        bytes: 64 << 20,
    };
}

/// The memory a kept thing takes.
pub(crate) trait Weigh {
    /// Its size, in bytes.
    fn bytes(&self) -> usize;
}

/// Things fetched by path, kept for the rooms that show them.
#[derive(Debug)]
pub(crate) struct RoomCache<T> {
    cache: FetchCache<T>,
    /// The rooms each path is shown in, of the open and recent ones.
    shown_in: HashMap<String, BTreeSet<RoomId>>,
    open: RoomId,
    /// The rooms open before the open one, the last first (never the open one).
    recent: Vec<RoomId>,
    limits: Limits,
}

impl<T: Weigh> RoomCache<T> {
    /// An empty cache, with `open` the open room.
    #[must_use]
    pub(crate) fn new(open: RoomId, limits: Limits) -> Self {
        Self {
            cache: FetchCache::default(),
            shown_in: HashMap::new(),
            open,
            recent: Vec::new(),
            limits,
        }
    }

    /// What is known of `path`; `None` if it was never asked for, or was dropped.
    #[must_use]
    pub(crate) fn get(&self, path: &str) -> Option<&Fetched<T>> {
        self.cache.get(path)
    }

    /// How many paths are held, in any state.
    #[must_use]
    pub(crate) fn len(&self) -> usize {
        self.cache.len()
    }

    /// The memory the things here take.
    #[must_use]
    pub(crate) fn bytes(&self) -> usize {
        self.cache.ready().map(|(_, value)| value.bytes()).sum()
    }

    /// `room` shows `path`: whether to fetch it now (see [`FetchCache::begin`]). A room that is
    /// neither open nor recent keeps nothing, so nothing is fetched for it.
    pub(crate) fn begin(&mut self, room: RoomId, path: &str, now: Instant) -> bool {
        if self.rank(room).is_none() {
            return false;
        }
        self.shown_in
            .entry(path.to_owned())
            .or_default()
            .insert(room);
        self.cache.begin(path, now)
    }

    /// The fetch of `path` came back at `now`; whatever that leaves out is handed back to be
    /// freed. `outcome` is kept only while `path` is loading: if the rooms showing it were
    /// dropped while it was on its way, or another fetch of it came back first (the room was
    /// dropped and opened again meanwhile), `outcome` is handed back itself, and what is held
    /// stays.
    pub(crate) fn finish(
        &mut self,
        path: String,
        outcome: Result<T, Failure>,
        now: Instant,
    ) -> Vec<T> {
        if !matches!(self.cache.get(&path), Some(Fetched::Loading)) {
            return outcome.into_iter().collect();
        }
        self.cache.finish(path, outcome, now);
        self.trim()
    }

    /// `room` is opened; the pictures that leaves no room for are handed back to be freed.
    pub(crate) fn open(&mut self, room: RoomId) -> Vec<T> {
        if room == self.open {
            return Vec::new();
        }
        self.recent.retain(|&recent| recent != room);
        self.recent.insert(0, self.open);
        self.open = room;
        self.trim()
    }

    /// Where `room` stands: 0 for the open room, then 1 for the room open last, and so on;
    /// `None` for a room whose things aren't kept.
    fn rank(&self, room: RoomId) -> Option<usize> {
        if room == self.open {
            return Some(0);
        }
        self.recent
            .iter()
            .position(|&recent| recent == room)
            .map(|at| at + 1)
    }

    /// Forgets the recent rooms past the limits, and drops what only they show.
    fn trim(&mut self) -> Vec<T> {
        // What each recent room's own pictures weigh (a picture counts for the newest room).
        let mut weights = vec![0; self.recent.len()];
        for (path, rooms) in &self.shown_in {
            let newest = rooms.iter().filter_map(|&room| self.rank(room)).min();
            if let Some(rank @ 1..) = newest
                && let Some(Fetched::Ready(value)) = self.cache.get(path)
            {
                weights[rank - 1] += value.bytes();
            }
        }
        let mut spent = 0;
        let mut kept = 0;
        for weight in weights.into_iter().take(self.limits.rooms) {
            spent += weight;
            if spent > self.limits.bytes {
                break;
            }
            kept += 1;
        }
        self.recent.truncate(kept);

        let Self {
            cache,
            shown_in,
            open,
            recent,
            ..
        } = self;
        let mut dropped = Vec::new();
        shown_in.retain(|path, rooms| {
            rooms.retain(|room| room == open || recent.contains(room));
            if !rooms.is_empty() {
                return true;
            }
            if let Some(Fetched::Ready(value)) = cache.remove(path) {
                dropped.push(value);
            }
            false
        });
        dropped
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAIN: RoomId = RoomId::MAIN;

    /// A picture of so many bytes.
    #[derive(Debug, PartialEq, Eq)]
    struct Picture(usize);

    impl Weigh for Picture {
        fn bytes(&self) -> usize {
            self.0
        }
    }

    fn room(id: i64) -> RoomId {
        RoomId::new(id)
    }

    fn cache(limits: Limits) -> RoomCache<Picture> {
        RoomCache::new(MAIN, limits)
    }

    /// `room` shows `path`, `bytes` big, and it came.
    fn show(
        cache: &mut RoomCache<Picture>,
        room: RoomId,
        path: &str,
        bytes: usize,
    ) -> Vec<Picture> {
        let now = Instant::now();
        assert!(cache.begin(room, path, now), "{path} is fetched");
        cache.finish(path.to_owned(), Ok(Picture(bytes)), now)
    }

    fn here(cache: &RoomCache<Picture>, path: &str) -> bool {
        matches!(cache.get(path), Some(Fetched::Ready(_)))
    }

    const ROOMS: Limits = Limits {
        rooms: 2,
        bytes: usize::MAX,
    };

    #[test]
    fn the_open_rooms_pictures_are_kept_whatever_they_weigh() {
        let mut cache = cache(Limits { rooms: 0, bytes: 0 });
        assert_eq!(show(&mut cache, MAIN, "/a", 100), []);
        assert_eq!(show(&mut cache, MAIN, "/b", 100), []);
        assert!(here(&cache, "/a") && here(&cache, "/b"));
        assert_eq!((cache.len(), cache.bytes()), (2, 200));
    }

    #[test]
    fn the_last_rooms_are_kept_and_older_ones_dropped() {
        let mut cache = cache(ROOMS);
        show(&mut cache, MAIN, "/main", 1);
        for id in 2..=3 {
            assert_eq!(cache.open(room(id)), []);
            show(&mut cache, room(id), &format!("/r{id}"), 1);
        }
        assert_eq!(
            (cache.len(), cache.bytes()),
            (3, 3),
            "Main and room 2 are recent"
        );

        assert_eq!(
            cache.open(room(4)),
            [Picture(1)],
            "Main's, now three rooms back"
        );
        assert_eq!(cache.get("/main"), None);
        assert!(here(&cache, "/r2") && here(&cache, "/r3"));
        assert_eq!((cache.len(), cache.bytes()), (2, 2));
    }

    #[test]
    fn going_back_to_a_recent_room_keeps_it() {
        let mut cache = cache(ROOMS);
        show(&mut cache, MAIN, "/main", 1);
        cache.open(room(2));
        show(&mut cache, room(2), "/r2", 1);
        assert_eq!(cache.open(MAIN), []);
        assert_eq!(cache.open(room(3)), [], "room 2 was open just before Main");
        assert!(here(&cache, "/main") && here(&cache, "/r2"));
    }

    #[test]
    fn other_rooms_past_the_byte_budget_are_dropped() {
        let mut cache = cache(Limits {
            rooms: 5,
            bytes: 10,
        });
        show(&mut cache, MAIN, "/main", 6);
        cache.open(room(2));
        show(&mut cache, room(2), "/r2", 6);
        assert_eq!(cache.bytes(), 12, "Main's 6 bytes fit");

        assert_eq!(
            cache.open(room(3)),
            [Picture(6)],
            "Main's 6 and room 2's 6 don't"
        );
        assert_eq!(cache.get("/main"), None);
        assert!(here(&cache, "/r2"));
        assert_eq!((cache.len(), cache.bytes()), (1, 6));
    }

    #[test]
    fn a_picture_the_open_room_shows_is_never_dropped() {
        let mut cache = cache(Limits { rooms: 1, bytes: 5 });
        show(&mut cache, MAIN, "/shared", 1);
        cache.open(room(2));
        assert!(
            !cache.begin(room(2), "/shared", Instant::now()),
            "already here"
        );
        show(&mut cache, room(2), "/r2", 1);
        assert_eq!(cache.open(MAIN), []);

        // Room 2's earlier chat comes late, with more than the budget.
        let mut dropped = show(&mut cache, room(2), "/big", 10);
        dropped.sort_by_key(|picture| picture.0);
        assert_eq!(dropped, [Picture(1), Picture(10)], "room 2's own");
        assert!(here(&cache, "/shared"), "Main shows it");
        assert_eq!((cache.len(), cache.bytes()), (1, 1));
    }

    #[test]
    fn a_picture_stays_while_a_kept_room_shows_it() {
        let mut cache = cache(Limits {
            rooms: 1,
            bytes: usize::MAX,
        });
        show(&mut cache, MAIN, "/shared", 1);
        show(&mut cache, MAIN, "/main", 2);
        cache.open(room(2));
        assert!(!cache.begin(room(2), "/shared", Instant::now()));
        assert_eq!(cache.open(room(3)), [Picture(2)], "Main's own");
        assert!(here(&cache, "/shared"), "room 2 is kept");
        assert_eq!(cache.open(room(4)), [Picture(1)], "room 2 went");
        assert_eq!(cache.len(), 0);
    }

    #[test]
    fn a_picture_that_comes_after_its_room_went_is_not_kept() {
        let mut cache = cache(Limits { rooms: 0, bytes: 0 });
        let now = Instant::now();
        assert!(cache.begin(MAIN, "/late", now));
        assert_eq!(cache.open(room(2)), [], "nothing came yet");
        assert_eq!(cache.get("/late"), None, "Loading is dropped too");
        assert_eq!(
            cache.finish("/late".into(), Ok(Picture(5)), now),
            [Picture(5)],
            "handed back to be freed"
        );
        assert_eq!((cache.len(), cache.bytes()), (0, 0));
    }

    #[test]
    fn a_second_fetch_of_a_picture_never_displaces_the_first() {
        let mut cache = cache(Limits { rooms: 0, bytes: 0 });
        let now = Instant::now();
        assert!(cache.begin(MAIN, "/slow", now));
        cache.open(room(2));
        cache.open(MAIN);
        assert!(
            cache.begin(MAIN, "/slow", now),
            "dropped, so asked for again"
        );

        assert_eq!(cache.finish("/slow".into(), Ok(Picture(1)), now), []);
        assert_eq!(
            cache.finish("/slow".into(), Ok(Picture(2)), now),
            [Picture(2)],
            "the second is handed back, not kept over the first"
        );
        assert_eq!(cache.get("/slow"), Some(&Fetched::Ready(Picture(1))));
        assert_eq!((cache.len(), cache.bytes()), (1, 1));
        assert_eq!(
            cache.finish("/slow".into(), Err(Failure::passing("gone")), now),
            [],
            "a late failure leaves it too"
        );
        assert_eq!(cache.get("/slow"), Some(&Fetched::Ready(Picture(1))));
    }

    #[test]
    fn nothing_is_fetched_for_a_room_that_keeps_nothing() {
        let mut cache = cache(ROOMS);
        assert!(!cache.begin(room(9), "/r9", Instant::now()));
        assert_eq!(cache.len(), 0);
    }

    #[test]
    fn a_dropped_room_fetches_its_pictures_again_when_opened() {
        let mut cache = cache(Limits { rooms: 0, bytes: 0 });
        show(&mut cache, MAIN, "/main", 1);
        cache.open(room(2));
        cache.open(MAIN);
        assert_eq!(show(&mut cache, MAIN, "/main", 1), []);
        assert!(here(&cache, "/main"));
    }
}
