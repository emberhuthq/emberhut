//! The owner's health and screen time: `POST /api/health` and `POST /api/screentime`, the
//! state's `health`, `healthAt` and `screentimeAt`, a walk closing its task, and the `health`
//! and `screen_time` tools.
#![expect(
    clippy::unwrap_used,
    reason = "test helpers: a failure here fails the test that called them"
)]

use std::sync::Arc;

use emberhut_api::paths;
use emberhut_api::state::State;
use emberhut_db::Database;
use jiff::Timestamp;
use serde_json::{Value, json};

use crate::common::{Call, TestApp, text};
use crate::fake_personal::{FakeWalks, Fakes};

/// An app whose clock reads 2026-10-12, 18:00 UTC.
async fn app(fakes: &Fakes) -> TestApp {
    let app = TestApp::with_fakes(fakes).await;
    let evening: Timestamp = "2026-10-12T18:00:00Z".parse().unwrap();
    app.clock.set(evening);
    app
}

async fn state(app: &TestApp) -> State {
    let session = app.sign_in().await;
    app.call(Call::get(paths::STATE).token(&session))
        .await
        .read_as()
}

async fn post(app: &TestApp, path: &str, body: &Value) -> crate::common::Answer {
    let session = app.sign_in().await;
    app.call(Call::post(path).token(&session).json(body)).await
}

#[tokio::test]
pub(crate) async fn the_phone_s_health_days_are_kept_and_today_shows_in_the_state() {
    let fakes = Fakes::default();
    let app = app(&fakes).await;
    let before = state(&app).await;
    assert_eq!(before.health_at, None);
    let health = before.health.unwrap();
    assert_eq!(health.date.as_str(), "2026-10-12");
    assert_eq!((health.steps, health.steps_target), (None, 8000.0));

    let saved = post(
        &app,
        "/api/health",
        &json!({ "days": [
            {
                "date": "2026-10-12", "steps": 4200, "exerciseMin": 25, "sleepMin": 410,
                "workouts": [{ "type": "Walking", "min": 21.6, "at": "07:40" }]
            },
            { "date": "2026-10-11", "steps": 9100 },
            { "date": "yesterday", "steps": 1 },
            { "steps": 2 }
        ]}),
    )
    .await;
    assert_eq!(saved.status, 200);
    assert_eq!(saved.json(), json!({ "ok": true, "saved": 2 }));
    let after = state(&app).await;
    assert_eq!(
        after
            .health_at
            .as_ref()
            .map(emberhut_api::time::Timestamp::as_str),
        Some("2026-10-12T18:00:00.000Z")
    );
    let today = after.health.unwrap();
    assert_eq!(today.steps, Some(4200.0));
    assert_eq!(today.exercise_min, Some(25.0));
    assert_eq!(today.move_min, Some(22.0), "the walk, rounded");
    assert_eq!(today.at, after.health_at);
    assert_eq!((today.project, today.tasks.len()), (None, 0));

    // The phone says why it sent nothing: kept, logged once, shown by the tool.
    for _ in 0..2 {
        let none = post(
            &app,
            paths::HEALTH_DAYS,
            &json!({ "days": [], "why": "  Health sharing is off  " }),
        )
        .await;
        assert_eq!(none.json(), json!({ "ok": true, "saved": 0 }));
    }
    let tool = app.call_tool("health", json!({ "days": 1 })).await;
    let tool: Value = serde_json::from_str(text(&tool)).unwrap();
    assert_eq!(tool["notSent"]["why"], "Health sharing is off");
    assert_eq!(tool["lastSync"], "2026-10-12T18:00:00.000Z");
    assert_eq!(tool["days"].as_array().unwrap().len(), 1);
    assert_eq!(
        tool["days"][0]["sleepMin"], 410,
        "the day as the phone sent it"
    );
    let db = Database::connect(&app.url).await.unwrap();
    let log: Vec<String> = db
        .log()
        .recent(5)
        .await
        .unwrap()
        .into_iter()
        .map(|l| l.text)
        .collect();
    assert_eq!(
        log,
        ["Health: the phone didn't send its days: Health sharing is off"]
    );

    post(
        &app,
        paths::HEALTH_DAYS,
        &json!({ "days": [{ "date": "2026-10-12", "steps": 5000 }] }),
    )
    .await;
    let tool = app.call_tool("health", json!({})).await;
    let tool: Value = serde_json::from_str(text(&tool)).unwrap();
    assert!(tool.get("notSent").is_none(), "days came again");
    assert_eq!(
        tool["days"][0]["steps"], 5000,
        "a day sent again replaces it"
    );

    let session = app.sign_in().await;
    let bad = app
        .call(
            Call::post(paths::HEALTH_DAYS)
                .token(&session)
                .raw("{ not json"),
        )
        .await;
    assert_eq!(bad.status, 400);
}

