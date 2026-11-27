//! Each room's chat as held on this device: the state's newest page joined to what came
//! before, and earlier pages added on request. The server's chat only ever grows, so what is
//! held stays correct as pages arrive.

use std::collections::{HashMap, HashSet};

use emberhut_api::chat::ChatMessage;
use emberhut_api::ids::{MessageId, RoomId};

pub(crate) use emberhut_api::chat::PAGE_SIZE;

/// The chats held, by room.
#[derive(Debug, Default)]
pub(crate) struct Chats {
    held: HashMap<RoomId, Vec<ChatMessage>>,
    /// Rooms whose first message is held: no earlier page to ask for.
    complete: HashSet<RoomId>,
    /// Rooms whose earlier page is on its way.
    loading_earlier: HashSet<RoomId>,
}

impl Chats {
    /// A room's messages, oldest first.
    #[must_use]
    pub(crate) fn messages(&self, room: RoomId) -> &[ChatMessage] {
        self.held.get(&room).map_or(&[], Vec::as_slice)
    }

    /// Joins the newest page of a room's chat to what is held; whether its messages changed
    /// (a page already held as it is changes nothing, though the room counts as held after).
    pub(crate) fn join(&mut self, room: RoomId, page: &[ChatMessage]) -> bool {
        let held = self.held.remove(&room).unwrap_or_default();
        let unchanged = page.first().is_none_or(|first| {
            held.iter()
                .position(|message| message.id == first.id)
                .is_some_and(|at| held[at..].starts_with(page))
        });
        self.held.insert(room, join(held, page));
        !unchanged
    }

    /// Whether anything of `room`'s chat is held (an empty room once its state came counts):
    /// a room never held shows that it is loading, a held one shows its messages or that it is
    /// empty.
    #[must_use]
    pub(crate) fn is_held(&self, room: RoomId) -> bool {
        self.held.contains_key(&room)
    }

    /// Joins messages that came since the newest held (a catch-up) to what is held; whether
    /// anything changed. Unlike [`Chats::join`], an empty page doesn't make the room held.
    pub(crate) fn join_new(&mut self, room: RoomId, page: &[ChatMessage]) -> bool {
        !page.is_empty() && self.join(room, page)
    }

    /// Takes in what was kept on disk for `room`: it goes before what is held now, whose copies
    /// win (they came from the server since).
    pub(crate) fn restore(&mut self, room: RoomId, saved: Vec<ChatMessage>) {
        let held = self.held.remove(&room).unwrap_or_default();
        self.held.insert(room, join(saved, &held));
    }

    /// Whether a "Show earlier" makes sense: the room has more than is held.
    #[must_use]
    pub(crate) fn has_earlier(&self, room: RoomId, server_says_earlier: bool) -> bool {
        !self.complete.contains(&room)
            && (self.messages(room).len() >= PAGE_SIZE || server_says_earlier)
    }

    /// Whether an earlier page of `room` is on its way.
    #[must_use]
    pub(crate) fn is_loading_earlier(&self, room: RoomId) -> bool {
        self.loading_earlier.contains(&room)
    }

    /// Starts loading the page before the oldest message held: the id to ask before, or
    /// `None` when there is nothing held or a page is already on its way.
    pub(crate) fn begin_earlier(&mut self, room: RoomId) -> Option<MessageId> {
        if self.loading_earlier.contains(&room) {
            return None;
        }
        let oldest = self.messages(room).first()?.id;
        self.loading_earlier.insert(room);
        Some(oldest)
    }

    /// The earlier page arrived (`None`: it failed): it goes before what is held.
    pub(crate) fn finish_earlier(
        &mut self,
        room: RoomId,
        before: MessageId,
        page: Option<Vec<ChatMessage>>,
    ) {
        self.loading_earlier.remove(&room);
        let Some(page) = page else { return };
        if page.len() < PAGE_SIZE {
            self.complete.insert(room);
        }
        let mut older: Vec<ChatMessage> = page.into_iter().filter(|m| m.id < before).collect();
        older.extend(self.held.remove(&room).unwrap_or_default());
        self.held.insert(room, older);
    }
}

