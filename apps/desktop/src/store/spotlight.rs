//! Search, as a spotlight over the window (⌘K): one field that finds tasks (by key, title or
//! text, done and archived ones too, asked of the server after a pause in the typing), projects,
//! agents (by name or who they work for, so an agent stays one search away while the sidebar
//! hides them) and people (what the assistant knows about them), and opens the one picked. With
//! nothing typed it lists the latest tasks. Projects, agents and people come first (few, and the
//! likeliest pick), then the tasks, open ones before finished ones.

use std::sync::Arc;
use std::time::Duration;

use emberhut_api::agents::AgentRow;
use emberhut_api::facts::Fact;
use emberhut_api::ids::{FactId, ProjectId, RoomId, TaskId};
use emberhut_api::state::State;
use emberhut_api::task_search::TaskQuery;
use emberhut_api::tasks::{Issue, Project, ProjectStatus, TaskStatus};
use gpui_kit::{AppContext as _, Context};

use super::tasks::search::{Found as Lookup, TYPING_PAUSE, TaskLookup, Ticket};
use super::{Store, lookup, update_or_log};

/// How many tasks a search asks for, and shows.
pub(crate) const TASKS_FOUND: u32 = 12;

/// How many of the latest tasks show with nothing typed.
pub(crate) const LATEST_SHOWN: usize = 8;

/// How many people show.
pub(crate) const PEOPLE_SHOWN: usize = 6;

/// Something the spotlight found, to open.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Found {
    /// A task: its page.
    Task(TaskId),
    /// A project: its page.
    Project(ProjectId),
    /// An agent: its room.
    Agent(RoomId),
    /// A person the assistant knows: their page.
    Person(FactId),
}

/// A person the assistant knows, as a page names them: a small key the store gives each
/// person opened (a fact's id is text, and a page is a plain value).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct PersonKey(usize);

/// The people opened as pages, each under its key.
#[derive(Debug, Default)]
pub(crate) struct PersonKeys(Vec<FactId>);

impl PersonKeys {
    /// `person`'s key, given now if they have none yet.
    pub(crate) fn key(&mut self, person: &FactId) -> PersonKey {
        let at = self
            .0
            .iter()
            .position(|known| known == person)
            .unwrap_or_else(|| {
                self.0.push(person.clone());
                self.0.len() - 1
            });
        PersonKey(at)
    }

    /// The person under `key`.
    #[must_use]
    pub(crate) fn person(&self, key: PersonKey) -> Option<&FactId> {
        self.0.get(key.0)
    }
}

/// The spotlight's field, what it found, and which result is picked.
#[derive(Debug, Default)]
pub(crate) struct Spotlight {
    text: String,
    tasks: TaskLookup,
    /// The result the arrows picked.
    selected: usize,
}

/// What the spotlight shows over one state.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Results<'a> {
    /// The projects whose name matches.
    pub(crate) projects: Vec<&'a Project>,
    /// The agents with rooms of their own whose name, or whom they work for, matches.
    pub(crate) agents: Vec<&'a AgentRow>,
    /// The people whose name or note matches.
    pub(crate) people: Vec<&'a Fact>,
    /// The tasks found: none yet while asking.
    pub(crate) tasks: Option<Vec<&'a Issue>>,
    /// Why the tasks didn't come, if they didn't.
    pub(crate) failed: Option<&'a str>,
    /// Every result in the order shown, for the arrows and Return.
    pub(crate) order: Vec<Found>,
    /// The one picked.
    pub(crate) selected: usize,
}

impl Spotlight {
    /// What is typed.
    #[must_use]
    pub(crate) fn text(&self) -> &str {
        &self.text
    }

    /// Whether nothing but blanks is typed: the latest tasks show.
    #[must_use]
    pub(crate) fn is_blank(&self) -> bool {
        self.text.trim().is_empty()
    }

