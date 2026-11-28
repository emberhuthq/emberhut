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
