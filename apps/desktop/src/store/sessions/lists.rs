//! The sessions as the app lists them, every room's: the user's own and the assistant's once
//! each; the sidebar's few (waiting ones first, then what runs, then the last finished); the
//! count of what runs now; and the Sessions screen's sections, narrowed by its filter: the
//! agents working now, the sessions running, the external threads still running and the recent ones
//! (waiting first, then newest).

use std::collections::{BTreeSet, HashSet};

use emberhut_api::ids::{RoomId, SessionId, TaskId};
use emberhut_api::runner::agents::AgentNow;
use emberhut_api::state::State;
use emberhut_api::threads::ExternalThread;
use emberhut_api::work::RunnerSession;

use super::filter::{Facts, Picks, SessionFilter};
use crate::store::lookup;

/// How many of the user's finished sessions the sidebar lists.
const SIDEBAR_FINISHED: usize = 3;

/// Every session once: the user's own (their list knows what a waiting one asks), then the
/// assistant's.
#[must_use]
pub(crate) fn all(state: &State) -> Vec<&RunnerSession> {
    let mine: HashSet<&SessionId> = state.my_sessions.iter().map(|s| &s.id).collect();
    state
        .my_sessions
        .iter()
        .chain(state.sessions.iter().filter(|s| !mine.contains(&s.id)))
        .collect()
}

/// A session by its id.
#[must_use]
pub(crate) fn find<'a>(state: &'a State, id: &SessionId) -> Option<&'a RunnerSession> {
    state
        .my_sessions
        .iter()
        .chain(&state.sessions)
        .find(|s| &s.id == id)
}

/// The sidebar's sessions: the user's waiting ones, the running ones (theirs and the
/// assistant's), then their last few finished ones.
#[must_use]
pub(crate) fn sidebar(state: &State) -> Vec<&RunnerSession> {
    let mine = &state.my_sessions;
    let assistants = state
        .sessions
        .iter()
        .filter(|s| !s.direct && s.is_running());
    mine.iter()
        .filter(|s| s.is_waiting())
        .chain(mine.iter().filter(|s| s.is_running()))
        .chain(assistants)
        .chain(
            mine.iter()
                .filter(|s| !s.is_running() && !s.is_waiting())
                .take(SIDEBAR_FINISHED),
        )
        .collect()
}

/// The sessions led by an agent working now: their agent's row stands for them.
fn led(state: &State) -> HashSet<&SessionId> {
    state
        .agents_now
        .iter()
        .filter(|a| a.lead)
        .filter_map(|a| a.viewer.as_ref())
        .collect()
}

/// What runs now, for the Sessions row's count: each agent, each running session no agent
/// stands for, and each external thread running.
#[must_use]
pub(crate) fn running_now(state: &State) -> usize {
    let led = led(state);
    state.agents_now.len()
        + all(state)
            .into_iter()
            .filter(|s| s.is_running() && !led.contains(&s.id))
            .count()
        + state
            .t3_running
            .iter()
            .filter(|t| t.status == "running")
            .count()
}

/// Whether there is any session, agent or external thread to show.
#[must_use]
pub(crate) fn has_any(state: &State) -> bool {
    !state.sessions.is_empty()
        || !state.my_sessions.is_empty()
        || !state.t3_running.is_empty()
        || !state.agents_now.is_empty()
}

/// The user's own sessions waiting on them, and running: the Your sessions row's note.
#[must_use]
pub(crate) fn mine_waiting_and_running(state: &State) -> (usize, usize) {
    let mine = &state.my_sessions;
    (
        mine.iter().filter(|s| s.is_waiting()).count(),
        mine.iter().filter(|s| s.is_running()).count(),
    )
}

/// A task's room: its project's, else Main.
#[must_use]
pub(crate) fn room_of_task(state: &State, task: TaskId) -> RoomId {
    let project = lookup::issue(state, task).and_then(|issue| issue.pid);
    state
        .rooms
        .iter()
        .find(|room| match project {
            Some(project) => room.project == Some(project),
            None => room.id.is_main(),
        })
        .map_or(RoomId::MAIN, |room| room.id)
}

