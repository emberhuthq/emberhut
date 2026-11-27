//! The tasks' routes and what the state shows of them: `GET /api/tasks` and `/api/tasks/{id}`,
//! `POST /api/tasks` (a program's job) and `/api/tasks/{id}/message` (a reply on a task's
//! page), what a task produced (`/api/artifacts/{id}`, `/thumb`), the Scheduled page, and the
//! state's `issues`, `today`, `projects`, `goals`, `todos`, `weekStart`, `focus`, `areas`,
//! `report`, `loops` and `tasks`.
#![expect(
    clippy::too_many_lines,
    reason = "each test walks one story through the routes from start to end"
)]

use std::sync::Arc;

use emberhut_api::background::{BackgroundTask, WorkStatus};
use emberhut_api::ids::RoomId;
use emberhut_api::scheduled::{ScheduledItem, ScheduledPage, ScheduledTask};
use emberhut_api::state::State;
use emberhut_api::task_message::TaskMessageReply;
use emberhut_api::task_page::TaskPage;
use emberhut_api::task_search::FoundTasks;
use emberhut_api::tasks::NewTaskReply;
use emberhut_core::task::board::ProjectChange;
use emberhut_core::task::{Owner, ProjectId, Status, TaskChange, TaskId, Who};
use emberhut_db::NewTask;
use emberhut_server::config::AuthMode;
use emberhut_server::services::plan::{FocusChange, GoalChange, LoopChange};
use emberhut_server::services::scheduled::ScheduleSource;
use futures_util::future::BoxFuture;
use serde_json::{Value, json};

use crate::common::{Call, TestApp};

fn new(title: &str) -> NewTask {
    NewTask {
        title: title.to_owned(),
        ..NewTask::default()
    }
}

#[tokio::test]
pub(crate) async fn tasks_are_found_by_key_or_words_and_opened_with_their_history() {
    let app = TestApp::new().await;
    let session = app.sign_in().await;
    assert_eq!(app.call(Call::get("/api/tasks")).await.status, 401);
    let tasks = app.state.tasks();
    let report = tasks
        .create(new("Name the example report"), Who::Person)
        .await
        .unwrap();
    let draft = tasks
        .create(
            NewTask {
                owner: Some(Owner::Assistant),
                body: Some("Write the draft".to_owned()),
                ..new("Draft the summary")
            },
            Who::Assistant,
        )
        .await
        .unwrap();
    tasks
        .create(
            NewTask {
                parent: Some(draft.id),
                ..new("First part")
            },
            Who::Assistant,
        )
        .await
        .unwrap();
    tasks
        .note(draft.id, Who::Worker, "Wrote the first part", true)
        .await
        .unwrap();

    let page: FoundTasks = app
        .call(Call::get("/api/tasks").query("q=report").token(&session))
        .await
        .read_as();
    assert_eq!(page.tasks.len(), 1);
    assert_eq!(page.tasks[0].key, report.key());
    assert_eq!(page.tasks[0].owner, "owner", "the wire word for the person");
    let by_key: FoundTasks = app
        .call(
            Call::get("/api/tasks")
                .query(&format!("q={}", draft.key()))
                .token(&session),
        )
        .await
        .read_as();
    assert_eq!(by_key.tasks[0].id.get(), draft.id.get());
    let mine: FoundTasks = app
        .call(
            Call::get("/api/tasks")
                .query("owner=owner&status=todo,blocked&limit=5")
                .token(&session),
        )
        .await
        .read_as();
    assert_eq!(
        mine.tasks.len(),
        1,
        "an unknown status is left out of the filter"
    );

    let detail: TaskPage = app
        .call(Call::get(&format!("/api/tasks/{}", draft.id)).token(&session))
        .await
        .read_as();
    assert_eq!(detail.task.title, "Draft the summary");
    assert_eq!(detail.task.body.as_deref(), Some("Write the draft"));
    assert_eq!(detail.history.len(), 2);
    assert_eq!(detail.history[1].who, "worker");
    assert_eq!(detail.subtasks.len(), 1);
    assert_eq!(detail.task.sub.as_ref().unwrap().total, 1);
    assert_eq!(detail.task.live.as_ref().unwrap().state.as_str(), "todo");
    assert_eq!(
        app.call(Call::get("/api/tasks/999").token(&session))
            .await
            .status,
        404
    );
    assert_eq!(app.call(Call::get("/api/tasks/1")).await.status, 401);
}

