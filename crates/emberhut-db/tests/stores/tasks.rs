//! Tasks against a real SQLite database: the `issues` table and its history (`issue_events`),
//! what they produced (`artifacts`), the `projects` and `areas` they belong to, the `links` to
//! the work that moves them, and their `runs`.
#![expect(
    clippy::too_many_lines,
    reason = "each test walks one story of a task's rows from start to end"
)]

use emberhut_core::task::area::AreaStatus;
use emberhut_core::task::board::{ProjectChange, ProjectStatus};
use emberhut_core::task::live::{NewRun, RunKind};
use emberhut_core::task::search::TaskSearch;
use emberhut_core::task::wire::KEY_PREFIX;
use emberhut_core::task::{
    AreaId, Landed, Owner, PARKED_BY_ARCHIVE, ProjectId, Status, TaskChange, TaskId, TaskRefusal,
    Who,
};
use emberhut_db::{AreaChange, NewTask, ProjectStoreError, TaskStoreError};
use jiff::civil::date;

use crate::common::{at, database};

fn new(title: &str) -> NewTask {
    NewTask {
        title: title.to_owned(),
        ..NewTask::default()
    }
}

#[tokio::test]
async fn a_task_is_made_with_its_history_and_requested_when_the_person_adds_it() {
    let (_dir, db) = database().await;
    let tasks = db.tasks();
    let mine = tasks
        .create(
            NewTask {
                requested: Some(r#"{"at":"x","msg":null,"room":null}"#.to_owned()),
                ..new("  Name the report ")
            },
            Who::Person,
            at(1),
        )
        .await
        .unwrap();
    assert_eq!(mine.title, "Name the report");
    assert_eq!(
        (mine.owner, mine.status, mine.priority),
        (Owner::Person, Status::Todo, 3)
    );
    assert!(mine.requested.is_some());
    assert_eq!(mine.last.as_deref(), Some("Created"));
    let theirs = tasks
        .create(
            NewTask {
                owner: Some(Owner::Assistant),
                requested: Some("{}".to_owned()),
                ..new("Draft it")
            },
            Who::Assistant,
            at(2),
        )
        .await
        .unwrap();
    assert!(
        theirs.requested.is_none(),
        "only the person's own add is their request"
    );
    let history = tasks.history(theirs.id).await.unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].text, "Created by the assistant");
    assert!(
        tasks
            .note(theirs.id, Who::Worker, "Wrote the draft", at(3))
            .await
            .unwrap()
    );
    assert!(
        !tasks
            .note(TaskId::new(99), Who::Worker, "x", at(3))
            .await
            .unwrap()
    );
    let history = tasks.history(theirs.id).await.unwrap();
    assert_eq!((history[1].who, history[1].at), (Who::Worker, at(3)));
    assert_eq!(
        tasks.get(theirs.id).await.unwrap().unwrap().last.as_deref(),
        Some("Wrote the draft")
    );
    assert!(matches!(
        tasks.create(new("  "), Who::Person, at(4)).await,
        Err(TaskStoreError::Refused(TaskRefusal::NoTitle))
    ));
}

#[tokio::test]
async fn a_status_change_writes_its_line_and_clears_what_no_longer_holds() {
    let (_dir, db) = database().await;
    let tasks = db.tasks();
    let t = tasks
        .create(new("Ship"), Who::Assistant, at(1))
        .await
        .unwrap();
    let waiting = tasks
        .update(
            t.id,
            TaskChange {
                action: Some(Some("Approve the deploy".to_owned())),
                today: Some(true),
                eta: Some(Some("about 2 hours".to_owned())),
                ..TaskChange::default()
            },
            Who::Assistant,
            None,
            at(2),
        )
        .await
        .unwrap();
    assert_eq!(waiting.status, Status::Waiting);
    assert_eq!(waiting.last.as_deref(), Some("Waiting on the owner"));
    let done = tasks
        .update(
            t.id,
            TaskChange::status(Status::Done),
            Who::Person,
            Some("Shipped it"),
            at(3),
        )
        .await
        .unwrap();
    assert_eq!(done.status, Status::Done);
    assert_eq!(done.done_at, Some(at(3)));
    assert!(!done.today && done.action.is_none() && done.eta.is_none());
    assert_eq!(done.last.as_deref(), Some("Shipped it"));
    assert!(matches!(
        tasks
            .update(
                t.id,
                TaskChange::status(Status::Waiting),
                Who::Assistant,
                None,
                at(4)
            )
            .await,
        Err(TaskStoreError::Refused(TaskRefusal::WaitingWithoutAction))
    ));
}

