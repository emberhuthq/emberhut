//! What your ember knows about you: the facts the assistant keeps about the user
//! (`GET /api/facts`), fetched when the page opens, the lessons it is learning from the user's
//! corrections, and the user's fixes to both. A fix shows at once (a corrected fact, a person
//! marked, a fact forgotten, a lesson kept or dropped) and goes to the server after. Then the
//! page is fetched again, so it shows what the server keeps (a correction's source carries its
//! date there); if the server refused, the app's line for a refused action says what failed
//! and why (`Not forgotten: <reason>`).
//!
//! A server older than `trialKeep` answers 200 to it and leaves the lesson on trial: the fetch
//! after shows it still there, and the app's line says the server can't keep lessons early.

use std::sync::Arc;

use gpui_kit::AppContext as _;
use gpui_kit::Context;

use crate::store::gate::reason;
use crate::store::{Store, update_or_log};
use emberhut_api::facts::{Fact, FactCategory, FactsPage, Trial};
use emberhut_api::ids::{FactId, TrialId};
use emberhut_api::settings::{FactChange, PersonChange, SettingsAction};
use jiff::Timestamp;

/// The page's title, as its head and every way to it name it.
pub(crate) const TITLE: &str = "What your ember knows about you";

/// The search field's placeholder.
pub(crate) const SEARCH: &str = "Search what your ember knows";

/// The app's line when the server took a `trialKeep` but the lesson is still on trial: a
/// server older than the action, which ignores it.
pub(crate) const CANT_KEEP: &str =
    "Your server can't keep lessons early yet; it will decide at the weekly review";

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
        FactCategory::Preferences => "Preferences",
        FactCategory::Health => "Health & habits",
        FactCategory::Home => "Home & family",
        FactCategory::Projects => "Projects",
        _ => "Other",
    }
}

/// Where a fact is listed: one of [`LISTED`]'s categories, or Other for a category this app
/// doesn't know.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Shelf {
    /// A category of [`LISTED`].
    Category(FactCategory),
    /// Any category the app doesn't know, listed last.
    Other,
}

impl Shelf {
    /// Where `category` is listed; none for evidence kept for later, which isn't.
    #[must_use]
    pub(crate) fn of(category: &FactCategory) -> Option<Self> {
        if LISTED.contains(category) {
            Some(Self::Category(category.clone()))
        } else if category.is_known() {
            None
        } else {
            Some(Self::Other)
        }
    }

    /// Its title: the category's, or `Other`.
    #[must_use]
    pub(crate) fn title(&self) -> &'static str {
        match self {
            Self::Category(category) => category_title(category),
            Self::Other => "Other",
        }
    }

    /// Every shelf, in the page's order.
    fn all() -> impl Iterator<Item = Self> {
        LISTED
            .iter()
            .cloned()
            .map(Self::Category)
            .chain([Self::Other])
    }
}

/// The source a fact shows once the user corrected it here.
pub(crate) const CORRECTED: &str = "you corrected it";

/// What the assistant knows, as last fetched.
#[derive(Debug, Default)]
pub(crate) struct FactsFeed {
    page: Option<FactsPage>,
    loading: bool,
    /// A fetch was asked for while one ran: another follows it.
    again: bool,
    /// Why the last fetch failed.
    read_problem: Option<String>,
    /// Lessons the server took a keep for since the last fetch began: the next fetch checks
    /// they left the trial.
    kept: Vec<TrialId>,
    /// Whether "Learning from you" is open (folded until the user opens it, for the session).
    learning_open: bool,
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