#[tokio::test]
pub(crate) async fn a_program_hands_the_assistant_a_job_with_the_key() {
    let app = TestApp::with_dependencies(AuthMode::Enforce, |deps, _| {
        emberhut_server::app::Dependencies {
            program_key: Some("program-key-for-tests".to_owned()),
            ..deps
        }
    })
    .await;
    let key = "Bearer program-key-for-tests";
    let refused = app
        .call(Call::post("/api/tasks").json(&json!({ "title": "x", "body": "y" })))
        .await;
    assert_eq!(refused.status, 401);
    assert_eq!(refused.json()["error"], "programs use the key");
    let session = app.sign_in().await;
    let with_session = app
        .call(
            Call::post("/api/tasks")
                .token(&session)
                .json(&json!({ "title": "x", "body": "y" })),
        )
        .await;
    assert_eq!(
        with_session.status, 401,
        "a device's session is not the programs' key"
    );
    let empty = app
        .call(
            Call::post("/api/tasks")
                .header("authorization", key)
                .json(&json!({ "title": "x" })),
        )
        .await;
    assert_eq!(empty.status, 400);
    assert_eq!(
        empty.json()["error"],
        "a task needs a title and a body (the brief)"
    );
    let reply: NewTaskReply = app
        .call(
            Call::post("/api/tasks")
                .header("authorization", key)
                .json(&json!({ "title": " Explain chapter 3 ", "body": "The tutor asked", "from": "the study app" })),
        )
        .await
        .read_as();
    assert!(reply.ok);
    let task = app
        .state
        .tasks()
        .issue(TaskId::new(reply.id.get()))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        (task.title.as_str(), task.owner.as_str()),
        ("Explain chapter 3", "assistant")
    );
    assert!(
        task.last
            .unwrap()
            .starts_with("Requested by the person in the study app")
    );
}

#[tokio::test]
pub(crate) async fn a_reply_on_a_task_page_goes_to_its_worker_or_the_assistant() {
    let app = TestApp::new().await;
    let session = app.sign_in().await;
    let tasks = app.state.tasks();
    let worked = tasks
        .create(
            NewTask {
                owner: Some(Owner::Assistant),
                body: Some("Do it".to_owned()),
                ..new("Worked")
            },
            Who::Assistant,
        )
        .await
        .unwrap();
    tasks
        .update(
            worked.id,
            TaskChange {
                action: Some(Some("Pick one".to_owned())),
                ..TaskChange::default()
            },
            Who::Assistant,
            None,
        )
        .await
        .unwrap();
    let reply: TaskMessageReply = app
        .call(
            Call::post(&format!("/api/tasks/{}/message", worked.id))
                .token(&session)
                .json(&json!({ "text": "Go with the first", "files": [] })),
        )
        .await
        .read_as();
    assert_eq!((reply.to.as_str(), reply.queued), ("worker", Some(false)));
    let after = app.state.tasks().issue(worked.id).await.unwrap().unwrap();
    assert_eq!(
        after.status.as_str(),
        "todo",
        "its worker carries on with it"
    );
    assert!(after.action.is_none());

    let mine = tasks.create(new("Mine"), Who::Person).await.unwrap();
    let reply: TaskMessageReply = app
        .call(
            Call::post(&format!("/api/tasks/{}/message", mine.id))
                .token(&session)
                .json(&json!({ "text": "", "files": [{ "kind": "image", "path": "/data/uploads/a.png" }] })),
        )
        .await
        .read_as();
    assert_eq!(
        reply.to,
        emberhut_api::task_message::MessageTo::Assistant,
        "no worker: the assistant answers on the task"
    );
    let detail = app.state.tasks().detail(mine.id).await.unwrap().unwrap();
    assert_eq!(detail.history.last().unwrap().text, "(1 photo attached)");
    let empty = app
        .call(
            Call::post(&format!("/api/tasks/{}/message", mine.id))
                .token(&session)
                .json(&json!({ "text": "  " })),
        )
        .await;
    assert_eq!(empty.status, 400);
    assert_eq!(empty.json()["error"], "nothing to send");
    assert_eq!(
        app.call(Call::post("/api/tasks/1/message").json(&json!({ "text": "x" })))
            .await
            .status,
        401
    );
}

