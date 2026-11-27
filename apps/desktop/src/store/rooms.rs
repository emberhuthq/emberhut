//! The room switcher: which rooms the sidebar lists, in which order, and which room each
//! shortcut opens (⌘0 Main, ⌘1…⌘9 the listed rooms in turn; Ctrl on Linux and Windows).
//!
//! The grouping is the contract's ([`emberhut_api::rooms::Switcher`]): Main has its own row, the
//! active rooms are listed and numbered, paused and done ones fold under "Paused", and archived
//! ones are out of the switcher. An agent's own room is left out of the groups (the sidebar
//! lists it under Agents), so it takes no shortcut.

use emberhut_api::ids::RoomId;
use emberhut_api::rooms::RoomRow;

/// The rooms as the switcher shows them.
#[derive(Debug)]
pub(crate) struct Switcher<'a> {
    main: Option<&'a RoomRow>,
    listed: Vec<&'a RoomRow>,
    folded: Vec<&'a RoomRow>,
    archived: Vec<&'a RoomRow>,
}

impl<'a> Switcher<'a> {
    /// The switcher over the state's rooms (in the server's order).
    #[must_use]
    pub(crate) fn new(rooms: &'a [RoomRow]) -> Self {
        let emberhut_api::rooms::Switcher {
            open,
            folded,
            archived,
        } = emberhut_api::rooms::Switcher::new(rooms);
        let (main, listed): (Vec<_>, Vec<_>) = open.into_iter().partition(|room| room.is_main());
        Self {
            main: main.first().copied(),
            listed,
            folded,
            archived,
        }
    }

    /// The same switcher without `rooms` (never Main): the agents' own rooms.
    #[must_use]
    pub(crate) fn without(mut self, rooms: &[RoomId]) -> Self {
        for group in [&mut self.listed, &mut self.folded, &mut self.archived] {
            group.retain(|room| !rooms.contains(&room.id));
        }
        self
    }

    /// Main.
    #[must_use]
    pub(crate) fn main(&self) -> Option<&'a RoomRow> {
        self.main
    }

    /// The listed rooms, in shortcut order.
    #[must_use]
    pub(crate) fn listed(&self) -> &[&'a RoomRow] {
        &self.listed
    }

    /// The rooms folded away under "Paused" (paused and done projects'), in the server's order.
    #[must_use]
    pub(crate) fn paused(&self) -> &[&'a RoomRow] {
        &self.folded
    }

    /// The archived rooms: out of the switcher, listed in Settings.
    #[must_use]
    pub(crate) fn archived(&self) -> &[&'a RoomRow] {
        &self.archived
    }

    /// The shortcut that opens `room`, if it has one (Main ⌘0, the first nine listed ⌘1…⌘9).
    #[must_use]
    pub(crate) fn shortcut_of(&self, room: RoomId) -> Option<Shortcut> {
        if self.main.is_some_and(|main| main.id == room) {
            return Some(Shortcut(0));
        }
        let at = self.listed.iter().take(9).position(|row| row.id == room)?;
        u8::try_from(at + 1).ok().map(Shortcut)
    }

    /// The room a shortcut opens.
    #[must_use]
    pub(crate) fn room_for(&self, shortcut: Shortcut) -> Option<RoomId> {
        match shortcut.0 {
            0 => self.main.map(|room| room.id),
            n => self.listed.get(usize::from(n) - 1).map(|room| room.id),
        }
    }
}

/// A room shortcut: 0 for Main, 1 to 9 for the listed rooms.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Shortcut(u8);

impl Shortcut {
    /// Its number key: 0 for Main, 1 to 9 for the listed rooms.
    #[must_use]
    pub(crate) fn digit(self) -> u8 {
        self.0
    }

    /// All of them, 0 to 9.
    pub(crate) const ALL: [Self; 10] = [
        Self(0),
        Self(1),
        Self(2),
        Self(3),
        Self(4),
        Self(5),
        Self(6),
        Self(7),
        Self(8),
        Self(9),
    ];
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    use crate::fixtures::room;

    fn rooms() -> Vec<RoomRow> {
        vec![
            room(1, json!({})),
            room(2, json!({})),
            room(3, json!({"active": false, "status": "paused"})),
            room(4, json!({})),
        ]
    }

