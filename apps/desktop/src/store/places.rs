//! What a project's page and a person's page fetch while they show: a project's done tasks
//! outside its areas (all of them, not only this week's the state has), and the people the
//! assistant knows with the tasks that name a person. Each is asked again whenever the page is
//! shown afresh; what came shows until the new answer does.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use emberhut_api::facts::{Fact, FactCategory};
use emberhut_api::ids::{AreaId, FactId, ProjectId, TaskId};
use emberhut_api::task_search::{ProjectScope, TaskQuery};
use gpui_kit::{AppContext as _, Context};

use super::projects::ProjectFilter;
use super::spotlight::{PersonKey, reason};
use super::tasks::search::{TaskLookup, Ticket};
use super::{Page, Store, update_or_log};

/// How many of a project's done tasks its page asks for.
pub(crate) const PROJECT_DONE_LIMIT: u32 = 200;

/// How many tasks naming a person their page asks for.
pub(crate) const PERSON_TASKS_LIMIT: u32 = 20;

/// The places' fetched lists.
#[derive(Debug, Default)]
pub(crate) struct Places {
    /// Each shown project's done tasks.
    project_done: HashMap<ProjectId, TaskLookup>,
    /// The tasks naming each shown person.
    person_tasks: HashMap<PersonKey, TaskLookup>,
    /// The people the assistant knows, once fetched.
    people: Option<Vec<Fact>>,
    /// The project pages' filter, for the session.
    project_filter: ProjectFilter,
    /// The folds opened (or a board's columns folded) on the places' pages, for the session.
    open: HashSet<String>,
    /// The scan each area's findings are narrowed to, for the session (none: every scan).
    area_scans: HashMap<AreaId, TaskId>,
}

impl Places {
    /// A project's done tasks, once asked.
    #[must_use]
    pub(crate) fn project_done(&self, project: ProjectId) -> Option<&TaskLookup> {
        self.project_done.get(&project)
    }

    /// The tasks naming a person, once asked.
    #[must_use]
    pub(crate) fn person_tasks(&self, person: PersonKey) -> Option<&TaskLookup> {
        self.person_tasks.get(&person)
    }

    /// The people the assistant knows, once fetched.
    #[must_use]
    pub(crate) fn people(&self) -> Option<&[Fact]> {
        self.people.as_deref()
    }

    /// The project pages' filter.
    #[must_use]
    pub(crate) fn project_filter(&self) -> ProjectFilter {
        self.project_filter
    }

    /// The scan `area`'s findings are narrowed to, if one is picked.
    #[must_use]
    pub(crate) fn area_scan(&self, area: AreaId) -> Option<TaskId> {
        self.area_scans.get(&area).copied()
    }

    /// Whether the fold `key` is turned from how it starts.
    #[must_use]
    pub(crate) fn is_flipped(&self, key: &str) -> bool {
        self.open.contains(key)
    }
}

/// What a person's page searches the tasks for: their first name (a task names them so).
#[must_use]
pub(crate) fn person_query(person: &Fact) -> String {
    person
        .title
        .split_whitespace()
        .next()
        .unwrap_or(&person.title)
        .to_owned()
}

impl Store {
    /// The places' fetched lists.
    #[must_use]
    pub(crate) fn places(&self) -> &Places {
        &self.places
    }

    /// Narrows the project pages.
    pub(crate) fn set_project_filter(&mut self, filter: ProjectFilter, cx: &mut Context<'_, Self>) {
        self.places.project_filter = filter;
        cx.notify();
    }

    /// Narrows `area`'s findings to `scan` (none: every scan).
    pub(crate) fn pick_area_scan(
        &mut self,
        area: AreaId,
        scan: Option<TaskId>,
        cx: &mut Context<'_, Self>,
    ) {
        match scan {
            Some(scan) => self.places.area_scans.insert(area, scan),
            None => self.places.area_scans.remove(&area),
        };
        cx.notify();
    }