#[tokio::test]
pub(crate) async fn what_a_task_produced_is_served_by_its_id() {
    let app = TestApp::new().await;
    let session = app.sign_in().await;
    let dir = app.dir().join("home");
    std::fs::create_dir_all(dir.join("out")).unwrap();
    std::fs::write(dir.join("out/chart.png"), b"0123456789").unwrap();
    std::fs::write(dir.join("out/report.pdf"), b"%PDF").unwrap();
    let tasks = app.state.tasks();
    let t = tasks
        .create(new("Make the chart"), Who::Assistant)
        .await
        .unwrap();
    tasks
        .note(
            t.id,
            Who::Worker,
            &format!(
                "Made {}/out/chart.png and https://example.com/pr/3",
                dir.display()
            ),
            true,
        )
        .await
        .unwrap();
    let off = tasks
        .attach(
            t.id,
            &[
                format!("{}/out/report.pdf", dir.display()),
                "/etc/x.png".to_owned(),
            ],
        )
        .await
        .unwrap();
    assert!(off.contains("/etc/x.png"), "{off}");
    let issue = tasks.issue(t.id).await.unwrap().unwrap();
    assert_eq!(issue.artifacts.len(), 3);
    let chart = issue.artifacts.iter().find(|a| a.kind == "image").unwrap();
    let pdf = issue.artifacts.iter().find(|a| a.kind == "pdf").unwrap();
    let link = issue.artifacts.iter().find(|a| a.kind == "link").unwrap();
    assert_eq!(link.url, "https://example.com/pr/3");
    assert_eq!(
        chart.thumb.as_deref(),
        Some(format!("/api/artifacts/{}/thumb", chart.id).as_str())
    );

    let whole = app.call(Call::get(&chart.url).token(&session)).await;
    assert_eq!(whole.status, 200);
    assert_eq!(whole.header("content-type"), "image/png");
    assert_eq!(whole.text, "0123456789");
    let part = app
        .call(
            Call::get(&chart.url)
                .token(&session)
                .header("range", "bytes=2-4"),
        )
        .await;
    assert_eq!(part.status, 206);
    assert_eq!(part.text, "234");
    let beyond = app
        .call(
            Call::get(&chart.url)
                .token(&session)
                .header("range", "bytes=50-"),
        )
        .await;
    assert_eq!(beyond.status, 416);
    let link_id = format!("/api/artifacts/{}", link.id);
    assert_eq!(
        app.call(Call::get(&link_id).token(&session)).await.status,
        404,
        "a link is not a file"
    );
    assert_eq!(
        app.call(Call::get("/api/artifacts/999").token(&session))
            .await
            .status,
        404
    );
    assert_eq!(app.call(Call::get(&chart.url)).await.status, 401);

    let thumb = app
        .call(Call::get(chart.thumb.as_deref().unwrap()).token(&session))
        .await;
    assert_eq!(thumb.status, 200);
    assert_eq!(thumb.header("content-type"), "image/jpeg");
    assert_eq!(
        app.call(Call::get(&format!("/api/artifacts/{}/thumb", pdf.id)).token(&session))
            .await
            .status,
        404,
        "a PDF has no thumbnail"
    );
    assert_eq!(
        app.call(Call::get(chart.thumb.as_deref().unwrap()))
            .await
            .status,
        401
    );

    std::fs::remove_file(dir.join("out/chart.png")).unwrap();
    let gone = tasks.issue(t.id).await.unwrap().unwrap();
    assert_eq!(
        gone.artifacts.len(),
        2,
        "a file that's gone leaves the list"
    );
}

