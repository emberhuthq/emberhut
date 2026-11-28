//! The import's store: the archive, its check against the source, and the file it leaves.
#![expect(
    clippy::unwrap_used,
    reason = "test helpers: a failure here fails the test that called them"
)]

use std::path::Path;

use emberhut_core::message::{Author, HistoryQuery, NewMessage};
use emberhut_core::upload::FileKind;
use emberhut_core::{DeviceDetails, RoomId};
use emberhut_db::import::{
    ArchivedRow, FileRecord, ImportTarget, ImportedFile, ImportedMessage, ImportedNote,
    ImportedTask, Numbered, Row, RunRecord, Scrub, SqlValue, keep_devices,
};
use emberhut_db::{Database, DatabaseUrl, DbError, MACHINE_ID};
use sea_orm::ConnectionTrait;

use crate::common::{at, tokens};

/// A source database in `dir` with one invented table, made the way any program would.
async fn source(dir: &Path) -> std::path::PathBuf {
    let path = dir.join("source.db");
    let conn = sea_orm::Database::connect(format!(
        "{}?mode=rwc",
        DatabaseUrl::from_path(&path).unwrap()
    ))
    .await
    .unwrap();
    for sql in [
        "CREATE TABLE notes (id integer primary key, n integer, x real, t text COLLATE NOCASE, b blob)",
        "INSERT INTO notes VALUES (1, 3, 1.5, 'Abc', x'00ff')",
        "INSERT INTO notes VALUES (2, NULL, -0.0, 'é', NULL)",
        "INSERT INTO notes VALUES (5, 7, 2.0, CAST(x'66ff' AS TEXT), x'')",
    ] {
        conn.execute_unprepared(sql).await.unwrap();
    }
    conn.close().await.unwrap();
    path
}

fn row(id: i64, n: SqlValue, x: f64, t: SqlValue, b: SqlValue) -> ArchivedRow {
    let mut row = Row::new();
    row.push("id", SqlValue::Integer(id));
    row.push("n", n);
    row.push("x", SqlValue::Real(x));
    row.push("t", t);
    row.push("b", b);
    ArchivedRow::new(id, id, Some(id.to_string()), row)
}

fn exact_rows() -> Vec<ArchivedRow> {
    vec![
        row(
            1,
            SqlValue::Integer(3),
            1.5,
            SqlValue::Text("Abc".into()),
            SqlValue::Blob(vec![0, 255]),
        ),
        row(
            2,
            SqlValue::Null,
            -0.0,
            SqlValue::Text("é".into()),
            SqlValue::Null,
        ),
        row(
            5,
            SqlValue::Integer(7),
            2.0,
            SqlValue::TextBytes(vec![0x66, 0xFF]),
            SqlValue::Blob(vec![]),
        ),
    ]
}

fn columns() -> Vec<String> {
    ["id", "n", "x", "t", "b"].map(String::from).to_vec()
}

#[tokio::test]
async fn an_exact_archive_passes_and_the_file_stands_alone() {
    let dir = tempfile::tempdir().unwrap();
    let copy = source(dir.path()).await;
    let target = dir.path().join("emberhut.db");
    let import = ImportTarget::create(&target, dir.path()).await.unwrap();
    import.attach("src", &copy).await.unwrap();
    let store = import.begin().await.unwrap();
    store
        .archive("example", "notes", &exact_rows())
        .await
        .unwrap();
    let check = store
        .check_archive("src", "example", "notes", &columns(), &[])
        .await
        .unwrap();
    assert!(check.is_exact(), "{check:?}");
    assert_eq!(check.source_rows, 3);
    assert_eq!(store.foreign_key_problems().await.unwrap(), []);
    assert_eq!(store.integrity().await.unwrap(), ["ok"]);
    let back = store
        .archived("example", "notes", Some(1), 10)
        .await
        .unwrap();
    assert_eq!(back, exact_rows()[1..]);
    store.commit().await.unwrap();

    for sibling in ["emberhut.db-wal", "emberhut.db-shm"] {
        assert!(!dir.path().join(sibling).exists(), "{sibling} was left");
    }
    let url: DatabaseUrl = DatabaseUrl::from_path(&target).unwrap();
    Database::open_existing(&url).await.unwrap();
}

