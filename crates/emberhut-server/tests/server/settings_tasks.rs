//! `POST /api/settings`'s task and plan actions: `pinFocus`, `dropFocus`, `parkFocus`,
//! `todoDone`, `todoPark`, `taskArchive`, `taskUnarchive`, `taskDecide`, `xPosted`,
//! `taskCreate`, `projectSet` and `taskSet`.

use emberhut_api::paths;
use emberhut_api::settings::SettingsReply;
use emberhut_core::task::ask::OptionIn;
use emberhut_core::task::{Owner, Status, TaskChange, TaskId, Who};
use emberhut_db::NewTask;
use emberhut_server::services::plan::FocusChange;
use emberhut_server::services::tasks::AskInput;
use serde_json::{Value, json};

use crate::common::{Call, TestApp};

fn new(title: &str) -> NewTask {
    NewTask {
        title: title.to_owned(),
        ..NewTask::default()
    }
}

async fn settings(app: &TestApp, body: Value) -> crate::common::Answer {
    let session = app.sign_in().await;
    app.call(Call::post(paths::SETTINGS).token(&session).json(&body))
        .await
}

#[tokio::test]
async fn the_person_pins_parks_and_drops_focus_items() {
    let app = TestApp::new().await;
    let plan = app.state.plan();
    let item = plan
        .set_focus(FocusChange {
            title: Some("The launch".to_owned()),
            line: Some("On its way".to_owned()),
            ..FocusChange::default()
        })
        .await
        .unwrap();
    let reply: SettingsReply = settings(&app, json!({ "pinFocus": item.id }))
        .await
        .read_as();
    assert!(reply.state.focus[0].pinned);
    assert!(
        plan.drop_focus(item.id, false).await.is_err(),
        "the assistant doesn't drop a pinned item on its own"
    );
    let reply: SettingsReply =
        settings(&app, json!({ "parkFocus": item.id, "until": "2026-12-01" }))
            .await
            .read_as();
    assert_eq!(
        reply.state.focus[0].parked_until.as_deref(),
        Some("2026-12-01")
    );
    assert_eq!(
        reply.state.focus[0].parked_why.as_deref(),
        Some("The person parked it")
    );
    let reply: SettingsReply = settings(&app, json!({ "parkFocus": item.id, "until": null }))
        .await
        .read_as();
    assert_eq!(reply.state.focus[0].parked_until, None, "brought back");
    let reply: SettingsReply = settings(&app, json!({ "pinFocus": item.id, "pinned": false }))
        .await
        .read_as();
    assert!(!reply.state.focus[0].pinned);
    let reply: SettingsReply = settings(&app, json!({ "dropFocus": item.id }))
        .await
        .read_as();
    assert_eq!(reply.state.focus, [] as [emberhut_api::plan::FocusItem; 0]);
    let refused = settings(&app, json!({ "dropFocus": 99 })).await;
    assert_eq!(refused.status, 400);
    assert_eq!(refused.json()["error"], "no focus item 99");
}

#[tokio::test]
async fn the_person_ticks_and_parks_steps() {
    let app = TestApp::new().await;
    let t = app
        .state
        .tasks()
        .create(new("A step"), Who::Person)
        .await
        .unwrap();
    let reply: SettingsReply = settings(&app, json!({ "todoDone": t.id.get() }))
        .await
        .read_as();
    assert!(reply.state.todos[0].done.is_some());
    let reply: SettingsReply = settings(&app, json!({ "todoDone": t.id.get(), "done": false }))
        .await
        .read_as();
    assert!(reply.state.todos[0].done.is_none(), "the tick taken back");
    settings(
        &app,
        json!({ "todoPark": t.id.get(), "until": "2999-01-01" }),
    )
    .await;
    let parked = app.state.tasks().issue(t.id).await.unwrap().unwrap();
    assert_eq!(parked.parked_until.as_deref(), Some("2999-01-01"));
    let reply: SettingsReply = settings(&app, json!({ "todoPark": t.id.get(), "until": null }))
        .await
        .read_as();
    assert_eq!(reply.state.todos.len(), 1, "back on the list");
    assert_eq!(settings(&app, json!({ "todoDone": 99 })).await.status, 400);
}