/// A source of two scheduled items, the later one first.
#[derive(Debug)]
struct Watches;

impl ScheduleSource for Watches {
    fn items(&self) -> BoxFuture<'_, Vec<ScheduledItem>> {
        let item = |id: &str, at: &str, task: i64| ScheduledItem {
            id: id.to_owned(),
            kind: "watch".into(),
            title: format!("Watch {id}"),
            next_at: Some(at.into()),
            repeat: Some(emberhut_core::schedule::every(30)),
            room: RoomId::MAIN,
            note: None,
            note_at: None,
            tasks: vec![ScheduledTask {
                id: emberhut_api::ids::TaskId::new(task),
                title: String::new(),
                status: "".into(),
            }],
            watch: Some(emberhut_api::ids::WatchId::new(1)),
            checkin: None,
        };
        Box::pin(async move {
            vec![
                item("watch:2", "2026-11-26T10:00:00.000Z", 1),
                item("watch:1", "2026-11-26T09:00:00.000Z", 99),
            ]
        })
    }
}

#[tokio::test]
pub(crate) async fn the_scheduled_page_lists_every_source_soonest_first() {
    let app = TestApp::with_dependencies(AuthMode::Enforce, |deps, _| {
        emberhut_server::app::Dependencies {
            schedule: vec![Arc::new(Watches)],
            ..deps
        }
    })
    .await;
    let session = app.sign_in().await;
    app.state
        .tasks()
        .create(new("Kept an eye on"), Who::Assistant)
        .await
        .unwrap();
    let page: ScheduledPage = app
        .call(Call::get("/api/scheduled").token(&session))
        .await
        .read_as();
    // The routines (the morning brief and the evening wrap) are the server's own items.
    let items: Vec<_> = page
        .items
        .iter()
        .filter(|i| i.kind.as_str() != "routine")
        .collect();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].id, "watch:1", "soonest first");
    assert_eq!(items[0].repeat.as_deref(), Some("every 30 min"));
    assert!(
        items[0].tasks.is_empty(),
        "a task that isn't open is left out"
    );
    assert_eq!(items[1].tasks[0].title, "Kept an eye on");
    assert_eq!(app.call(Call::get("/api/scheduled")).await.status, 401);
}