#[tokio::test]
async fn subtasks_roll_up_and_settle_their_parent() {
    let (_dir, db) = database().await;
    let tasks = db.tasks();
    let parent = tasks
        .create(
            NewTask {
                owner: Some(Owner::Assistant),
                pid: Some(ProjectId::new(7)),
                ..new("Episodes")
            },
            Who::Assistant,
            at(1),
        )
        .await
        .unwrap();
    let a = tasks
        .create(
            NewTask {
                parent: Some(parent.id),
                total: Some(10),
                ..new("Season one")
            },
            Who::Assistant,
            at(2),
        )
        .await
        .unwrap();
    assert_eq!(
        (a.owner, a.pid),
        (Owner::Assistant, Some(ProjectId::new(7))),
        "takes its parent's"
    );
    let b = tasks
        .create(
            NewTask {
                parent: Some(parent.id),
                ..new("Season two")
            },
            Who::Assistant,
            at(3),
        )
        .await
        .unwrap();
    let family = tasks.family().await.unwrap();
    assert_eq!(family.rollup(parent.id).unwrap().total, 2);
    tasks
        .update(
            a.id,
            TaskChange {
                progress: Some(10),
                ..TaskChange::default()
            },
            Who::Assistant,
            None,
            at(4),
        )
        .await
        .unwrap();
    assert_eq!(
        tasks.get(a.id).await.unwrap().unwrap().status,
        Status::Done,
        "its count reached its total"
    );
    tasks
        .update(
            b.id,
            TaskChange::status(Status::Canceled),
            Who::Assistant,
            None,
            at(5),
        )
        .await
        .unwrap();
    let settled = tasks.get(parent.id).await.unwrap().unwrap();
    assert_eq!(
        settled.status,
        Status::Done,
        "every subtask is done or canceled"
    );
    tasks
        .update(
            b.id,
            TaskChange::status(Status::Todo),
            Who::Assistant,
            None,
            at(6),
        )
        .await
        .unwrap();
    assert_eq!(
        tasks.get(parent.id).await.unwrap().unwrap().status,
        Status::Todo,
        "open again"
    );
    assert!(matches!(
        tasks
            .update(
                parent.id,
                TaskChange {
                    parent: Some(Some(a.id)),
                    ..TaskChange::default()
                },
                Who::Assistant,
                None,
                at(7)
            )
            .await,
        Err(TaskStoreError::Refused(TaskRefusal::Loop { .. }))
    ));
    assert_eq!(tasks.subtasks(parent.id).await.unwrap().len(), 2);
}

