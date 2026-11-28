//! What your ember knows about you: the facts the assistant keeps about the user
//! (`GET /api/facts`), fetched when the page opens, the lessons it is learning from the user's
//! corrections, and the user's fixes to both. A fix shows at once (a corrected fact, a person
//! marked, a fact confirmed, pinned, hidden or forgotten everywhere, a lesson kept or dropped)
//! and goes to the server after. Then the page is fetched again, so it shows what the server
//! keeps (a correction's source carries its date there); if the server refused, the app's line
//! for a refused action says what failed and why (`Not forgotten: <reason>`).
//!
//! Facts kept from something the user didn't start (someone else's message) wait to be
//! confirmed ([`pending`]) and aren't listed with the others until they are. Everything known
//! is exported as one zip into Downloads ([`Store::export_memory`]), and the nightly review of
//! what is kept is switched on or off ([`Store::set_memory_nightly`]).
//!
//! A server older than `trialKeep` answers 200 to it and leaves the lesson on trial: the fetch
//! after shows it still there, and the app's line says the server can't keep lessons early.

use std::path::PathBuf;
use std::sync::Arc;

use gpui_kit::AppContext as _;
use gpui_kit::Context;

use crate::store::files::{downloads_dir, save_bytes};
use crate::store::gate::reason;
use crate::store::{Store, update_or_log};
use emberhut_api::facts::{Fact, FactCategory, FactsPage, Trial};
use emberhut_api::ids::{FactId, TrialId};
use emberhut_api::settings::{FactChange, FactForget, FactProfile, PersonChange, SettingsAction};
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

/// The name an export is saved as when the server gave none.
const EXPORT_NAME: &str = "emberhut-memory.zip";

/// The user's say on whether a fact is in what the assistant always knows (its profile).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ProfileSay {
    /// As it comes: the assistant decides.
    Neither,
    /// Always in it.
    Pin,
    /// Never in it.
    Hide,
}

impl ProfileSay {
    /// Every say, in the order a menu offers them.
    pub(crate) const ALL: [Self; 3] = [Self::Pin, Self::Hide, Self::Neither];

    /// The say `fact` carries (a word this app doesn't know reads as neither).
    #[must_use]
    pub(crate) fn of(fact: &Fact) -> Self {
        match fact.profile.as_deref() {
            Some("pin") => Self::Pin,
            Some("hide") => Self::Hide,
            _ => Self::Neither,
        }
    }

    /// Its word on the wire: `pin`, `hide`, or none.
    #[must_use]
    pub(crate) fn wire(self) -> Option<String> {
        match self {
            Self::Pin => Some("pin".to_owned()),
            Self::Hide => Some("hide".to_owned()),
            Self::Neither => None,
        }
    }

    /// What it does, as a menu offers it.
    #[must_use]
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Pin => "Always keep in mind",
            Self::Hide => "Keep out of mind",
            Self::Neither => "As it comes",
        }
    }

    /// How it stands, on the fact's row; none when it is as it comes.
    #[must_use]
    pub(crate) fn badge(self) -> Option<&'static str> {
        match self {
            Self::Pin => Some("Pinned"),
            Self::Hide => Some("Hidden"),
            Self::Neither => None,
        }
    }
}

/// Why the assistant knows a fact, in a few words to the user: who put it there and where it
/// was said (`You told me · chat #22`); none when the server keeps neither.
#[must_use]
pub(crate) fn why(fact: &Fact) -> Option<String> {
    let from = fact
        .from
        .as_deref()
        .map(str::trim)
        .filter(|from| !from.is_empty());
    let who = match fact.origin.as_deref() {
        Some("owner") => Some("You added it"),
        Some("assistant") => Some("You told me"),
        Some("message") => Some("From someone else's message"),
        Some("web") => Some("From the web"),
        Some("import") => Some("Imported"),
        _ => None,
    };
    match (who, from) {
        (Some(who), Some(from)) => Some(format!("{who} · {from}")),
        (Some(who), None) => Some(who.to_owned()),
        (None, Some(from)) => Some(format!("From {from}")),
        (None, None) => None,
    }
}

/// Why an export wasn't saved.
enum Unsaved {
    /// The server didn't send it.
    Server(crate::backend::Error),
    /// It came but couldn't be saved here, in words for the user.
    Here(&'static str),
}

/// How the last export went.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Export {
    /// Coming down.
    Saving,
    /// Saved into Downloads, under this name.
    Saved(String),
}

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
    /// The memory's own controls.
    memory: MemoryControls,
}