#[tokio::test]
pub(crate) async fn screen_time_is_kept_per_day_and_device() {
    let fakes = Fakes::default();
    let app = app(&fakes).await;
    let empty = app.call_tool("screen_time", json!({})).await;
    assert_eq!(
        text(&empty),
        "No Screen Time yet: the helper on the user's Mac sends it."
    );
    let no_health = app.call_tool("health", json!({})).await;
    assert!(text(&no_health).starts_with("No Health data yet"));

    let saved = post(
        &app,
        "/api/screentime",
        &json!({ "days": [
            { "date": "2026-10-12", "device": "iPhone", "minutes": 70, "apps": [{ "id": "com.example.chat", "min": 30 }] },
            { "date": "2026-10-12", "device": "Mac", "minutes": 50, "apps": [] },
            { "date": "2026-09-01", "device": "Mac", "minutes": 5, "apps": [] },
            { "date": "2026-10-12", "minutes": 1 }
        ]}),
    )
    .await;
    assert_eq!(saved.status, 200);
    assert_eq!(saved.json(), json!({ "ok": true, "saved": 3 }));
    assert_eq!(
        state(&app)
            .await
            .screentime_at
            .map(|at| at.as_str().to_owned())
            .as_deref(),
        Some("2026-10-12T18:00:00.000Z")
    );
    let week = app.call_tool("screen_time", json!({ "days": 7 })).await;
    let week: Value = serde_json::from_str(text(&week)).unwrap();
    let devices: Vec<&str> = week["days"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["device"].as_str().unwrap())
        .collect();
    assert_eq!(devices, ["Mac", "iPhone"], "the last week only, by device");
    assert_eq!(week["lastSync"], "2026-10-12T18:00:00.000Z");

    let session = app.sign_in().await;
    let bad = app
        .call(Call::post(paths::SCREENTIME).token(&session).raw("[1,"))
        .await;
    assert_eq!(bad.status, 400);
}

#[tokio::test]
async fn a_real_walk_closes_its_walk_task_once() {
    let walks = Arc::new(FakeWalks::default());
    walks.open(&[
        (5, "First walk: 20 minutes"),
        (6, "Walk 45 min"),
        (7, "Read a book"),
    ]);
    let fakes = Fakes {
        walks: Some(Arc::clone(&walks)),
        ..Fakes::default()
    };
    let app = app(&fakes).await;
    let health = state(&app).await.health.unwrap();
    assert_eq!(health.project, Some(emberhut_api::ids::ProjectId::from(9)));
    let targets: Vec<(i64, f64)> = health
        .tasks
        .iter()
        .map(|t| (t.id.get(), t.target))
        .collect();
    assert_eq!(targets, [(5, 20.0), (6, 45.0)]);

    let day = json!({ "days": [{
        "date": "2026-10-12", "steps": 12345,
        "workouts": [{ "type": "Hiking", "min": 31.6, "at": "07:40" }]
    }]});
    for _ in 0..2 {
        post(&app, paths::HEALTH_DAYS, &day).await;
    }
    let closed = walks.closed();
    assert_eq!(closed.len(), 1, "a day's walk counts once");
    assert_eq!(closed[0].0, 5, "the first walk task it reaches");
    assert_eq!(
        closed[0].1,
        "Walked 32 min, 07:40, 12,345 steps that day (Apple Health, 2026-10-12)"
    );
    assert_eq!(closed[0].2.to_string(), "2026-10-12T07:40:00Z");

    // Yesterday's exercise, with no walk task it reaches: logged as a done task.
    post(
        &app,
        paths::HEALTH_DAYS,
        &json!({ "days": [{ "date": "2026-10-11", "exerciseMin": 24 }] }),
    )
    .await;
    assert_eq!(
        walks.logged(),
        [(
            "Exercised 24 min".to_owned(),
            "Exercised 24 min (Apple Health, 2026-10-11)".to_owned()
        )]
    );
    let db = Database::connect(&app.url).await.unwrap();
    let log = db.log().recent(1).await.unwrap();
    assert_eq!(
        log[0].text,
        "Health: Exercised 24 min (Apple Health, 2026-10-11) → task 77 done"
    );
}

