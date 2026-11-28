//! The person renames a task (`PATCH /api/tasks/{id}`): the task comes back renamed, its history
//! says so once, and a blank or long title, a task that isn't there or no signed-in device is
//! refused.

use emberhut_api::error::ErrorBody;
use emberhut_api::tasks::Issue;
use emberhut_core::task::{Owner, TaskId, Who};
use emberhut_db::NewTask;
use serde_json::{Value, json};

use crate::common::{Answer, Call, TestApp};

async fn renamed(app: &TestApp, id: &str, body: &Value) -> Answer {
    let session = app.sign_in().await;
    app.call(
        Call::patch(&format!("/api/tasks/{id}"))
            .token(&session)
            .json(body),
    )
    .await
}

#[tokio::test]
pub(crate) async fn the_person_renames_a_task() {
    let app = TestApp::new().await;
    let id = app
        .state
        .tasks()
        .create(
            NewTask {
                title: "Session on the Mac".to_owned(),
                owner: Some(Owner::Person),
                ..NewTask::default()
            },
            Who::Person,
        )
        .await
        .unwrap()
        .id;
    let answer = renamed(&app, "1", &json!({ "title": "  Fix the lexer  " })).await;
    assert_eq!(answer.status, 200, "{}", answer.text);
    let task: Issue = answer.read_as();
    assert_eq!(
        (task.id.get(), task.title.as_str()),
        (id.get(), "Fix the lexer")
    );
    let history = app.state.tasks().history_of(id).await.unwrap();
    let last = history.last().unwrap();
    assert_eq!(last.text, "Renamed: Session on the Mac → Fix the lexer");
    assert_eq!(last.who, Who::Person);
    let again = renamed(&app, "1", &json!({ "title": "Fix the lexer" })).await;
    assert_eq!(again.status, 200, "{}", again.text);
    assert_eq!(
        app.state.tasks().history_of(id).await.unwrap().len(),
        history.len(),
        "the same title writes nothing"
    );
}

#[tokio::test]
pub(crate) async fn a_rename_needs_a_title_a_task_and_a_device() {
    let app = TestApp::new().await;
    let id = app
        .state
        .tasks()
        .create(
            NewTask {
                title: "The lexer".to_owned(),
                ..NewTask::default()
            },
            Who::Person,
        )
        .await
        .unwrap()
        .id;
    let refused = async |id: &str, body: Value| {
        let answer = renamed(&app, id, &body).await;
        (answer.status.as_u16(), answer.read_as::<ErrorBody>().error)
    };
    let said = |status: u16, error: &str| (status, error.to_owned());
    assert_eq!(
        refused("1", json!({ "title": " " })).await,
        said(400, "a task's title can't be blank")
    );
    assert_eq!(
        refused("1", json!({ "title": "a".repeat(201) })).await,
        said(400, "a task's title is at most 200 characters")
    );
    assert_eq!(
        refused("1", json!({})).await,
        said(400, "say the new title: { title }")
    );
    assert_eq!(
        refused("999", json!({ "title": "New" })).await,
        said(404, "no such task")
    );
    assert_eq!(
        refused("abc", json!({ "title": "New" })).await,
        said(404, "no such task")
    );
    let anonymous = app
        .call(Call::patch("/api/tasks/1").json(&json!({ "title": "New" })))
        .await;
    assert_eq!(anonymous.status, 401);
    let page = app
        .state
        .tasks()
        .detail(TaskId::new(id.get()))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(page.task.title, "The lexer", "left as it was");
}
