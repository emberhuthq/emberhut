//! What the assistant remembers: `GET /api/facts`, the quick notes (`GET`/`POST /api/notes`),
//! the state's `contextAt`, the memory tools, and recall's index (`docs`, `docs_fts`, `vecs`,
//! `marks`, and `transcript_offsets`, the transcripts' read offsets) and its search by words
//! and by meaning.
#![expect(
    clippy::unwrap_used,
    reason = "the test helpers here: a failure fails the test that called them"
)]

use std::path::Path;
use std::sync::Arc;

use emberhut_api::facts::FactsPage;
use emberhut_api::notes::{NoteAuthor as NoteWho, NoteSaved, NotesPage};
use emberhut_api::paths;
use emberhut_api::state::State;
use emberhut_core::memory::recall::Filter;
use emberhut_core::message::{Author, NewMessage};
use emberhut_core::{ManualClock, RoomId};
use emberhut_db::Database;
use emberhut_server::services::context::{Awareness, CONTEXT};
use emberhut_server::services::facts::{FactService, SetFact};
use emberhut_server::services::recall::{
    EmbedError, Embedder, NoEmbedder, RecallParts, RecallService, SLICE_BYTES,
};
use futures_util::future::BoxFuture;
use serde_json::{Value, json};

use crate::common::{Call, TestApp, text};

#[tokio::test]
pub(crate) async fn facts_lessons_and_trials_make_the_about_you_page() {
    let app = TestApp::new().await;
    let session = app.sign_in().await;
    let empty: FactsPage = app
        .call(Call::get(paths::FACTS).token(&session))
        .await
        .read_as();
    assert!(empty.facts.is_empty() && empty.lessons.is_empty() && empty.trials.is_empty());

    let kept = app
        .call_tool(
            "fact_set",
            json!({ "category": "home", "title": "Keys", "text": "Under the mat", "source": "they said" }),
        )
        .await;
    assert_eq!(text(&kept), "Kept as home/keys.");
    let logged = app
        .call_tool(
            "correction_log",
            json!({ "said": "you didn't check", "wrong": "I guessed", "lesson": "Check first." }),
        )
        .await;
    assert!(
        text(&logged).ends_with("Its lesson is trial rule T1 for 14 days."),
        "{}",
        text(&logged)
    );
    let lesson = app
        .call_tool("lesson_set", json!({ "text": "Say when you are unsure." }))
        .await;
    assert_eq!(text(&lesson), "Lesson #1 kept.");

    let answer = app.call(Call::get(paths::FACTS).token(&session)).await;
    assert_eq!(answer.status, 200);
    let page: FactsPage = answer.read_as();
    assert_eq!(page.facts.len(), 1);
    assert_eq!(page.facts[0].id.as_str(), "home/keys");
    assert_eq!(page.facts[0].source, "they said");
    assert_eq!(page.lessons[0].text, "Say when you are unsure.");
    assert_eq!(page.trials[0].id.get(), 1);
    assert_eq!(page.trials[0].text, "Check first.");
    assert_eq!(page.trials[0].said, "you didn't check");

    assert_eq!(app.call(Call::get(paths::FACTS)).await.status, 401);
}