    #[test]
    fn main_has_its_own_row_and_paused_rooms_fold() {
        let rooms = rooms();
        let switcher = Switcher::new(&rooms);
        assert_eq!(switcher.main().map(|r| r.id), Some(RoomId::MAIN));
        let listed: Vec<_> = switcher.listed().iter().map(|r| r.id).collect();
        assert_eq!(listed, [RoomId::new(2), RoomId::new(4)]);
        assert_eq!(switcher.paused().len(), 1);
    }

    #[test]
    fn shortcuts_count_the_listed_rooms() {
        let rooms = rooms();
        let switcher = Switcher::new(&rooms);
        let [zero, one, two, three, ..] = Shortcut::ALL;
        assert_eq!(switcher.room_for(zero), Some(RoomId::MAIN));
        assert_eq!(switcher.room_for(one), Some(RoomId::new(2)));
        assert_eq!(
            switcher.room_for(two),
            Some(RoomId::new(4)),
            "the paused room takes no number"
        );
        assert_eq!(switcher.room_for(three), None);
    }

    #[test]
    fn an_agents_own_room_is_left_out_and_takes_no_number() {
        let rooms = rooms();
        let switcher = Switcher::new(&rooms).without(&[RoomId::new(2), RoomId::new(3)]);
        let listed: Vec<_> = switcher.listed().iter().map(|r| r.id).collect();
        assert_eq!(listed, [RoomId::new(4)]);
        assert_eq!(switcher.paused().len(), 0);
        assert_eq!(switcher.room_for(Shortcut::ALL[1]), Some(RoomId::new(4)));
        assert_eq!(switcher.shortcut_of(RoomId::new(2)), None);
    }

    /// Main, an active room, an archived one, a done one, a paused one, another active one.
    fn rooms_out_of_the_switcher() -> Vec<RoomRow> {
        vec![
            room(1, json!({})),
            room(2, json!({})),
            room(3, json!({"active": false, "status": "archived"})),
            room(4, json!({"active": false, "status": "done"})),
            room(5, json!({"active": false, "status": "paused"})),
            room(6, json!({})),
        ]
    }

    #[test]
    fn archived_and_done_rooms_are_not_listed() {
        let rooms = rooms_out_of_the_switcher();
        let switcher = Switcher::new(&rooms);
        let listed: Vec<_> = switcher.listed().iter().map(|r| r.id).collect();
        assert_eq!(listed, [RoomId::new(2), RoomId::new(6)]);
        assert_eq!(
            switcher.paused().len(),
            2,
            "the paused and the done room fold; the archived one is out of the switcher"
        );
    }

    #[test]
    fn archived_and_done_rooms_take_no_number() {
        let rooms = rooms_out_of_the_switcher();
        let switcher = Switcher::new(&rooms);
        let numbered: Vec<_> = Shortcut::ALL
            .iter()
            .skip(1)
            .map_while(|&s| switcher.room_for(s))
            .collect();
        assert_eq!(
            numbered,
            [RoomId::new(2), RoomId::new(6)],
            "⌘1 and ⌘2 open the shown rooms, in turn"
        );
        assert!(
            Shortcut::ALL
                .iter()
                .skip(numbered.len() + 1)
                .all(|&s| switcher.room_for(s).is_none()),
            "no number past the shown rooms"
        );
    }

    #[test]
    fn each_shown_room_knows_its_shortcut_and_the_rest_are_apart() {
        let rooms = rooms_out_of_the_switcher();
        let switcher = Switcher::new(&rooms);
        let digit = |id| switcher.shortcut_of(RoomId::new(id)).map(Shortcut::digit);
        assert_eq!(digit(1), Some(0));
        assert_eq!(digit(2), Some(1));
        assert_eq!(digit(6), Some(2));
        assert_eq!(digit(5), None, "a paused room has none");
        let ids = |rooms: &[&RoomRow]| rooms.iter().map(|r| r.id.get()).collect::<Vec<_>>();
        assert_eq!(ids(switcher.paused()), [4, 5]);
        assert_eq!(ids(switcher.archived()), [3]);
    }

    #[test]
    fn no_rooms_no_shortcuts() {
        let switcher = Switcher::new(&[]);
        assert!(
            Shortcut::ALL
                .iter()
                .all(|&s| switcher.room_for(s).is_none())
        );
    }
}