    /// Turns the fold `key` on a place's page the other way.
    pub(crate) fn flip_place_fold(&mut self, key: &str, cx: &mut Context<'_, Self>) {
        if !self.places.open.remove(key) {
            self.places.open.insert(key.to_owned());
        }
        cx.notify();
    }

    /// The key a person's page goes by.
    pub(crate) fn person_key(&mut self, person: &FactId) -> PersonKey {
        self.person_keys.key(person)
    }

    /// The person a page's key stands for, if the assistant still knows them.
    #[must_use]
    pub(crate) fn person(&self, key: PersonKey) -> Option<&Fact> {
        let id = self.person_keys.person(key)?;
        self.places.people()?.iter().find(|fact| fact.id == *id)
    }

    /// A project's or a person's page is shown afresh: what it lists is asked for.
    pub(super) fn place_shown(&mut self, page: Page, cx: &mut Context<'_, Self>) {
        match page {
            Page::Project(project) => {
                let ticket = self.places.project_done.entry(project).or_default().ask();
                let query = TaskQuery {
                    project: Some(ProjectScope::Project(project)),
                    closed: true,
                    limit: Some(PROJECT_DONE_LIMIT),
                    ..TaskQuery::default()
                };
                self.find_for_place(Found::Project(project), ticket, query, cx);
            }
            Page::Person(person) => self.fetch_people(Some(person), cx),
            _ => {}
        }
    }

    /// Fetches the people the assistant knows; then, for `person`, the tasks naming them.
    pub(super) fn fetch_people(&mut self, person: Option<PersonKey>, cx: &mut Context<'_, Self>) {
        let backend = Arc::clone(&self.backend);
        cx.spawn(async move |this, cx| {
            let page = cx.background_spawn(async move { backend.facts() }).await;
            update_or_log(&this, cx, |store, cx| {
                match page {
                    Ok(page) => {
                        store.places.people = Some(
                            page.facts
                                .into_iter()
                                .filter(|fact| fact.category == FactCategory::People)
                                .collect(),
                        );
                    }
                    Err(error) => log::warn!("couldn't fetch the people: {error}"),
                }
                if let Some(key) = person
                    && let Some(query) = store.person(key).map(person_query)
                {
                    let ticket = store.places.person_tasks.entry(key).or_default().ask();
                    let query = TaskQuery {
                        limit: Some(PERSON_TASKS_LIMIT),
                        ..TaskQuery::search(query)
                    };
                    store.find_for_place(Found::Person(key), ticket, query, cx);
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Asks the server for `query`, answering the lookup of `place` with `ticket`.
    fn find_for_place(
        &mut self,
        place: Found,
        ticket: Ticket,
        query: TaskQuery,
        cx: &mut Context<'_, Self>,
    ) {
        let backend = Arc::clone(&self.backend);
        cx.spawn(async move |this, cx| {
            let found = cx
                .background_spawn(async move { backend.find_tasks(&query) })
                .await
                .map_err(|error| reason(&error));
            update_or_log(&this, cx, |store, cx| {
                let lookup = match place {
                    Found::Project(project) => store.places.project_done.get_mut(&project),
                    Found::Person(person) => store.places.person_tasks.get_mut(&person),
                };
                if let Some(lookup) = lookup {
                    lookup.finish(ticket, found);
                }
                cx.notify();
            });
        })
        .detach();
    }
}

/// Whose lookup an answer is for.
#[derive(Clone, Copy, Debug)]
enum Found {
    Project(ProjectId),
    Person(PersonKey),
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn a_persons_tasks_are_found_by_their_first_name() {
        let person: Fact = serde_json::from_value(json!({"id": "people/a", "category": "people",
            "title": "Example Person", "text": "", "source": "", "updated": "2026-11-20T10:00:00Z"}))
        .unwrap();
        assert_eq!(person_query(&person), "Example");
    }
}