    /// Whether "Learning from you" is open.
    #[must_use]
    pub(crate) fn learning_open(&self) -> bool {
        self.learning_open
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
            SettingsAction::TrialDrop(id) | SettingsAction::TrialKeep(id) => {
                page.trials.retain(|trial| &trial.id != id);
            }
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

/// The facts that match `query` (by their title and text), by shelf in the page's order; a
/// shelf with none is left out.
#[must_use]
pub(crate) fn grouped<'a>(page: &'a FactsPage, query: &str) -> Vec<(Shelf, Vec<&'a Fact>)> {
    Shelf::all()
        .filter_map(|shelf| {
            let facts: Vec<_> = page
                .facts
                .iter()
                .filter(|fact| {
                    Shelf::of(&fact.category).as_ref() == Some(&shelf)
                        && matches(query, &format!("{} {}", fact.title, fact.text))
                })
                .collect();
            (!facts.is_empty()).then_some((shelf, facts))
        })
        .collect()
}

/// The facts listed in all: every one but evidence kept for later.
#[must_use]
pub(crate) fn listed_count(page: &FactsPage) -> usize {
    page.facts
        .iter()
        .filter(|fact| Shelf::of(&fact.category).is_some())
        .count()
}

/// The category chips over the list, for the facts that match `query`: All, then each shelf
/// with facts and how many.
#[must_use]
pub(crate) fn chips(page: &FactsPage, query: &str) -> Vec<(Option<Shelf>, usize)> {
    let shelves = grouped(page, query);
    let all = shelves.iter().map(|(_, facts)| facts.len()).sum();
    std::iter::once((None, all))
        .chain(
            shelves
                .into_iter()
                .map(|(shelf, facts)| (Some(shelf), facts.len())),
        )
        .collect()
}

/// The lessons on trial that match `query`, by what the user said and the lesson.
#[must_use]
pub(crate) fn trials<'a>(page: &'a FactsPage, query: &str) -> Vec<&'a Trial> {
    page.trials
        .iter()
        .filter(|trial| matches(query, &format!("{} {}", trial.said, lesson(trial))))
        .collect()
}

/// A lesson in plain words to the user: its own line when the assistant gave one, else the
/// rule as the assistant follows it.
#[must_use]
pub(crate) fn lesson(trial: &Trial) -> &str {
    trial
        .lesson
        .as_deref()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .unwrap_or(&trial.text)
}

/// The earliest a lesson of `trials` ends: the day to decide by.
#[must_use]
pub(crate) fn decide_by<'a>(trials: impl IntoIterator<Item = &'a Trial>) -> Option<Timestamp> {
    trials
        .into_iter()
        .filter_map(|trial| trial.ends.as_str().parse::<Timestamp>().ok())
        .min()
}