/// A change to the archived rows that the check must see.
type Change = Box<dyn Fn(&mut Vec<ArchivedRow>)>;

#[tokio::test]
async fn the_check_sees_a_changed_storage_class_a_changed_case_and_a_missing_row() {
    let dir = tempfile::tempdir().unwrap();
    let copy = source(dir.path()).await;
    let cases: [(&str, Change); 4] = [
        (
            "the integer 3 as the text '3'",
            Box::new(|rows| {
                rows[0] = row(
                    1,
                    SqlValue::Text("3".into()),
                    1.5,
                    SqlValue::Text("Abc".into()),
                    SqlValue::Blob(vec![0, 255]),
                );
            }),
        ),
        (
            "another case in a NOCASE column",
            Box::new(|rows| {
                rows[0] = row(
                    1,
                    SqlValue::Integer(3),
                    1.5,
                    SqlValue::Text("abc".into()),
                    SqlValue::Blob(vec![0, 255]),
                );
            }),
        ),
        (
            "text bytes as a blob",
            Box::new(|rows| {
                rows[2] = row(
                    5,
                    SqlValue::Integer(7),
                    2.0,
                    SqlValue::Blob(vec![0x66, 0xFF]),
                    SqlValue::Blob(vec![]),
                );
            }),
        ),
        (
            "a row left out",
            Box::new(|rows| {
                rows.pop();
            }),
        ),
    ];
    for (i, (what, change)) in cases.iter().enumerate() {
        let target = dir.path().join(format!("emberhut-{i}.db"));
        let import = ImportTarget::create(&target, dir.path()).await.unwrap();
        import.attach("src", &copy).await.unwrap();
        let store = import.begin().await.unwrap();
        let mut rows = exact_rows();
        change(&mut rows);
        store.archive("example", "notes", &rows).await.unwrap();
        let check = store
            .check_archive("src", "example", "notes", &columns(), &[])
            .await
            .unwrap();
        assert!(!check.is_exact(), "{what}: {check:?}");
        store.abandon().await.unwrap();
    }
}

#[tokio::test]
async fn a_scrubbed_column_is_compared_with_what_the_scrub_leaves() {
    let dir = tempfile::tempdir().unwrap();
    let copy = source(dir.path()).await;
    // The archive with `t` scrubbed in every row, and with it scrubbed in row 1 only.
    let scrubbed = |all: bool| -> Vec<ArchivedRow> {
        exact_rows()
            .into_iter()
            .map(|archived| {
                let mut row = archived.row().clone();
                if all || archived.rowid == 1 {
                    row.scrub("t");
                }
                ArchivedRow::new(archived.ordinal, archived.rowid, archived.key, row)
            })
            .collect()
    };
    let whole = Scrub {
        column: "t".to_owned(),
        when: None,
    };
    let in_rows = |values: [&str; 1]| Scrub {
        column: "t".to_owned(),
        when: Some(("t".to_owned(), values.map(String::from).to_vec())),
    };
    for (i, (rows, scrub, exact)) in [
        (scrubbed(true), whole.clone(), true),
        (scrubbed(false), whole, false),
        (scrubbed(false), in_rows(["Abc"]), true),
        (scrubbed(false), in_rows(["zzz"]), false),
        (exact_rows(), in_rows(["Abc"]), false),
    ]
    .into_iter()
    .enumerate()
    {
        let import = ImportTarget::create(&dir.path().join(format!("emberhut-{i}.db")), dir.path())
            .await
            .unwrap();
        import.attach("src", &copy).await.unwrap();
        let store = import.begin().await.unwrap();
        store.archive("example", "notes", &rows).await.unwrap();
        let check = store
            .check_archive("src", "example", "notes", &columns(), &[scrub])
            .await
            .unwrap();
        assert_eq!(check.is_exact(), exact, "case {i}: {check:?}");
        store.abandon().await.unwrap();
    }
}

