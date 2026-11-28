//! A fresh hut with the corpus in it: the facts in a context folder, the chat, the agent
//! transcripts on disk (sub-agents' included, as a real folder has them), the tasks and their
//! history, the voice notes other people sent and the notes. Each piece of evidence is mapped to
//! the reference recall gives it, so a hit can be checked against the gold keys.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use emberhut_core::memory::recall::{Filter, Found, base_reference};
use emberhut_core::message::{Author, NewMessage};
use emberhut_core::messaging::voice::{SeenNote, VoiceSource};
use emberhut_core::task::Who;
use emberhut_core::{ManualClock, RoomId};
use emberhut_db::messaging::Outcome;
use emberhut_db::{Database, NewTask, OwnerNote};
use emberhut_server::services::facts::{FactService, ProjectChange, SetFact, SetPerson};
use emberhut_server::services::recall::{Embedder, RecallParts, RecallService};
use jiff::Timestamp;
use serde_json::{Value, json};

use crate::corpus::{Corpus, Line};

/// When the questions are asked: the morning after the history ends.
pub(crate) const ASKED_AT: &str = "2026-11-01T09:00:00Z";

pub(crate) struct Hut {
    _dir: tempfile::TempDir,
    pub(crate) facts: FactService,
    pub(crate) recall: RecallService,
    /// Each piece of evidence's reference in the index, by its key.
    pub(crate) refs: HashMap<String, String>,
    /// The facts' ids, by key.
    pub(crate) fact_ids: HashMap<String, String>,
}

fn at(text: &str) -> Timestamp {
    text.parse()
        .unwrap_or_else(|error| panic!("{text}: {error}"))
}

/// A transcript line as an agent session writes it.
fn transcript_line(line: &Line) -> String {
    let value: Value = if line.role == "user" {
        json!({ "type": "user", "timestamp": line.at, "message": { "content": line.text } })
    } else {
        json!({ "type": "assistant", "timestamp": line.at,
            "message": { "content": [{ "type": "text", "text": line.text }] } })
    };
    format!("{value}\n")
}

/// Writes `lines` to the transcript `path`, mapping each key to its `path:line` reference.
fn write_transcript(path: &PathBuf, lines: &[Line], refs: &mut HashMap<String, String>) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut text = String::new();
    for (n, line) in lines.iter().enumerate() {
        text.push_str(&transcript_line(line));
        refs.insert(line.key.clone(), format!("{}:{}", path.display(), n + 1));
    }
    std::fs::write(path, text).unwrap();
}

impl Hut {
    /// A hut holding `corpus`, its recall searching with `embedder`.
    #[expect(clippy::too_many_lines, reason = "each store of the corpus in turn")]
    pub(crate) async fn new(corpus: &Corpus, embedder: Arc<dyn Embedder>) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let url = format!("sqlite://{}", dir.path().join("emberhut.db").display())
            .parse()
            .unwrap();
        let db = Database::connect(&url).await.unwrap();
        let clock = Arc::new(ManualClock::new(at("2026-09-01T00:00:00Z")));
        let facts = FactService::new(dir.path().join("context"), clock.clone(), db.events());
        let transcripts = dir.path().join("transcripts");
        std::fs::create_dir_all(&transcripts).unwrap();
        let recall = RecallService::new(RecallParts {
            store: db.recall(),
            conversation: db.conversation(),
            notes: db.notes(),
            facts: facts.clone(),
            tasks: db.tasks(),
            voice: db.voice_notes(),
            embedder,
            transcripts: Some(transcripts.clone()),
            zone: jiff::tz::TimeZone::UTC,
        });
        let mut refs = HashMap::new();
        let mut fact_ids = HashMap::new();

        for fact in &corpus.facts {
            clock.set(at(&fact.updated));
            // Only a person's card has a "now"; another fact says it in its text.
            let person = fact.category == emberhut_core::memory::facts::PEOPLE;
            let text = match (&fact.now, person) {
                (Some(now), false) => format!("{}\nNow: {now}", fact.text),
                _ => fact.text.clone(),
            };
            let id = facts
                .set_fact(SetFact {
                    category: fact.category.clone(),
                    title: fact.title.clone(),
                    text,
                    project: ProjectChange::given(fact.project.as_deref()),
                    ..SetFact::default()
                })
                .await
                .unwrap_or_else(|error| panic!("{}: {error}", fact.key));
            if let Some(now) = fact.now.as_ref().filter(|_| person) {
                facts
                    .set_person(SetPerson {
                        id: Some(id.clone()),
                        now: Some(now.clone()),
                        ..SetPerson::default()
                    })
                    .await
                    .unwrap_or_else(|error| panic!("{}: {error}", fact.key));
            }
            refs.insert(fact.key.clone(), id.clone());
            fact_ids.insert(fact.key.clone(), id);
        }