    /// What it shows over `state` and the `people` the assistant knows, projects left out
    /// while the server hides them.
    #[must_use]
    pub(crate) fn results<'a>(
        &'a self,
        state: Option<&'a State>,
        people: Option<&'a [Fact]>,
    ) -> Results<'a> {
        let words: Vec<String> = self
            .text
            .split_whitespace()
            .map(str::to_lowercase)
            .collect();
        let hits = |text: &str| {
            let text = text.to_lowercase();
            !words.is_empty() && words.iter().all(|word| text.contains(word.as_str()))
        };
        let projects: Vec<&Project> = state
            .filter(|state| !lookup::shows_tasks(state))
            .map_or_default(|state| {
                state
                    .projects
                    .iter()
                    .filter(|project| project.status != ProjectStatus::Done && hits(&project.name))
                    .collect()
            });
        let agents: Vec<&AgentRow> = state.map_or_default(|state| {
            state
                .agents
                .iter()
                .filter(|agent| !agent.id.is_main() && !agent.room.is_main())
                .filter(|agent| hits(&format!("{} {}", agent.name, agent.principal)))
                .collect()
        });
        let people: Vec<&Fact> = people.map_or_default(|people| {
            people
                .iter()
                .filter(|person| hits(&format!("{} {}", person.title, person.text)))
                .take(PEOPLE_SHOWN)
                .collect()
        });
        let shown = if self.is_blank() {
            LATEST_SHOWN
        } else {
            TASKS_FOUND as usize
        };
        let (tasks, failed) = match self.tasks.found() {
            Lookup::Tasks(found) => (Some(open_first(found, shown)), None),
            Lookup::Failed(reason) => (Some(Vec::new()), Some(reason)),
            Lookup::Idle | Lookup::Loading => (None, None),
        };
        let order: Vec<Found> = projects
            .iter()
            .map(|project| Found::Project(project.id))
            .chain(agents.iter().map(|agent| Found::Agent(agent.room)))
            .chain(people.iter().map(|person| Found::Person(person.id.clone())))
            .chain(tasks.iter().flatten().map(|task| Found::Task(task.id)))
            .collect();
        let selected = self.selected.min(order.len().saturating_sub(1));
        Results {
            projects,
            agents,
            people,
            tasks,
            failed,
            order,
            selected,
        }
    }

    /// Moves the pick by `step` within `count` results.
    fn step(&mut self, step: isize, count: usize) {
        let last = count.saturating_sub(1);
        self.selected = self
            .selected
            .min(last)
            .saturating_add_signed(step)
            .min(last);
    }
}

/// The reason to show for `error`: the server's own words, else ours.
pub(crate) fn reason(error: &crate::backend::Error) -> String {
    error
        .server_message()
        .map_or_else(|| super::status::problem(error), str::to_owned)
}

/// The tasks found, open ones first, each part as the server sent it (newest first), `shown`
/// at most.
fn open_first(found: &[Issue], shown: usize) -> Vec<&Issue> {
    let finished = |task: &&Issue| matches!(task.status, TaskStatus::Done | TaskStatus::Canceled);
    found
        .iter()
        .filter(|task| !finished(task))
        .chain(found.iter().filter(finished))
        .take(shown)
        .collect()
}

impl Store {
    /// The spotlight.
    #[must_use]
    pub(crate) fn spotlight(&self) -> &Spotlight {
        &self.spotlight
    }

    /// The spotlight opened: the field starts empty with the latest tasks, and the people the
    /// assistant knows are fetched.
    pub(crate) fn open_spotlight(&mut self, cx: &mut Context<'_, Self>) {
        self.spotlight.text.clear();
        self.spotlight.selected = 0;
        let ticket = self.spotlight.tasks.ask();
        Self::ask_spotlight(ticket, Duration::ZERO, cx);
        self.fetch_people(None, cx);
        cx.notify();
    }

