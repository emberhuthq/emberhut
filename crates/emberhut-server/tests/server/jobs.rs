//! Background jobs: the runners' sessions with a turn on and the runs each runner lists, and
//! stopping one.

use emberhut_api::jobs::JobsPage as JobList;
use serde_json::json;

use crate::common::{Call, TestApp};

const BUILD: &str = "token-of-build";

async fn app() -> TestApp {
    let app = TestApp::new().await;
    app.runners.on(&[
        ("build", "Build box", BUILD),
        ("laptop", "Laptop", "token-of-laptop"),
    ]);
    let event = json!({
        "machine": "build", "kind": "activity", "at": "2026-11-25T10:00:00.000Z", "doing": "Testing",
        "session": { "id": "r-1a", "title": "Fix the build", "project": "", "status": "running" },
    });
    let answer = app
        .call(
            Call::post("/api/runner/events")
                .header("authorization", &format!("Bearer {BUILD}"))
                .json(&event),
        )
        .await;
    assert_eq!(answer.status, 200);
    app.runners.answer(
        "build",
        "/jobs",
        json!({
            "procs": [{ "pid": 4242, "kind": "codex", "args": "codex exec --full-auto", "cwd": "/work/app",
                        "age": 60, "cpuTime": 12.5, "rss": 1024,
                        "last": { "text": "Wrote a.rs", "at": "2026-11-25T10:00:00.000Z", "from": "codex" } }],
            "launchd": [{ "label": "example.nightly", "pid": null, "status": 1 }],
        }),
    );
    app.runners
        .refuse("laptop", "/jobs", "the runner is asleep");
    app
}

#[tokio::test]
pub(crate) async fn the_jobs_are_the_runners_sessions_and_runs() {
    let app = app().await;
    let session = app.sign_in().await;
    let answer = app.call(Call::get("/api/jobs").token(&session)).await;
    assert_eq!(answer.status, 200, "{}", answer.text);
    let list: JobList = answer.read_as();
    let ids: Vec<&str> = list.jobs.iter().map(|job| job.id.as_str()).collect();
    assert_eq!(
        ids,
        [
            "build:launchd:example.nightly",
            "build:pid:4242",
            "session:r-1a"
        ],
        "failed first"
    );
    let run = &list.jobs[1];
    assert_eq!(run.place, "Build box");
    assert_eq!(run.title, "Codex run");
    assert_eq!(run.cpu_time, Some(12.5));
    assert_eq!(run.last.as_ref().unwrap().text, "Wrote a.rs");
    let session_job = &list.jobs[2];
    assert_eq!(session_job.last.as_ref().unwrap().text, "Testing");
    assert!(session_job.stop);
    assert_eq!(list.jobs[0].exit.as_deref(), Some("exit 1"));
    assert!(!list.mac.ok, "a runner didn't answer");
    assert_eq!(list.mac.error.as_deref(), Some("the runner is asleep"));
    let looks = app.runners.requests_to("/jobs").len();
    app.call(Call::get("/api/jobs").token(&session)).await;
    assert_eq!(
        app.runners.requests_to("/jobs").len(),
        looks,
        "a look is kept a few seconds"
    );
    app.call(Call::get("/api/jobs").query("fresh=1").token(&session))
        .await;
    assert_eq!(
        app.runners.requests_to("/jobs").len(),
        looks + 2,
        "fresh looks again"
    );
    assert_eq!(app.call(Call::get("/api/jobs")).await.status, 401);
}

#[tokio::test]
pub(crate) async fn a_job_is_stopped_where_it_runs() {
    let app = app().await;
    let session = app.sign_in().await;
    let stop = |id: &str| {
        Call::post("/api/jobs/stop")
            .token(&session)
            .json(&json!({ "id": id }))
    };
    let answer = app.call(stop("build:pid:4242")).await;
    assert_eq!(answer.status, 200, "{}", answer.text);
    let stopped = &app.runners.requests_to("/jobs/stop")[0];
    assert_eq!(stopped.machine, "build");
    assert_eq!(stopped.body, Some(json!({ "pid": 4242 })));
    let db = emberhut_db::Database::connect(&app.url).await.unwrap();
    let line = db.log().recent(1).await.unwrap().remove(0).text;
    assert!(
        line.starts_with("Stopped the background job ") && line.ends_with(" from the app"),
        "{line}"
    );
    let answer = app.call(stop("session:r-1a")).await;
    assert_eq!(answer.status, 200);
    assert_eq!(app.runners.requests_to("/sessions/r-1a/stop").len(), 1);
    let answer = app.call(stop("build:launchd:example.nightly")).await;
    assert_eq!(answer.status, 400);
    assert_eq!(
        answer.json(),
        json!({ "ok": false, "error": "this one can't be stopped from here" })
    );
    let answer = app.call(stop("build:pid:1")).await;
    assert_eq!(answer.json()["error"], "that job isn't running anymore");
    assert_eq!(app.call(Call::post("/api/jobs/stop")).await.status, 401);
}
