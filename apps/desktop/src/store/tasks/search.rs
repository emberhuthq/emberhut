//! A search over every task (done and archived ones too), and the archived list: each asked of
//! the server. A search waits a moment while the user types, and only the answer to the latest
//! question is kept, so a slow answer to an older one never shows. Each follows the state: a
//! task found that the state has shows as the state has it, and each is asked again (what came
//! showing until the new answer does) when the state's tasks change, since a task archived,
//! unarchived or done moves in or out of what was found.

use std::time::Duration;

use emberhut_api::ids::TaskId;
use emberhut_api::tasks::Issue;

/// How long a search waits after a key before it asks.
pub(crate) const TYPING_PAUSE: Duration = Duration::from_millis(300);

/// How many tasks a search asks for.
pub(crate) const SEARCH_LIMIT: u32 = 50;

/// How many archived tasks the archived list asks for.
pub(crate) const ARCHIVED_LIMIT: u32 = 200;

/// Which question an answer is to.
pub(crate) type Ticket = u64;

/// Where a question to the server stands.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Found<'a> {
    /// Nothing asked.
    Idle,
    /// On its way.
    Loading,
    /// These came.
    Tasks(&'a [Issue]),
    /// It failed, worded for the user.
    Failed(&'a str),
}

/// One question about tasks to the server, asked again as it changes.
#[derive(Debug, Default)]
pub(crate) struct TaskLookup {
    ticket: Ticket,
    asked: bool,
    answer: Option<Result<Vec<Issue>, String>>,
}

impl TaskLookup {
    /// Asks anew: the earlier answer is dropped; the ticket to answer with.
    pub(crate) fn ask(&mut self) -> Ticket {
        self.ticket += 1;
        self.asked = true;
        self.answer = None;
        self.ticket
    }

    /// Stops asking: nothing shows.
    pub(crate) fn stop(&mut self) {
        self.ticket += 1;
        self.asked = false;
        self.answer = None;
    }

    /// Whether `ticket` is still the latest question.
    #[must_use]
    pub(crate) fn is_current(&self, ticket: Ticket) -> bool {
        self.asked && ticket == self.ticket
    }

    /// The answer to `ticket` came; one to an older question is dropped.
    pub(crate) fn finish(&mut self, ticket: Ticket, answer: Result<Vec<Issue>, String>) {
        if self.is_current(ticket) {
            self.answer = Some(answer);
        }
    }

    /// Asks again while it is asked, keeping what came until the new answer does: the ticket
    /// to answer with; `None` when nothing is asked.
    pub(crate) fn refresh(&mut self) -> Option<Ticket> {
        self.asked.then(|| {
            self.ticket += 1;
            self.ticket
        })
    }

    /// A new state's `tasks`: a task found that it has shows as it has it now (its status
    /// first of all).
    pub(crate) fn follow(&mut self, tasks: &[Issue]) {
        if let Some(Ok(found)) = &mut self.answer {
            for task in found.iter_mut() {
                if let Some(now) = tasks.iter().find(|now| now.id == task.id) {
                    task.clone_from(now);
                }
            }
        }
    }

    /// A new state's `tasks`, for the archived list: a task found that it has is out of it,
    /// since the state holds no archived task (the server leaves them out).
    pub(crate) fn drop_listed(&mut self, tasks: &[Issue]) {
        if let Some(Ok(found)) = &mut self.answer {
            found.retain(|task| tasks.iter().all(|now| now.id != task.id));
        }
    }

    /// Where it stands.
    #[must_use]
    pub(crate) fn found(&self) -> Found<'_> {
        match (&self.answer, self.asked) {
            (_, false) => Found::Idle,
            (None, true) => Found::Loading,
            (Some(Ok(tasks)), true) => Found::Tasks(tasks),
            (Some(Err(reason)), true) => Found::Failed(reason),
        }
    }
}

/// Which tasks a state holds, to tell when that changes (a task archived, unarchived or added
/// comes or goes).
#[derive(Debug, Default)]
pub(crate) struct StateTasks(Vec<TaskId>);

impl StateTasks {
    /// A new state's `tasks`: whether they are other tasks than the last state's.
    pub(crate) fn changed(&mut self, tasks: &[Issue]) -> bool {
        let mut ids: Vec<TaskId> = tasks.iter().map(|task| task.id).collect();
        ids.sort_unstable();
        ids.dedup();
        if ids == self.0 {
            return false;
        }
        self.0 = ids;
        true
    }
}

/// The search field's text and its lookup.
#[derive(Debug, Default)]
pub(crate) struct TaskSearch {
    text: String,
    lookup: TaskLookup,
}

impl TaskSearch {
    /// What is typed.
    #[must_use]
    pub(crate) fn text(&self) -> &str {
        &self.text
    }

