//! About you: what the assistant knows about the user (`GET /api/facts`), fetched when the page
//! opens, and the user's fixes to it. A fix shows at once (a corrected fact, a person marked,
//! a fact forgotten, a rule dropped) and goes to the server after. Then the page is fetched
//! again, so it shows what the server keeps (a correction's source carries its date there); if
//! the server refused, the app's line for a refused action says what failed and why
//! (`Not forgotten: <reason>`).

use std::sync::Arc;

use gpui_kit::AppContext as _;
use gpui_kit::Context;

use crate::store::gate::reason;
use crate::store::{Store, update_or_log};
use emberhut_api::facts::{Fact, FactCategory, FactsPage, Trial};
use emberhut_api::ids::{FactId, TrialId};
use emberhut_api::settings::{FactChange, PersonChange, SettingsAction};

/// The categories listed, in their order (evidence kept for later isn't listed).
pub(crate) const LISTED: [FactCategory; 7] = [
    FactCategory::People,
    FactCategory::Work,
    FactCategory::Money,
    FactCategory::Preferences,
    FactCategory::Health,
    FactCategory::Home,
    FactCategory::Projects,
];

/// A category's title.
#[must_use]
pub(crate) fn category_title(category: &FactCategory) -> &'static str {
    match category {
        FactCategory::People => "People",
        FactCategory::Work => "Work",
        FactCategory::Money => "Money",
        FactCategory::Preferences => "How you like things",
        FactCategory::Health => "Health & habits",
        FactCategory::Home => "Home & family",
        FactCategory::Projects => "Projects",
        _ => "Other",
    }
}

/// The source a fact shows once the user corrected it here.
pub(crate) const CORRECTED: &str = "you corrected it";

/// What the assistant knows, as last fetched.
#[derive(Debug, Default)]
pub(crate) struct FactsFeed {
    page: Option<FactsPage>,
    loading: bool,
    /// Why the last fetch failed.
    read_problem: Option<String>,
}

impl FactsFeed {
    /// The page, once it came.
    #[must_use]
    pub(crate) fn page(&self) -> Option<&FactsPage> {
        self.page.as_ref()
    }

    /// Whether it is being fetched.
    #[must_use]
    pub(crate) fn is_loading(&self) -> bool {
        self.loading
    }

    /// Why the last fetch failed, worded for the user (a fix that failed says so on the app's
    /// line for refused actions, [`Store::action_problem`]).
    #[must_use]
    pub(crate) fn problem(&self) -> Option<&str> {
        self.read_problem.as_deref()
    }

    /// A fact, by its id.
    #[must_use]
    pub(crate) fn fact(&self, id: &FactId) -> Option<&Fact> {
        self.page.as_ref()?.facts.iter().find(|fact| &fact.id == id)
    }

    /// Applies a fix to the page on hand, as the server will.
    fn apply(&mut self, action: &SettingsAction) {
        let Some(page) = &mut self.page else {
            return;
        };
        match action {
            SettingsAction::FactSet(change) => {
                if let Some(fact) = page.facts.iter_mut().find(|fact| fact.id == change.id) {
                    fact.text.clone_from(&change.text);
                    CORRECTED.clone_into(&mut fact.source);
                }
            }
            SettingsAction::PersonSet(change) => {
                if let Some(fact) = page.facts.iter_mut().find(|fact| fact.id == change.id) {
                    fact.important = Some(change.important);
                }
            }
            SettingsAction::FactDrop(id) => page.facts.retain(|fact| &fact.id != id),
            SettingsAction::TrialDrop(id) => page.trials.retain(|trial| &trial.id != id),
            _ => {}
        }
    }
}

/// Whether `text` holds `query`, ignoring case; a blank query matches everything.
#[must_use]
pub(crate) fn matches(query: &str, text: &str) -> bool {
    let query = query.trim();
    query.is_empty() || text.to_lowercase().contains(&query.to_lowercase())
}

/// The facts that match `query` (by their title and text), by category in [`LISTED`]'s order;
/// a category with none is left out.
#[must_use]
pub(crate) fn grouped<'a>(page: &'a FactsPage, query: &str) -> Vec<(FactCategory, Vec<&'a Fact>)> {
    LISTED
        .iter()
        .filter_map(|category| {
            let facts: Vec<_> = page
                .facts
                .iter()
                .filter(|fact| {
                    &fact.category == category
                        && matches(query, &format!("{} {}", fact.title, fact.text))
                })
                .collect();
            (!facts.is_empty()).then(|| (category.clone(), facts))
        })
        .collect()
}

/// The rules on trial that match `query`.
#[must_use]
pub(crate) fn trials<'a>(page: &'a FactsPage, query: &str) -> Vec<&'a Trial> {
    page.trials
        .iter()
        .filter(|trial| matches(query, &trial.text))
        .collect()
}

/// What a fix that failed was, for its line: `Not saved`, `Not forgotten`, `Not dropped`.
fn failure(action: &SettingsAction) -> &'static str {
    match action {
        SettingsAction::FactDrop(_) => "Not forgotten",
        SettingsAction::TrialDrop(_) => "Not dropped",
        _ => "Not saved",
    }
}

impl Store {
    /// What the assistant knows about the user.
    #[must_use]
    pub(crate) fn facts(&self) -> &FactsFeed {
        &self.settings.facts
    }

