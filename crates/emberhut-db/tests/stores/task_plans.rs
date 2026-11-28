//! Tasks' plans against a real SQLite database: the `plan_steps` rows and the plan's columns of
//! `issues`, each change with its one history line.

use emberhut_core::task::plan::{AddStep, Edit, NewStep, PlanRefusal, StepRef, StepState};
use emberhut_core::task::{TaskId, TaskRefusal, Who};
use emberhut_db::{NewTask, TaskStoreError};

use crate::common::{at, database, reopen};

fn step(title: &str, state: Option<StepState>) -> NewStep {
    NewStep {
        title: title.to_owned(),
        state,
        ..NewStep::default()
    }
}

#[tokio::test]
async fn a_plan_is_written_kept_and_read_back_in_order() {
    let (dir, db) = database().await;
    let tasks = db.tasks();
    let task = tasks
        .create(
            NewTask {
                title: "Ship the app".to_owned(),
                ..NewTask::default()
            },
            Who::Person,
            at(0),
        )
        .await
        .unwrap();
    let empty = tasks.plan(task.id).await.unwrap().unwrap();
    assert!(
        empty.steps.is_empty(),
        "an existing task starts with no plan"
    );
    assert_eq!((empty.updated, empty.unplanned_turn), (None, None));

    let written = tasks
        .set_plan(
            task.id,
            &[
                NewStep {
                    steps: vec![step("Compile", Some(StepState::Done))],
                    ..step("Build the Mac app", Some(StepState::Doing))
                },
                step("Sign the build", None),
            ],
            Some(Some("about 2 hours".to_owned())),
            Who::Worker,
            at(1),
        )
        .await
        .unwrap();
    assert_eq!(written.steps.len(), 3);
    assert_eq!(written.eta.as_deref(), Some("about 2 hours"));
    assert_eq!(written.updated, Some(at(1)));
    let history = tasks.history(task.id).await.unwrap();
    let last = history.last().unwrap();
    assert_eq!(
        (last.who, last.text.as_str()),
        (Who::Worker, "Plan set: 2 steps, 1 done")
    );

    let edited = tasks
        .edit_plan(
            task.id,
            &Edit {
                step: Some(StepRef::Title("sign the build".to_owned())),
                state: Some(StepState::Blocked),
                note: Some("needs the certificate".to_owned()),
                ..Edit::default()
            },
            None,
            Who::Worker,
            at(2),
        )
        .await
        .unwrap();
    assert_eq!(edited.steps[2].state, StepState::Blocked);
    let db = {
        db.close().await.unwrap();
        reopen(&dir).await
    };
    let tasks = db.tasks();
    let kept = tasks.plan(task.id).await.unwrap().unwrap();
    assert_eq!(kept.steps, edited.steps, "read back as written");
    assert_eq!(kept.last_id, 3);
    assert_eq!(
        tasks.get(task.id).await.unwrap().unwrap().last.as_deref(),
        Some("Blocked: Sign the build — needs the certificate")
    );

    let added = tasks
        .edit_plan(
            task.id,
            &Edit {
                add: Some(AddStep {
                    title: "Strip the symbols".to_owned(),
                    parent: Some(StepRef::Id(1)),
                    ..AddStep::default()
                }),
                ..Edit::default()
            },
            Some(None),
            Who::Assistant,
            at(3),
        )
        .await
        .unwrap();
    let order: Vec<u32> = added.steps.iter().map(|s| s.id).collect();
    assert_eq!(order, [1, 2, 4, 3]);
    assert_eq!(added.eta, None, "the estimate cleared");
    assert_eq!(
        tasks.history(task.id).await.unwrap().last().unwrap().text,
        "Added step: Strip the symbols; Plan estimate cleared"
    );
}