/// What a fix that failed was, for its line: `Not saved`, `Not forgotten`, `Not kept`,
/// `Not dropped`.
fn failure(action: &SettingsAction) -> &'static str {
    match action {
        SettingsAction::FactDrop(_) => "Not forgotten",
        SettingsAction::TrialKeep(_) => "Not kept",
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

    /// Fetches what the assistant knows, one fetch at a time: asked for during one, another
    /// follows it. A lesson kept before the fetch began and still on trial in its page means
    /// a server that can't keep lessons early, and the app's line says so.
    pub(crate) fn load_facts(&mut self, cx: &mut Context<'_, Self>) {
        let feed = &mut self.settings.facts;
        if feed.loading {
            feed.again = true;
            return;
        }
        feed.loading = true;
        let kept = std::mem::take(&mut feed.kept);
        let backend = Arc::clone(&self.backend);
        cx.spawn(async move |this, cx| {
            let page = cx.background_spawn(async move { backend.facts() }).await;
            update_or_log(&this, cx, |store, cx| {
                let feed = &mut store.settings.facts;
                feed.loading = false;
                match page {
                    Ok(page) => {
                        let unkept = page.trials.iter().any(|trial| kept.contains(&trial.id));
                        (feed.page, feed.read_problem) = (Some(page), None);
                        if unkept {
                            store.show_action_problem(CANT_KEEP.to_owned(), cx);
                        }
                    }
                    Err(error) => {
                        feed.read_problem =
                            Some(format!("Couldn't read what's known: {}", reason(&error)));
                    }
                }
                if std::mem::take(&mut store.settings.facts.again) {
                    store.load_facts(cx);
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Opens or folds "Learning from you"; it stays so for the session.
    pub(crate) fn toggle_learning(&mut self, cx: &mut Context<'_, Self>) {
        let feed = &mut self.settings.facts;
        feed.learning_open = !feed.learning_open;
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

    /// Drops a lesson on trial: the assistant stops following it.
    pub(crate) fn drop_trial(&mut self, id: TrialId, cx: &mut Context<'_, Self>) {
        self.fix(SettingsAction::TrialDrop(id), cx);
    }

    /// Keeps a lesson on trial now, as a standing rule, instead of leaving it to the weekly
    /// review.
    pub(crate) fn keep_trial(&mut self, id: TrialId, cx: &mut Context<'_, Self>) {
        self.fix(SettingsAction::TrialKeep(id), cx);
    }

    /// Shows a fix at once and sends it, then fetches the page again; a refusal shows on the
    /// app's line for refused actions, with what failed and why.
    fn fix(&mut self, action: SettingsAction, cx: &mut Context<'_, Self>) {
        self.settings.facts.apply(&action);
        self.hide_action_problem(cx);
        let failure = failure(&action);
        let kept = match &action {
            SettingsAction::TrialKeep(id) => Some(*id),
            _ => None,
        };
        let backend = Arc::clone(&self.backend);
        cx.spawn(async move |this, cx| {
            let done = cx
                .background_spawn(async move { backend.settings(action) })
                .await;
            update_or_log(&this, cx, |store, cx| {
                match done {
                    Err(error) => store.action_failed(failure, &error, cx),
                    Ok(()) => store.settings.facts.kept.extend(kept),
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
                {"id": "pets/cat", "category": "pets", "title": "Cat", "text": "Has a cat.",
                 "source": "", "updated": "2026-11-20T10:00:00Z"},
                {"id": "proof/a", "category": "proof", "title": "Kept", "text": "x", "source": "",
                 "updated": "2026-11-20T10:00:00Z"}
            ],
            "trials": [
                {"id": 4, "text": "Before telling the owner a file changed, say which.",
                 "at": "2026-11-20T10:00:00Z", "ends": "2026-12-04T10:00:00Z",
                 "said": "Which file?", "lesson": "I say which file a change is in."},
                {"id": 5, "text": "Reply in the owner's language.", "at": "2026-11-10T10:00:00Z",
                 "ends": "2026-11-24T10:00:00Z", "said": "Answer in English"}
            ]
        }))
        .unwrap()
    }

    #[test]
    fn facts_group_by_shelf_in_order_and_filter_by_their_words() {
        let page = page();
        let all = grouped(&page, "");
        let shelves: Vec<_> = all.iter().map(|(shelf, _)| shelf.clone()).collect();
        assert_eq!(
            shelves,
            [
                Shelf::Category(FactCategory::People),
                Shelf::Category(FactCategory::Work),
                Shelf::Other
            ],
            "evidence isn't listed; a category the app doesn't know is Other, last"
        );
        let found = grouped(&page, "  HOME ");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].1[0].title, "Desk");
        assert_eq!(listed_count(&page), 3);
    }

    #[test]
    fn chips_count_what_the_search_finds() {
        let page = page();
        let titles: Vec<_> = chips(&page, "")
            .into_iter()
            .map(|(shelf, count)| (shelf.map_or("All", |shelf| shelf.title()), count))
            .collect();
        assert_eq!(
            titles,
            [("All", 3), ("People", 1), ("Work", 1), ("Other", 1)]
        );
        assert_eq!(chips(&page, "cat"), [(None, 1), (Some(Shelf::Other), 1)]);
    }

    #[test]
    fn a_lesson_reads_in_the_users_words_and_is_found_by_them() {
        let page = page();
        assert_eq!(lesson(&page.trials[0]), "I say which file a change is in.");
        assert_eq!(
            lesson(&page.trials[1]),
            "Reply in the owner's language.",
            "no line of its own: the rule"
        );
        assert_eq!(trials(&page, "which file").len(), 1);
        assert_eq!(trials(&page, "english").len(), 1, "by what was said");
        assert_eq!(trials(&page, "home").len(), 0);
        assert_eq!(
            decide_by(&page.trials).map(|at| at.to_string()),
            Some("2026-11-24T10:00:00Z".to_owned()),
            "the earliest end"
        );
        assert_eq!(decide_by(&[]), None);
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
        assert_eq!(feed.page().unwrap().trials.len(), 1);
        feed.apply(&SettingsAction::TrialKeep(TrialId::new(5)));
        assert_eq!(feed.page().unwrap().trials.len(), 0, "kept: off the trial");
    }

    #[test]
    fn a_failed_fix_says_what_failed() {
        assert_eq!(
            failure(&SettingsAction::TrialDrop(TrialId::new(1))),
            "Not dropped"
        );
        assert_eq!(
            failure(&SettingsAction::TrialKeep(TrialId::new(1))),
            "Not kept"
        );
        assert_eq!(
            failure(&SettingsAction::FactDrop(FactId::new("work/a"))),
            "Not forgotten"
        );
        assert_eq!(category_title(&FactCategory::Preferences), "Preferences");
    }
}
