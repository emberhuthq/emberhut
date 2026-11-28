//! Facts: the context folder, one small markdown file per fact (`emberhut_core::memory::facts`),
//! which the assistant keeps with its fact tools and the owner corrects, forgets and marks in
//! the app ("About you").
//!
//! The folder is `context/` beside the database, one file per fact, so a folder laid out the
//! same is used as it is; it goes into the server's backup with the database. Every
//! change goes through one lock, so two at once can't pick the same new file.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use emberhut_core::event::NewEvent;
use emberhut_core::memory::facts::{
    CATEGORIES, Fact, PEOPLE, free_id, is_id, parse, person_name, plain, render, slug,
};
use emberhut_core::memory::iso;
use emberhut_core::{Clock, RoomId};
use emberhut_db::{DbError, EventStore};
use jiff::Timestamp;
use tokio::sync::Mutex;

/// The context folder's name in the data folder.
pub const FOLDER: &str = "context";

/// Why a fact was not kept or forgotten.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum FactError {
    /// The request can't be done; the text says why, for the one who asked.
    #[error("{0}")]
    Refused(String),
    /// The folder could not be read or written.
    #[error("the context folder could not be read or written")]
    Folder(#[source] io::Error),
    /// The event telling the assistant could not be stored.
    #[error("the assistant could not be told")]
    Event(#[source] DbError),
}

/// What a change does to the project a fact belongs to.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum ProjectChange {
    /// Left as it was.
    #[default]
    Keep,
    /// Shared by every room again.
    Shared,
    /// This project's (by name).
    In(String),
}

impl ProjectChange {
    /// The change a given project name asks for: none or empty is shared.
    #[must_use]
    pub fn given(project: Option<&str>) -> Self {
        match project {
            Some(name) if !name.is_empty() => Self::In(name.to_owned()),
            _ => Self::Shared,
        }
    }
}

/// A fact to keep: new (no id, or a title its category doesn't have yet) or changed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SetFact {
    /// The fact's id; none: the fact with this title in its category, or a new one.
    pub id: Option<String>,
    /// One of the categories.
    pub category: String,
    /// A few words.
    pub title: String,
    /// One or two sentences.
    pub text: String,
    /// Where it came from; none: what it had.
    pub source: Option<String>,
    /// The project it belongs to.
    pub project: ProjectChange,
}

/// A person's card to keep: found by id, by a chat that is theirs, or by name; made if new.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SetPerson {
    /// The card's id.
    pub id: Option<String>,
    /// The person's name (a description after it is cut off).
    pub who: Option<String>,
    /// What is going on with them now, rewritten whole.
    pub now: Option<String>,
    /// A chat that is theirs, added to their chats.
    pub chat: Option<String>,
    /// Whether their messages wake the assistant at once; none: as it was.
    pub important: Option<bool>,
    /// The project they belong to.
    pub project: ProjectChange,
}

/// Keeps the context folder.
#[derive(Clone, Debug)]
pub struct FactService {
    root: PathBuf,
    clock: Arc<dyn Clock>,
    events: EventStore,
    writing: Arc<Mutex<()>>,
}

impl FactService {
    /// A service keeping the facts in the folder `root`, telling the assistant through
    /// `events`.
    #[must_use]
    pub fn new(root: PathBuf, clock: Arc<dyn Clock>, events: EventStore) -> Self {
        Self {
            root,
            clock,
            events,
            writing: Arc::new(Mutex::new(())),
        }
    }

    /// The context folder beside the database file `database`.
    #[must_use]
    pub fn folder_beside(database: &Path) -> PathBuf {
        database
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(FOLDER)
    }

    /// Every fact (of `category`, when given), most recently changed first.
    ///
    /// # Errors
    ///
    /// [`FactError::Folder`].
    pub async fn facts(&self, category: Option<&str>) -> Result<Vec<Fact>, FactError> {
        let root = self.root.clone();
        let category = category.map(str::to_owned);
        blocking(move || read_all(&root, category.as_deref())).await
    }

    /// Keeps `change`: a new fact, or a changed one (moving to another category, or to a
    /// title with another file name, moves its file). Its id.
    ///
    /// # Errors
    ///
    /// [`FactError::Refused`] for a category that isn't one, or an id that isn't a fact;
    /// [`FactError::Folder`].
    pub async fn set_fact(&self, change: SetFact) -> Result<String, FactError> {
        let _writing = self.writing.lock().await;
        let root = self.root.clone();
        let now = iso(self.clock.now());
        blocking(move || set_fact(&root, &change, &now)).await
    }