#[tokio::test]
async fn an_import_never_writes_over_a_file() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("emberhut.db");
    std::fs::write(&target, b"keep me").unwrap();
    assert!(matches!(
        ImportTarget::create(&target, dir.path()).await,
        Err(DbError::TargetExists(_))
    ));
    assert_eq!(std::fs::read(&target).unwrap(), b"keep me");
}

#[tokio::test]
async fn devices_sessions_and_browser_profiles_are_kept_into_an_unused_database_only() {
    let old_dir = tempfile::tempdir().unwrap();
    let new_dir = tempfile::tempdir().unwrap();
    let url = |dir: &tempfile::TempDir| -> DatabaseUrl {
        DatabaseUrl::from_path(&dir.path().join("emberhut.db")).unwrap()
    };
    let old = Database::connect(&url(&old_dir)).await.unwrap();
    old.sessions()
        .open(
            &DeviceDetails::new(Some("Laptop"), Some("Example 1"), None),
            &tokens(0),
            at(0),
        )
        .await
        .unwrap();
    old.sessions()
        .open(
            &DeviceDetails::new(Some("Phone"), Some("Example 2"), Some("install-2")),
            &tokens(1),
            at(1),
        )
        .await
        .unwrap();
    let work = old
        .chrome()
        .insert("Work", "Work profile", "hash-work", at(2))
        .await
        .unwrap();
    old.chrome()
        .insert("Home", "Home profile", "hash-home", at(3))
        .await
        .unwrap();
    old.chrome()
        .set_task_browser(7, work.id, at(4))
        .await
        .unwrap();
    old.close().await.unwrap();
    Database::connect(&url(&new_dir))
        .await
        .unwrap()
        .close()
        .await
        .unwrap();

    let kept = keep_devices(&url(&old_dir), &url(&new_dir)).await.unwrap();
    assert_eq!(
        (kept.devices, kept.sessions, kept.browser_profiles),
        (2, 2, 2)
    );
    assert!(
        !kept.machine,
        "the old database was never served with a machine key"
    );
    let new = Database::open_existing(&url(&new_dir)).await.unwrap();
    assert_eq!(new.sessions().signed_in(at(2)).await.unwrap().len(), 2);
    // A profile keeps its id and its token, so the extension stays paired.
    let kept_work = new.chrome().by_token("hash-work").await.unwrap().unwrap();
    assert_eq!(kept_work.id, work.id);
    assert_eq!(new.chrome().profiles().await.unwrap().len(), 2);
    // A task's browser choice is not carried: the import makes the tasks again, and task 7 of the
    // old file may be another task in the new one.
    assert_eq!(new.chrome().task_browser(7).await.unwrap(), None);
    new.close().await.unwrap();

    assert!(matches!(
        keep_devices(&url(&old_dir), &url(&new_dir)).await,
        Err(DbError::NotUnused(_))
    ));
}