    /// The user typed `text` in the spotlight: the pick goes back to the first result, and the
    /// tasks are asked once the typing pauses (at once when it is cleared: the latest).
    pub(crate) fn spotlight_type(&mut self, text: &str, cx: &mut Context<'_, Self>) {
        if self.spotlight.text == text {
            return;
        }
        text.clone_into(&mut self.spotlight.text);
        self.spotlight.selected = 0;
        let ticket = self.spotlight.tasks.ask();
        let pause = if self.spotlight.is_blank() {
            Duration::ZERO
        } else {
            TYPING_PAUSE
        };
        Self::ask_spotlight(ticket, pause, cx);
        cx.notify();
    }

    /// Moves the spotlight's pick by `step` (↑ is -1, ↓ is 1).
    pub(crate) fn spotlight_step(&mut self, step: isize, cx: &mut Context<'_, Self>) {
        let count = self
            .spotlight
            .results(self.state.as_ref(), self.places.people())
            .order
            .len();
        self.spotlight.step(step, count);
        cx.notify();
    }

    /// Asks the server for the spotlight's tasks with `ticket` after `pause`, if it is still
    /// the latest question.
    fn ask_spotlight(ticket: Ticket, pause: Duration, cx: &mut Context<'_, Self>) {
        cx.spawn(async move |this, cx| {
            if !pause.is_zero() {
                cx.background_executor().timer(pause).await;
            }
            let asked = update_or_log(&this, cx, |store, _| {
                store.spotlight.tasks.is_current(ticket).then(|| {
                    let query = TaskQuery {
                        limit: Some(TASKS_FOUND),
                        ..TaskQuery::search(store.spotlight.text.trim())
                    };
                    (Arc::clone(&store.backend), query)
                })
            })
            .flatten();
            let Some((backend, query)) = asked else {
                return;
            };
            let found = cx
                .background_spawn(async move { backend.find_tasks(&query) })
                .await;
            update_or_log(&this, cx, |store, cx| {
                store
                    .spotlight
                    .tasks
                    .finish(ticket, found.map_err(|error| reason(&error)));
                cx.notify();
            });
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::fixtures::{issue, state};

    fn person(id: &str, title: &str, text: &str) -> Fact {
        serde_json::from_value(
            json!({"id": id, "category": "people", "title": title, "text": text,
                                      "source": "", "updated": "2026-11-20T10:00:00Z"}),
        )
        .unwrap()
    }

    fn project(id: i64, name: &str, status: &str) -> serde_json::Value {
        json!({"id": id, "name": name, "goal": null, "priority": 0, "kind": "ongoing",
            "deadline": null, "hard": false, "status": status, "pausedUntil": null, "open": 2,
            "doneWeek": 0, "health": null, "next": null, "hoursLeft": 0.0})
    }

    /// The people the tests' assistant knows (kept for the test run, as results borrow them).
    fn people() -> &'static [Fact] {
        Box::leak(Box::new([
            person("people/a", "Example Person", "Runs the bakery"),
            person("people/b", "Someone Else", "A neighbour"),
        ]))
    }

    fn typed(text: &str) -> Spotlight {
        let mut spotlight = Spotlight {
            text: text.to_owned(),
            ..Spotlight::default()
        };
        let ticket = spotlight.tasks.ask();
        spotlight.tasks.finish(
            ticket,
            Ok((1..=14)
                .map(|id| issue(id, if id % 2 == 0 { "done" } else { "todo" }, json!({})))
                .collect()),
        );
        spotlight
    }

    #[test]
    fn projects_and_people_come_first_then_open_tasks_then_finished_ones() {
        let lists = state(json!({
            "ui": {"hideProjects": false, "hideGoals": false},
            "projects": [project(1, "Bakery ads", "active"), project(2, "Old bakery", "done"),
                         project(3, "Garden", "paused")],
        }));
        let spotlight = typed("bakery");
        let results = spotlight.results(Some(&lists), Some(people()));
        assert_eq!(results.projects.len(), 1, "a done project isn't found");
        assert_eq!(results.people.len(), 1, "its note matches");
        let tasks: Vec<i64> = results
            .tasks
            .as_ref()
            .unwrap()
            .iter()
            .map(|task| task.id.get())
            .collect();
        assert_eq!(tasks, [1, 3, 5, 7, 9, 11, 13, 2, 4, 6, 8, 10], "12 at most");
        assert_eq!(results.order[0], Found::Project(ProjectId::new(1)));
        assert_eq!(results.order[1], Found::Person(FactId::from("people/a")));
        assert_eq!(results.order[2], Found::Task(TaskId::new(1)));
    }

    #[test]
    fn every_word_has_to_match_and_nothing_typed_lists_the_latest() {
        let lists = state(json!({"ui": {"hideProjects": false, "hideGoals": false},
                                 "projects": [project(1, "Bakery ads", "active")]}));
        let found = |text: &str| {
            typed(text)
                .results(Some(&lists), Some(people()))
                .projects
                .len()
        };
        assert_eq!(found("ads bakery"), 1);
        assert_eq!(found("ads garden"), 0);
        let blank = typed("  ");
        let results = blank.results(Some(&lists), Some(people()));
        assert_eq!((results.projects.len(), results.people.len()), (0, 0));
        assert_eq!(results.tasks.unwrap().len(), LATEST_SHOWN);
    }

    /// An agent is found by its name or whom it works for and opens its room; the default
    /// agent, whose room is Main, is not listed.
    #[test]
    fn agents_are_found_by_name_or_whom_they_work_for() {
        let lists = state(json!({
            "rooms": [crate::fixtures::room(1, json!({})), crate::fixtures::room(9, json!({}))],
            "agents": [
                {"id": "main", "name": "Main helper", "principal": "the owner", "room": 1,
                 "enabled": true, "pendingApprovals": 0},
                {"id": "client-a", "name": "Example helper", "principal": "Example Bakery",
                 "room": 9, "enabled": false, "pendingApprovals": 0}
            ]
        }));
        let found = |text: &str| {
            typed(text)
                .results(Some(&lists), Some(people()))
                .agents
                .iter()
                .map(|agent| agent.room.get())
                .collect::<Vec<_>>()
        };
        assert_eq!(found("helper"), [9], "the default agent isn't listed");
        assert_eq!(found("bakery example"), [9], "whom it works for");
        assert_eq!(found("garden"), Vec::<i64>::new());
        let spotlight = typed("bakery");
        let results = spotlight.results(Some(&lists), Some(people()));
        assert_eq!(results.order[0], Found::Agent(RoomId::new(9)));
        assert_eq!(results.order[1], Found::Person(FactId::from("people/a")));
    }

    #[test]
    fn projects_stay_out_while_the_server_hides_them() {
        let lists = state(json!({"projects": [project(1, "Bakery ads", "active")]}));
        assert_eq!(
            typed("bakery")
                .results(Some(&lists), Some(people()))
                .projects
                .len(),
            0
        );
    }

    #[test]
    fn the_arrows_move_the_pick_within_the_results() {
        let mut spotlight = typed("bakery");
        let count = spotlight.results(None, Some(people())).order.len();
        assert_eq!(count, 13, "a person and 12 tasks");
        spotlight.step(-1, count);
        assert_eq!(spotlight.selected, 0, "not above the first");
        spotlight.step(1, count);
        spotlight.step(1, count);
        assert_eq!(spotlight.results(None, Some(people())).selected, 2);
        for _ in 0..20 {
            spotlight.step(1, count);
        }
        assert_eq!(spotlight.selected, 12, "not past the last");
    }

    #[test]
    fn while_asking_there_are_no_tasks_yet() {
        let mut spotlight = Spotlight::default();
        spotlight.tasks.ask();
        let results = spotlight.results(None, Some(people()));
        assert_eq!(results.tasks, None);
        assert_eq!(results.order, []);
    }
}