#[tokio::test]
async fn plans_for_many_tasks_come_at_once_and_refusals_change_nothing() {
    let (_dir, db) = database().await;
    let tasks = db.tasks();
    let mut ids = Vec::new();
    for n in 0..3 {
        let made = tasks
            .create(
                NewTask {
                    title: format!("Task {n}"),
                    ..NewTask::default()
                },
                Who::Person,
                at(0),
            )
            .await
            .unwrap();
        ids.push(made.id);
    }
    tasks
        .set_plan(ids[1], &[step("One", None)], None, Who::Assistant, at(1))
        .await
        .unwrap();
    let plans = tasks
        .plans(&[ids[0], ids[1], TaskId::new(999)])
        .await
        .unwrap();
    assert_eq!(plans.len(), 2, "no such task, no plan");
    assert_eq!(plans[&ids[0]].steps.len(), 0, "no plan yet");
    assert_eq!(plans[&ids[1]].steps[0].title, "One");

    let refused = tasks
        .edit_plan(
            ids[1],
            &Edit {
                step: Some(StepRef::Id(7)),
                state: Some(StepState::Done),
                ..Edit::default()
            },
            None,
            Who::Assistant,
            at(2),
        )
        .await;
    assert!(matches!(
        refused,
        Err(TaskStoreError::Plan(PlanRefusal::NoStep(_)))
    ));
    assert!(matches!(
        tasks
            .set_plan(TaskId::new(999), &[], None, Who::Assistant, at(2))
            .await,
        Err(TaskStoreError::Refused(TaskRefusal::NoTask(_)))
    ));
    assert_eq!(
        tasks.history(ids[1]).await.unwrap().last().unwrap().text,
        "Plan set: 1 step"
    );
}

#[tokio::test]
async fn a_turn_with_no_plan_is_noted_once_until_a_plan_is_written() {
    let (_dir, db) = database().await;
    let tasks = db.tasks();
    let task = tasks
        .create(
            NewTask {
                title: "Look into it".to_owned(),
                ..NewTask::default()
            },
            Who::Person,
            at(0),
        )
        .await
        .unwrap();
    assert!(tasks.turn_unplanned(task.id, at(1)).await.unwrap());
    assert!(
        !tasks.turn_unplanned(task.id, at(2)).await.unwrap(),
        "a turn in a row: no second line"
    );
    let plan = tasks.plan(task.id).await.unwrap().unwrap();
    assert_eq!(plan.unplanned_turn, Some(at(2)), "the last such turn");
    let lines = tasks.history(task.id).await.unwrap();
    assert_eq!(
        lines
            .iter()
            .filter(|line| line.text == "A turn ended with no plan")
            .count(),
        1
    );
    let written = tasks
        .set_plan(
            task.id,
            &[step("Read the logs", None)],
            None,
            Who::Assistant,
            at(3),
        )
        .await
        .unwrap();
    assert_eq!(written.unplanned_turn, None, "writing a plan clears it");
    assert!(
        !tasks.turn_unplanned(task.id, at(4)).await.unwrap(),
        "it has a plan"
    );
    assert!(!tasks.turn_unplanned(TaskId::new(999), at(4)).await.unwrap());
}

#[tokio::test]
async fn a_sessions_turn_taken_twice_at_once_opens_one_run() {
    use emberhut_core::task::live::{NewRun, RunKind};

    let (_dir, db) = database().await;
    let task = db
        .tasks()
        .create(
            NewTask {
                title: "The lexer".to_owned(),
                ..NewTask::default()
            },
            Who::Person,
            at(0),
        )
        .await
        .unwrap();
    let runs = db.runs();
    let new = |started: bool| NewRun {
        task: task.id,
        machine: "build".to_owned(),
        thread: "r-1e8".to_owned(),
        kind: RunKind::Thread,
        asked: None,
        started,
    };
    let (queued, running) = tokio::join!(
        runs.open_thread_once(new(false), at(1)),
        runs.open_thread_once(new(true), at(1)),
    );
    let ((first, a), (second, b)) = (queued.unwrap(), running.unwrap());
    assert_eq!(first.id, second.id, "one run for the turn");
    assert!(a ^ b, "opened by one of the two");
    assert!(
        runs.close(first.id, "completed", None, at(2))
            .await
            .unwrap()
    );
    assert!(
        runs.open_thread("build", "r-1e8").await.unwrap().is_none(),
        "nothing left open"
    );
    let (next, opened) = runs.open_thread_once(new(true), at(3)).await.unwrap();
    assert!(
        opened && next.id != first.id,
        "the next turn is a run of its own"
    );
}