#[tokio::test]
pub(crate) async fn quick_notes_are_saved_edited_resent_and_deleted() {
    let app = TestApp::new().await;
    let session = app.sign_in().await;
    let save = |body: Value| Call::post(paths::NOTES).token(&session).json(&body);

    let saved: NoteSaved = app
        .call(save(json!({ "uid": "note-0001", "text": "  Slept well  ", "at": "2020-01-01T08:00:00.000Z" })))
        .await
        .read_as();
    let note = saved.note.unwrap();
    assert!(saved.ok);
    assert_eq!(note.text, "Slept well");
    assert_eq!(note.who, NoteWho::Person);
    assert_eq!(
        note.at.as_str(),
        "2020-01-01T08:00:00.000Z",
        "an offline note keeps its time"
    );

    let future = app
        .call(save(
            json!({ "uid": "note-0002", "text": "Later", "at": "2999-01-01T00:00:00.000Z" }),
        ))
        .await
        .read_as::<NoteSaved>()
        .note
        .unwrap();
    assert!(
        future.at.as_str() < "2999",
        "a clock in the future isn't when it was written"
    );

    let edited: NoteSaved = app
        .call(save(json!({ "uid": "note-0001", "text": "Slept badly" })))
        .await
        .read_as();
    let edited = edited.note.unwrap();
    assert_eq!(edited.text, "Slept badly");
    assert_eq!(edited.at, note.at, "an edit keeps its time");

    for (body, error) in [
        (
            json!({ "uid": "short", "text": "x" }),
            "a note needs its uid",
        ),
        (json!({ "text": "x" }), "a note needs its uid"),
        (
            json!({ "uid": "note-0003", "text": "x".repeat(20_001) }),
            "a quick note is at most 20,000 characters",
        ),
    ] {
        let answer = app.call(save(body)).await;
        assert_eq!(answer.status, 400);
        assert_eq!(answer.json(), json!({ "ok": false, "error": error }));
    }

    let page: NotesPage = app
        .call(Call::get(paths::NOTES).token(&session))
        .await
        .read_as();
    assert_eq!(page.notes.len(), 2);
    assert_eq!(page.notes[0].uid.as_str(), "note-0002", "newest first");

    let deleted: NoteSaved = app
        .call(save(json!({ "uid": "note-0001", "text": "" })))
        .await
        .read_as();
    assert_eq!(
        deleted,
        NoteSaved {
            ok: true,
            note: None
        }
    );
    let page: NotesPage = app
        .call(Call::get(paths::NOTES).token(&session))
        .await
        .read_as();
    assert_eq!(page.notes.len(), 1);
    assert_eq!(app.call(Call::get(paths::NOTES)).await.status, 401);
}