/// The Sessions screen, as its filter leaves it.
#[derive(Debug, Default)]
pub(crate) struct Screen<'a> {
    /// The agents working now, in the server's order (each lead before its helpers).
    pub(crate) working: Vec<&'a AgentNow>,
    /// The sessions running that no agent row stands for.
    pub(crate) running: Vec<&'a RunnerSession>,
    /// The external threads still running (with every room only).
    pub(crate) threads: Vec<&'a ExternalThread>,
    /// The rest: waiting on the user first, then the newest.
    pub(crate) recent: Vec<&'a RunnerSession>,
    /// How many rows the filter hides.
    pub(crate) hidden: usize,
    /// The worker and environment menus' picks.
    pub(crate) picks: Picks,
    /// The rooms that have something here, so a room pick never comes up empty.
    pub(crate) rooms: Vec<RoomId>,
}

impl Screen<'_> {
    /// Whether nothing shows.
    #[must_use]
    pub(crate) fn is_empty(&self) -> bool {
        self.working.is_empty()
            && self.running.is_empty()
            && self.threads.is_empty()
            && self.recent.is_empty()
    }
}

/// The Sessions screen for `state` under `filter`.
#[must_use]
pub(crate) fn screen<'a>(state: &'a State, filter: &SessionFilter) -> Screen<'a> {
    let led = led(state);
    let sessions: Vec<&RunnerSession> = all(state)
        .into_iter()
        .filter(|s| !(s.is_running() && led.contains(&s.id)))
        .collect();
    let in_room = |room: RoomId| filter.room.is_none_or(|picked| picked == room);
    let room_agents: Vec<&AgentNow> = state
        .agents_now
        .iter()
        .filter(|a| in_room(room_of_task(state, a.task)))
        .collect();
    let room_sessions: Vec<&RunnerSession> = sessions
        .iter()
        .copied()
        .filter(|s| in_room(s.room))
        .collect();
    let room_threads: Vec<&ExternalThread> = if filter.room.is_none() {
        state.t3_running.iter().collect()
    } else {
        Vec::new()
    };
    let facts: Vec<Facts> = room_agents
        .iter()
        .map(|a| Facts::of_agent(a))
        .chain(room_sessions.iter().map(|s| Facts::of_session(s)))
        .chain(room_threads.iter().map(|t| Facts::of_thread(t)))
        .collect();
    let working: Vec<&AgentNow> = room_agents
        .into_iter()
        .filter(|a| filter.keeps(&Facts::of_agent(a)))
        .collect();
    let shown: Vec<&RunnerSession> = room_sessions
        .into_iter()
        .filter(|s| filter.keeps(&Facts::of_session(s)))
        .collect();
    let threads: Vec<&ExternalThread> = room_threads
        .into_iter()
        .filter(|t| filter.keeps(&Facts::of_thread(t)))
        .collect();
    let (running, mut recent): (Vec<_>, Vec<_>) = shown.into_iter().partition(|s| s.is_running());
    recent.sort_by(|a, b| {
        b.is_waiting()
            .cmp(&a.is_waiting())
            .then_with(|| b.updated_at.as_str().cmp(a.updated_at.as_str()))
    });
    let everything = state.agents_now.len() + sessions.len() + state.t3_running.len();
    let hidden =
        everything.saturating_sub(working.len() + running.len() + recent.len() + threads.len());
    let rooms: BTreeSet<RoomId> = sessions
        .iter()
        .map(|s| s.room)
        .chain(state.agents_now.iter().map(|a| room_of_task(state, a.task)))
        .collect();
    Screen {
        working,
        running,
        threads,
        recent,
        hidden,
        picks: filter.picks(&facts),
        rooms: rooms.into_iter().collect(),
    }
}