    /// Keeps `change` on a person's card; its id.
    ///
    /// # Errors
    ///
    /// [`FactError::Refused`] for an id that isn't a card, or a new card without a name;
    /// [`FactError::Folder`].
    pub async fn set_person(&self, change: SetPerson) -> Result<String, FactError> {
        let _writing = self.writing.lock().await;
        let root = self.root.clone();
        let now = iso(self.clock.now());
        blocking(move || set_person(&root, change, &now)).await
    }

    /// Forgets the fact `id`.
    ///
    /// # Errors
    ///
    /// [`FactError::Refused`] when there is no such fact; [`FactError::Folder`].
    pub async fn drop_fact(&self, id: &str) -> Result<(), FactError> {
        let _writing = self.writing.lock().await;
        let root = self.root.clone();
        let id = id.to_owned();
        blocking(move || {
            let path = file_of(&root, &id);
            if !is_id(&id) || !path.is_file() {
                return Err(FactError::Refused(format!("no fact {id}")));
            }
            fs::remove_file(path).map_err(FactError::Folder)
        })
        .await
    }

    /// The card of the person whose chat `chat` is, if any.
    ///
    /// # Errors
    ///
    /// [`FactError::Folder`].
    pub async fn person_for(&self, chat: &str) -> Result<Option<Fact>, FactError> {
        Ok(self.facts(Some(PEOPLE)).await?.into_iter().find(|fact| {
            fact.chats
                .as_ref()
                .is_some_and(|chats| chats.iter().any(|known| known == chat))
        }))
    }

    /// Tells Main's assistant `text` (a `fact` event), for what the owner changed in the app.
    ///
    /// # Errors
    ///
    /// [`FactError::Event`].
    pub async fn tell(&self, text: String) -> Result<(), FactError> {
        self.events
            .add(NewEvent::new(RoomId::MAIN, "fact", text), self.clock.now())
            .await
            .map(drop)
            .map_err(FactError::Event)
    }

    /// When the clock says it is.
    #[must_use]
    pub fn now(&self) -> Timestamp {
        self.clock.now()
    }
}

/// A fact on the wire.
#[must_use]
pub fn fact_to_api(fact: Fact) -> emberhut_api::facts::Fact {
    emberhut_api::facts::Fact {
        id: emberhut_api::ids::FactId::new(fact.id),
        category: emberhut_api::facts::FactCategory::from(fact.category),
        title: fact.title,
        text: fact.text,
        source: fact.source,
        updated: emberhut_api::time::Timestamp::from(fact.updated),
        now: fact.now,
        now_at: fact.now_at.map(emberhut_api::time::Timestamp::from),
        chats: fact.chats,
        important: fact.important,
        project: fact.project,
    }
}

/// Runs `work` on a blocking thread.
async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, FactError> + Send + 'static,
) -> Result<T, FactError> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|error| FactError::Folder(io::Error::other(error)))?
}

/// The file of the fact `id`.
fn file_of(root: &Path, id: &str) -> PathBuf {
    root.join(format!("{id}.md"))
}

/// Every fact in `root` (of `category` only, when given), most recently changed first; within
/// a category, by file name.
#[expect(
    clippy::case_sensitive_file_extension_comparisons,
    reason = "a fact's file ends in `.md` exactly, as it is written"
)]
fn read_all(root: &Path, category: Option<&str>) -> Result<Vec<Fact>, FactError> {
    let categories: Vec<&str> = match category {
        Some(category) => vec![category],
        None => CATEGORIES.to_vec(),
    };
    let mut facts = Vec::new();
    for category in categories {
        let folder = root.join(category);
        let entries = match fs::read_dir(&folder) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(FactError::Folder(error)),
        };
        let mut names: Vec<String> = entries
            .filter_map(Result::ok)
            .filter_map(|entry| entry.file_name().into_string().ok())
            .filter(|name| name.ends_with(".md"))
            .collect();
        names.sort_unstable();
        for name in names {
            let raw = fs::read_to_string(folder.join(&name)).map_err(FactError::Folder)?;
            facts.push(parse(category, &name, &raw));
        }
    }
    facts.sort_by(|a, b| b.updated.cmp(&a.updated));
    Ok(facts)
}

/// Writes `fact` to its file at `now`.
fn write(root: &Path, fact: &Fact, now: &str) -> Result<(), FactError> {
    fs::create_dir_all(root.join(&fact.category)).map_err(FactError::Folder)?;
    fs::write(file_of(root, &fact.id), render(fact, now)).map_err(FactError::Folder)
}

/// The project `change` leaves a fact that was in `was`.
fn project(change: ProjectChange, was: Option<String>) -> Option<String> {
    match change {
        ProjectChange::Keep => was,
        ProjectChange::Shared => None,
        ProjectChange::In(name) => Some(name),
    }
}

