//! What the user put away over the composer, kept across launches in `chat-put-away.json`: the
//! ask cards (by their keys, so a new question brings its card back; only the keys of asks
//! still waiting somewhere are kept) and Main's line for the other rooms' asks (by the count it
//! had, so more asks bring it back; a new ask that comes as another is answered, between two
//! looks at the state, leaves the count as it was and the line away).

use std::collections::BTreeSet;

use gpui_kit::AppContext as _;
use gpui_kit::Context;
use serde::{Deserialize, Serialize};

use super::Store;
use super::asks::{self, AskKey};
use crate::config::AppFiles;
use crate::json_file;

/// What is put away.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PutAway {
    /// The keys of the ask cards put away (sorted, so the file only changes when they do).
    #[serde(default)]
    ask_cards: BTreeSet<AskKey>,
    /// How many asks the other rooms had when Main's line was put away.
    #[serde(default)]
    room_asks: u32,
}

impl PutAway {
    /// The keys of the ask cards put away.
    #[must_use]
    pub(crate) fn ask_cards(&self) -> &BTreeSet<AskKey> {
        &self.ask_cards
    }

    /// The cards of `keys` are put away too.
    fn put_away_cards(&mut self, keys: impl IntoIterator<Item = AskKey>) {
        self.ask_cards.extend(keys);
    }

    /// Keeps only the keys of asks still `waiting` (in any room); whether that changed
    /// anything.
    fn keep_cards_waiting(&mut self, waiting: &BTreeSet<AskKey>) -> bool {
        let before = self.ask_cards.len();
        self.ask_cards.retain(|key| waiting.contains(key));
        self.ask_cards.len() != before
    }

    /// Whether Main's line shows for `count` asks in the other rooms: there are some, and more
    /// than when it was put away.
    #[must_use]
    pub(crate) fn shows_room_asks(&self, count: u32) -> bool {
        count > self.room_asks
    }

    /// The count went down (asks answered): from there, any new one brings the line back.
    /// Whether that changed anything.
    fn settle_room_asks(&mut self, count: u32) -> bool {
        let lower = count < self.room_asks;
        if lower {
            self.room_asks = count;
        }
        lower
    }

    /// The file, from `files`; nothing put away when there is none or it can't be read.
    fn load(files: Option<&AppFiles>) -> Self {
        files
            .and_then(|files| {
                json_file::read(&files.put_away(), "what was put away", "showing everything")
            })
            .unwrap_or_default()
    }
}

impl Store {
    /// What the user put away over the composer, as read at launch.
    pub(super) fn load_put_away(files: Option<&AppFiles>) -> PutAway {
        PutAway::load(files)
    }

    /// What is put away.
    #[must_use]
    pub(crate) fn put_away(&self) -> &PutAway {
        &self.put_away
    }

    /// Hide on the ask card: every ask `keys` names is put away, along with those put away
    /// before in any room; of them all, only the asks still waiting in some room are kept, so
    /// the list doesn't grow.
    pub(crate) fn put_away_asks(&mut self, keys: Vec<AskKey>, cx: &mut Context<'_, Self>) {
        self.put_away.put_away_cards(keys);
        if let Some(state) = self.state() {
            let waiting = asks::waiting_keys(state);
            self.put_away.keep_cards_waiting(&waiting);
        }
        self.keep_put_away(cx);
    }

    /// Hide on Main's line: it stays away until the other rooms have more asks than now.
    pub(crate) fn put_away_room_asks(&mut self, cx: &mut Context<'_, Self>) {
        self.put_away.room_asks = self.state().map_or(0, asks::in_other_rooms);
        self.keep_put_away(cx);
    }

    /// A new state: the cards of asks no longer waiting anywhere are forgotten, and asks
    /// answered in the other rooms lower the count Main's line was put away at, so the next new
    /// ask brings it back.
    pub(super) fn settle_put_away(&mut self, cx: &mut Context<'_, Self>) {
        let Some(state) = self.state() else { return };
        let (waiting, count) = (asks::waiting_keys(state), asks::in_other_rooms(state));
        let cards = self.put_away.keep_cards_waiting(&waiting);
        let rooms = self.put_away.settle_room_asks(count);
        if cards || rooms {
            self.keep_put_away(cx);
        }
    }

