//! What the assistant remembers: the `notes`, the `corrections`, and recall's index (`docs`
//! with its `docs_fts` full-text index, `vecs`, `marks` and `transcript_offsets`).

use std::collections::HashSet;

use emberhut_core::RoomId;
use emberhut_core::memory::learning::{CorrectionSource, NewCorrection, TrialJudge, trial_window};
use emberhut_core::memory::notes::NoteAuthor;
use emberhut_core::memory::recall::NewDoc;
use emberhut_core::message::{Author, NewMessage};
use emberhut_db::{IndexBatch, OwnerNote, TranscriptOffset, TrialEnd};

use crate::common::{at, database};

#[tokio::test]
async fn notes_are_saved_edited_resent_and_deleted() {
    let (_dir, db) = database().await;
    let notes = db.notes();
    let first = OwnerNote {
        uid: "note-0001",
        text: "Slept well",
        at: at(0),
        day: "2027-01-15",
    };
    let saved = notes.save_owners(&first, at(1)).await.unwrap();
    assert_eq!(saved.who, NoteAuthor::Owner);
    assert_eq!((saved.created, saved.updated), (at(1), at(1)));

    let resent = notes.save_owners(&first, at(5)).await.unwrap();
    assert_eq!(resent.updated, at(1), "the same text again changes nothing");

    let edit = OwnerNote {
        text: "Slept badly",
        at: at(9),
        day: "2027-01-16",
        ..first.clone()
    };
    let edited = notes.save_owners(&edit, at(10)).await.unwrap();
    assert_eq!(edited.text, "Slept badly");
    assert_eq!(
        (edited.at, edited.day.as_str()),
        (at(0), "2027-01-15"),
        "an edit keeps its time"
    );
    assert_eq!(edited.updated, at(10));

    let entry = notes
        .write_assistants("2027-01-15", "A calm day.", at(20))
        .await
        .unwrap();
    assert_eq!(entry.who, NoteAuthor::Assistant);
    let rewritten = notes
        .write_assistants("2027-01-15", "A good day.", at(30))
        .await
        .unwrap();
    assert_eq!(
        (rewritten.uid, rewritten.created),
        (entry.uid.clone(), at(20))
    );

    let listed = notes
        .list(Some("2027-01-15"), Some("2027-01-15"))
        .await
        .unwrap();
    assert_eq!(listed.len(), 2);
    assert_eq!(listed[0].who, NoteAuthor::Assistant, "newest first");
    assert_eq!(
        notes.list(Some("2027-01-16"), None).await.unwrap(),
        [] as [emberhut_core::memory::notes::Note; 0]
    );

    let taken_over = OwnerNote {
        uid: &entry.uid,
        ..first.clone()
    };
    assert_eq!(
        notes.save_owners(&taken_over, at(40)).await.unwrap().text,
        "A good day.",
        "the app can't overwrite the assistant's entry"
    );
    assert_eq!(
        notes.owners_changed_after(Some(at(5))).await.unwrap().len(),
        1
    );
    assert_eq!(
        notes.owners_changed_after(Some(at(10))).await.unwrap(),
        [] as [emberhut_core::memory::notes::Note; 0]
    );

    assert!(!notes.delete_owners(&entry.uid).await.unwrap());
    assert!(notes.delete_owners("note-0001").await.unwrap());
    assert_eq!(notes.get("note-0001").await.unwrap(), None);
}

fn correction(minutes: i64, lesson: Option<&str>) -> NewCorrection {
    NewCorrection {
        said: "you didn't check".to_owned(),
        wrong: "I guessed".to_owned(),
        lesson: lesson.map(str::to_owned),
        lesson_to_owner: lesson.map(|_| "I check first now.".to_owned()),
        source: CorrectionSource::Assistant,
        at: at(minutes),
    }
}

