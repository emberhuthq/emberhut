//! What an import brings in, as the routes serve it: a project's room with its asks, who asked
//! for a task (its own request or its parent's), a send card's result, a task's runs and the
//! last call. The rows are written the way an import writes them, in the wire shapes, on
//! invented data.
#![expect(
    clippy::unwrap_used,
    clippy::panic,
    reason = "test helpers: a failure here fails the test that called them"
)]

use emberhut_core::task::Owner;
use sea_orm::ConnectionTrait;
use serde_json::Value;

use crate::common::{Call, TestApp};

/// Writes `statements` into the app's database, as an import does before the server starts.
async fn write(app: &TestApp, statements: &[&str]) {
    let conn = sea_orm::Database::connect(app.url.as_str()).await.unwrap();
    for statement in statements {
        conn.execute_unprepared(statement)
            .await
            .unwrap_or_else(|error| panic!("{statement}: {error}"));
    }
    conn.close().await.unwrap();
}

/// The task `id` in the state's `issues`.
fn issue(state: &Value, id: i64) -> &Value {
    state["issues"]
        .as_array()
        .unwrap()
        .iter()
        .find(|issue| issue["id"] == id)
        .unwrap_or_else(|| panic!("task {id} is not in the state"))
}

/// The room `id` in the state's `rooms`.
fn room(state: &Value, id: i64) -> &Value {
    state["rooms"]
        .as_array()
        .unwrap()
        .iter()
        .find(|room| room["id"] == id)
        .unwrap_or_else(|| panic!("room {id} is not in the state"))
}

const AT: &str = "2026-11-20T08:00:00.000Z";

async fn imported() -> TestApp {
    let app = TestApp::new().await;
    let ms = 1_795_651_200_000_i64;
    let (person, assistant) = (Owner::Person.as_str(), Owner::Assistant.as_str());
    let rows = [
        format!(
            "INSERT INTO projects (id, name, goal, status, created, updated) VALUES \
             (3, 'Garden', 2, 'active', {ms}, {ms}), (4, 'Old', NULL, 'archived', {ms}, {ms})"
        ),
        format!(
            "INSERT INTO rooms (id, name, created_at, project_id) VALUES \
             (2, 'Garden', {ms}, 3), (5, 'Old', {ms}, 4)"
        ),
        format!(
            "INSERT INTO issues (id, title, owner, status, pid, created, updated, requested, \
             requested_by, action, decision) VALUES \
             (7, 'Plant bulbs', '{person}', 'waiting', 3, {ms}, {ms}, \
              '{{\"at\":\"{AT}\",\"msg\":10,\"room\":2}}', \
              '{{\"who\":\"Someone\",\"at\":\"{AT}\",\"how\":\"named\"}}', 'Pick a spot', NULL), \
             (8, 'Water them', '{assistant}', 'todo', 3, {ms}, {ms}, NULL, NULL, NULL, NULL), \
             (9, 'Old ask', '{person}', 'waiting', 4, {ms}, {ms}, NULL, NULL, NULL, \
              '{{\"question\":\"Keep it?\",\"options\":[\"Yes\",\"No\"]}}')"
        ),
        "UPDATE issues SET parent = 7 WHERE id = 8".to_owned(),
        format!(
            "INSERT INTO runs (id, task, machine, thread, turn, kind, asked, requested, started, \
             ended, outcome, inferred, applied) VALUES \
             (4, 7, 'build', 'th-1', 'turn-1', 'thread', 'Plant them', {ms}, {ms}, {ms}, 'done', \
              0, 1)"
        ),
        format!(
            "INSERT INTO settings (key, value) VALUES \
             ('sendTaps', '{{\"7\":{{\"state\":\"sent\",\"at\":\"{AT}\",\"via\":\"Slack\",\"to\":\"Ada\"}}}}'), \
             ('lastCall', '{{\"report\":\"Ten minutes\",\"transcript\":\"Hello\"}}')"
        ),
    ];
    write(&app, &rows.iter().map(String::as_str).collect::<Vec<_>>()).await;
    app
}

#[tokio::test]
async fn a_project_room_shows_its_project_and_the_asks_waiting_there() {
    let app = imported().await;
    let session = app.sign_in().await;
    let state = app
        .call(Call::get("/api/state").token(&session))
        .await
        .json();
    let garden = room(&state, 2);
    assert_eq!(garden["project"], 3);
    assert_eq!(garden["goal"], 2);
    assert_eq!(garden["active"], true);
    assert_eq!(garden["status"], "active");
    assert_eq!(garden["needs"], 1);
    assert_eq!(garden["ask"], "Pick a spot");
    assert_eq!(garden["askId"], 7);
    let old = room(&state, 5);
    assert_eq!(
        (&old["active"], &old["status"]),
        (&Value::Bool(false), &Value::from("archived"))
    );
    assert_eq!(old["needs"], 0);
    let main = room(&state, 1);
    assert_eq!(main["project"], Value::Null);
    assert_eq!(
        (&main["needs"], &main["askId"]),
        (&Value::from(1), &Value::from(9)),
        "an archived project's ask is Main's"
    );
    assert_eq!(state["lastCall"]["report"], "Ten minutes");
}

#[tokio::test]
async fn a_task_says_who_asked_for_it_and_what_its_send_card_came_to() {
    let app = imported().await;
    let session = app.sign_in().await;
    let state = app
        .call(Call::get("/api/state").token(&session))
        .await
        .json();
    let asked = issue(&state, 7);
    assert_eq!(
        asked["requested"],
        serde_json::json!({ "at": AT, "msg": 10, "room": 2 })
    );
    assert_eq!(asked["requestedBy"]["who"], "Someone");
    assert_eq!(
        asked["send"],
        serde_json::json!({ "state": "sent", "at": AT, "via": "Slack", "to": "Ada" })
    );
    let sub = issue(&state, 8);
    assert_eq!(
        sub["requested"]["msg"], 10,
        "a subtask's request is its parent's"
    );
    assert_eq!(sub["requestedBy"]["who"], "Someone");
    assert_eq!(sub["send"], Value::Null);

    let page = app
        .call(Call::get("/api/tasks/7").token(&session))
        .await
        .json();
    assert_eq!(page["requestedBy"]["how"], "named");
    assert_eq!(page["subtasks"][0]["requested"]["room"], 2);
    let runs = page["runs"].as_array().unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(
        (&runs[0]["id"], &runs[0]["kind"], &runs[0]["machine"]),
        (
            &Value::from(4),
            &Value::from("thread"),
            &Value::from("build")
        )
    );

    let listed = app
        .call(Call::get("/api/tasks").token(&session))
        .await
        .json();
    let in_list = listed["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == 7)
        .unwrap();
    assert_eq!(in_list["send"]["state"], "sent");
}
