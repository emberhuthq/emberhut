//! The sessions with no task, on a server that keeps plans: the Tasks list shows them as items
//! of their own at its foot, so no work disappears when the sessions leave the main navigation:
//! those waiting on the user first, then those running, then the rest that moved in the last
//! day; older finished ones stay in Sessions.

use emberhut_api::state::State;
use emberhut_api::work::RunnerSession;
use jiff::Timestamp;

use super::clock;
use crate::store::sessions::lists;

/// How long a finished session with no task stays on the Tasks list, in seconds.
const RECENT_FOR: i64 = 24 * 60 * 60;

/// How many finished sessions with no task the list shows.
const RECENT_SHOWN: usize = 5;

/// The sessions with no task, as the Tasks list places them.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct Loose<'a> {
    /// Waiting on the user.
    pub(crate) waiting: Vec<&'a RunnerSession>,
    /// Running or about to.
    pub(crate) running: Vec<&'a RunnerSession>,
    /// Finished in the last day, newest first.
    pub(crate) recent: Vec<&'a RunnerSession>,
}

impl Loose<'_> {
    /// How many there are.
    #[must_use]
    pub(crate) fn len(&self) -> usize {
        self.waiting.len() + self.running.len() + self.recent.len()
    }

    /// Whether there are none: the list shows no foot for them then.
    #[must_use]
    pub(crate) fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// The sessions in `state` that work on no task, as of `now`: the user's own and the
/// assistant's once each.
#[must_use]
pub(crate) fn loose(state: &State, now: Timestamp) -> Loose<'_> {
    let taskless: Vec<&RunnerSession> = lists::all(state)
        .into_iter()
        .filter(|session| session.tasks.is_empty())
        .collect();
    let fresh = |session: &RunnerSession| {
        clock::instant(&session.updated_at)
            .is_some_and(|at| now.as_second() - at.as_second() <= RECENT_FOR)
    };
    let mut recent: Vec<&RunnerSession> = taskless
        .iter()
        .copied()
        .filter(|s| !s.is_waiting() && !s.is_running() && fresh(s))
        .collect();
    recent.sort_by(|a, b| b.updated_at.as_str().cmp(a.updated_at.as_str()));
    recent.truncate(RECENT_SHOWN);
    Loose {
        waiting: taskless
            .iter()
            .copied()
            .filter(|s| s.is_waiting())
            .collect(),
        running: taskless
            .iter()
            .copied()
            .filter(|s| s.is_running())
            .collect(),
        recent,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;
    use crate::fixtures::state;

    fn session(id: &str, status: &str, updated: &str, tasks: &[i64]) -> Value {
        json!({"id": id, "machine": "pc", "title": id, "project": "", "status": status,
            "updatedAt": updated, "place": "PC", "doing": null, "account": null,
            "accountName": null, "ask": null, "last": null, "direct": true, "model": null,
            "tasks": tasks})
    }

    fn ids(sessions: &[&RunnerSession]) -> Vec<String> {
        sessions.iter().map(|s| s.id.as_str().to_owned()).collect()
    }

    #[test]
    fn sessions_with_no_task_go_where_they_belong_and_old_ones_stay_out() {
        let now: Timestamp = "2026-11-28T09:00:00Z".parse().unwrap();
        let state = state(json!({
            "mySessions": [
                session("ask", "waiting", "2026-11-20T00:00:00Z", &[]),
                session("run", "running", "2026-11-28T08:59:00Z", &[]),
                session("tasked", "running", "2026-11-28T08:59:00Z", &[4]),
                session("old", "completed", "2026-11-26T00:00:00Z", &[]),
                session("new", "completed", "2026-11-28T08:00:00Z", &[]),
            ],
            "sessions": [
                session("run", "running", "2026-11-28T08:59:00Z", &[]),
                session("newer", "failed", "2026-11-28T08:30:00Z", &[]),
            ],
        }));
        let loose = loose(&state, now);
        assert_eq!(ids(&loose.waiting), ["ask"], "waiting, however old");
        assert_eq!(
            ids(&loose.running),
            ["run"],
            "once, and not one with a task"
        );
        assert_eq!(
            ids(&loose.recent),
            ["newer", "new"],
            "the last day's, newest first"
        );
        assert_eq!(loose.len(), 4);
    }

    /// Once every session is on a task (the server gives new ones one), and the finished ones
    /// with none are older than a day, there is no foot to show.
    #[test]
    fn with_every_session_on_a_task_there_are_none() {
        let now: Timestamp = "2026-11-28T09:00:00Z".parse().unwrap();
        let state = state(json!({
            "mySessions": [
                session("mine", "running", "2026-11-28T08:59:00Z", &[4]),
                session("old", "completed", "2026-11-20T00:00:00Z", &[]),
            ],
            "sessions": [session("assistant", "waiting", "2026-11-28T08:00:00Z", &[5])],
        }));
        assert!(loose(&state, now).is_empty());
        assert!(!loose(&state, "2026-11-20T09:00:00Z".parse().unwrap()).is_empty());
    }
}