#[tokio::test]
async fn the_state_shows_the_board_the_plan_and_the_report() {
    let app = TestApp::new().await;
    let session = app.sign_in().await;
    let state = app.state.clone();
    let plan = state.plan();
    let goal = plan
        .set_goal(GoalChange {
            title: Some("Ship the example".to_owned()),
            why: Some("Made up".to_owned()),
            short: Some(Some("Ship".to_owned())),
            ..GoalChange::default()
        })
        .await
        .unwrap();
    let tasks = state.tasks();
    let pid: ProjectId = tasks
        .set_project(
            None,
            ProjectChange {
                name: Some("Example".to_owned()),
                goal: Some(Some(emberhut_core::task::GoalId::new(goal.id.get()))),
                priority: Some(1),
                ..ProjectChange::default()
            },
        )
        .await
        .unwrap();
    let open = tasks
        .create(
            NewTask {
                pid: Some(pid),
                owner: Some(Owner::Person),
                ..new("Open one")
            },
            Who::Person,
        )
        .await
        .unwrap();
    let done = tasks
        .create(
            NewTask {
                pid: Some(pid),
                owner: Some(Owner::Assistant),
                ..new("Done one")
            },
            Who::Assistant,
        )
        .await
        .unwrap();
    tasks
        .update(
            done.id,
            TaskChange::status(Status::Done),
            Who::Assistant,
            None,
        )
        .await
        .unwrap();
    plan.set_focus(FocusChange {
        title: Some("The launch".to_owned()),
        line: Some("Waiting for the review".to_owned()),
        happened: Some("Sent for review".to_owned()),
        ..FocusChange::default()
    })
    .await
    .unwrap();
    plan.set_loop(LoopChange {
        who: "A colleague".to_owned(),
        what: "Waits for the numbers".to_owned(),
        ..LoopChange::default()
    })
    .await
    .unwrap();
    plan.set_concern("Freedom".to_owned(), "All the goals serve it".to_owned())
        .await
        .unwrap();
    plan.jobs_running(
        RoomId::MAIN,
        vec![BackgroundTask {
            id: "job-1".to_owned(),
            description: "Reading the logs".to_owned(),
            kind: "agent".to_owned(),
            status: WorkStatus::Running,
            started_at: Some("2026-11-26T09:00:00.000Z".into()),
            ended_at: None,
            summary: None,
            room: None,
        }],
    )
    .await
    .unwrap();

    let state: State = app
        .call(Call::get("/api/state").token(&session))
        .await
        .read_as();
    assert_eq!(state.issues.len(), 2);
    assert_eq!(
        state.today,
        [emberhut_api::ids::TaskId::new(open.id.get())],
        "the person's best-scored task"
    );
    let project = &state.projects[0];
    assert_eq!(
        (project.name.as_str(), project.open, project.done_week),
        ("Example", 1, 1)
    );
    assert_eq!(
        state.goals[0].week.by(emberhut_core::task::wire::ASSISTANT),
        1,
        "the done step counts for its project's goal"
    );
    assert_eq!(state.todos.len(), 2);
    assert!(
        state.todos[1].done.is_some(),
        "open ones first, then the done ones"
    );
    assert!(state.week_start.is_some());
    assert_eq!(state.focus[0].timeline[0].text, "Sent for review");
    assert_eq!(state.loops[0].who, "A colleague");
    assert_eq!(state.tasks[0].description, "Reading the logs");
    let report = state.report.unwrap();
    assert_eq!(report.concern.unwrap().title, "Freedom");
    assert_eq!(report.weeks.len(), 8);
    assert_eq!(*report.weeks.last().unwrap(), 1);
    assert_eq!(report.goals[0].done, 1);
    assert_eq!(report.done[0].project.as_deref(), Some("Example"));
    assert_eq!(state.areas, [] as [emberhut_api::areas::Area; 0]);
    let raw: Value = app
        .call(Call::get("/api/state").token(&session))
        .await
        .json();
    assert_eq!(
        raw["goals"][0]["week"],
        json!({ "owner": 0, "assistant": 1 })
    );
}