#[tokio::test]
async fn the_open_thread_runs_are_listed_the_oldest_first() {
    use emberhut_core::task::live::{NewRun, RunKind};

    let (_dir, db) = database().await;
    let task = db
        .tasks()
        .create(
            NewTask {
                title: "The lexer".to_owned(),
                ..NewTask::default()
            },
            Who::Person,
            at(0),
        )
        .await
        .unwrap();
    let runs = db.runs();
    let new = |thread: &str, kind: RunKind| NewRun {
        task: task.id,
        machine: "build".to_owned(),
        thread: thread.to_owned(),
        kind,
        asked: None,
        started: true,
    };
    let first = runs.open(new("r-1", RunKind::Thread), at(1)).await.unwrap();
    let closed = runs.open(new("r-2", RunKind::Thread), at(2)).await.unwrap();
    runs.open(new("unit-3", RunKind::Worker), at(3))
        .await
        .unwrap();
    let last = runs.open(new("r-4", RunKind::Thread), at(4)).await.unwrap();
    assert!(
        runs.close(closed.id, "completed", None, at(5))
            .await
            .unwrap()
    );
    let open: Vec<_> = runs
        .open_threads()
        .await
        .unwrap()
        .into_iter()
        .map(|run| run.id)
        .collect();
    assert_eq!(open, [first.id, last.id], "open, of threads only");
}

#[tokio::test]
async fn a_task_made_for_a_session_says_so_and_a_moved_sessions_open_run_follows_it() {
    use emberhut_core::task::live::{NewRun, RunKind};

    let (dir, db) = database().await;
    let tasks = db.tasks();
    let new = |title: &str| NewTask {
        title: title.to_owned(),
        ..NewTask::default()
    };
    let made = tasks
        .create(new("Tidy up"), Who::Person, at(0))
        .await
        .unwrap();
    let other = tasks
        .create(new("The lexer"), Who::Person, at(0))
        .await
        .unwrap();
    assert_eq!(tasks.made_for_session(made.id).await.unwrap(), None);
    assert!(tasks.set_made_for_session(made.id, "r-1e8").await.unwrap());
    assert!(
        !tasks
            .set_made_for_session(TaskId::new(999), "r-1e8")
            .await
            .unwrap()
    );
    drop(db);
    let db = reopen(&dir).await;
    let tasks = db.tasks();
    assert_eq!(
        tasks.made_for_session(made.id).await.unwrap().as_deref(),
        Some("r-1e8"),
        "kept"
    );
    assert_eq!(tasks.made_for_session(other.id).await.unwrap(), None);

    let runs = db.runs();
    let (run, _) = runs
        .open_thread_once(
            NewRun {
                task: made.id,
                machine: "build".to_owned(),
                thread: "r-1e8".to_owned(),
                kind: RunKind::Thread,
                asked: None,
                started: true,
            },
            at(1),
        )
        .await
        .unwrap();
    assert!(runs.move_to(run.id, other.id).await.unwrap());
    assert_eq!(
        runs.open_thread("build", "r-1e8")
            .await
            .unwrap()
            .unwrap()
            .task,
        Some(other.id)
    );
    assert_eq!(runs.of_task(made.id, 10).await.unwrap().len(), 0);
    assert!(runs.close(run.id, "completed", None, at(2)).await.unwrap());
    assert!(
        !runs.move_to(run.id, made.id).await.unwrap(),
        "an ended run stays where it ended"
    );
}