        for chat in &corpus.chat {
            let message = db
                .messages()
                .append(
                    NewMessage {
                        room: RoomId::MAIN,
                        author: if chat.who == "owner" {
                            Author::User
                        } else {
                            Author::Assistant
                        },
                        text: chat.text.clone(),
                        reply_to: None,
                    },
                    at(&chat.at),
                )
                .await
                .unwrap();
            refs.insert(chat.key.clone(), format!("chat:{}", message.id.get()));
        }

        for session in &corpus.sessions {
            let path = transcripts.join(&session.folder).join(&session.file);
            write_transcript(&path, &session.lines, &mut refs);
        }
        for agent in &corpus.subagents {
            let path = transcripts
                .join(&agent.folder)
                .join(&agent.session)
                .join("subagents")
                .join(&agent.file);
            write_transcript(&path, &agent.lines, &mut refs);
        }

        let tasks = db.tasks();
        let mut line_keys: HashMap<(String, String), String> = HashMap::new();
        for task in &corpus.tasks {
            let made = tasks
                .create(
                    NewTask {
                        title: task.title.clone(),
                        body: Some(task.body.clone()),
                        ..NewTask::default()
                    },
                    Who::Person,
                    at(&task.created),
                )
                .await
                .unwrap();
            refs.insert(task.key.clone(), made.key());
            for line in &task.lines {
                tasks
                    .note(made.id, Who::Worker, &line.text, at(&line.at))
                    .await
                    .unwrap();
                line_keys.insert((made.key(), line.text.clone()), line.key.clone());
            }
        }
        for line in tasks.lines_after(0, 100_000).await.unwrap() {
            if let Some(key) = line_keys.get(&(line.task.key(), line.line.text.clone())) {
                refs.insert(key.clone(), format!("{}@{}", line.task.key(), line.id));
            }
        }

        for voice in &corpus.voice {
            let seen = SeenNote {
                id: voice.key.clone(),
                source: if voice.source == "slack" {
                    VoiceSource::Slack
                } else {
                    VoiceSource::WhatsApp
                },
                reference: format!("eval:{}", voice.key),
                name: None,
                seconds: None,
                chat: Some(voice.chat.clone()),
                chat_name: Some(voice.chat.clone()),
                sender: Some(voice.sender.clone()),
                msg_at: Some(at(&voice.at)),
                msg: None,
            };
            db.voice_notes().seen(&seen, at(&voice.at)).await.unwrap();
            db.voice_notes()
                .finish(
                    &voice.key,
                    &Outcome::Done {
                        text: voice.text.clone(),
                        seconds: None,
                    },
                    at(&voice.at),
                )
                .await
                .unwrap();
            refs.insert(voice.key.clone(), voice.key.clone());
        }

        for note in &corpus.notes {
            let when = at(&note.at);
            let day = note.at.get(..10).unwrap().to_owned();
            let kept = if note.who == "owner" {
                db.notes()
                    .save_owners(
                        &OwnerNote {
                            uid: &note.key,
                            text: &note.text,
                            at: when,
                            day: &day,
                        },
                        when,
                    )
                    .await
                    .unwrap()
            } else {
                db.notes()
                    .write_assistants(&day, &note.text, when)
                    .await
                    .unwrap()
            };
            refs.insert(note.key.clone(), format!("note:{}", kept.uid));
        }

        clock.set(at(ASKED_AT));
        let hut = Self {
            _dir: dir,
            facts,
            recall,
            refs,
            fact_ids,
        };
        hut.index().await;
        hut
    }

    /// Brings the index up to date, vectors included.
    pub(crate) async fn index(&self) {
        for _ in 0..4 {
            self.recall.index().await.unwrap();
        }
    }

    /// The best 8 for `q` in `since..until`.
    pub(crate) async fn ask(&self, q: &str, since: Option<&str>, until: Option<&str>) -> Found {
        let filter = Filter {
            since: since.and_then(emberhut_core::memory::recall::read_bound),
            until: until.and_then(emberhut_core::memory::recall::read_bound),
            ..Filter::default()
        };
        self.recall.search(q, 8, &filter).await.unwrap()
    }

    /// The keys of `found`'s hits, in order (a hit that is no piece of evidence: its reference).
    pub(crate) fn keys_of(&self, found: &Found) -> Vec<String> {
        let by_ref: HashMap<&str, &str> = self
            .refs
            .iter()
            .map(|(key, reference)| (reference.as_str(), key.as_str()))
            .collect();
        found
            .hits
            .iter()
            .map(|hit| {
                let base = base_reference(&hit.reference);
                by_ref
                    .get(base)
                    .map_or_else(|| base.to_owned(), |key| (*key).to_owned())
            })
            .collect()
    }
}