#[tokio::test]
async fn corrections_and_their_trials() {
    let (_dir, db) = database().await;
    let corrections = db.corrections();
    let with_lesson = corrections
        .log(correction(10, Some("Check first.")))
        .await
        .unwrap();
    let without = corrections.log(correction(20, None)).await.unwrap();
    let kept = corrections
        .log(correction(30, Some("Say when unsure.")))
        .await
        .unwrap();

    assert_eq!(corrections.since(at(0)).await.unwrap().len(), 3);
    assert_eq!(corrections.since(at(15)).await.unwrap()[0].id, without);
    let open: Vec<i64> = corrections
        .open_trials(at(0))
        .await
        .unwrap()
        .iter()
        .map(|c| c.id)
        .collect();
    assert_eq!(open, vec![with_lesson, kept]);
    assert_eq!(
        corrections.since(at(0)).await.unwrap()[0]
            .lesson_to_owner
            .as_deref(),
        Some("I check first now."),
        "the owner's line is kept with the lesson"
    );

    assert!(
        corrections
            .keep_trial(kept, 4, TrialJudge::Owner, at(40))
            .await
            .unwrap()
    );
    assert_eq!(
        corrections.trial_end(kept).await.unwrap(),
        Some(TrialEnd::Kept(Some(4)))
    );
    assert!(
        !corrections
            .drop_trial(without, TrialJudge::Owner, at(40), at(0))
            .await
            .unwrap(),
        "no lesson, no trial"
    );
    assert!(
        !corrections
            .drop_trial(with_lesson, TrialJudge::Owner, at(40), at(10))
            .await
            .unwrap(),
        "past its days"
    );
    assert!(
        corrections
            .drop_trial(with_lesson, TrialJudge::Owner, at(40), trial_window(at(40)))
            .await
            .unwrap()
    );
    assert_eq!(
        corrections.trial_end(with_lesson).await.unwrap(),
        Some(TrialEnd::Dropped(Some("owner".to_owned())))
    );
    assert_eq!(
        corrections.open_trials(at(0)).await.unwrap(),
        [] as [emberhut_core::memory::learning::Correction; 0]
    );
    assert_eq!(corrections.trial_end(99).await.unwrap(), None);
}

fn doc(source: &str, reference: &str, text: &str) -> NewDoc {
    NewDoc {
        source: source.to_owned(),
        reference: reference.to_owned(),
        at: Some(at(1)),
        title: format!("{source} {reference}"),
        text: text.to_owned(),
        place: reference.to_owned(),
    }
}