/// Whether `category` is a configured agent's own (`agent-<id>`): a folder of its facts, apart
/// from the owner's categories and never read with them.
fn is_agent_category(category: &str) -> bool {
    emberhut_core::agents::AgentId::from_facts_category(category).is_some()
}

fn set_fact(root: &Path, change: &SetFact, now: &str) -> Result<String, FactError> {
    if !CATEGORIES.contains(&change.category.as_str()) && !is_agent_category(&change.category) {
        return Err(FactError::Refused(format!(
            "category must be one of {}",
            CATEGORIES.join(", ")
        )));
    }
    let old = match change.id.as_deref().filter(|id| !id.is_empty()) {
        Some(id) => Some(
            read_all(
                root,
                id.split_once('/')
                    .map(|(category, _)| category)
                    .filter(|category| is_agent_category(category)),
            )?
            .into_iter()
            .find(|fact| fact.id == id)
            .ok_or_else(|| FactError::Refused(format!("no fact {id}")))?,
        ),
        // No id: the same title in its category is the same fact.
        None => read_all(root, Some(&change.category))?
            .into_iter()
            .find(|fact| plain(&fact.title) == plain(&change.title)),
    };
    // A title that slugs the same keeps its id (slug-2 stays slug-2); another is a new file.
    let id = match &old {
        Some(old) if old.category == change.category && slug(&old.title) == slug(&change.title) => {
            old.id.clone()
        }
        _ => free_id(
            &change.category,
            &change.title,
            old.as_ref().map(|old| old.id.as_str()),
            |id| file_of(root, id).is_file(),
        ),
    };
    if let Some(old) = old.as_ref().filter(|old| old.id != id) {
        fs::remove_file(file_of(root, &old.id)).map_err(FactError::Folder)?;
    }
    let was = old.unwrap_or_default();
    let fact = Fact {
        id: id.clone(),
        category: change.category.clone(),
        title: change.title.clone(),
        text: change.text.clone(),
        source: change.source.clone().unwrap_or_else(|| was.source.clone()),
        project: project(change.project.clone(), was.project.clone()),
        ..was
    };
    write(root, &fact, now)?;
    Ok(id)
}

fn set_person(root: &Path, mut change: SetPerson, now: &str) -> Result<String, FactError> {
    if let Some(who) = change.who.as_mut().filter(|who| !who.is_empty()) {
        *who = person_name(who);
    }
    let people = read_all(root, Some(PEOPLE))?;
    let by_id = change
        .id
        .as_deref()
        .and_then(|id| people.iter().find(|fact| fact.id == id));
    let by_chat = || {
        change
            .chat
            .as_deref()
            .filter(|chat| !chat.is_empty())
            .and_then(|chat| {
                people.iter().find(|fact| {
                    fact.chats
                        .as_ref()
                        .is_some_and(|chats| chats.iter().any(|known| known == chat))
                })
            })
    };
    let by_name = || {
        change
            .who
            .as_deref()
            .filter(|who| !who.is_empty())
            .and_then(|who| people.iter().find(|fact| plain(&fact.title) == plain(who)))
    };
    let old = by_id.or_else(by_chat).or_else(by_name).cloned();
    if let Some(id) = change.id.as_deref().filter(|id| !id.is_empty())
        && old.is_none()
    {
        return Err(FactError::Refused(format!("no person {id}")));
    }
    let who = change.who.as_deref().map_or_default(str::trim);
    if old.is_none() && who.is_empty() {
        return Err(FactError::Refused("a new person needs who".to_owned()));
    }
    let id = match &old {
        Some(old) => old.id.clone(),
        None => free_id(PEOPLE, who, None, |id| file_of(root, id).is_file()),
    };
    let mut chats = old
        .as_ref()
        .and_then(|old| old.chats.clone())
        .unwrap_or_default();
    if let Some(chat) = change.chat.filter(|chat| !chat.is_empty())
        && !chats.contains(&chat)
    {
        chats.push(chat);
    }
    let was = old.unwrap_or_default();
    let mut fact = Fact {
        id: id.clone(),
        category: PEOPLE.to_owned(),
        title: if was.title.is_empty() {
            who.to_owned()
        } else {
            was.title.clone()
        },
        chats: Some(chats),
        important: match change.important {
            Some(true) => Some(true),
            Some(false) => None,
            None => was.important,
        },
        project: project(change.project, was.project.clone()),
        ..was
    };
    if let Some(running) = change.now {
        fact.now = Some(running);
        fact.now_at = Some(now.to_owned());
    }
    write(root, &fact, now)?;
    Ok(id)
}

