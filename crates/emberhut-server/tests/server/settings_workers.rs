//! `taskStop`: the owner stops a task's worker from its page; its session ends, its run closes
//! and the task goes back to the assistant's to do.

use emberhut_core::task::Status;
use serde_json::json;

use crate::common::Call;
use crate::fake_cli::{FakeCli, init, is_user};
use crate::workers::{assistant_task, until_ended, worker_app};

#[tokio::test]
async fn task_stop_stops_the_task_s_worker() {
    // A worker that never answers: it runs until it is stopped.
    let cli = FakeCli::new(|_, line| {
        if is_user(line) {
            vec![init("s-1")]
        } else {
            Vec::new()
        }
    });
    let app = worker_app(&cli).await;
    let task = assistant_task(&app, "a long job", false).await;
    app.state.tasks().start_worker(task).await.unwrap();
    assert!(app.state.workers().is_running(task));
    assert_eq!(
        app.state.tasks().status_of(task).await.unwrap(),
        Some(Status::Working)
    );

    let session = app.sign_in().await;
    for body in [
        json!({ "taskStop": "x" }),
        json!({ "taskStop": task.get() }),
    ] {
        let answer = app
            .call(Call::post("/api/settings").token(&session).json(&body))
            .await;
        assert_eq!(answer.status, 200, "{}", answer.text);
    }
    until_ended(&app, task).await;
    assert_eq!(cli.stopped(), [0]);
    assert_eq!(
        app.state.tasks().status_of(task).await.unwrap(),
        Some(Status::Todo)
    );
    let detail = app.state.tasks().detail(task).await.unwrap().unwrap();
    assert_eq!(
        detail.runs.as_ref().unwrap()[0].outcome.as_deref(),
        Some("stopped"),
        "{:?}",
        detail.runs
    );

    let missing = app
        .call(
            Call::post("/api/settings")
                .token(&session)
                .json(&json!({ "taskStop": 999 })),
        )
        .await;
    assert_eq!(missing.status, 400, "no such task: {}", missing.text);
}