#[tokio::test]
async fn the_state_has_voice_last_call_recording_and_health_on_the_wire() {
    let app = app(&Fakes::default()).await;
    let session = app.sign_in().await;
    let state = app
        .call(Call::get(paths::STATE).token(&session))
        .await
        .json();
    assert_eq!(state["voice"], true);
    assert_eq!(state["lastCall"], Value::Null);
    assert_eq!(state["recording"], Value::Null);
    assert_eq!(state["healthAt"], Value::Null);
    assert_eq!(state["screentimeAt"], Value::Null);
    assert_eq!(
        state["health"],
        json!({
            "date": "2026-10-12", "steps": null, "exerciseMin": null, "moveMin": null,
            "stepsTarget": 8000.0, "at": null, "project": null, "tasks": []
        })
    );
}

#[tokio::test]
async fn a_walk_closes_the_health_project_s_walk_task_among_the_server_s_tasks() {
    use emberhut_core::task::board::ProjectChange;
    use emberhut_core::task::{Owner, Status, Who};
    use emberhut_db::NewTask;

    let app = app(&Fakes::default()).await;
    let tasks = app.state.tasks();
    let health = tasks
        .set_project(
            None,
            ProjectChange {
                name: Some("Health".to_owned()),
                ..ProjectChange::default()
            },
        )
        .await
        .unwrap();
    let task = |title: &str| NewTask {
        title: title.to_owned(),
        owner: Some(Owner::Person),
        pid: Some(health),
        ..NewTask::default()
    };
    let walk = tasks
        .create(task("First walk: 20 minutes"), Who::Person)
        .await
        .unwrap();
    let long = tasks
        .create(task("Walk 45 min"), Who::Person)
        .await
        .unwrap();
    tasks
        .create(task("Read a book"), Who::Person)
        .await
        .unwrap();
    let shown = state(&app).await.health.unwrap();
    assert_eq!(
        shown.project,
        Some(emberhut_api::ids::ProjectId::from(health.get()))
    );
    let targets: Vec<(i64, f64)> = shown.tasks.iter().map(|t| (t.id.get(), t.target)).collect();
    assert_eq!(targets, [(walk.id.get(), 20.0), (long.id.get(), 45.0)]);

    let day = json!({ "days": [{
        "date": "2026-10-12", "steps": 12345,
        "workouts": [{ "type": "Walking", "min": 31.6, "at": "07:40" }]
    }]});
    for _ in 0..2 {
        post(&app, paths::HEALTH_DAYS, &day).await;
    }
    assert_eq!(tasks.status_of(walk.id).await.unwrap(), Some(Status::Done));
    assert_eq!(
        tasks.status_of(long.id).await.unwrap(),
        Some(Status::Todo),
        "a day's walk closes one task, the first it reaches"
    );
    let history = tasks.history_of(walk.id).await.unwrap();
    assert!(history.iter().any(|line| line.text
        == "Walked 32 min, 07:40, 12,345 steps that day (Apple Health, 2026-10-12)"));
    let done = tasks.issue(walk.id).await.unwrap().unwrap();
    assert_eq!(
        done.done_at.unwrap().as_str(),
        "2026-10-12T07:40:00.000Z",
        "done on the walk's own time"
    );

    post(
        &app,
        paths::HEALTH_DAYS,
        &json!({ "days": [{ "date": "2026-10-11", "exerciseMin": 24 }] }),
    )
    .await;
    let board = tasks.plan_state().await.unwrap().issues;
    let logged = board
        .iter()
        .find(|i| i.title == "Exercised 24 min")
        .expect("a walk with no task it reaches is logged as a done task");
    assert_eq!(
        logged.pid,
        Some(emberhut_api::ids::ProjectId::from(health.get()))
    );
    assert_eq!(logged.status, emberhut_api::tasks::TaskStatus::Done);
}