#[tokio::test]
pub(crate) async fn saving_a_note_needs_a_session_and_a_small_body() {
    let app = TestApp::new().await;
    let anonymous = app
        .call(Call::post(paths::NOTES).json(&json!({ "uid": "note-0001", "text": "x" })))
        .await;
    assert_eq!(anonymous.status, 401);
    let session = app.sign_in().await;
    let huge = format!(r#"{{"uid":"note-0001","text":"{}"}}"#, "x".repeat(3 << 20));
    let answer = app
        .call(Call::post(paths::NOTES).token(&session).raw(&huge))
        .await;
    assert_eq!(answer.status, 413);
    let not_json = app
        .call(Call::post(paths::NOTES).token(&session).raw("{not json"))
        .await;
    assert_eq!(not_json.status, 400);
}

#[tokio::test]
async fn the_diary_tools_write_and_read_the_days() {
    let app = TestApp::new().await;
    let session = app.sign_in().await;
    app.call(
        Call::post(paths::NOTES)
            .token(&session)
            .json(&json!({ "uid": "note-0001", "text": "Insomnia again" })),
    )
    .await;
    let written = app
        .call_tool("diary_write", json!({ "text": "A slow, quiet day." }))
        .await;
    let today = app.state.notes().today();
    assert_eq!(
        text(&written),
        format!("Written in the user's diary for {today}.")
    );
    let short = app
        .call_tool("diary_write", json!({ "text": "Too short" }))
        .await;
    assert_eq!(
        short.is_error,
        Some(true),
        "an entry is ten characters or more"
    );
    let ahead = app
        .call_tool(
            "diary_write",
            json!({ "text": "Not yet written.", "day": "2999-01-01" }),
        )
        .await;
    assert_eq!(ahead.is_error, Some(true));
    assert_eq!(text(&ahead), "day: YYYY-MM-DD, today or before");

    let notes = app.call_tool("notes", json!({})).await;
    let shown = text(&notes);
    assert!(
        shown.contains(": Insomnia again\n- Your entry: A slow, quiet day."),
        "{shown}"
    );
    let range = app
        .call_tool("notes", json!({ "from": "2999-01-01", "to": "2999-12-31" }))
        .await;
    assert_eq!(text(&range), "No notes on those days.");

    let found = app
        .call_tool("note_search", json!({ "q": "insomnia" }))
        .await;
    let found = text(&found);
    assert!(found.starts_with("(Meaning search is off"), "{found}");
    assert!(found.contains("note · Quick note, "), "{found}");
    assert!(found.contains("Insomnia again"));
    assert!(!found.contains("A slow, quiet day."), "only what matches");

    let page: NotesPage = app
        .call(Call::get(paths::NOTES).token(&session))
        .await
        .read_as();
    assert_eq!(
        page.notes
            .iter()
            .filter(|n| n.who == NoteWho::Assistant)
            .count(),
        1
    );
}

#[tokio::test]
async fn the_fact_and_person_tools_keep_the_context_folder() {
    let app = TestApp::new().await;
    let set = app
        .call_tool("fact_set", json!({ "category": "work", "title": "Office", "text": "Third floor", "project": "Garden" }))
        .await;
    assert_eq!(text(&set), "Kept as work/office.");
    let refused = app
        .call_tool(
            "fact_set",
            json!({ "category": "pets", "title": "Cat", "text": "Grey" }),
        )
        .await;
    assert_eq!(refused.is_error, Some(true));

    let card = app
        .call_tool(
            "person_set",
            json!({ "who": "Ada Lovelace (engine team)", "now": "Waiting on the contract", "chat": "wa:662", "important": true }),
        )
        .await;
    assert_eq!(text(&card), "Card people/ada-lovelace.");
    let nobody = app.call_tool("person_set", json!({ "now": "x" })).await;
    assert_eq!(nobody.is_error, Some(true));
    assert!(text(&nobody).contains("a new person needs who"));

    let facts = app
        .call_tool("facts", json!({ "category": "people" }))
        .await;
    let facts: Value = serde_json::from_str(text(&facts)).unwrap();
    assert_eq!(facts[0]["id"], "people/ada-lovelace");
    assert_eq!(facts[0]["important"], true);
    assert_eq!(facts[0]["chats"], json!(["wa:662"]));
    let all = app.call_tool("facts", json!({})).await;
    let all: Value = serde_json::from_str(text(&all)).unwrap();
    assert_eq!(all.as_array().unwrap().len(), 2);
    assert_eq!(
        app.call_tool("facts", json!({ "category": "pets" }))
            .await
            .is_error,
        Some(true)
    );

    let dropped = app
        .call_tool("fact_drop", json!({ "id": "work/office" }))
        .await;
    assert_eq!(text(&dropped), "Forgotten.");
    let again = app
        .call_tool("fact_drop", json!({ "id": "work/office" }))
        .await;
    assert_eq!(again.is_error, Some(true));
    assert!(text(&again).contains("no fact work/office"));
}

#[tokio::test]
async fn corrections_lessons_and_trials_through_the_tools() {
    let app = TestApp::new().await;
    let plain = app
        .call_tool(
            "correction_log",
            json!({ "said": "what do you mean", "wrong": "Unclear" }),
        )
        .await;
    assert_eq!(text(&plain), "Logged correction #1.");
    app.call_tool(
        "correction_log",
        json!({ "said": "wrong again", "wrong": "Guessed", "lesson": "Check the date." }),
    )
    .await;
    app.call_tool(
        "correction_log",
        json!({ "said": "I already told you", "wrong": "Forgot", "lesson": "Read the notes first." }),
    )
    .await;

    let kept = app
        .call_tool(
            "lesson_set",
            json!({ "text": "Read the notes first, always.", "trial": 3 }),
        )
        .await;
    assert_eq!(text(&kept), "Lesson #1 kept.");
    let dropped = app.call_tool("trial_drop", json!({ "id": 2 })).await;
    assert_eq!(text(&dropped), "Dropped.");
    let none = app.call_tool("trial_drop", json!({ "id": 2 })).await;
    assert_eq!(
        text(&none),
        "No trial rule T2 is running (kept, dropped or past its 14 days)."
    );

    let log = app.call_tool("corrections", json!({ "days": 7 })).await;
    let log: Value = serde_json::from_str(text(&log)).unwrap();
    assert_eq!(log["corrections"].as_array().unwrap().len(), 3);
    assert_eq!(log["corrections"][0]["source"], "assistant");
    assert_eq!(log["lessons"][0]["text"], "Read the notes first, always.");
    assert_eq!(log["trials"], json!([]), "one kept, one dropped");
    assert_eq!(
        app.state.learning().rules_text().await.unwrap(),
        "\n\nStanding rules the user OKed (from your corrections; follow them always):\n1. \
         Read the notes first, always."
    );

    let dropped = app.call_tool("lesson_drop", json!({ "id": 1 })).await;
    assert_eq!(text(&dropped), "Dropped.");
    assert_eq!(
        app.state.learning().lessons().await.unwrap(),
        [] as [emberhut_core::memory::learning::Lesson; 0]
    );
    assert_eq!(
        app.call_tool("corrections", json!({ "days": 400 }))
            .await
            .is_error,
        Some(true)
    );

    // The server notices what sounds like a correction.
    let noticed = app
        .state
        .learning()
        .notice("You didn't check the calendar", Some("It's free."))
        .await
        .unwrap();
    assert_eq!(noticed, Some(4));
    assert_eq!(
        app.state.learning().notice("Thanks!", None).await.unwrap(),
        None
    );
}

async fn say(db: &Database, room: RoomId, author: Author, text: &str, app: &TestApp) {
    db.messages()
        .append(
            NewMessage {
                room,
                author,
                text: text.to_owned(),
                reply_to: None,
            },
            app.clock_now(),
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn the_chat_log_reads_every_room_a_page_at_a_time() {
    let app = TestApp::new().await;
    let empty = app.call_tool("chat_log", json!({})).await;
    assert_eq!(text(&empty), "No messages in those days.");
    let db = Database::connect(&app.url).await.unwrap();
    let long = "x".repeat(1500);
    for n in 0..45 {
        app.advance_minutes(1);
        let author = if n % 2 == 0 {
            Author::User
        } else {
            Author::Assistant
        };
        say(&db, RoomId::MAIN, author, &format!("{n} {long}"), &app).await;
    }
    say(&db, RoomId::MAIN, Author::Event, "a program said so", &app).await;

    let first = app.call_tool("chat_log", json!({ "days": 7 })).await;
    let first = text(&first);
    assert!(first.starts_with("Messages 1-"), "{first}");
    assert!(first.contains("] User: 0 x"));
    assert!(!first.contains("a program said so"));
    let next = first
        .rsplit("after \"")
        .next()
        .and_then(|rest| rest.strip_suffix("\"."))
        .unwrap()
        .to_owned();
    let second = app
        .call_tool("chat_log", json!({ "days": 7, "after": next }))
        .await;
    let second = text(&second);
    assert!(second.ends_with("End of the log."), "{second}");
    assert!(second.contains("] User: 44 x"), "{second}");
    let past = app
        .call_tool("chat_log", json!({ "after": "2999-01-01T00:00:00.000Z" }))
        .await;
    assert_eq!(
        text(&past),
        "No messages after 2999-01-01T00:00:00.000Z (45 in those days). End of the log."
    );
}

#[tokio::test]
async fn the_state_says_when_the_awareness_was_last_brought_up_to_date() {
    let app = TestApp::new().await;
    let session = app.sign_in().await;
    let state: State = app
        .call(Call::get(paths::STATE).token(&session))
        .await
        .read_as();
    assert_eq!(state.context_at, None);

    let db = Database::connect(&app.url).await.unwrap();
    let mut open = serde_json::Map::new();
    open.insert("id".to_owned(), json!(1));
    open.insert("who".to_owned(), json!("Ada"));
    open.insert("what".to_owned(), json!("Wants the contract"));
    open.insert("since".to_owned(), json!("2020-01-01T00:00:00.000Z"));
    open.insert("maybe".to_owned(), json!("2020-01-02T00:00:00.000Z"));
    let awareness = Awareness {
        loops: vec![open],
        notes: vec![json!({ "at": "2020-01-01T00:00:00.000Z", "text": "Busy week" })],
        at: Some("2026-11-24T10:00:00.000Z".to_owned()),
    };
    db.settings().set(&CONTEXT, &awareness).await.unwrap();

    let answer = app.call(Call::get(paths::STATE).token(&session)).await;
    assert_eq!(answer.json()["contextAt"], "2026-11-24T10:00:00.000Z");
    let state: State = answer.read_as();
    assert_eq!(
        state
            .context_at
            .as_ref()
            .map(emberhut_api::time::Timestamp::as_str),
        Some("2026-11-24T10:00:00.000Z")
    );

    let context = app.call_tool("context", json!({})).await;
    let context: Value = serde_json::from_str(text(&context)).unwrap();
    assert_eq!(context["at"], "2026-11-24T10:00:00.000Z");
    assert_eq!(context["loops"][0]["who"], "Ada");
    assert_eq!(
        context["loops"][0]["maybeAnswered"],
        "The user wrote in its chat at 2020-01-02T00:00:00.000Z"
    );
    assert!(context["loops"][0]["stale"].is_string());
    assert_eq!(context["notes"][0]["text"], "Busy week");
    assert_eq!(context["routines"], json!([]));
    assert_eq!(context["quiet"], Value::Null);
}

#[tokio::test]
async fn the_recall_tool_finds_the_chat_facts_and_notes_by_their_words() {
    let app = TestApp::new().await;
    let db = Database::connect(&app.url).await.unwrap();
    say(
        &db,
        RoomId::MAIN,
        Author::User,
        "The roof tiles arrive on Monday",
        &app,
    )
    .await;
    say(
        &db,
        RoomId::MAIN,
        Author::Assistant,
        "Noted: the roof tiles come Monday.",
        &app,
    )
    .await;
    app.call_tool(
        "fact_set",
        json!({ "category": "home", "title": "Roofer", "text": "Grace fixes the roof" }),
    )
    .await;

    let found = app
        .call_tool("recall", json!({ "q": "what about the roof?" }))
        .await;
    let found = text(&found);
    assert!(
        found.contains("chat · User to assistant\n  chat #1\nThe roof tiles arrive on Monday"),
        "{found}"
    );
    assert!(
        found.contains("fact · home: Roofer\n  fact home/roofer\nGrace fixes the roof"),
        "{found}"
    );

    let only_facts = app
        .call_tool("recall", json!({ "q": "roof", "source": "fact" }))
        .await;
    assert!(!text(&only_facts).contains("chat ·"));
    let later = app
        .call_tool("recall", json!({ "q": "roof", "since": "2999-01-01" }))
        .await;
    assert!(
        text(&later).ends_with("Nothing found in 3 indexed pieces."),
        "{}",
        text(&later)
    );
    assert_eq!(
        app.call_tool("recall", json!({ "q": "r" })).await.is_error,
        Some(true)
    );
    assert_eq!(
        app.call_tool("recall", json!({ "q": "roof", "source": "elsewhere" }))
            .await
            .is_error,
        Some(true)
    );

    // A fact forgotten leaves the index.
    app.call_tool("fact_drop", json!({ "id": "home/roofer" }))
        .await;
    let after = app.call_tool("recall", json!({ "q": "Grace" })).await;
    assert!(text(&after).contains("Nothing found"), "{}", text(&after));
}

#[tokio::test]
async fn search_syntax_in_a_query_is_searched_as_plain_words() {
    let app = TestApp::new().await;
    let session = app.sign_in().await;
    for (uid, text) in [
        ("note-0001", "The roof leaks over the hall"),
        ("note-0002", "Practised the xylophone by the window"),
    ] {
        let saved = app
            .call(
                Call::post(paths::NOTES)
                    .token(&session)
                    .json(&json!({ "uid": uid, "text": text })),
            )
            .await;
        assert_eq!(saved.status, 200);
    }

    // Read as FTS5, these would be a syntax error (the stray quote), a prefix search (`x*`
    // finds the xylophone), a column filter, a NEAR group and a NOT (which would drop the
    // roof); as words they find the roof and only it.
    for q in [
        "roof\" OR NEAR(x*) title:^-",
        "x* OR title:roof",
        "NEAR(roof x*, 5) NOT hall",
    ] {
        for tool in ["recall", "note_search"] {
            let found = app.call_tool(tool, json!({ "q": q })).await;
            assert_ne!(found.is_error, Some(true), "{tool} {q}: {}", text(&found));
            let found = text(&found);
            assert!(
                found.contains("The roof leaks over the hall"),
                "{tool} {q}: {found}"
            );
            assert!(!found.contains("xylophone"), "{tool} {q} widened: {found}");
        }
    }

    // Only syntax: no words to search, and no error.
    for tool in ["recall", "note_search"] {
        let found = app.call_tool(tool, json!({ "q": "\"*^:-()\"" })).await;
        assert_ne!(found.is_error, Some(true), "{tool}: {}", text(&found));
        assert!(
            text(&found).contains("Nothing found"),
            "{tool}: {}",
            text(&found)
        );
    }
}

#[test]
fn a_notes_who_on_the_wire_is_the_stored_spelling() {
    use emberhut_core::memory::wire;
    assert_eq!(json!(NoteWho::Person), json!(wire::OWNER));
    assert_eq!(json!(NoteWho::Assistant), json!(wire::ASSISTANT));
}

/// A deterministic stand-in for an embedding model: each word adds to the dimension of its
/// meaning, so words that mean the same thing land together.
#[derive(Debug, Default)]
struct FakeEmbedder;

/// Words of one meaning share a dimension; every other word gets one by its length.
fn dimension(word: &str) -> usize {
    match word {
        "sleep" | "slept" | "insomnia" | "tired" => 0,
        "roof" | "tiles" | "roofer" => 1,
        _ => 2 + word.len() % 14,
    }
}

impl Embedder for FakeEmbedder {
    fn embed<'a>(
        &'a self,
        texts: &'a [String],
    ) -> BoxFuture<'a, Result<Vec<Vec<f32>>, EmbedError>> {
        Box::pin(async move {
            Ok(texts
                .iter()
                .map(|text| {
                    let mut vector = vec![0.0_f32; 16];
                    let body = text.rsplit(": ").next().unwrap_or(text);
                    for word in body
                        .split(|c: char| !c.is_alphanumeric())
                        .filter(|w| !w.is_empty())
                    {
                        vector[dimension(&word.to_lowercase())] += 1.0;
                    }
                    vector
                })
                .collect())
        })
    }
}

/// Recall over a fresh database in `dir`, with `embedder` and the transcripts in `dir`'s
/// `transcripts` folder.
async fn recall(dir: &Path, embedder: Arc<dyn Embedder>) -> (Database, RecallService, FactService) {
    let url = format!("sqlite://{}", dir.join("emberhut.db").display())
        .parse()
        .unwrap();
    let db = Database::connect(&url).await.unwrap();
    let clock = Arc::new(ManualClock::new("2026-11-24T10:00:00Z".parse().unwrap()));
    let facts = FactService::new(dir.join("context"), clock, db.events());
    let service = RecallService::new(RecallParts {
        store: db.recall(),
        conversation: db.conversation(),
        notes: db.notes(),
        facts: facts.clone(),
        tasks: db.tasks(),
        voice: db.voice_notes(),
        embedder,
        transcripts: Some(dir.join("transcripts")),
        zone: jiff::tz::TimeZone::UTC,
    });
    (db, service, facts)
}

fn line(value: &Value) -> String {
    format!("{value}\n")
}

#[tokio::test]
async fn meaning_finds_other_words_and_transcripts_are_read_a_line_at_a_time() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("transcripts").join("-srv-garden");
    std::fs::create_dir_all(&project).unwrap();
    let transcript = project.join("session.jsonl");
    let typed = json!({ "type": "user", "timestamp": "2026-11-20T09:00:00.000Z",
        "message": { "content": "I had insomnia all week" } });
    let tool = json!({ "type": "user", "message": { "content": [{ "type": "tool_result", "tool_use_id": "t1" }] } });
    let answer = json!({ "type": "assistant", "timestamp": "2026-11-20T09:01:00.000Z",
        "message": { "content": [{ "type": "text", "text": "Then rest early tonight." }] } });
    std::fs::write(
        &transcript,
        format!("{}{}{}", line(&typed), line(&tool), line(&answer)),
    )
    .unwrap();

    let (db, service, facts) = recall(dir.path(), Arc::new(FakeEmbedder)).await;
    facts
        .set_fact(SetFact {
            category: "home".to_owned(),
            title: "Roofer".to_owned(),
            text: "Grace fixes the tiles".to_owned(),
            ..SetFact::default()
        })
        .await
        .unwrap();
    service.index().await.unwrap();

    let path = transcript.to_string_lossy().into_owned();
    let read = db.recall().transcript_offset(&path).await.unwrap().unwrap();
    assert_eq!(read.line, 3, "every line read, the tool output's too");
    assert_eq!(db.recall().count_of("session").await.unwrap(), 2);
    assert_eq!(
        db.recall().vector_ids().await.unwrap().len(),
        3,
        "every piece has its vector"
    );

    // "sleep" is in none of the pieces: only the meaning search finds the insomnia.
    let found = service
        .search("sleep", 8, &Filter::default())
        .await
        .unwrap();
    assert!(found.meaning);
    assert_eq!(found.embedded, 3);
    assert_eq!(found.hits[0].text, "I had insomnia all week");
    assert_eq!(found.hits[0].title, "user · /srv/garden");
    assert_eq!(found.hits[0].place, format!("{path}:1"));

    // A line still being written waits; once whole, it is read.
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(&transcript)
        .unwrap();
    let late =
        json!({ "type": "user", "message": { "content": "The roof leaks again" } }).to_string();
    std::io::Write::write_all(&mut file, &late.as_bytes()[..10]).unwrap();
    service.index().await.unwrap();
    assert_eq!(
        db.recall()
            .transcript_offset(&path)
            .await
            .unwrap()
            .unwrap()
            .line,
        3
    );
    std::io::Write::write_all(&mut file, &late.as_bytes()[10..]).unwrap();
    std::io::Write::write_all(&mut file, b"\n").unwrap();
    service.index().await.unwrap();
    assert_eq!(
        db.recall()
            .transcript_offset(&path)
            .await
            .unwrap()
            .unwrap()
            .line,
        4
    );

    // Both searches find the leak: it comes first.
    let found = service.search("roof", 8, &Filter::default()).await.unwrap();
    assert_eq!(found.hits[0].text, "The roof leaks again");
    assert_eq!(
        found.hits[1].source, "fact",
        "the tiles' fact, by meaning alone"
    );

    // A transcript that got shorter was rewritten: it is read again from the start.
    std::fs::write(&transcript, line(&typed)).unwrap();
    service.index().await.unwrap();
    assert_eq!(
        db.recall()
            .transcript_offset(&path)
            .await
            .unwrap()
            .unwrap()
            .line,
        1
    );
    assert_eq!(db.recall().count_of("session").await.unwrap(), 1);

    // The marks: the chat is indexed up to its last message.
    say_plain(&db, "Paint the door").await;
    service.sync().await.unwrap();
    assert_eq!(
        db.recall().mark("chat").await.unwrap().as_deref(),
        Some("1")
    );
}