#[tokio::test]
async fn archive_puts_a_task_away_with_its_open_subtasks_and_unarchive_brings_them_back() {
    let app = TestApp::new().await;
    let tasks = app.state.tasks();
    let parent = tasks.create(new("Parent"), Who::Person).await.unwrap();
    let open = tasks
        .create(
            NewTask {
                parent: Some(parent.id),
                ..new("Open kid")
            },
            Who::Person,
        )
        .await
        .unwrap();
    let done = tasks
        .create(
            NewTask {
                parent: Some(parent.id),
                ..new("Done kid")
            },
            Who::Person,
        )
        .await
        .unwrap();
    tasks
        .update(done.id, TaskChange::status(Status::Done), Who::Person, None)
        .await
        .unwrap();
    tasks
        .worker_started(open.id, "worker-kid", "The brief")
        .await
        .unwrap();
    settings(&app, json!({ "taskArchive": parent.id.get() })).await;
    assert_eq!(
        *app.task_workers.stopped.lock().unwrap(),
        vec![open.id.get()],
        "the archived subtask's worker stops"
    );
    let kid = tasks.detail(open.id).await.unwrap().unwrap();
    assert_eq!(
        kid.runs.as_ref().unwrap()[0].outcome.as_deref(),
        Some("completed")
    );
    assert!(
        kid.runs.as_ref().unwrap()[0].ended.is_some(),
        "its run is closed"
    );
    let status = |id: TaskId| {
        let tasks = app.state.tasks().clone();
        async move {
            tasks
                .issue(id)
                .await
                .unwrap()
                .unwrap()
                .status
                .as_str()
                .to_owned()
        }
    };
    assert_eq!(status(parent.id).await, "canceled");
    assert_eq!(status(open.id).await, "canceled");
    assert_eq!(status(done.id).await, "done", "a done subtask stays done");
    let detail = tasks.detail(parent.id).await.unwrap().unwrap();
    assert_eq!(detail.history.last().unwrap().text, "Archived by the owner");
    settings(&app, json!({ "taskUnarchive": parent.id.get() })).await;
    assert_eq!(status(parent.id).await, "todo");
    assert_eq!(
        status(open.id).await,
        "todo",
        "the subtask its archive took comes back"
    );
    tasks
        .update(done.id, TaskChange::status(Status::Done), Who::Person, None)
        .await
        .unwrap();
    let finished = tasks.create(new("Finished"), Who::Person).await.unwrap();
    tasks
        .update(
            finished.id,
            TaskChange::status(Status::Done),
            Who::Person,
            None,
        )
        .await
        .unwrap();
    let refused = settings(&app, json!({ "taskArchive": finished.id.get() })).await;
    assert_eq!(refused.status, 400);
    assert_eq!(
        refused.json()["error"],
        format!("{} is done already", finished.key())
    );
}

#[tokio::test]
async fn the_persons_answer_carries_the_task_on_or_puts_it_away() {
    let app = TestApp::new().await;
    let tasks = app.state.tasks();
    let mine = tasks.create(new("Pick a name"), Who::Person).await.unwrap();
    let ask = |options: &[&str]| AskInput {
        question: "Which name?".to_owned(),
        options: options
            .iter()
            .map(|o| OptionIn::Plain((*o).to_owned()))
            .collect(),
        action: Some("Pick a name".to_owned()),
        ..AskInput::default()
    };
    tasks
        .ask(mine.id, ask(&["Report A", "Report B"]), Who::Assistant)
        .await
        .unwrap();
    let reply: SettingsReply = settings(
        &app,
        json!({ "taskDecide": mine.id.get(), "choice": "Report A" }),
    )
    .await
    .read_as();
    let after = reply
        .state
        .issues
        .iter()
        .find(|t| t.id.get() == mine.id.get())
        .unwrap();
    assert_eq!(
        after.status.as_str(),
        "todo",
        "the person's own task goes back to their list"
    );
    assert!(after.decision.is_none() && after.action.is_none());
    let worked = tasks
        .create(
            NewTask {
                owner: Some(Owner::Assistant),
                body: Some("Brief".to_owned()),
                ..new("Worked")
            },
            Who::Assistant,
        )
        .await
        .unwrap();
    tasks
        .ask(worked.id, ask(&["Go", "Leave it"]), Who::Worker)
        .await
        .unwrap();
    settings(
        &app,
        json!({ "taskDecide": worked.id.get(), "choice": "Leave it" }),
    )
    .await;
    assert_eq!(
        tasks
            .issue(worked.id)
            .await
            .unwrap()
            .unwrap()
            .status
            .as_str(),
        "canceled"
    );
    settings(&app, json!({ "taskDecide": worked.id.get() })).await;
    assert_eq!(
        settings(&app, json!({ "taskDecide": 99, "choice": "Go" }))
            .await
            .status,
        400
    );
}

/// The assistant's task with a brief: the person's answer waits for its worker's next run.
#[tokio::test]
async fn the_persons_answer_is_queued_for_the_assistants_worker() {
    let app = TestApp::new().await;
    let tasks = app.state.tasks();
    let ask = |options: &[&str]| AskInput {
        question: "Which name?".to_owned(),
        options: options
            .iter()
            .map(|o| OptionIn::Plain((*o).to_owned()))
            .collect(),
        action: Some("Pick a name".to_owned()),
        ..AskInput::default()
    };
    let briefed = tasks
        .create(
            NewTask {
                owner: Some(Owner::Assistant),
                body: Some("Brief".to_owned()),
                ..new("Briefed")
            },
            Who::Assistant,
        )
        .await
        .unwrap();
    tasks
        .ask(briefed.id, ask(&["Report A", "Report B"]), Who::Worker)
        .await
        .unwrap();
    settings(
        &app,
        json!({ "taskDecide": briefed.id.get(), "choice": "Report B" }),
    )
    .await;
    let after = tasks.issue(briefed.id).await.unwrap().unwrap();
    assert_eq!(
        after.status.as_str(),
        "todo",
        "back to to do, for its worker to carry on"
    );
    assert!(after.decision.is_none() && after.action.is_none() && after.decide_by.is_none());
    let db = emberhut_db::Database::connect(&app.url).await.unwrap();
    let queued = db.tasks().get(briefed.id).await.unwrap().unwrap();
    assert_eq!(
        queued.next.as_deref(),
        Some("The person answered \"Which name?\": Report B."),
        "its worker carries on with the answer"
    );
    let detail = tasks.detail(briefed.id).await.unwrap().unwrap();
    assert_eq!(
        detail.history.last().unwrap().text,
        "Chose: Report B",
        "the person's answer is on its history"
    );
}