/// The nightly review's switch and the export.
#[derive(Debug, Default)]
struct MemoryControls {
    /// The nightly review was last switched off. The server's own value isn't in what it
    /// sends yet, so the switch starts on (the server's default) and keeps the last value
    /// sent.
    nightly_off: bool,
    /// The last export, while it comes and once it is saved.
    export: Option<Export>,
    /// Where exports are saved; none: the system's Downloads (a test gives its own folder).
    export_to: Option<PathBuf>,
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

    /// Whether the nightly review of what is kept is on, as last sent (on to begin with).
    #[must_use]
    pub(crate) fn nightly(&self) -> bool {
        !self.memory.nightly_off
    }

    /// How the last export went.
    #[must_use]
    pub(crate) fn export(&self) -> Option<&Export> {
        self.memory.export.as_ref()
    }

    /// Applies a fix to the page on hand, as the server will.
    fn apply(&mut self, action: &SettingsAction) {
        if let SettingsAction::MemoryNightly(on) = action {
            self.memory.nightly_off = !on;
        }
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
            SettingsAction::FactConfirm(id) => {
                if let Some(fact) = page.facts.iter_mut().find(|fact| &fact.id == id) {
                    fact.pending = false;
                }
            }
            SettingsAction::FactProfile(say) => {
                if let Some(fact) = page.facts.iter_mut().find(|fact| fact.id == say.id) {
                    fact.profile.clone_from(&say.profile);
                }
            }
            SettingsAction::FactDrop(id) | SettingsAction::FactForget(FactForget { id, .. }) => {
                page.facts.retain(|fact| &fact.id != id);
            }
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

/// Whether `fact` holds `query`, by its title and text.
fn found(fact: &Fact, query: &str) -> bool {
    matches(query, &format!("{} {}", fact.title, fact.text))
}

/// The facts waiting for the user to confirm them that match `query`, as the page has them
/// (the most recently changed first); evidence kept for later isn't listed.
#[must_use]
pub(crate) fn pending<'a>(page: &'a FactsPage, query: &str) -> Vec<&'a Fact> {
    page.facts
        .iter()
        .filter(|fact| fact.pending && Shelf::of(&fact.category).is_some() && found(fact, query))
        .collect()
}

/// The confirmed facts that match `query` (by their title and text), by shelf in the page's
/// order; a shelf with none is left out. Facts waiting to be confirmed are listed apart
/// ([`pending`]).
#[must_use]
pub(crate) fn grouped<'a>(page: &'a FactsPage, query: &str) -> Vec<(Shelf, Vec<&'a Fact>)> {
    Shelf::all()
        .filter_map(|shelf| {
            let facts: Vec<_> = page
                .facts
                .iter()
                .filter(|fact| {
                    !fact.pending
                        && Shelf::of(&fact.category).as_ref() == Some(&shelf)
                        && found(fact, query)
                })
                .collect();
            (!facts.is_empty()).then_some((shelf, facts))
        })
        .collect()
}

/// The facts listed in all, waiting to be confirmed or not: every one but evidence kept for
/// later.
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

