//! Whether the user is reading a room, and when to tell the server so. A room is read while its
//! chat is on screen in the active window (beside whatever page the column shows: the chat
//! stays in view), the platform shows that window (not
//! minimised, covered or behind a locked screen), the sign-in screen doesn't cover it, and the
//! user used the window in the last few minutes (a window left in front of an empty desk isn't
//! read): its state polls then say it is being looked at (the server holds its pushes to the
//! phone), and its new answers are marked seen, once each, so its unread clears on every
//! device.

use std::time::{Duration, Instant};

use emberhut_api::ids::{MessageId, RoomId};
use emberhut_api::state::State;

use super::presence::Presence;

/// How long the window can go without input before the user counts as away (as the
/// reference client and the contract have it).
pub(crate) const AWAY_AFTER: Duration = Duration::from_mins(5);

/// When the user last used the window: a key, a click, a scroll, the pointer moving over it, or
/// bringing it to the front.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct LastInput(Option<Instant>);

impl LastInput {
    /// The user used the window at `at`; whether they were away until then (so what is on
    /// screen is read again from now on).
    pub(crate) fn input(&mut self, at: Instant) -> bool {
        let back = !self.is_recent(at);
        self.0 = Some(at);
        back
    }

    /// Whether the user used the window less than [`AWAY_AFTER`] before `now`.
    #[must_use]
    pub(crate) fn is_recent(self, now: Instant) -> bool {
        self.0
            .is_some_and(|at| now.saturating_duration_since(at) < AWAY_AFTER)
    }
}

/// What is in front of the user, as far as reading goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct OnScreen {
    /// The room whose chat the window shows (beside any page in the column).
    pub(crate) room: RoomId,
    /// Whether the sign-in screen covers it.
    pub(crate) signing_in: bool,
    /// Where the window is for the user. Only in front is it read: not behind another app's
    /// window, and not when no one can see it even though it is the active one (the screen
    /// locked, the display asleep, another app's full-screen window over it).
    pub(crate) window: Presence,
    /// Whether the user used the window lately ([`LastInput::is_recent`]).
    pub(crate) at_keyboard: bool,
}

impl OnScreen {
    /// The room being read, if one is.
    #[must_use]
    pub(crate) fn reading(self) -> Option<RoomId> {
        (self.window == Presence::Front && self.at_keyboard && !self.signing_in)
            .then_some(self.room)
    }

    /// Whether a state poll for `room` says it is being looked at.
    #[must_use]
    pub(crate) fn is_looking_at(self, room: RoomId) -> bool {
        self.reading() == Some(room)
    }
}

/// The answers marked seen: a room up to its newest message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SeenMark {
    /// The room.
    pub(crate) room: RoomId,
    /// Its newest message then (none in an empty room).
    pub(crate) newest: Option<MessageId>,
}

/// What the server was last told was seen, so the same answers aren't marked twice (a poll
/// that left before the mark arrived still says they are unread), and the last mark that
/// failed, so its retries don't warn again.
#[derive(Debug, Default)]
pub(crate) struct SeenMarks {
    told: Option<SeenMark>,
    failed: Option<SeenMark>,
}

impl SeenMarks {
    /// The mark to send now, given the room being read and the state on hand: the server says
    /// the room has unread answers and it wasn't told about these yet. The mark is taken as
    /// told.
    #[must_use]
    pub(crate) fn due(&mut self, reading: Option<RoomId>, state: &State) -> Option<SeenMark> {
        if reading != Some(state.room) || !state.unread {
            return None;
        }
        let mark = SeenMark {
            room: state.room,
            newest: state.chat.last().map(|message| message.id),
        };
        if self.told == Some(mark) {
            return None;
        }
        self.told = Some(mark);
        Some(mark)
    }