#[tokio::test]
async fn findings_hang_off_their_scan_and_their_state_gives_their_status() {
    let (_dir, db) = database().await;
    let tasks = db.tasks();
    let scan = tasks
        .create(new("Audit the cache"), Who::Assistant, at(1))
        .await
        .unwrap();
    assert!(matches!(
        tasks
            .create(
                NewTask {
                    finding: true,
                    ..new("Leak")
                },
                Who::Assistant,
                at(2)
            )
            .await,
        Err(TaskStoreError::Refused(TaskRefusal::FindingWithoutScan))
    ));
    let leak = tasks
        .create(
            NewTask {
                finding: true,
                parent: Some(scan.id),
                ..new("Leak")
            },
            Who::Assistant,
            at(2),
        )
        .await
        .unwrap();
    assert!(leak.finding && leak.tracked);
    assert_eq!(
        (leak.landed, leak.status),
        (Some(Landed::Unchecked), Status::Todo)
    );
    assert_eq!(
        leak.last.as_deref(),
        Some(format!("Filed as a finding of {KEY_PREFIX}1: unchecked").as_str())
    );
    let landed = tasks
        .update(
            leak.id,
            TaskChange {
                landed: Some(Landed::Landed),
                ..TaskChange::default()
            },
            Who::Assistant,
            None,
            at(3),
        )
        .await
        .unwrap();
    assert_eq!(landed.status, Status::Done);
    assert_eq!(landed.last.as_deref(), Some("Now landed"));
    assert_eq!(tasks.findings(scan.id).await.unwrap().len(), 1);
    assert_eq!(
        tasks.subtasks(scan.id).await.unwrap(),
        [] as [emberhut_core::task::Task; 0]
    );
    assert_eq!(
        tasks.get(scan.id).await.unwrap().unwrap().status,
        Status::Todo,
        "a finding doesn't settle its scan"
    );
    assert!(matches!(
        tasks
            .create(
                NewTask {
                    parent: Some(leak.id),
                    ..new("Under it")
                },
                Who::Assistant,
                at(4)
            )
            .await,
        Err(TaskStoreError::Refused(TaskRefusal::UnderAFinding(_)))
    ));
}

#[tokio::test]
async fn a_search_finds_by_key_then_by_words_and_lists_newest_first() {
    let (_dir, db) = database().await;
    let tasks = db.tasks();
    let report = tasks
        .create(new("Name the report"), Who::Person, at(1))
        .await
        .unwrap();
    let body = tasks
        .create(
            NewTask {
                body: Some("the report's 100% figures".to_owned()),
                ..new("Check figures")
            },
            Who::Person,
            at(2),
        )
        .await
        .unwrap();
    tasks
        .create(new("Something else"), Who::Person, at(3))
        .await
        .unwrap();
    let found = tasks.search(&TaskSearch::new("report")).await.unwrap();
    assert_eq!(
        found.iter().map(|t| t.id).collect::<Vec<_>>(),
        [report.id, body.id],
        "title matches first"
    );
    let percent = tasks.search(&TaskSearch::new("100%")).await.unwrap();
    assert_eq!(percent.len(), 1, "a % in the query is matched as itself");
    let by_key = tasks
        .search(&TaskSearch::new(&format!("{KEY_PREFIX}2")))
        .await
        .unwrap();
    assert_eq!(by_key[0].id, body.id);
    let all = tasks
        .search(&TaskSearch::new("").limited(Some(2)))
        .await
        .unwrap();
    assert_eq!(all.len(), 2);
    assert_eq!(all[0].title, "Something else", "newest first");
    tasks
        .update(
            body.id,
            TaskChange::status(Status::Canceled),
            Who::Person,
            None,
            at(4),
        )
        .await
        .unwrap();
    let archived = tasks
        .search(&TaskSearch {
            archived: true,
            ..TaskSearch::new("")
        })
        .await
        .unwrap();
    assert_eq!(archived.iter().map(|t| t.id).collect::<Vec<_>>(), [body.id]);
    let listed = tasks.listed(at(0), at(5)).await.unwrap();
    assert_eq!(listed.len(), 2, "open ones; the canceled one left out");
}