    /// Whether a search is on (something but blanks typed).
    #[must_use]
    pub(crate) fn is_on(&self) -> bool {
        !self.text.trim().is_empty()
    }

    /// The user typed `text`: blank stops the search; else the ticket to ask with once the
    /// typing pauses.
    pub(crate) fn set_text(&mut self, text: &str) -> Option<Ticket> {
        text.clone_into(&mut self.text);
        if self.is_on() {
            Some(self.lookup.ask())
        } else {
            self.lookup.stop();
            None
        }
    }

    /// The filter changed: a search on is asked anew.
    pub(crate) fn again(&mut self) -> Option<Ticket> {
        self.is_on().then(|| self.lookup.ask())
    }

    /// The lookup.
    #[must_use]
    pub(crate) fn lookup(&self) -> &TaskLookup {
        &self.lookup
    }

    /// The lookup, to finish.
    pub(crate) fn lookup_mut(&mut self) -> &mut TaskLookup {
        &mut self.lookup
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::fixtures::issue;

    #[test]
    fn only_the_latest_searchs_answer_shows() {
        let mut search = TaskSearch::default();
        assert_eq!(search.lookup().found(), Found::Idle);
        let first = search.set_text("rep").unwrap();
        let second = search.set_text("report").unwrap();
        assert!(!search.lookup().is_current(first));
        search
            .lookup_mut()
            .finish(first, Ok(vec![issue(1, "todo", json!({}))]));
        assert_eq!(
            search.lookup().found(),
            Found::Loading,
            "an older answer is dropped"
        );
        search.lookup_mut().finish(second, Ok(Vec::new()));
        assert_eq!(search.lookup().found(), Found::Tasks(&[]));
        assert_eq!(search.again().map(|t| t > second), Some(true));
        assert_eq!(search.set_text("  "), None);
        assert_eq!(search.lookup().found(), Found::Idle);
        assert!(search.again().is_none());
    }

    #[test]
    fn asked_again_what_came_shows_until_the_new_answer() {
        let mut lookup = TaskLookup::default();
        assert_eq!(
            lookup.refresh(),
            None,
            "nothing asked: nothing to ask again"
        );
        let first = lookup.ask();
        lookup.finish(first, Ok(vec![issue(1, "todo", json!({}))]));
        let again = lookup.refresh().unwrap();
        assert!(again > first);
        assert!(matches!(lookup.found(), Found::Tasks(tasks) if tasks.len() == 1));
        lookup.finish(first, Ok(Vec::new()));
        assert!(
            matches!(lookup.found(), Found::Tasks(tasks) if tasks.len() == 1),
            "an older answer is still dropped"
        );
        lookup.finish(again, Ok(Vec::new()));
        assert_eq!(lookup.found(), Found::Tasks(&[]));
    }

    #[test]
    fn what_was_found_follows_the_state() {
        let mut lookup = TaskLookup::default();
        let ticket = lookup.ask();
        lookup.finish(
            ticket,
            Ok(vec![
                issue(1, "todo", json!({})),
                issue(2, "todo", json!({})),
            ]),
        );
        lookup.follow(&[issue(2, "done", json!({})), issue(3, "todo", json!({}))]);
        let Found::Tasks(tasks) = lookup.found() else {
            panic!("the tasks found stay")
        };
        let statuses: Vec<&str> = tasks.iter().map(|task| task.status.as_str()).collect();
        assert_eq!(statuses, ["todo", "done"], "task 2 as the state has it now");
        lookup.drop_listed(&[issue(1, "todo", json!({}))]);
        assert!(
            matches!(lookup.found(), Found::Tasks(tasks) if tasks.len() == 1 && tasks[0].id == TaskId::new(2)),
            "task 1 is open again: out of the archived list"
        );
    }

    #[test]
    fn the_states_tasks_change_when_one_comes_or_goes() {
        let mut seen = StateTasks::default();
        let (one, two) = (issue(1, "todo", json!({})), issue(2, "todo", json!({})));
        assert!(!seen.changed(&[]), "none, as before");
        assert!(seen.changed(&[one.clone(), two.clone()]));
        assert!(
            !seen.changed(&[two.clone(), one.clone()]),
            "order doesn't count"
        );
        assert!(
            !seen.changed(&[issue(1, "done", json!({})), two.clone()]),
            "a status alone isn't a change of tasks"
        );
        assert!(seen.changed(&[two]), "one went");
    }

    #[test]
    fn a_failed_lookup_says_why() {
        let mut lookup = TaskLookup::default();
        let ticket = lookup.ask();
        lookup.finish(ticket, Err("offline".into()));
        assert_eq!(lookup.found(), Found::Failed("offline"));
        lookup.stop();
        lookup.finish(ticket, Ok(Vec::new()));
        assert_eq!(lookup.found(), Found::Idle);
    }
}