#[tokio::test]
async fn the_index_finds_words_and_keeps_its_pieces_in_step() {
    let (_dir, db) = database().await;
    let recall = db.recall();
    let batch = IndexBatch {
        docs: vec![
            doc("chat", "chat:1", "The roof tiles arrive on Monday"),
            doc("fact", "home/roof", "The roofer is called Grace"),
        ],
        only: Vec::new(),
        marks: vec![("chat".to_owned(), "1".to_owned())],
    };
    assert!(recall.index(&batch).await.unwrap());
    assert!(
        !recall.index(&batch).await.unwrap(),
        "nothing changed the second time"
    );
    assert_eq!(recall.mark("chat").await.unwrap().as_deref(), Some("1"));
    assert_eq!(recall.count().await.unwrap(), 2);

    // docs_fts: the words, stemmed ("tile" finds "tiles").
    let found = recall
        .search_words("\"tile\" OR \"nothing\"", 10)
        .await
        .unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].reference, "chat:1");

    // A long text is in pieces; when it gets shorter, its later pieces go.
    let long = format!("{}\n\n{}", "alpha ".repeat(200), "omega ".repeat(200));
    let pieces = IndexBatch {
        docs: vec![doc("note", "note:a", &long)],
        ..IndexBatch::default()
    };
    recall.index(&pieces).await.unwrap();
    assert_eq!(recall.count_of("note").await.unwrap(), 2);
    assert_eq!(
        recall.search_words("\"omega\"", 10).await.unwrap()[0].reference,
        "note:a#1"
    );
    let shorter = IndexBatch {
        docs: vec![doc("note", "note:a", "alpha only")],
        ..IndexBatch::default()
    };
    recall.index(&shorter).await.unwrap();
    assert_eq!(recall.count_of("note").await.unwrap(), 1);
    assert_eq!(
        recall.search_words("\"omega\"", 10).await.unwrap(),
        [] as [emberhut_core::memory::recall::Doc; 0]
    );

    // vecs: a changed piece loses its vector; a dropped one too.
    let ids: Vec<i64> = recall
        .unembedded(32)
        .await
        .unwrap()
        .iter()
        .map(|d| d.id)
        .collect();
    assert_eq!(ids.len(), 3);
    let fact = recall.search_words("\"roofer\"", 1).await.unwrap()[0].id;
    assert_eq!(ids[0], fact, "the facts first");
    recall
        .store_vectors(
            &ids.iter()
                .map(|id| (*id, vec![0.5, -0.25]))
                .collect::<Vec<_>>(),
        )
        .await
        .unwrap();
    assert_eq!(
        recall.unembedded(32).await.unwrap(),
        [] as [emberhut_core::memory::recall::Doc; 0]
    );
    assert_eq!(recall.vectors().await.unwrap()[0].1, vec![0.5, -0.25]);
    let changed = IndexBatch {
        docs: vec![doc("chat", "chat:1", "The roof tiles arrive on Tuesday")],
        ..IndexBatch::default()
    };
    recall.index(&changed).await.unwrap();
    assert_eq!(recall.vector_ids().await.unwrap().len(), 2);
    assert_eq!(
        recall.search_words("\"tuesday\"", 10).await.unwrap()[0].reference,
        "chat:1",
        "the update trigger indexed the new words"
    );
    assert_eq!(
        recall.search_words("\"monday\"", 10).await.unwrap(),
        [] as [emberhut_core::memory::recall::Doc; 0],
        "and dropped the old ones"
    );
    let only = IndexBatch {
        only: vec![("fact".to_owned(), HashSet::new())],
        ..IndexBatch::default()
    };
    assert!(
        recall.index(&only).await.unwrap(),
        "a fact no longer kept is dropped"
    );
    assert_eq!(recall.vector_ids().await.unwrap().len(), 1);
    assert_eq!(
        recall.search_words("\"roofer\"", 1).await.unwrap(),
        [] as [emberhut_core::memory::recall::Doc; 0]
    );
    let known = recall.docs(&[ids[0], ids[1], ids[2]]).await.unwrap();
    assert_eq!(known.len(), 2);
}

#[tokio::test]
async fn transcript_offsets_are_kept_and_forgotten() {
    let (_dir, db) = database().await;
    let recall = db.recall();
    let path = "/data/transcripts/a.jsonl";
    let read = TranscriptOffset {
        offset: 120,
        line: 2,
    };
    let docs = [doc(
        "session",
        &format!("{path}:2"),
        "we chose the blue door",
    )];
    assert!(recall.read_transcript(path, read, &docs).await.unwrap());
    assert_eq!(recall.transcript_offset(path).await.unwrap(), Some(read));
    assert_eq!(
        recall.transcript_offsets().await.unwrap().get(path),
        Some(&read)
    );
    recall.forget_transcript(path).await.unwrap();
    assert_eq!(recall.transcript_offset(path).await.unwrap(), None);
    assert_eq!(recall.count().await.unwrap(), 0);
}

#[tokio::test]
async fn the_conversation_reads_back_without_events() {
    let (_dir, db) = database().await;
    let messages = db.messages();
    for (author, text, minutes) in [
        (Author::User, "Hello", 1),
        (Author::Event, "a program said something", 2),
        (Author::Assistant, "Hi there", 3),
        (Author::Assistant, "", 4),
    ] {
        messages
            .append(
                NewMessage {
                    room: RoomId::MAIN,
                    author,
                    text: text.to_owned(),
                    reply_to: None,
                },
                at(minutes),
            )
            .await
            .unwrap();
    }
    let conversation = db.conversation();
    let said = conversation.said_since(at(0)).await.unwrap();
    assert_eq!(
        said.iter().map(|s| s.text.as_str()).collect::<Vec<_>>(),
        ["Hello", "Hi there"]
    );
    assert!(said[0].mine && !said[1].mine);
    assert_eq!(said[0].room_name, "Main");
    let after = conversation.said_after(Some(said[0].id), 10).await.unwrap();
    assert_eq!(after.len(), 1);
    assert_eq!(
        conversation.said_after(None, 1).await.unwrap()[0].text,
        "Hello"
    );
}