#[tokio::test]
async fn the_machine_the_old_database_was_served_as_is_kept_too() {
    let dir = tempfile::tempdir().unwrap();
    let url =
        |name: &str| -> DatabaseUrl { DatabaseUrl::from_path(&dir.path().join(name)).unwrap() };
    let old = Database::connect(&url("old.db")).await.unwrap();
    old.settings()
        .set(&MACHINE_ID, &"machine-a".to_owned())
        .await
        .unwrap();
    old.sessions()
        .open(
            &DeviceDetails::new(Some("Laptop"), None, None),
            &tokens(0),
            at(0),
        )
        .await
        .unwrap();
    old.close().await.unwrap();

    Database::connect(&url("new.db"))
        .await
        .unwrap()
        .close()
        .await
        .unwrap();
    let kept = keep_devices(&url("old.db"), &url("new.db")).await.unwrap();
    assert!(kept.machine);
    let new = Database::open_existing(&url("new.db")).await.unwrap();
    assert_eq!(
        new.settings().get(&MACHINE_ID).await.unwrap().as_deref(),
        Some("machine-a")
    );
    new.close().await.unwrap();

    // A database served as another machine takes nothing.
    let other = Database::connect(&url("other.db")).await.unwrap();
    other
        .settings()
        .set(&MACHINE_ID, &"machine-b".to_owned())
        .await
        .unwrap();
    other.close().await.unwrap();
    assert!(matches!(
        keep_devices(&url("old.db"), &url("other.db")).await,
        Err(DbError::OtherMachine { .. })
    ));
    let other = Database::open_existing(&url("other.db")).await.unwrap();
    assert_eq!(other.sessions().signed_in(at(1)).await.unwrap().len(), 0);
    other.close().await.unwrap();
}

/// A task with only what the test reads set.
fn task(id: i64, status: &str, updated: i64) -> ImportedTask {
    ImportedTask {
        id,
        title: format!("Task {id}"),
        body: None,
        owner: "owner".into(),
        status: status.into(),
        goal: None,
        project: None,
        pid: None,
        hard: false,
        size: None,
        priority: 3,
        due: None,
        today: false,
        parked_until: None,
        decide_by: None,
        decision: None,
        last_ask: None,
        action: None,
        created: 1_000,
        updated,
        done_at: None,
        last_note: None,
        parent: None,
        progress: None,
        total: None,
        count_unit: None,
        tracked: false,
        eta: None,
        ultra: false,
        ultra_parallel: None,
        ultra_budget: None,
        model: None,
        area: None,
        kind: None,
        landed: None,
        fixed_by: None,
        runner: None,
        next: None,
        unit: None,
        session: None,
        requested: None,
        room: None,
        ultra_run: None,
        turn_file: None,
        requested_by: None,
    }
}

fn said_by_owner(text: &str) -> NewMessage {
    NewMessage {
        room: RoomId::MAIN,
        author: Author::User,
        text: text.into(),
        reply_to: None,
    }
}

fn file_record(path: &str, sha256: &str) -> FileRecord {
    FileRecord {
        tree: "uploads".into(),
        path: path.into(),
        bytes: 1,
        sha256: sha256.into(),
    }
}