/// The agents working now grouped by task, in the order the tasks first come; within each,
/// whether the group has a lead (helpers then sit under it).
#[must_use]
pub(crate) fn by_task<'a>(agents: &[&'a AgentNow]) -> Vec<(bool, Vec<&'a AgentNow>)> {
    let mut groups: Vec<(TaskId, Vec<&AgentNow>)> = Vec::new();
    for agent in agents {
        match groups.iter_mut().find(|(task, _)| *task == agent.task) {
            Some((_, group)) => group.push(agent),
            None => groups.push((agent.task, vec![agent])),
        }
    }
    groups
        .into_iter()
        .map(|(_, group)| (group.iter().any(|a| a.lead), group))
        .collect()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::fixtures;
    use crate::store::sessions::filter::{Choice, Who};

    fn session(
        id: &str,
        status: &str,
        direct: bool,
        room: i64,
        updated: &str,
    ) -> serde_json::Value {
        json!({"id": id, "machine": "runner", "title": id, "project": "", "status": status,
            "updatedAt": updated, "place": "Linux PC", "doing": null, "account": null,
            "accountName": null, "ask": null, "last": null, "direct": direct, "model": null,
            "tasks": [], "room": room})
    }

    fn state() -> State {
        let mut state = fixtures::state(json!({
            "sessions": [
                session("r-run", "running", false, 2, "2026-11-25T08:00:00Z"),
                session("r-led", "running", false, 2, "2026-11-25T08:00:00Z"),
                session("r-mine", "waiting", true, 1, "2026-11-25T07:00:00Z")
            ],
            "mySessions": [
                session("r-mine", "waiting", true, 1, "2026-11-25T07:00:00Z"),
                session("r-done", "completed", true, 1, "2026-11-25T09:00:00Z"),
                session("r-old", "completed", true, 1, "2026-11-24T09:00:00Z")
            ],
            "agentsNow": [
                {"id": "lead-5", "task": 5, "key": "EX-5", "lead": true, "viewer": "r-led", "kind": "session"},
                {"id": "agent-1", "task": 5, "key": "EX-5", "lead": false, "kind": "helper"}
            ],
            "t3Running": [{"id": "t-1", "machine": "t3-mac", "title": "Old", "project": "", "status": "running",
                "updatedAt": "2026-11-25T06:00:00Z"}]
        }));
        state.rooms = vec![
            fixtures::room(1, json!({"project": null})),
            fixtures::room(2, json!({"project": 7})),
        ];
        state.issues = vec![fixtures::issue(5, "working", json!({"pid": 7}))];
        state
    }

    fn ids(sessions: &[&RunnerSession]) -> Vec<String> {
        sessions.iter().map(|s| s.id.to_string()).collect()
    }

    #[test]
    fn every_session_shows_once_and_the_sidebar_puts_waiting_first() {
        let state = state();
        assert_eq!(
            ids(&all(&state)),
            ["r-mine", "r-done", "r-old", "r-run", "r-led"]
        );
        assert_eq!(
            ids(&sidebar(&state)),
            ["r-mine", "r-run", "r-led", "r-done", "r-old"]
        );
        assert_eq!(
            running_now(&state),
            2 + 1 + 1,
            "two agents, r-run and the thread"
        );
        assert!(has_any(&state));
        assert_eq!(mine_waiting_and_running(&state), (1, 0));
        assert_eq!(room_of_task(&state, TaskId::new(5)), RoomId::new(2));
        assert_eq!(room_of_task(&state, TaskId::new(9)), RoomId::MAIN);
    }

    #[test]
    fn the_screen_has_its_sections_in_order() {
        let state = state();
        let screen = screen(&state, &SessionFilter::default());
        assert_eq!(screen.working.len(), 2);
        assert_eq!(
            ids(&screen.running),
            ["r-run"],
            "the led one is its agent's row"
        );
        assert_eq!(screen.threads.len(), 1);
        assert_eq!(ids(&screen.recent), ["r-mine", "r-done", "r-old"]);
        assert_eq!(screen.hidden, 0);
        assert_eq!(screen.rooms, [RoomId::MAIN, RoomId::new(2)]);
        assert!(by_task(&screen.working)[0].0);
    }

    #[test]
    fn a_room_and_a_worker_narrow_it() {
        let state = state();
        let mut filter = SessionFilter {
            room: Some(RoomId::new(2)),
            ..SessionFilter::default()
        };
        let screen = screen(&state, &filter);
        assert_eq!(ids(&screen.running), ["r-run"]);
        assert!(screen.recent.is_empty() && screen.threads.is_empty());
        assert_eq!(screen.working.len(), 2, "the task's room is Room 2");
        filter.room = None;
        filter.toggle(&Choice::Worker(Who::Direct));
        let screen = super::screen(&state, &filter);
        assert!(screen.working.is_empty() && screen.running.is_empty());
        assert_eq!(screen.recent.len(), 3);
        assert_eq!(screen.hidden, 4);
        assert!(!screen.is_empty());
    }
}