#[tokio::test]
async fn a_transcript_line_longer_than_a_slice_is_skipped_to_its_end() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("transcripts").join("-srv-garden");
    std::fs::create_dir_all(&project).unwrap();
    let transcript = project.join("session.jsonl");
    let typed = json!({ "type": "user", "message": { "content": "Water the beans" } });
    let output = json!({ "type": "user", "message": { "content": [{ "type": "tool_result",
        "tool_use_id": "t1", "content": "x".repeat(SLICE_BYTES + 4096) }] } })
    .to_string();
    let answer = json!({ "type": "assistant",
        "message": { "content": [{ "type": "text", "text": "The beans are watered." }] } });
    std::fs::write(&transcript, format!("{}{output}", line(&typed))).unwrap();

    let (db, service, _facts) = recall(dir.path(), Arc::new(NoEmbedder)).await;
    let path = transcript.to_string_lossy().into_owned();
    service.index().await.unwrap();
    let read = db.recall().transcript_offset(&path).await.unwrap().unwrap();
    assert_eq!(
        read.line, 1,
        "the long line is still being written: it waits"
    );
    assert_eq!(read.offset, line(&typed).len() as u64);

    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(&transcript)
        .unwrap();
    std::io::Write::write_all(&mut file, format!("\n{}", line(&answer)).as_bytes()).unwrap();
    service.index().await.unwrap();
    let read = db.recall().transcript_offset(&path).await.unwrap().unwrap();
    assert_eq!(
        read.line, 3,
        "skipped to its end, its place saved, and read on"
    );
    assert_eq!(read.offset, std::fs::metadata(&transcript).unwrap().len());
    assert_eq!(db.recall().count_of("session").await.unwrap(), 2);
    let found = service
        .search("beans", 8, &Filter::default())
        .await
        .unwrap();
    let texts: Vec<&str> = found.hits.iter().map(|hit| hit.text.as_str()).collect();
    assert_eq!(texts.len(), 2, "{texts:?}");
    assert!(texts.contains(&"Water the beans") && texts.contains(&"The beans are watered."));
}

async fn say_plain(db: &Database, text: &str) {
    db.messages()
        .append(
            NewMessage {
                room: RoomId::MAIN,
                author: Author::User,
                text: text.to_owned(),
                reply_to: None,
            },
            "2026-11-24T10:00:00Z".parse().unwrap(),
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn without_a_model_recall_searches_by_words() {
    let dir = tempfile::tempdir().unwrap();
    let (db, service, _facts) = recall(dir.path(), Arc::new(NoEmbedder)).await;
    say_plain(&db, "I had insomnia all week").await;
    service.index().await.unwrap();
    let words = service
        .search("insomnia", 8, &Filter::default())
        .await
        .unwrap();
    assert!(!words.meaning);
    assert_eq!(words.hits.len(), 1);
    assert_eq!(words.hits[0].title, "User to assistant");
    let meaning = service
        .search("sleep", 8, &Filter::default())
        .await
        .unwrap();
    assert_eq!(meaning.hits, [] as [emberhut_core::memory::recall::Doc; 0]);
    assert_eq!(meaning.indexed, 1);
}
