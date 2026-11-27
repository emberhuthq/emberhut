//! Rooms: Main, and one conversation per project. The state's `rooms` has a line for each, for
//! the room switcher.

use serde::{Deserialize, Serialize};

use crate::ids::{GoalId, ProjectId, RoomId, TaskId};
use crate::tasks::ProjectStatus;
use crate::time::{Day, Timestamp};

/// One room's line in the switcher: what waits there and the last thing said.
///
/// The server sends Main first, then the rooms by their last activity, then the quiet ones by
/// name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct RoomRow {
    /// The room.
    pub id: RoomId,
    /// Its name (Main's, or its project's).
    pub name: String,
    /// Its project; none for Main.
    pub project: Option<ProjectId>,
    /// Its project's goal.
    pub goal: Option<GoalId>,
    /// Whether its project is active (Main always is).
    pub active: bool,
    /// Its project's status; `active` for Main.
    #[serde(default = "ProjectStatus::active")]
    pub status: ProjectStatus,
    /// A paused room's day to come back by itself.
    #[serde(default)]
    pub paused_until: Option<Day>,
    /// The assistant's messages there not seen yet (0 while paused or archived).
    pub unread: u32,
    /// The tasks waiting on the person there.
    pub needs: u32,
    /// The first of those, as one line: what to do.
    pub ask: Option<String>,
    /// The task behind `ask`.
    #[serde(default)]
    pub ask_id: Option<TaskId>,
    /// The assistant is working there now.
    pub busy: bool,
    /// What it is doing, one line, while busy.
    pub doing: Option<String>,
    /// The last thing said there, on one line (`Sent a file` for a file alone).
    pub last: Option<String>,
    /// When.
    pub last_at: Option<Timestamp>,
    /// The person said it (a sidebar shows "You: ").
    #[serde(default)]
    pub last_mine: bool,
}

impl RoomRow {
    /// Whether this is Main.
    #[must_use]
    pub fn is_main(&self) -> bool {
        self.id.is_main()
    }

    /// Whether its project is paused: folded away, no badges, back on `paused_until`.
    #[must_use]
    pub fn is_paused(&self) -> bool {
        self.status == ProjectStatus::Paused
    }

    /// Whether its project is archived: out of the switcher.
    #[must_use]
    pub fn is_archived(&self) -> bool {
        self.status == ProjectStatus::Archived
    }
}

/// The rooms as a switcher groups them, each group in the server's order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Switcher<'a> {
    /// Main and the active rooms: the ones to switch between.
    pub open: Vec<&'a RoomRow>,
    /// Paused (and done) projects' rooms: folded at the bottom.
    pub folded: Vec<&'a RoomRow>,
    /// Archived rooms: not in the switcher; listed apart.
    pub archived: Vec<&'a RoomRow>,
}

impl<'a> Switcher<'a> {
    /// Groups the state's rooms.
    ///
    /// ```
    /// use emberhut_api::rooms::{RoomRow, Switcher};
    /// let rooms: Vec<RoomRow> = vec![]; // the state's `rooms`
    /// let switcher = Switcher::new(&rooms);
    /// assert!(switcher.open.is_empty());
    /// ```
    #[must_use]
    pub fn new(rooms: &'a [RoomRow]) -> Self {
        let mut switcher = Self::default();
        for room in rooms {
            if room.active || room.is_main() {
                switcher.open.push(room);
            } else if room.is_archived() {
                switcher.archived.push(room);
            } else {
                switcher.folded.push(room);
            }
        }
        switcher
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn room(id: i64, active: bool, status: &str) -> RoomRow {
        RoomRow {
            id: RoomId::new(id),
            name: format!("Room {id}"),
            project: (id != 1).then(|| ProjectId::new(id)),
            goal: None,
            active,
            status: ProjectStatus::from(status),
            paused_until: None,
            unread: 0,
            needs: 0,
            ask: None,
            ask_id: None,
            busy: false,
            doing: None,
            last: None,
            last_at: None,
            last_mine: false,
        }
    }

    #[test]
    fn groups_rooms_like_the_switcher() {
        let rooms = [
            room(1, true, "active"),
            room(2, true, "active"),
            room(3, false, "paused"),
            room(4, false, "archived"),
            room(5, false, "done"),
        ];
        let switcher = Switcher::new(&rooms);
        let ids = |rooms: &[&RoomRow]| rooms.iter().map(|r| r.id.get()).collect::<Vec<_>>();
        assert_eq!(ids(&switcher.open), [1, 2]);
        assert_eq!(ids(&switcher.folded), [3, 5]);
        assert_eq!(ids(&switcher.archived), [4]);
    }

    #[test]
    fn main_stays_open_whatever_it_says() {
        let rooms = [room(1, false, "paused")];
        assert_eq!(Switcher::new(&rooms).open.len(), 1);
    }

    #[test]
    fn status_helpers() {
        assert!(room(1, true, "active").is_main());
        assert!(room(3, false, "paused").is_paused());
        assert!(room(4, false, "archived").is_archived());
        assert!(!room(2, true, "active").is_paused());
    }
}