/// What a fix that failed was, for its line: `Not saved`, `Not forgotten`, `Not confirmed`,
/// `Not changed`, `Not kept`, `Not dropped`.
fn failure(action: &SettingsAction) -> &'static str {
    match action {
        SettingsAction::FactDrop(_) | SettingsAction::FactForget(_) => "Not forgotten",
        SettingsAction::FactConfirm(_) => "Not confirmed",
        SettingsAction::MemoryNightly(_) => "Not changed",
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

    /// Forgets a fact everywhere: the fact, what was said about it in the record recall
    /// keeps, and a mark so it isn't kept again.
    pub(crate) fn forget_fact(&mut self, id: FactId, cx: &mut Context<'_, Self>) {
        self.fix(SettingsAction::FactDrop(id), cx);
    }

    /// Confirms a fact that waited for the user: from now on the assistant uses it.
    pub(crate) fn confirm_fact(&mut self, id: FactId, cx: &mut Context<'_, Self>) {
        self.fix(SettingsAction::FactConfirm(id), cx);
    }

    /// Pins a fact in what the assistant always knows, hides it from that, or leaves it to the
    /// assistant.
    pub(crate) fn set_fact_profile(
        &mut self,
        id: FactId,
        say: ProfileSay,
        cx: &mut Context<'_, Self>,
    ) {
        let profile = say.wire();
        self.fix(SettingsAction::FactProfile(FactProfile { id, profile }), cx);
    }

    /// Turns the nightly review of what is kept on or off; refused, the switch goes back.
    pub(crate) fn set_memory_nightly(&mut self, on: bool, cx: &mut Context<'_, Self>) {
        self.fix(SettingsAction::MemoryNightly(on), cx);
    }

    /// Saves everything the assistant knows, as the server's zip, into Downloads under the
    /// name the server gives it (numbered when taken), and shows it there. One at a time.
    pub(crate) fn export_memory(&mut self, cx: &mut Context<'_, Self>) {
        let memory = &mut self.settings.facts.memory;
        if memory.export == Some(Export::Saving) {
            return;
        }
        memory.export = Some(Export::Saving);
        let folder = memory.export_to.clone();
        self.hide_action_problem(cx);
        let backend = Arc::clone(&self.backend);
        cx.spawn(async move |this, cx| {
            let saved = cx
                .background_spawn(async move {
                    let file = backend.memory_export().map_err(Unsaved::Server)?;
                    let dir = folder
                        .or_else(downloads_dir)
                        .ok_or(Unsaved::Here("there's no Downloads folder"))?;
                    let name = file.name.as_deref().unwrap_or(EXPORT_NAME);
                    save_bytes(&file.bytes, &dir, name).map_err(|error| {
                        log::warn!("couldn't save the memory export: {error}");
                        Unsaved::Here("couldn't write it into Downloads")
                    })
                })
                .await;
            update_or_log(&this, cx, |store, cx| {
                store.settings.facts.memory.export = None;
                match saved {
                    Ok(to) => {
                        let name = to
                            .file_name()
                            .map_or_else(String::new, |name| name.to_string_lossy().into());
                        store.settings.facts.memory.export = Some(Export::Saved(name));
                        store.reveal_saved(&to, cx);
                    }
                    Err(Unsaved::Server(error)) => store.action_failed("Not exported", &error, cx),
                    Err(Unsaved::Here(why)) => {
                        store.show_action_problem(format!("Not exported: {why}"), cx);
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Saves exports into `folder` instead of Downloads.
    #[cfg(test)]
    pub(crate) fn export_memory_to(&mut self, folder: PathBuf) {
        self.settings.facts.memory.export_to = Some(folder);
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
        // The nightly switch isn't in the page fetched again: refused, it goes back here.
        let nightly = matches!(action, SettingsAction::MemoryNightly(_))
            .then(|| self.settings.facts.nightly());
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
                    Err(error) => {
                        if let Some(was) = nightly {
                            store.settings.facts.memory.nightly_off = !was;
                        }
                        store.action_failed(failure, &error, cx);
                    }
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

    /// A page with a fact kept from someone else's message, waiting, and one the user told.
    fn waiting_page() -> FactsPage {
        serde_json::from_value(json!({"facts": [
            {"id": "work/launch", "category": "work", "title": "Launch", "text": "Ships in May.",
             "source": "", "updated": "2026-11-21T10:00:00Z", "origin": "message",
             "from": "chat #40", "pending": true},
            {"id": "work/desk", "category": "work", "title": "Desk", "text": "Works from home.",
             "source": "", "updated": "2026-11-20T10:00:00Z", "origin": "assistant",
             "from": "chat #22", "profile": "pin"},
            {"id": "proof/a", "category": "proof", "title": "Kept", "text": "x", "source": "",
             "updated": "2026-11-20T10:00:00Z", "pending": true}
        ]}))
        .unwrap()
    }

    #[test]
    fn facts_to_confirm_are_listed_apart_from_the_others() {
        let page = waiting_page();
        let waiting: Vec<_> = pending(&page, "")
            .iter()
            .map(|fact| fact.title.as_str())
            .collect();
        assert_eq!(waiting, ["Launch"], "evidence isn't listed, even waiting");
        let shelves = grouped(&page, "");
        assert_eq!(shelves.len(), 1);
        let titles: Vec<_> = shelves[0]
            .1
            .iter()
            .map(|fact| fact.title.as_str())
            .collect();
        assert_eq!(titles, ["Desk"], "not listed twice");
        assert_eq!(
            chips(&page, ""),
            [(None, 1), (Some(Shelf::Category(FactCategory::Work)), 1)]
        );
        assert_eq!(
            listed_count(&page),
            2,
            "a fact to confirm is still one known"
        );
        assert!(pending(&page, "desk").is_empty(), "found by their words");
    }

    #[test]
    fn why_a_fact_is_known_reads_from_who_and_where() {
        let page = waiting_page();
        assert_eq!(
            why(&page.facts[0]).as_deref(),
            Some("From someone else's message · chat #40")
        );
        assert_eq!(
            why(&page.facts[1]).as_deref(),
            Some("You told me · chat #22")
        );
        let mut fact = page.facts[1].clone();
        (fact.origin, fact.from) = (Some("owner".into()), None);
        assert_eq!(why(&fact).as_deref(), Some("You added it"));
        (fact.origin, fact.from) = (None, Some(" chat #9 ".into()));
        assert_eq!(why(&fact).as_deref(), Some("From chat #9"));
        (fact.origin, fact.from) = (Some("later".into()), Some(String::new()));
        assert_eq!(why(&fact), None, "a word the app doesn't know, and nowhere");
        for (origin, said) in [("web", "From the web"), ("import", "Imported")] {
            fact.origin = Some(origin.into());
            assert_eq!(why(&fact).as_deref(), Some(said));
        }
    }

    #[test]
    fn a_say_on_the_profile_reads_and_goes_as_its_word() {
        let page = waiting_page();
        assert_eq!(ProfileSay::of(&page.facts[1]), ProfileSay::Pin);
        assert_eq!(ProfileSay::of(&page.facts[0]), ProfileSay::Neither);
        let mut fact = page.facts[0].clone();
        fact.profile = Some("hide".into());
        assert_eq!(ProfileSay::of(&fact), ProfileSay::Hide);
        fact.profile = Some("later".into());
        assert_eq!(ProfileSay::of(&fact), ProfileSay::Neither);
        let words: Vec<_> = ProfileSay::ALL.into_iter().map(ProfileSay::wire).collect();
        assert_eq!(words, [Some("pin".into()), Some("hide".into()), None]);
        assert_eq!(ProfileSay::Pin.badge(), Some("Pinned"));
        assert_eq!(ProfileSay::Neither.badge(), None);
    }

    #[test]
    fn confirming_pinning_and_forgetting_show_at_once() {
        let mut feed = FactsFeed {
            page: Some(waiting_page()),
            ..FactsFeed::default()
        };
        let (launch, desk) = (FactId::new("work/launch"), FactId::new("work/desk"));
        feed.apply(&SettingsAction::FactConfirm(launch.clone()));
        assert!(!feed.fact(&launch).unwrap().pending);
        feed.apply(&SettingsAction::FactProfile(FactProfile {
            id: desk.clone(),
            profile: Some("hide".into()),
        }));
        assert_eq!(ProfileSay::of(feed.fact(&desk).unwrap()), ProfileSay::Hide);
        feed.apply(&SettingsAction::FactProfile(FactProfile {
            id: desk.clone(),
            profile: None,
        }));
        assert_eq!(feed.fact(&desk).unwrap().profile, None);
        feed.apply(&SettingsAction::FactForget(FactForget {
            id: desk.clone(),
            pieces: Some(vec![12]),
        }));
        assert!(feed.fact(&desk).is_none());
    }

    #[test]
    fn the_nightly_review_starts_on_and_keeps_what_was_sent() {
        let mut feed = FactsFeed::default();
        assert!(feed.nightly(), "on, as the server starts");
        feed.apply(&SettingsAction::MemoryNightly(false));
        assert!(!feed.nightly(), "kept without a page");
        feed.apply(&SettingsAction::MemoryNightly(true));
        assert!(feed.nightly());
    }

    #[test]
    fn a_failed_fix_says_what_failed() {
        assert_eq!(
            failure(&SettingsAction::FactConfirm(FactId::new("work/a"))),
            "Not confirmed"
        );
        assert_eq!(
            failure(&SettingsAction::MemoryNightly(false)),
            "Not changed"
        );
        assert_eq!(
            failure(&SettingsAction::FactProfile(FactProfile {
                id: FactId::new("work/a"),
                profile: None,
            })),
            "Not saved"
        );
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