#[cfg(test)]
mod tests {
    use emberhut_core::ManualClock;

    use super::*;

    async fn service() -> (tempfile::TempDir, FactService) {
        let dir = tempfile::tempdir().unwrap();
        let url = emberhut_db::DatabaseUrl::from_path(&dir.path().join("emberhut.db")).unwrap();
        let db = emberhut_db::Database::connect(&url).await.unwrap();
        let clock = Arc::new(ManualClock::new("2026-11-24T10:00:00Z".parse().unwrap()));
        let facts = FactService::new(dir.path().join(FOLDER), clock, db.events());
        (dir, facts)
    }

    fn fact(category: &str, title: &str, text: &str) -> SetFact {
        SetFact {
            category: category.to_owned(),
            title: title.to_owned(),
            text: text.to_owned(),
            ..SetFact::default()
        }
    }

    #[tokio::test]
    async fn a_fact_is_kept_found_by_title_moved_and_forgotten() {
        let (dir, facts) = service().await;
        let id = facts
            .set_fact(fact("home", "Keys", "Under the mat"))
            .await
            .unwrap();
        assert_eq!(id, "home/keys");
        let raw =
            fs::read_to_string(dir.path().join("context").join("home").join("keys.md")).unwrap();
        assert!(
            raw.starts_with("---\ntitle: Keys\nsource: \nupdated: 2026-11-24T10:00:00.000Z\n"),
            "{raw}"
        );

        let again = facts
            .set_fact(fact("home", "keys", "In the drawer"))
            .await
            .unwrap();
        assert_eq!(again, "home/keys", "the same title is the same fact");
        assert_eq!(facts.facts(None).await.unwrap().len(), 1);

        let other = facts
            .set_fact(fact("home", "Keys!", "A second set"))
            .await
            .unwrap();
        assert_eq!(
            other, "home/keys-2",
            "another title never overwrites a fact"
        );

        let moved = facts
            .set_fact(SetFact {
                id: Some("home/keys".to_owned()),
                ..fact("work", "Office keys", "At reception")
            })
            .await
            .unwrap();
        assert_eq!(moved, "work/office-keys");
        assert!(
            !dir.path()
                .join("context")
                .join("home")
                .join("keys.md")
                .exists()
        );

        let refused = facts
            .set_fact(fact("pets", "Cat", "Grey"))
            .await
            .unwrap_err();
        assert!(
            refused
                .to_string()
                .starts_with("category must be one of people")
        );
        let missing = facts
            .set_fact(SetFact {
                id: Some("work/nothing".to_owned()),
                ..fact("work", "x", "y")
            })
            .await
            .unwrap_err();
        assert_eq!(missing.to_string(), "no fact work/nothing");

        facts.drop_fact("work/office-keys").await.unwrap();
        let left = facts.facts(None).await.unwrap();
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].id, "home/keys-2");
        assert_eq!(
            facts
                .drop_fact("../emberhut")
                .await
                .unwrap_err()
                .to_string(),
            "no fact ../emberhut"
        );
    }

    #[tokio::test]
    async fn a_card_is_found_by_chat_or_name_and_keeps_what_it_had() {
        let (_dir, facts) = service().await;
        let id = facts
            .set_person(SetPerson {
                who: Some("Ada Lovelace (engine team)".to_owned()),
                now: Some("Waiting on the contract".to_owned()),
                chat: Some("wa:662".to_owned()),
                ..SetPerson::default()
            })
            .await
            .unwrap();
        assert_eq!(id, "people/ada-lovelace");
        let same = facts
            .set_person(SetPerson {
                chat: Some("wa:662".to_owned()),
                important: Some(true),
                ..SetPerson::default()
            })
            .await
            .unwrap();
        assert_eq!(same, id);
        let card = facts.person_for("wa:662").await.unwrap().unwrap();
        assert_eq!(card.title, "Ada Lovelace");
        assert_eq!(card.now.as_deref(), Some("Waiting on the contract"));
        assert_eq!(card.important, Some(true));
        facts
            .set_person(SetPerson {
                id: Some(id.clone()),
                important: Some(false),
                ..SetPerson::default()
            })
            .await
            .unwrap();
        assert_eq!(
            facts.person_for("wa:662").await.unwrap().unwrap().important,
            None
        );
        let refused = facts.set_person(SetPerson::default()).await.unwrap_err();
        assert_eq!(refused.to_string(), "a new person needs who");
        let missing = facts
            .set_person(SetPerson {
                id: Some("people/nobody".to_owned()),
                ..SetPerson::default()
            })
            .await
            .unwrap_err();
        assert_eq!(missing.to_string(), "no person people/nobody");
    }
}