    /// Writes what is put away, off the UI thread (newest first), and redraws.
    fn keep_put_away(&mut self, cx: &mut Context<'_, Self>) {
        if let Some(files) = self.files.clone() {
            let put_away = self.put_away.clone();
            let (number, written) = self.put_away_writes.next();
            cx.background_spawn(async move {
                let path = files.put_away();
                json_file::write(&path, &put_away, number, &written, "what was put away");
            })
            .detach();
        }
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use gpui_kit::{Entity, TestAppContext};
    use serde_json::json;

    use super::*;
    use crate::backend::fake::FakeBackend;
    use crate::fixtures::{issue, room, state};
    use emberhut_api::ids::RoomId;
    use emberhut_api::tasks::TaskStatus;

    fn key(text: &str) -> AskKey {
        serde_json::from_value(json!(text)).unwrap()
    }

    fn ask(id: i64, pid: i64) -> serde_json::Value {
        let task = issue(
            id,
            "waiting",
            json!({"pid": pid, "decision": {"question": format!("Question {id}?"),
                "options": ["Yes", "No"], "asked": "2026-11-25T08:00:00Z"}}),
        );
        serde_json::to_value(task).unwrap()
    }

    /// Asks 20 in room 2 and 30 in room 3; the other rooms have `needs` asks between them.
    fn lists(needs: u32, also: &[serde_json::Value]) -> emberhut_api::state::State {
        let mut issues = vec![ask(20, 2), ask(30, 3)];
        issues.extend_from_slice(also);
        state(json!({
            "rooms": [room(1, json!({})), room(2, json!({"needs": needs})), room(3, json!({}))],
            "issues": issues,
        }))
    }

    fn store(cx: &mut TestAppContext) -> Entity<Store> {
        let backend = std::sync::Arc::new(FakeBackend::new([]));
        cx.new(|_| Store::for_tests(backend))
    }

    fn cards_put_away(store: &Entity<Store>, cx: &mut TestAppContext) -> Vec<String> {
        store.read_with(cx, |store, _| {
            store
                .put_away()
                .ask_cards()
                .iter()
                .map(ToString::to_string)
                .collect()
        })
    }

    #[gpui_kit::test]
    fn hiding_in_one_room_keeps_what_another_room_put_away(cx: &mut TestAppContext) {
        let store = store(cx);
        store.update(cx, |store, cx| {
            store.take_state_for_tests(lists(1, &[]), cx);
            store.switch_room(RoomId::new(2), cx);
            store.put_away_asks(vec![key("20@2026-11-25T08:00:00Z")], cx);
            store.switch_room(RoomId::new(3), cx);
            store.put_away_asks(vec![key("30@2026-11-25T08:00:00Z")], cx);
        });
        assert_eq!(
            cards_put_away(&store, cx),
            ["20@2026-11-25T08:00:00Z", "30@2026-11-25T08:00:00Z"],
            "both stay put away"
        );
    }

    #[gpui_kit::test]
    fn the_cards_of_asks_no_longer_waiting_are_forgotten(cx: &mut TestAppContext) {
        let store = store(cx);
        store.update(cx, |store, cx| {
            store.take_state_for_tests(lists(1, &[]), cx);
            store.put_away_asks(
                vec![
                    key("20@2026-11-25T08:00:00Z"),
                    key("99@2026-11-25T08:00:00Z"),
                ],
                cx,
            );
        });
        assert_eq!(
            cards_put_away(&store, cx),
            ["20@2026-11-25T08:00:00Z"],
            "99 waits nowhere"
        );
        let done = serde_json::to_value(issue(20, "done", json!({}))).unwrap();
        store.update(cx, |store, cx| {
            let mut answered = lists(1, &[done]);
            answered
                .issues
                .retain(|task| task.id.get() != 20 || task.status != TaskStatus::Waiting);
            store.take_state_for_tests(answered, cx);
        });
        assert_eq!(cards_put_away(&store, cx), Vec::<String>::new());
    }

    #[gpui_kit::test]
    fn mains_line_stays_away_until_more_asks_come(cx: &mut TestAppContext) {
        let store = store(cx);
        let shows = |store: &Entity<Store>, cx: &mut TestAppContext| {
            store.read_with(cx, |store, _| {
                let count = store.state().map_or(0, asks::in_other_rooms);
                store.put_away().shows_room_asks(count)
            })
        };
        store.update(cx, |store, cx| {
            store.take_state_for_tests(lists(2, &[]), cx);
        });
        assert!(shows(&store, cx));
        store.update(cx, Store::put_away_room_asks);
        assert!(!shows(&store, cx), "put away at 2");
        store.update(cx, |store, cx| {
            store.take_state_for_tests(lists(1, &[]), cx);
        });
        assert!(!shows(&store, cx), "one answered");
        store.update(cx, |store, cx| {
            store.take_state_for_tests(lists(2, &[]), cx);
        });
        assert!(shows(&store, cx), "a new one after that brings it back");
    }

    #[test]
    fn what_is_put_away_lasts_across_launches() {
        let dir = tempfile::tempdir().unwrap();
        let files = AppFiles::in_dir(&dir.path().join("emberhut"));
        assert_eq!(PutAway::load(Some(&files)), PutAway::default());
        let put_away = PutAway {
            ask_cards: BTreeSet::from([key("12@2026-11-25T08:00:00Z")]),
            room_asks: 3,
        };
        json_file::write(
            &files.put_away(),
            &put_away,
            1,
            &std::sync::Mutex::new(0),
            "what was put away",
        );
        assert_eq!(PutAway::load(Some(&files)), put_away);
        std::fs::write(files.put_away(), "{").unwrap();
        assert_eq!(PutAway::load(Some(&files)), PutAway::default());
        assert_eq!(PutAway::load(None), PutAway::default());
    }

    #[test]
    fn mains_line_comes_back_with_more_asks() {
        let mut put_away = PutAway {
            room_asks: 3,
            ..PutAway::default()
        };
        assert!(!put_away.shows_room_asks(3));
        assert!(put_away.shows_room_asks(4));
        assert!(put_away.settle_room_asks(2), "one answered");
        assert!(!put_away.settle_room_asks(2));
        assert!(put_away.shows_room_asks(3), "a new one after that");
        assert!(!PutAway::default().shows_room_asks(0), "no asks: no line");
    }
}