#[tokio::test]
#[expect(
    clippy::too_many_lines,
    reason = "one run of the server and two later imports, as they follow each other"
)]
async fn a_later_import_adds_to_a_database_in_use() {
    let dir = tempfile::tempdir().unwrap();
    let url: DatabaseUrl = DatabaseUrl::from_path(&dir.path().join("emberhut.db")).unwrap();
    let missing = ImportTarget::open_existing(&url, dir.path()).await;
    assert!(matches!(missing, Err(DbError::Missing(_))), "never made");
    // The server, running all along.
    let server = Database::connect(&url).await.unwrap();

    let target = ImportTarget::open_existing(&url, dir.path()).await.unwrap();
    let reading = target.read().await.unwrap();
    assert_eq!(reading.next_id(Numbered::Messages).await.unwrap(), 1);
    assert_eq!(reading.runs().await.unwrap(), Vec::<RunRecord>::new());
    // A snapshot that reads keeps no writer waiting.
    let said = server
        .messages()
        .append(said_by_owner("First"), at(1))
        .await
        .unwrap();
    assert_eq!(said.id.get(), 1);
    let store = reading.end_read().await.unwrap().begin().await.unwrap();
    store
        .insert_messages(&[ImportedMessage {
            id: 7,
            room_id: 1,
            author: Author::User,
            text: "Hello".into(),
            created_at: 1_000,
            reply_to_id: None,
        }])
        .await
        .unwrap();
    store
        .insert_files(&[ImportedFile {
            id: 2,
            stored_name: "0a0b0c0d-a.png".into(),
            name: "a.png".into(),
            kind: FileKind::Image,
            size: 1,
            created_at: 1_000,
        }])
        .await
        .unwrap();
    store.insert_tasks(&[task(4, "todo", 1_000)]).await.unwrap();
    store
        .archive("source", "notes", &exact_rows())
        .await
        .unwrap();
    store
        .record_files(&[file_record("0a0b0c0d-a.png", "aa")])
        .await
        .unwrap();
    store
        .record_run(&RunRecord {
            ran_at: at(0),
            version: "1".into(),
            input: "later".into(),
            sources: "{}".into(),
            report: "done".into(),
        })
        .await
        .unwrap();
    store.commit().await.unwrap();
    assert!(
        dir.path().join("emberhut.db-wal").exists(),
        "a file in use is left to its server's checkpoints"
    );

    let store = ImportTarget::open_existing(&url, dir.path())
        .await
        .unwrap()
        .begin()
        .await
        .unwrap();
    assert_eq!(store.next_id(Numbered::Messages).await.unwrap(), 8);
    assert_eq!(store.next_id(Numbered::Files).await.unwrap(), 3);
    let found = store
        .files_by_stored_name(&["0a0b0c0d-a.png".into(), "other".into()])
        .await
        .unwrap();
    assert_eq!(found.iter().map(|file| file.id).collect::<Vec<_>>(), [2]);
    let note = ImportedNote {
        id: 3,
        uid: "diary:2026-11-28".into(),
        who: "assistant".into(),
        day: "2026-11-28".into(),
        at: 1_000,
        text: "A day".into(),
        created: 1_000,
        updated: 1_000,
    };
    store
        .insert_notes(std::slice::from_ref(&note))
        .await
        .unwrap();
    assert_eq!(
        store
            .notes_by_uid(&["diary:2026-11-28".into(), "other".into()])
            .await
            .unwrap(),
        [note]
    );
    assert_eq!(store.next_id(Numbered::Notes).await.unwrap(), 4);
    store.update_tasks(&[task(4, "done", 2_000)]).await.unwrap();
    assert_eq!(
        store.tasks_by_id(&[4]).await.unwrap(),
        [task(4, "done", 2_000)]
    );
    assert!(store.update_tasks(&[task(5, "done", 2_000)]).await.is_err());
    let mut changed = exact_rows();
    changed.truncate(1);
    let mut values = Row::new();
    values.push("id", SqlValue::Integer(1));
    values.push("n", SqlValue::Integer(4));
    changed[0] = ArchivedRow::new(1, 1, Some("1".into()), values);
    store.rearchive("source", "notes", &changed).await.unwrap();
    let archived = store.archived("source", "notes", None, 10).await.unwrap();
    assert_eq!(archived[0], changed[0]);
    assert_eq!(archived.len(), 3);
    let elsewhere = ArchivedRow::new(9, 9, None, Row::new());
    assert!(matches!(
        store.rearchive("source", "notes", &[elsewhere]).await,
        Err(DbError::NotArchived { ordinal: 9, .. })
    ));
    store
        .record_files(&[file_record("0a0b0c0d-a.png", "bb"), file_record("b", "cc")])
        .await
        .unwrap();
    let files = store.recorded_files().await.unwrap();
    assert_eq!(
        files,
        [file_record("0a0b0c0d-a.png", "bb"), file_record("b", "cc")]
    );
    let runs = store.runs().await.unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!((runs[0].ran_at, runs[0].input.as_str()), (at(0), "later"));
    store.commit().await.unwrap();

    // The server sees what was added, and numbers its own after it.
    let page = server
        .messages()
        .page(RoomId::MAIN, HistoryQuery::default())
        .await
        .unwrap();
    let texts: Vec<&str> = page.iter().map(|message| message.text.as_str()).collect();
    assert_eq!(texts, ["First", "Hello"]);
    let said = server
        .messages()
        .append(said_by_owner("Next"), at(2))
        .await
        .unwrap();
    assert_eq!(said.id.get(), 8);
    server.close().await.unwrap();
}