#[tokio::test]
async fn an_archived_project_parks_what_doesnt_need_the_person() {
    let (_dir, db) = database().await;
    let projects = db.projects();
    let pid = projects
        .set(
            None,
            ProjectChange {
                name: Some(" Example ".to_owned()),
                priority: Some(2),
                ..ProjectChange::default()
            },
            at(1),
        )
        .await
        .unwrap();
    assert_eq!(
        projects
            .set(
                None,
                ProjectChange {
                    name: Some("example".to_owned()),
                    ..ProjectChange::default()
                },
                at(2)
            )
            .await
            .unwrap(),
        pid,
        "a name already there is that project"
    );
    assert!(matches!(
        projects.set(None, ProjectChange::default(), at(2)).await,
        Err(ProjectStoreError::NeedsName)
    ));
    let tasks = db.tasks();
    let quiet = tasks
        .create(
            NewTask {
                pid: Some(pid),
                ..new("Quiet")
            },
            Who::Person,
            at(3),
        )
        .await
        .unwrap();
    let due = tasks
        .create(
            NewTask {
                pid: Some(pid),
                hard: true,
                due: Some(date(2026, 12, 1)),
                ..new("Due")
            },
            Who::Person,
            at(3),
        )
        .await
        .unwrap();
    projects
        .set(
            Some(pid),
            ProjectChange {
                status: Some(ProjectStatus::Archived),
                ..ProjectChange::default()
            },
            at(4),
        )
        .await
        .unwrap();
    assert_eq!(
        tasks
            .get(quiet.id)
            .await
            .unwrap()
            .unwrap()
            .parked_until
            .as_deref(),
        Some(PARKED_BY_ARCHIVE)
    );
    assert_eq!(
        tasks.get(due.id).await.unwrap().unwrap().parked_until,
        None,
        "a hard deadline stays"
    );
    projects
        .set(
            Some(pid),
            ProjectChange {
                status: Some(ProjectStatus::Paused),
                paused_until: Some(Some(date(2026, 11, 20))),
                ..ProjectChange::default()
            },
            at(5),
        )
        .await
        .unwrap();
    assert_eq!(
        tasks.get(quiet.id).await.unwrap().unwrap().parked_until,
        None,
        "taken back"
    );
    let resumed = projects
        .resume_due(date(2026, 11, 25), at(6))
        .await
        .unwrap();
    assert_eq!(resumed[0].status, ProjectStatus::Active);
    let listed = projects.list().await.unwrap();
    assert_eq!(
        (listed[0].name.as_str(), listed[0].priority),
        ("example", 2),
        "named again as it was given"
    );
}

#[tokio::test]
async fn areas_hold_a_tree_and_move_it_with_them() {
    let (_dir, db) = database().await;
    let pid = db
        .projects()
        .set(
            None,
            ProjectChange {
                name: Some("Harness".to_owned()),
                ..ProjectChange::default()
            },
            at(1),
        )
        .await
        .unwrap();
    let areas = db.areas();
    let area = areas
        .set(
            AreaChange {
                pid: Some(pid),
                name: Some("Checks".to_owned()),
                goal: Some(Some(" Green ".to_owned())),
                ..AreaChange::default()
            },
            at(2),
        )
        .await
        .unwrap();
    let again = areas
        .set(
            AreaChange {
                pid: Some(pid),
                name: Some("checks".to_owned()),
                ..AreaChange::default()
            },
            at(3),
        )
        .await
        .unwrap();
    assert_eq!(again, area);
    let got = areas.get(area).await.unwrap().unwrap();
    assert_eq!(
        (got.name.as_str(), got.goal.as_deref()),
        ("Checks", Some("Green"))
    );
    assert_eq!(
        areas.find("CHECKS", Some(pid)).await.unwrap().unwrap().id,
        area
    );
    let tasks = db.tasks();
    let top = tasks
        .create(
            NewTask {
                area: Some(area),
                ..new("Top")
            },
            Who::Assistant,
            at(4),
        )
        .await
        .unwrap();
    assert_eq!(top.pid, Some(pid), "an area's task is in its project");
    let kid = tasks
        .create(
            NewTask {
                parent: Some(top.id),
                ..new("Kid")
            },
            Who::Assistant,
            at(5),
        )
        .await
        .unwrap();
    assert_eq!(kid.area, Some(area), "a subtask sits in its parent's area");
    assert!(matches!(
        tasks
            .update(
                kid.id,
                TaskChange {
                    area: Some(None),
                    ..TaskChange::default()
                },
                Who::Assistant,
                None,
                at(6)
            )
            .await,
        Err(TaskStoreError::Refused(TaskRefusal::SubtaskArea { .. }))
    ));
    tasks
        .update(
            top.id,
            TaskChange {
                area: Some(None),
                ..TaskChange::default()
            },
            Who::Assistant,
            None,
            at(7),
        )
        .await
        .unwrap();
    assert_eq!(
        tasks.get(kid.id).await.unwrap().unwrap().area,
        None,
        "the subtree moved"
    );
    areas
        .set(
            AreaChange {
                id: Some(area),
                status: Some(AreaStatus::Done),
                ..AreaChange::default()
            },
            at(8),
        )
        .await
        .unwrap();
    assert_eq!(
        areas.list(Some(pid)).await.unwrap()[0].status,
        AreaStatus::Done
    );
    assert_eq!(
        tasks.in_area(area).await.unwrap(),
        [] as [emberhut_core::task::Task; 0]
    );
    assert!(matches!(
        areas
            .set(
                AreaChange {
                    id: Some(AreaId::new(99)),
                    ..AreaChange::default()
                },
                at(9)
            )
            .await,
        Err(emberhut_db::AreaStoreError::NoArea(_))
    ));
}