    /// Fetches what the assistant knows (once at a time).
    pub(crate) fn load_facts(&mut self, cx: &mut Context<'_, Self>) {
        let feed = &mut self.settings.facts;
        if feed.loading {
            return;
        }
        feed.loading = true;
        let backend = Arc::clone(&self.backend);
        cx.spawn(async move |this, cx| {
            let page = cx.background_spawn(async move { backend.facts() }).await;
            update_or_log(&this, cx, |store, cx| {
                let feed = &mut store.settings.facts;
                feed.loading = false;
                match page {
                    Ok(page) => (feed.page, feed.read_problem) = (Some(page), None),
                    Err(error) => {
                        feed.read_problem =
                            Some(format!("Couldn't read what's known: {}", reason(&error)));
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Corrects a fact's text (the assistant hears about it and uses the new one).
    pub(crate) fn fix_fact(&mut self, fact: &Fact, text: String, cx: &mut Context<'_, Self>) {
        self.fix(
            SettingsAction::FactSet(FactChange {
                id: fact.id.clone(),
                category: fact.category.clone(),
                title: fact.title.clone(),
                text,
            }),
            cx,
        );
    }

    /// Marks whether a person's messages wake the assistant at once.
    pub(crate) fn mark_important(
        &mut self,
        id: FactId,
        important: bool,
        cx: &mut Context<'_, Self>,
    ) {
        self.fix(
            SettingsAction::PersonSet(PersonChange { id, important }),
            cx,
        );
    }

    /// Forgets a fact.
    pub(crate) fn forget_fact(&mut self, id: FactId, cx: &mut Context<'_, Self>) {
        self.fix(SettingsAction::FactDrop(id), cx);
    }

    /// Drops a rule on trial.
    pub(crate) fn drop_trial(&mut self, id: TrialId, cx: &mut Context<'_, Self>) {
        self.fix(SettingsAction::TrialDrop(id), cx);
    }

    /// Shows a fix at once and sends it, then fetches the page again; a refusal shows on the
    /// app's line for refused actions, with what failed and why.
    fn fix(&mut self, action: SettingsAction, cx: &mut Context<'_, Self>) {
        self.settings.facts.apply(&action);
        self.hide_action_problem(cx);
        let failure = failure(&action);
        let backend = Arc::clone(&self.backend);
        cx.spawn(async move |this, cx| {
            let done = cx
                .background_spawn(async move { backend.settings(action) })
                .await;
            update_or_log(&this, cx, |store, cx| {
                if let Err(error) = done {
                    store.action_failed(failure, &error, cx);
                }
                store.load_facts(cx);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn page() -> FactsPage {
        serde_json::from_value(json!({
            "facts": [
                {"id": "people/example", "category": "people", "title": "Example Person",
                 "text": "Runs the example team.", "source": "chat", "updated": "2026-11-20T10:00:00Z",
                 "important": false},
                {"id": "work/desk", "category": "work", "title": "Desk", "text": "Works from home.",
                 "source": "", "updated": "2026-11-20T10:00:00Z"},
                {"id": "proof/a", "category": "proof", "title": "Kept", "text": "x", "source": "",
                 "updated": "2026-11-20T10:00:00Z"}
            ],
            "trials": [{"id": 4, "text": "Say which file.", "at": "2026-11-20T10:00:00Z",
                        "ends": "2026-12-04T10:00:00Z", "said": "Which file?"}]
        }))
        .unwrap()
    }

    #[test]
    fn facts_group_by_category_in_order_and_filter_by_their_words() {
        let page = page();
        let all = grouped(&page, "");
        let categories: Vec<_> = all.iter().map(|(category, _)| category.clone()).collect();
        assert_eq!(categories, [FactCategory::People, FactCategory::Work]);
        let found = grouped(&page, "  HOME ");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].1[0].title, "Desk");
        assert_eq!(trials(&page, "file").len(), 1);
        assert_eq!(trials(&page, "home").len(), 0);
    }

    #[test]
    fn a_fix_shows_at_once() {
        let mut feed = FactsFeed {
            page: Some(page()),
            ..FactsFeed::default()
        };
        let person = FactId::new("people/example");
        feed.apply(&SettingsAction::FactSet(FactChange {
            id: person.clone(),
            category: FactCategory::People,
            title: "Example Person".into(),
            text: "Runs two teams.".into(),
        }));
        let fact = feed.fact(&person).unwrap();
        assert_eq!(
            (fact.text.as_str(), fact.source.as_str()),
            ("Runs two teams.", CORRECTED)
        );
        feed.apply(&SettingsAction::PersonSet(PersonChange {
            id: person.clone(),
            important: true,
        }));
        assert!(feed.fact(&person).unwrap().is_important());
        feed.apply(&SettingsAction::FactDrop(person.clone()));
        assert!(feed.fact(&person).is_none());
        feed.apply(&SettingsAction::TrialDrop(TrialId::new(4)));
        assert_eq!(feed.page().unwrap().trials.len(), 0);
    }

    #[test]
    fn a_failed_fix_says_what_failed() {
        assert_eq!(
            failure(&SettingsAction::TrialDrop(TrialId::new(1))),
            "Not dropped"
        );
        assert_eq!(
            failure(&SettingsAction::FactDrop(FactId::new("work/a"))),
            "Not forgotten"
        );
        assert_eq!(
            category_title(&FactCategory::Preferences),
            "How you like things"
        );
    }
}