#[tokio::test]
async fn a_posted_x_reply_is_kept_and_closes_its_task() {
    let app = TestApp::new().await;
    let tasks = app.state.tasks();
    let t = tasks
        .create(new("Reply to the post"), Who::Assistant)
        .await
        .unwrap();
    let kept = tasks
        .attach_replies(
            t.id,
            &[json!({ "post_url": "https://x.com/someone/status/42", "author": "someone", "replies": ["Nice one", "Agreed"] })],
        )
        .await
        .unwrap();
    assert_eq!(kept, 1);
    let artifact = tasks.issue(t.id).await.unwrap().unwrap().artifacts[0].clone();
    assert_eq!(
        (artifact.kind.as_str(), artifact.name.as_str()),
        ("x_reply", "@someone")
    );
    let reply: SettingsReply = settings(&app, json!({ "xPosted": artifact.id.get(), "option": 1 }))
        .await
        .read_as();
    let done = reply
        .state
        .issues
        .iter()
        .find(|i| i.id.get() == t.id.get())
        .unwrap();
    assert_eq!(done.status.as_str(), "done");
    let x = done.artifacts[0].x.as_ref().unwrap();
    assert_eq!(x.posted.as_ref().unwrap().text, "Agreed");
    let detail = tasks.detail(t.id).await.unwrap().unwrap();
    assert!(
        detail.history.iter().any(|h| h
            .text
            .starts_with("Posted reply 2 to @someone on X: \"Agreed\"")),
        "{:?}",
        detail.history
    );
}

#[tokio::test]
async fn quick_add_and_the_persons_project_and_task_fields() {
    let app = TestApp::new().await;
    let reply: SettingsReply = settings(
        &app,
        json!({ "taskCreate": "  Buy the tickets ", "goal": 1 }),
    )
    .await
    .read_as();
    let added = &reply.state.issues[0];
    assert_eq!(
        (added.title.as_str(), added.owner.as_str()),
        ("Buy the tickets", "owner")
    );
    let id = added.id.get();
    settings(&app, json!({ "taskCreate": "   " })).await;
    let pid = app
        .state
        .tasks()
        .set_project(
            None,
            emberhut_core::task::board::ProjectChange {
                name: Some("Trip".to_owned()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let reply: SettingsReply = settings(
        &app,
        json!({ "projectSet": { "id": pid.get(), "priority": 2, "deadline": "2026-12-20", "hard": true } }),
    )
    .await
    .read_as();
    let project = &reply.state.projects[0];
    assert_eq!((project.priority, project.hard), (2, true));
    assert_eq!(project.deadline.as_ref().unwrap().as_str(), "2026-12-20");
    let reply: SettingsReply = settings(
        &app,
        json!({ "projectSet": { "id": pid.get(), "deadline": null, "status": "paused", "pausedUntil": "2026-12-01" } }),
    )
    .await
    .read_as();
    let project = &reply.state.projects[0];
    assert!(project.deadline.is_none(), "null clears it");
    assert_eq!(
        project.paused_until.as_ref().unwrap().as_str(),
        "2026-12-01"
    );
    let reply: SettingsReply = settings(
        &app,
        json!({ "taskSet": { "id": id, "deadline": "2026-12-10T10:00", "size": "M", "project": pid.get(), "body": " Two seats ", "decideBy": "2026-12-05" } }),
    )
    .await
    .read_as();
    let task = reply
        .state
        .issues
        .iter()
        .find(|t| t.id.get() == id)
        .unwrap();
    assert_eq!(task.due.as_ref().unwrap().as_str(), "2026-12-10");
    assert_eq!(
        (
            task.size
                .as_ref()
                .map(emberhut_api::tasks::TaskSize::as_str),
            task.body.as_deref()
        ),
        (Some("M"), Some("Two seats"))
    );
    assert_eq!(task.pid.unwrap().get(), pid.get());
    assert_eq!(task.project.as_deref(), Some("Trip"));
    assert!(
        task.decide_by.is_none(),
        "a decide-by day is an ask's: it doesn't wait"
    );
    let detail = app
        .state
        .tasks()
        .detail(TaskId::new(id))
        .await
        .unwrap()
        .unwrap();
    assert!(
        detail
            .history
            .iter()
            .any(|h| h.text == "Changed the description")
    );
    let refused = settings(&app, json!({ "taskSet": { "id": id, "deadline": "soon" } })).await;
    assert_eq!(refused.status, 400);
    assert_eq!(
        refused.json()["error"],
        "deadline must be a date like 2026-10-15, got \"soon\""
    );
}