#[tokio::test]
async fn artifacts_links_and_runs_hang_off_their_task() {
    let (_dir, db) = database().await;
    let t = db
        .tasks()
        .create(new("Make the chart"), Who::Assistant, at(1))
        .await
        .unwrap();
    let artifacts = db.artifacts();
    let chart = artifacts
        .attach(t.id, "/tmp/chart.png", "image", None, at(2))
        .await
        .unwrap();
    let again = artifacts
        .attach(t.id, "/tmp/chart.png", "image", Some("The chart"), at(3))
        .await
        .unwrap();
    assert_eq!(again.id, chart.id);
    assert_eq!(again.caption.as_deref(), Some("The chart"));
    let reply = artifacts
        .put_reply(t.id, "https://x.com/a/status/1", "{\"replies\":[]}", at(4))
        .await
        .unwrap();
    artifacts
        .put_reply(t.id, "https://x.com/a/status/1", "{\"replies\":[1]}", at(5))
        .await
        .unwrap();
    assert_eq!(
        artifacts
            .get(reply.id)
            .await
            .unwrap()
            .unwrap()
            .data
            .as_deref(),
        Some("{\"replies\":[1]}")
    );
    let of = artifacts.of(&[t.id]).await.unwrap();
    assert_eq!(of[&t.id].len(), 2);
    assert_eq!(of[&t.id][0].kind, "x_reply", "newest first");

    let links = db.links();
    links.link(t.id, "ticket", "cal-12", None).await.unwrap();
    links
        .link(t.id, "thread", "r-1", Some("runner"))
        .await
        .unwrap();
    assert_eq!(links.tasks_of("ticket", "CAL-12").await.unwrap(), [t.id]);
    assert_eq!(links.unlink(t.id, "CAL-12").await.unwrap(), 1);
    assert_eq!(
        links.of(t.id).await.unwrap()[0].machine.as_deref(),
        Some("runner")
    );

    let runs = db.runs();
    let queued = runs
        .open(
            NewRun {
                task: t.id,
                machine: "runner".to_owned(),
                thread: "r-1".to_owned(),
                kind: RunKind::Thread,
                asked: Some("Make it\n  now".to_owned()),
                started: false,
            },
            at(6),
        )
        .await
        .unwrap();
    assert_eq!(queued.asked.as_deref(), Some("Make it now"));
    let worker = runs
        .open(
            NewRun {
                task: t.id,
                machine: "worker".to_owned(),
                thread: "w-1".to_owned(),
                kind: RunKind::Worker,
                asked: None,
                started: true,
            },
            at(7),
        )
        .await
        .unwrap();
    let latest = runs.latest(&[t.id]).await.unwrap();
    assert_eq!(
        latest[&t.id].0.as_ref().unwrap().id,
        worker.id,
        "a started run before a queued one"
    );
    assert_eq!(
        runs.open_worker("w-1").await.unwrap().unwrap().id,
        worker.id
    );
    assert!(
        runs.close(worker.id, "completed", Some("Done: the chart"), at(8))
            .await
            .unwrap()
    );
    assert!(
        !runs
            .close(worker.id, "completed", None, at(9))
            .await
            .unwrap(),
        "closed once"
    );
    let latest = runs.latest(&[t.id]).await.unwrap();
    assert_eq!(
        latest[&t.id].1.as_ref().unwrap().outcome.as_deref(),
        Some("completed")
    );
    assert_eq!(runs.of_task(t.id, 10).await.unwrap().len(), 2);
    assert!(
        runs.close(queued.id, "interrupted", None, at(9))
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn every_ask_has_its_own_id_and_an_answer_names_the_latest_ask() {
    let (_dir, db) = database().await;
    let tasks = db.tasks();
    let t = tasks
        .create(new("Write to Ann"), Who::Person, at(1))
        .await
        .unwrap();
    assert_eq!(tasks.last_ask(t.id).await.unwrap(), None);
    let ask = |line: &'static str, minutes: i64| {
        let tasks = tasks.clone();
        async move {
            let (_, id) = tasks
                .ask(
                    t.id,
                    "{}".to_owned(),
                    "Send it".to_owned(),
                    None,
                    line,
                    Who::Assistant,
                    at(minutes),
                )
                .await
                .unwrap();
            id
        }
    };
    let first = ask("Send this Slack message to Ann?", 2).await;
    assert!(
        tasks
            .answer(t.id, Who::Person, "Chose: Send it", at(3))
            .await
            .unwrap()
    );
    let again = ask("Re-checked: still needs you", 4).await;
    assert_ne!(first, again, "a re-check is an ask of its own");
    assert_eq!(tasks.last_ask(t.id).await.unwrap(), Some(again));
    let answered: Vec<String> = tasks
        .answers_to(t.id, first)
        .await
        .unwrap()
        .into_iter()
        .map(|line| line.text)
        .collect();
    assert_eq!(answered, ["Chose: Send it"]);
    assert_eq!(tasks.answers_to(t.id, again).await.unwrap().len(), 0);
    assert!(
        !tasks
            .answer(TaskId::new(999), Who::Person, "Chose: Send it", at(5))
            .await
            .unwrap(),
        "no such task"
    );
}

#[tokio::test]
async fn a_task_remembers_the_room_it_was_made_in() {
    let (_dir, db) = database().await;
    let tasks = db.tasks();
    let room = db.rooms().create("Example helper", at(0)).await.unwrap();
    let asked = r#"{"who":"Example person","at":"2027-01-15T08:00:00.000Z","how":"in a room"}"#;
    let theirs = tasks
        .create(
            NewTask {
                room: Some(room.id),
                requested_by: Some(asked.to_owned()),
                ..new("Draft the launch post")
            },
            Who::Assistant,
            at(1),
        )
        .await
        .unwrap();
    let other = tasks
        .create(new("Something of the owner's"), Who::Assistant, at(2))
        .await
        .unwrap();
    assert_eq!(tasks.room_of(theirs.id).await.unwrap(), Some(room.id));
    assert_eq!(tasks.room_of(other.id).await.unwrap(), None);
    let stored = tasks.get(theirs.id).await.unwrap().unwrap();
    assert_eq!(stored.requested_by.as_deref(), Some(asked));
    assert_eq!(stored.requested, None);
    let mine: Vec<TaskId> = tasks
        .in_room(room.id, 10)
        .await
        .unwrap()
        .iter()
        .map(|task| task.id)
        .collect();
    assert_eq!(mine, [theirs.id]);
    assert_eq!(tasks.in_room(room.id, 0).await.unwrap(), Vec::new());
}
