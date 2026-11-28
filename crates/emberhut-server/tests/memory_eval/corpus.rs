//! The eval's data: a synthetic two-month history (`data/corpus.json`) and the questions asked
//! of it (`data/questions.json`). Every piece of evidence has a key of its own (`c-0012`,
//! `f-007`), and the questions name their gold evidence by those keys.

use std::path::{Path, PathBuf};

use serde::Deserialize;

/// The folder the data is in.
pub(crate) fn data() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/memory_eval/data")
}

#[derive(Debug, Deserialize)]
pub(crate) struct Corpus {
    pub(crate) facts: Vec<Fact>,
    pub(crate) chat: Vec<Chat>,
    pub(crate) sessions: Vec<Session>,
    pub(crate) subagents: Vec<Subagent>,
    pub(crate) tasks: Vec<Task>,
    pub(crate) voice: Vec<Voice>,
    pub(crate) notes: Vec<Note>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Fact {
    pub(crate) key: String,
    pub(crate) category: String,
    pub(crate) title: String,
    pub(crate) text: String,
    pub(crate) now: Option<String>,
    pub(crate) project: Option<String>,
    pub(crate) updated: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Chat {
    pub(crate) key: String,
    pub(crate) at: String,
    pub(crate) who: String,
    pub(crate) room: String,
    pub(crate) text: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Line {
    pub(crate) key: String,
    pub(crate) at: String,
    pub(crate) role: String,
    pub(crate) text: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Session {
    pub(crate) folder: String,
    pub(crate) file: String,
    pub(crate) lines: Vec<Line>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Subagent {
    pub(crate) folder: String,
    pub(crate) session: String,
    pub(crate) file: String,
    pub(crate) lines: Vec<Line>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct TaskLine {
    pub(crate) key: String,
    pub(crate) at: String,
    pub(crate) text: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Task {
    pub(crate) key: String,
    pub(crate) title: String,
    pub(crate) body: String,
    pub(crate) created: String,
    pub(crate) lines: Vec<TaskLine>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Voice {
    pub(crate) key: String,
    pub(crate) at: String,
    pub(crate) source: String,
    pub(crate) sender: String,
    pub(crate) chat: String,
    pub(crate) text: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Note {
    pub(crate) key: String,
    pub(crate) at: String,
    pub(crate) who: String,
    pub(crate) text: String,
}

/// One question and what answers it.
#[derive(Clone, Debug, Deserialize)]
pub(crate) struct Question {
    pub(crate) id: String,
    #[serde(rename = "type")]
    pub(crate) kind: String,
    pub(crate) lang: String,
    pub(crate) q: String,
    #[serde(default)]
    pub(crate) gold: Vec<String>,
    #[serde(default)]
    pub(crate) since: Option<String>,
    #[serde(default)]
    pub(crate) until: Option<String>,
    /// Knowledge updates: the evidence that is out of date.
    #[serde(default)]
    pub(crate) stale: Vec<String>,
    /// Forget: the fact the owner forgets, and words of it that must not come back.
    #[serde(default)]
    pub(crate) forget: Option<String>,
    #[serde(default)]
    pub(crate) words: Vec<String>,
    /// Injection: the third party's piece, and what it tries to have saved.
    #[serde(default)]
    pub(crate) inject: Option<String>,
    #[serde(default)]
    pub(crate) claim: Option<String>,
    /// Dedupe: the copies of one thing.
    #[serde(default)]
    pub(crate) copies: Vec<String>,
}

/// The corpus and the questions.
pub(crate) fn load() -> (Corpus, Vec<Question>) {
    let read = |name: &str| {
        std::fs::read_to_string(data().join(name)).unwrap_or_else(|error| panic!("{name}: {error}"))
    };
    let corpus = serde_json::from_str(&read("corpus.json")).unwrap();
    let questions = serde_json::from_str(&read("questions.json")).unwrap();
    (corpus, questions)
}