/// A page of a room's chat joined to what's held: the page's copies win, and what's held
/// before it stays only when nothing can be missing between them (they overlap, or the page
/// isn't full); after a full page with a gap, the page alone. What's held after the page (an
/// answer that came since) stays.
#[must_use]
pub(crate) fn join(held: Vec<ChatMessage>, page: &[ChatMessage]) -> Vec<ChatMessage> {
    let (Some(first), Some(last)) = (page.first().map(|m| m.id), page.last().map(|m| m.id)) else {
        return held;
    };
    let touches = page.len() < PAGE_SIZE || held.iter().any(|m| m.id >= first);
    let (before, rest): (Vec<_>, Vec<_>) = held.into_iter().partition(|m| m.id < first);
    let mut joined = if touches { before } else { Vec::new() };
    joined.extend_from_slice(page);
    joined.extend(rest.into_iter().filter(|m| m.id > last));
    joined
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::fixtures::message;

    fn msgs(ids: impl IntoIterator<Item = i64>) -> Vec<ChatMessage> {
        ids.into_iter().map(message).collect()
    }

    fn ids(messages: &[ChatMessage]) -> Vec<i64> {
        messages.iter().map(|m| m.id.get()).collect()
    }

    #[test]
    fn an_overlapping_page_extends_what_is_held() {
        assert_eq!(ids(&join(msgs(1..=5), &msgs(4..=7))), [1, 2, 3, 4, 5, 6, 7]);
    }

    #[test]
    fn the_pages_copies_win() {
        let mut page = msgs(4..=5);
        page[0].text = "edited".into();
        let joined = join(msgs(1..=5), &page);
        assert_eq!(joined[3].text, "edited");
    }

    #[test]
    fn a_full_page_after_a_gap_replaces_what_is_held() {
        let page = msgs(100..100 + i64::try_from(PAGE_SIZE).unwrap());
        assert_eq!(join(msgs(1..=5), &page), page);
    }

    #[test]
    fn a_short_page_keeps_what_came_before_it() {
        assert_eq!(
            ids(&join(msgs(1..=3), &msgs(10..=12))),
            [1, 2, 3, 10, 11, 12]
        );
    }

    #[test]
    fn what_came_after_the_page_stays() {
        assert_eq!(
            ids(&join(msgs(1..=8), &msgs(3..=5))),
            [1, 2, 3, 4, 5, 6, 7, 8]
        );
    }

    #[test]
    fn an_empty_page_changes_nothing() {
        assert_eq!(ids(&join(msgs(1..=3), &[])), [1, 2, 3]);
    }

    #[test]
    fn joining_says_whether_the_messages_changed() {
        let room = RoomId::MAIN;
        let mut chats = Chats::default();
        assert!(!chats.join(room, &[]), "an empty room");
        assert!(chats.join(room, &msgs(1..=5)));
        assert!(!chats.join(room, &msgs(1..=5)), "the same page again");
        assert!(!chats.join(room, &msgs(3..=4)), "a part of what is held");
        let mut edited = msgs(4..=5);
        edited[0].text = "edited".into();
        assert!(chats.join(room, &edited), "an edit");
        assert!(chats.join(room, &msgs(5..=6)), "a new message");
    }

    #[test]
    fn earlier_pages_go_before_and_a_short_one_completes_the_room() {
        let room = RoomId::new(2);
        let mut chats = Chats::default();
        chats.join(room, &msgs(61..=120));
        assert!(chats.has_earlier(room, false));
        let before = chats.begin_earlier(room).unwrap();
        assert_eq!(before, MessageId::new(61));
        assert_eq!(chats.begin_earlier(room), None, "one page at a time");
        chats.finish_earlier(room, before, Some(msgs(40..=61)));
        assert_eq!(
            chats.messages(room).first().map(|m| m.id),
            Some(MessageId::new(40))
        );
        assert_eq!(chats.messages(room).len(), 81, "the overlap isn't doubled");
        assert!(!chats.has_earlier(room, true));
    }

    #[test]
    fn a_failed_earlier_page_can_be_asked_for_again() {
        let room = RoomId::new(2);
        let mut chats = Chats::default();
        chats.join(room, &msgs(1..=3));
        let before = chats.begin_earlier(room).unwrap();
        chats.finish_earlier(room, before, None);
        assert!(!chats.is_loading_earlier(room));
        assert_eq!(chats.begin_earlier(room), Some(before));
    }

    #[test]
    fn a_catch_up_adds_what_came_since_and_says_whether_it_did() {
        let room = RoomId::new(2);
        let mut chats = Chats::default();
        assert!(!chats.join_new(room, &[]), "nothing came");
        assert!(!chats.is_held(room), "an empty catch-up holds nothing");
        assert!(chats.join_new(room, &msgs(1..=3)));
        assert!(chats.join_new(room, &msgs(4..=5)));
        assert_eq!(ids(chats.messages(room)), [1, 2, 3, 4, 5]);
        assert!(!chats.join_new(room, &msgs(4..=5)), "the same again");
    }

    #[test]
    fn a_room_held_empty_is_held() {
        let room = RoomId::new(2);
        let mut chats = Chats::default();
        assert!(!chats.is_held(room));
        chats.join(room, &[]);
        assert!(chats.is_held(room), "its state came, and it is empty");
    }

    #[test]
    fn what_was_kept_on_disk_goes_before_what_came_since() {
        let room = RoomId::new(2);
        let mut chats = Chats::default();
        chats.join(room, &msgs(4..=6));
        let mut saved = msgs(1..=5);
        saved[4].text = "old copy".into();
        chats.restore(room, saved);
        assert_eq!(ids(chats.messages(room)), [1, 2, 3, 4, 5, 6]);
        assert_ne!(
            chats.messages(room)[4].text,
            "old copy",
            "the server's copy wins"
        );
        let other = RoomId::new(3);
        chats.restore(other, msgs(1..=2));
        assert_eq!(ids(chats.messages(other)), [1, 2]);
    }

    #[test]
    fn a_room_with_nothing_held_has_no_earlier_page() {
        let mut chats = Chats::default();
        assert_eq!(chats.messages(RoomId::new(9)), []);
        assert_eq!(chats.begin_earlier(RoomId::new(9)), None);
        assert!(
            chats.has_earlier(RoomId::new(9), true),
            "the server says there is"
        );
        assert!(!chats.has_earlier(RoomId::new(9), false));
    }
}