    /// `mark` failed. If the failure can heal (`retry`: the server wasn't reached), the mark is
    /// due again at the next chance; otherwise it stays told, so a refusal isn't sent again
    /// with every poll (newer answers are a new mark, which is tried). Whether this is the
    /// mark's first failure, worth a warning; its retries' failures aren't.
    #[must_use = "a mark's first failure is worth a warning"]
    pub(crate) fn failed(&mut self, mark: SeenMark, retry: bool) -> bool {
        if retry && self.told == Some(mark) {
            self.told = None;
        }
        let first = self.failed != Some(mark);
        self.failed = Some(mark);
        first
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::fixtures::{message, state};

    const OTHER: RoomId = RoomId::new(2);

    fn on_screen(room: RoomId) -> OnScreen {
        OnScreen {
            room,
            signing_in: false,
            window: Presence::Front,
            at_keyboard: true,
        }
    }

    /// Main's state, with unread answers up to message `newest`.
    fn unread_up_to(newest: i64) -> State {
        state(json!({"unread": true, "chat": [message(newest - 1), message(newest)]}))
    }

    #[test]
    fn the_open_room_is_looked_at_while_its_chat_is_in_the_active_window() {
        let here = on_screen(RoomId::MAIN);
        assert_eq!(here.reading(), Some(RoomId::MAIN));
        assert!(here.is_looking_at(RoomId::MAIN));
        assert!(!here.is_looking_at(OTHER), "a poll for another room isn't");
    }

    #[test]
    fn no_room_is_looked_at_from_an_inactive_or_unseen_window_or_sign_in() {
        let behind = OnScreen {
            window: Presence::Behind,
            ..on_screen(RoomId::MAIN)
        };
        assert!(
            !behind.is_looking_at(RoomId::MAIN),
            "the window is in the background"
        );
        let out_of_sight = OnScreen {
            window: Presence::Hidden,
            ..on_screen(RoomId::MAIN)
        };
        assert!(
            !out_of_sight.is_looking_at(RoomId::MAIN),
            "the active window, but the screen is locked or covered"
        );
        let covered = OnScreen {
            signing_in: true,
            ..on_screen(RoomId::MAIN)
        };
        assert!(
            !covered.is_looking_at(RoomId::MAIN),
            "the sign-in screen covers it"
        );
        let away = OnScreen {
            at_keyboard: false,
            ..on_screen(RoomId::MAIN)
        };
        assert!(
            !away.is_looking_at(RoomId::MAIN),
            "no one has used the window lately"
        );
    }

    #[test]
    fn the_user_is_at_the_keyboard_until_five_minutes_without_input() {
        let start = Instant::now();
        let mut last = LastInput::default();
        assert!(!last.is_recent(start), "no input yet");
        assert!(last.input(start), "the first input is a coming back");
        assert!(last.is_recent(start));
        assert!(last.is_recent(start + AWAY_AFTER.saturating_sub(Duration::from_millis(1))));
        assert!(!last.is_recent(start + AWAY_AFTER), "five minutes on: away");
    }

    #[test]
    fn input_after_being_away_is_a_coming_back() {
        let start = Instant::now();
        let mut last = LastInput::default();
        assert!(last.input(start));
        assert!(
            !last.input(start + AWAY_AFTER.saturating_sub(Duration::from_millis(1))),
            "still there"
        );
        let later = start + AWAY_AFTER * 3;
        assert!(last.input(later), "back after five minutes away");
        assert!(last.is_recent(later));
    }

    #[test]
    fn unread_answers_being_read_are_marked_seen_once() {
        let mut seen = SeenMarks::default();
        let state = unread_up_to(9);
        let mark = SeenMark {
            room: RoomId::MAIN,
            newest: Some(MessageId::new(9)),
        };
        assert_eq!(seen.due(Some(RoomId::MAIN), &state), Some(mark));
        assert_eq!(
            seen.due(Some(RoomId::MAIN), &state),
            None,
            "a poll from before the mark still says unread"
        );
    }

    #[test]
    fn a_new_answer_while_reading_is_marked_seen_again() {
        let mut seen = SeenMarks::default();
        assert!(seen.due(Some(RoomId::MAIN), &unread_up_to(9)).is_some());
        assert_eq!(
            seen.due(Some(RoomId::MAIN), &unread_up_to(10)),
            Some(SeenMark {
                room: RoomId::MAIN,
                newest: Some(MessageId::new(10)),
            })
        );
    }

    #[test]
    fn nothing_is_marked_seen_unless_the_room_on_hand_is_read_and_unread() {
        let mut seen = SeenMarks::default();
        assert_eq!(seen.due(None, &unread_up_to(9)), None, "not being read");
        assert_eq!(
            seen.due(Some(OTHER), &unread_up_to(9)),
            None,
            "the state on hand is still Main's"
        );
        assert_eq!(
            seen.due(Some(RoomId::MAIN), &state(json!({"chat": [message(9)]}))),
            None,
            "nothing unread"
        );
        assert!(
            seen.due(Some(RoomId::MAIN), &unread_up_to(9)).is_some(),
            "none of those counted as told"
        );
    }

    #[test]
    fn a_mark_that_didnt_reach_the_server_is_due_again() {
        let mut seen = SeenMarks::default();
        let state = unread_up_to(9);
        let mark = seen.due(Some(RoomId::MAIN), &state).expect("due");
        assert!(seen.failed(mark, true), "its first failure");
        assert_eq!(seen.due(Some(RoomId::MAIN), &state), Some(mark));
        assert!(!seen.failed(mark, true), "a retry's failure isn't news");
    }

    #[test]
    fn a_refused_mark_isnt_sent_again_but_newer_answers_are() {
        let mut seen = SeenMarks::default();
        let mark = seen.due(Some(RoomId::MAIN), &unread_up_to(9)).expect("due");
        assert!(seen.failed(mark, false));
        assert_eq!(
            seen.due(Some(RoomId::MAIN), &unread_up_to(9)),
            None,
            "the server said no to these"
        );
        let newer = seen
            .due(Some(RoomId::MAIN), &unread_up_to(10))
            .expect("due");
        assert!(seen.failed(newer, false), "a new mark's failure is news");
    }

    #[test]
    fn an_older_mark_failing_leaves_the_newer_one_told() {
        let mut seen = SeenMarks::default();
        let older = seen.due(Some(RoomId::MAIN), &unread_up_to(9)).expect("due");
        let newer = unread_up_to(10);
        assert!(seen.due(Some(RoomId::MAIN), &newer).is_some());
        assert!(seen.failed(older, true));
        assert_eq!(seen.due(Some(RoomId::MAIN), &newer), None);
    }
}