#[tokio::test]
async fn the_tick_rechecks_stale_asks_and_brings_paused_projects_back_and_runs_are_kept() {
    let app = TestApp::new().await;
    let tasks = app.state.tasks();
    let t = tasks
        .create(
            NewTask {
                owner: Some(Owner::Assistant),
                body: Some("Brief".to_owned()),
                ..new("Asks")
            },
            Who::Assistant,
        )
        .await
        .unwrap();
    tasks
        .ask(
            t.id,
            emberhut_server::services::tasks::AskInput {
                question: "Go?".to_owned(),
                options: vec![emberhut_core::task::ask::OptionIn::Plain("Go".to_owned())],
                ..Default::default()
            },
            Who::Worker,
        )
        .await
        .unwrap();
    let yesterday = (app.clock_now() - jiff::SignedDuration::from_hours(30))
        .to_zoned_datetime(tasks.zone().clone())
        .date();
    let pid = tasks
        .set_project(
            None,
            ProjectChange {
                name: Some("Paused".to_owned()),
                status: Some(emberhut_core::task::board::ProjectStatus::Paused),
                paused_until: Some(Some(yesterday)),
                ..ProjectChange::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(
        tasks.tick().await.unwrap(),
        (1, 0),
        "the project is back; the ask is fresh"
    );
    app.advance_minutes(13 * 60);
    assert_eq!(tasks.tick().await.unwrap(), (0, 1));
    let db = emberhut_db::Database::connect(&app.url).await.unwrap();
    let events = db
        .events()
        .waiting(emberhut_core::RoomId::MAIN, 100)
        .await
        .unwrap();
    assert!(
        events
            .iter()
            .any(|e| e.kind == "recheck" && e.text.contains(&t.key())),
        "{events:?}"
    );
    assert!(
        events
            .iter()
            .any(|e| e.text.contains("\"Paused\" is back from its pause"))
    );
    assert_eq!(tasks.tick().await.unwrap(), (0, 0), "once per check");
    assert_eq!(
        tasks.project_name(pid).await.unwrap().as_deref(),
        Some("Paused")
    );

    tasks
        .worker_started(
            t.id,
            "worker-1",
            "Do the brief\n\nEnd your reply with its STATUS line: done, working or blocked.",
        )
        .await
        .unwrap();
    let running = tasks.issue(t.id).await.unwrap().unwrap();
    let live = running.live.unwrap();
    assert_eq!(live.state.as_str(), "running");
    assert!(live.run.unwrap().worker);
    tasks
        .note(
            t.id,
            Who::Worker,
            "💬 Keep replies short\nDid the brief\nSTATUS: done",
            false,
        )
        .await
        .unwrap();
    tasks.worker_ended("worker-1").await.unwrap();
    let detail = tasks.detail(t.id).await.unwrap().unwrap();
    assert_eq!(detail.runs.as_deref().unwrap_or_default().len(), 1);
    assert_eq!(
        detail.runs.as_ref().unwrap()[0].outcome.as_deref(),
        Some("completed")
    );
    assert_eq!(
        detail.runs.as_ref().unwrap()[0].asked.as_deref(),
        Some("Do the brief"),
        "without the STATUS instruction"
    );
    assert_eq!(
        detail.runs.as_ref().unwrap()[0].answer.as_deref(),
        Some("Did the brief"),
        "without the coaching line and the STATUS line"
    );
    assert_eq!(
        detail.runs.as_ref().unwrap()[0].status.as_deref(),
        Some("STATUS: done")
    );
    tasks
        .update(t.id, TaskChange::status(Status::Done), Who::Worker, None)
        .await
        .unwrap();
}

#[tokio::test]
pub(crate) async fn a_reply_on_a_task_needs_a_signed_in_device_even_in_report_mode() {
    let app = TestApp::with_mode(AuthMode::Report).await;
    let mine = app
        .state
        .tasks()
        .create(new("Mine"), Who::Person)
        .await
        .unwrap();
    let refused = app
        .call(
            Call::post(&format!("/api/tasks/{}/message", mine.id))
                .json(&json!({ "text": "Go with the first" })),
        )
        .await;
    assert_eq!(refused.status, 401);
    assert_eq!(
        refused.json(),
        json!({ "error": "Sign in first.", "signIn": true })
    );
    let detail = app.state.tasks().detail(mine.id).await.unwrap().unwrap();
    assert!(
        detail
            .history
            .iter()
            .all(|line| !line.text.contains("Go with the first")),
        "nothing was said on the task"
    );
}

#[tokio::test]
pub(crate) async fn a_reply_over_its_limits_is_refused() {
    let app = TestApp::new().await;
    let session = app.sign_in().await;
    let mine = app
        .state
        .tasks()
        .create(new("Mine"), Who::Person)
        .await
        .unwrap();
    let url = format!("/api/tasks/{}/message", mine.id);
    let long = "x".repeat(emberhut_core::task::BODY_MAX + 1);
    let too_long = app
        .call(
            Call::post(&url)
                .token(&session)
                .json(&json!({ "text": long })),
        )
        .await;
    assert_eq!(too_long.status, 400);
    assert_eq!(
        too_long.json()["error"],
        "A reply is at most 20000 characters."
    );
    let files: Vec<Value> = (0..21)
        .map(|n| json!({ "kind": "image", "path": format!("/data/uploads/{n}.png") }))
        .collect();
    let too_many = app
        .call(
            Call::post(&url)
                .token(&session)
                .json(&json!({ "text": "see these", "files": files })),
        )
        .await;
    assert_eq!(too_many.status, 400);
    assert_eq!(too_many.json()["error"], "A reply takes at most 20 files.");
    let just_right = app
        .call(
            Call::post(&url)
                .token(&session)
                .json(&json!({ "text": "x".repeat(emberhut_core::task::BODY_MAX) })),
        )
        .await;
    assert_eq!(just_right.status, 200);
}

#[tokio::test]
pub(crate) async fn a_title_or_a_brief_over_its_limit_is_refused_for_every_caller() {
    let app = TestApp::new().await;
    let tasks = app.state.tasks();
    let title = "t".repeat(emberhut_core::task::TITLE_MAX + 1);
    let made = tasks.create(new(&title), Who::Assistant).await;
    assert!(
        matches!(&made, Err(emberhut_server::services::tasks::TaskError::Refused(why)) if why.contains("500")),
        "{made:?}"
    );
    let mine = tasks.create(new("Mine"), Who::Person).await.unwrap();
    let brief = "b".repeat(emberhut_core::task::BODY_MAX + 1);
    let changed = tasks
        .update(
            mine.id,
            TaskChange {
                body: Some(Some(brief.clone())),
                ..TaskChange::default()
            },
            Who::Person,
            None,
        )
        .await;
    assert!(
        matches!(&changed, Err(emberhut_server::services::tasks::TaskError::Refused(why)) if why.contains("20000")),
        "{changed:?}"
    );

    // The app's settings actions share the limits.
    let session = app.sign_in().await;
    let quick = app
        .call(
            Call::post(emberhut_api::paths::SETTINGS)
                .token(&session)
                .json(&json!({ "taskCreate": title })),
        )
        .await;
    assert_eq!(quick.status, 400);
    let set = app
        .call(
            Call::post(emberhut_api::paths::SETTINGS)
                .token(&session)
                .json(&json!({ "taskSet": { "id": mine.id.get(), "body": brief } })),
        )
        .await;
    assert_eq!(set.status, 400);
    let kept = tasks.issue(mine.id).await.unwrap().unwrap();
    assert!(kept.body.is_none(), "the brief was not cut and kept");
}

#[tokio::test]
pub(crate) async fn the_task_settings_actions_need_a_signed_in_device_even_in_report_mode() {
    let app = TestApp::with_mode(AuthMode::Report).await;
    let mine = app
        .state
        .tasks()
        .create(new("Mine"), Who::Person)
        .await
        .unwrap();
    for body in [
        json!({ "taskCreate": "Buy milk" }),
        json!({ "todoDone": mine.id.get() }),
        json!({ "taskArchive": mine.id.get() }),
        json!({ "taskSet": { "id": mine.id.get(), "body": "A brief" } }),
    ] {
        let refused = app
            .call(Call::post(emberhut_api::paths::SETTINGS).json(&body))
            .await;
        assert_eq!(refused.status, 401, "{body}");
    }
    let after = app.state.tasks().issue(mine.id).await.unwrap().unwrap();
    assert_eq!(after.status.as_str(), "todo");
    assert!(after.body.is_none());
}
