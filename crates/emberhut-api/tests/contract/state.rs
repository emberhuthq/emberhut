//! The state: read from the wire, written back in the same shape.

use emberhut_api::auth::AuthMode;
use emberhut_api::ids::{MessageId, ProjectId, RoomId, TaskId};
use emberhut_api::live::Live;
use emberhut_api::rooms::Switcher;
use emberhut_api::state::State;
use emberhut_api::task_live::LiveState;
use emberhut_api::tasks::{AskKind, ProjectHealth, ProjectStatus, TaskSize, TaskStatus};
use emberhut_api::theme::Appearance;
use emberhut_api::work::WorkState;
use serde_json::json;

use crate::support::{
    assert_writes_shape, fixture, from_value, read, to_value, with_unknown_fields,
};

#[test]
fn reads_the_state_fixture() {
    let state: State = read("state.json");
    assert_eq!(state.room, RoomId::new(2));
    assert!(state.on && state.busy && state.unread && state.earlier);
    assert_eq!(state.pending, 3);
    assert_eq!(
        state.live,
        Some(Live {
            text: String::new(),
            doing: Some("Reading the example files".to_owned())
        })
    );

    let room = state.current_room().unwrap();
    assert_eq!(room.name, "Room 2");
    assert_eq!(room.project, Some(ProjectId::new(2)));
    assert!(room.busy && room.last_mine);
    assert_eq!(room.unread, 2);
    assert_eq!(state.rooms[0].ask_id, Some(TaskId::new(12)));
    assert!(state.rooms[2].is_paused());
    assert_eq!(
        state.rooms[2].paused_until.as_ref().unwrap().as_str(),
        "2026-12-01"
    );

    let switcher = Switcher::new(&state.rooms);
    assert_eq!(switcher.open.len(), 2);
    assert_eq!(switcher.folded.len(), 1);
    assert_eq!(switcher.archived.len(), 1);
}

#[test]
fn reads_the_chat_in_the_state() {
    let state: State = read("state.json");
    let [mine, event, answer] = state.chat.as_slice() else {
        panic!("three messages")
    };
    assert!(mine.mine && !mine.event);
    assert_eq!(
        mine.files[0].path.as_deref(),
        Some("/data/uploads/notes.txt")
    );
    assert!(event.event);
    let reply = answer.reply.as_ref().unwrap();
    assert_eq!(reply.id, MessageId::new(40));
    assert!(reply.file.is_none());
    assert_eq!(answer.refs.len(), 2);
    assert_eq!(answer.refs[0].task, Some(TaskId::new(12)));
    assert_eq!(
        answer.refs[1].url.as_deref(),
        Some("https://github.com/example/repo/pull/3")
    );
}

#[test]
fn reads_the_tasks_and_work_in_the_state() {
    let state: State = read("state.json");
    let task = &state.issues[0];
    assert_eq!(task.status, TaskStatus::Waiting);
    assert_eq!(task.key, "T-12");
    assert!((task.score - 4.5).abs() < f64::EPSILON);
    assert_eq!(task.deadline.as_ref().unwrap().as_str(), "2026-11-28");
    let decision = task.decision.as_ref().unwrap();
    assert_eq!(decision.options, ["Report A", "Report B", "Leave it"]);
    assert_eq!(decision.recommend, Some(0));
    assert_eq!(
        task.artifacts[0].thumb.as_deref(),
        Some("/api/artifacts/7/thumb")
    );
    assert_eq!(state.issues[1].parent, Some(TaskId::new(12)));
    assert_eq!(task.size, Some(TaskSize::Small));
    assert_eq!(task.body.as_deref(), Some("A made-up task."));
    assert_eq!(task.live.as_ref().unwrap().state, LiveState::Waiting);
    let subtask = &state.issues[1];
    assert_eq!((subtask.progress, subtask.total), (Some(1), Some(3)));
    assert_eq!(subtask.unit.as_deref(), Some("parts"));
    assert_eq!(subtask.eta.as_deref(), Some("about an hour"));
    assert_eq!(subtask.live.as_ref().unwrap().state, LiveState::Running);
    assert!(!subtask.is_finding() && subtask.is_open());
    assert_eq!(state.open_tasks().count(), 1);

    assert_eq!(state.today, [TaskId::new(12)]);
    assert_eq!(state.projects[1].status, ProjectStatus::Paused);
    assert_eq!(state.projects[0].health, Some(ProjectHealth::OnTrack));
    assert_eq!(state.goals[0].rank, 1);

    let [waiting, thread, asking] = state.working.as_slice() else {
        panic!("three rows")
    };
    let asks = asking
        .waiting
        .as_ref()
        .expect("a waiting thread row has the group");
    assert_eq!(asks.need, AskKind::Approve);
    assert!(asks.checked.is_none());
    assert!(
        asks.deadline.is_none(),
        "a thread row has no decideBy or hardBy"
    );
    assert!(waiting.needs_you());
    let asked = waiting.waiting.as_ref().unwrap();
    assert_eq!(asked.need, AskKind::Choose);
    let deadline = asked.deadline.as_ref().unwrap();
    assert_eq!(deadline.decide_by.as_ref().unwrap().as_str(), "2026-11-26");
    assert!(!deadline.hard_by);
    assert_eq!(thread.state, WorkState::Running);
    assert!(thread.waiting.is_none() && !thread.paused);

    let session = &state.sessions[0];
    assert_eq!(session.room, RoomId::new(2));
    assert_eq!(session.tasks, [TaskId::new(13)]);
    assert!(session.direct);
}

#[test]
fn reads_the_settings_in_the_state() {
    let state: State = read("state.json");
    let theme = state.theme.unwrap();
    assert_eq!(
        (theme.preset.as_str(), theme.appearance),
        ("blue", Appearance::System)
    );
    let ember = state.ember_name.unwrap();
    assert_eq!(
        (ember.name.as_str(), ember.custom()),
        ("Juniper", Some("Juniper"))
    );
    let ui = state.ui.unwrap();
    assert!(ui.hide_goals && ui.hide_projects);
    let account = state.account.unwrap();
    assert_eq!(account.used5h, Some(0.25));
    assert_eq!(account.limited_until, None);
    let auth = state.auth.unwrap();
    assert_eq!(auth.mode, AuthMode::Report);
    assert_eq!(auth.devices[0].name.as_deref(), Some("Example laptop"));
    assert!(state.push);
}

/// What the fixture has that the contract doesn't model yet (docs/api.md: partly covered).
const NOT_MODELLED: &[&str] = &[
    "checkins",
    "watches",
    "contextAt",
    "awareness",
    "lastCall",
    "healthAt",
    "screentimeAt",
    "did",
    "issues[].requested",
    "issues[].requestedBy",
    "issues[].send",
    "accounts[].provider",
    "accounts[].optIn",
    "accounts[].region",
    "accounts[].models",
    "working[].live",
];

#[test]
fn writes_the_state_shape() {
    assert_writes_shape::<State>("state.json", NOT_MODELLED);
}

#[test]
fn tolerates_unknown_fields_everywhere() {
    let mut body = fixture("state.json");
    with_unknown_fields(&mut body);
    let state: State = from_value(body);
    assert_eq!(state, read::<State>("state.json"));
}

#[test]
fn reads_an_older_server() {
    let state: State = read("state_older.json");
    let main = &state.rooms[0];
    assert_eq!(main.status, ProjectStatus::Active);
    assert!(main.ask_id.is_none() && !main.last_mine && main.paused_until.is_none());
    assert_eq!(state.chat[0].files, []);
    assert!(state.issues.is_empty() && state.working.is_empty() && state.sessions.is_empty());
    assert!(
        state.theme.is_none()
            && state.ember_name.is_none()
            && state.ui.is_none()
            && state.account.is_none()
            && state.auth.is_none()
    );
}

#[test]
fn keeps_words_it_doesnt_know() {
    let mut body = fixture("state.json");
    body["issues"][0]["status"] = json!("blocked");
    body["rooms"][1]["status"] = json!("someday");
    let state: State = from_value(body);
    assert_eq!(
        state.issues[0].status,
        TaskStatus::Other("blocked".to_owned())
    );
    let written = to_value(&state);
    assert_eq!(written["issues"][0]["status"], "blocked");
    assert_eq!(written["rooms"][1]["status"], "someday");
}

#[test]
fn needs_the_fields_every_server_sends() {
    for field in [
        "room", "rooms", "on", "busy", "unread", "pending", "chat", "earlier",
    ] {
        let mut body = fixture("state_older.json");
        body.as_object_mut().unwrap().remove(field);
        assert!(
            serde_json::from_value::<State>(body).is_err(),
            "{field} is required"
        );
    }
    let mut body = fixture("state_older.json");
    body.as_object_mut().unwrap().remove("live");
    assert!(
        from_value::<State>(body).live.is_none(),
        "live may be left out: none"
    );
}

#[test]
fn a_malformed_waiting_group_is_an_error_not_a_row_that_doesnt_wait() {
    for (field, value) in [
        ("asked", json!(42)),
        ("hardBy", json!(null)),
        ("need", json!(7)),
    ] {
        let mut body = fixture("state.json");
        body["working"][0][field] = value.clone();
        let error = serde_json::from_value::<State>(body).unwrap_err();
        assert!(
            error.to_string().contains("invalid type"),
            "{field} = {value}: {error}"
        );
    }
}

#[test]
fn group_fields_without_a_need_are_ignored() {
    let mut body = fixture("state.json");
    let thread = body["working"][1].as_object_mut().unwrap();
    thread.insert("asked".to_owned(), json!("2026-11-24T21:40:00.000Z"));
    thread.insert("hardBy".to_owned(), json!(true));
    let state: State = from_value(body);
    assert!(state.working[1].waiting.is_none());
}

#[test]
fn a_waiting_group_reads_on_its_own() {
    use emberhut_api::work::Waiting;
    let group: Waiting =
        serde_json::from_value(json!({ "need": "do", "asked": null, "checked": null })).unwrap();
    assert!(group.deadline.is_none());
    let error = serde_json::from_value::<Waiting>(json!({ "asked": null })).unwrap_err();
    assert!(
        error.to_string().contains("missing field `need`"),
        "{error}"
    );
}

#[test]
fn reads_the_push_switch_and_the_background_work() {
    let state: State = read("state.json");
    assert!(state.push);
    let [running, ended] = state.tasks.as_slice() else {
        panic!("two pieces of background work")
    };
    assert!(running.is_running() && !ended.is_running());
    assert_eq!(running.kind, "local_bash");
    assert_eq!(running.room, Some(RoomId::new(2)));
    assert_eq!(ended.room, None, "left out for Main");
    let older: State = read("state_older.json");
    assert!(!older.push && older.tasks.is_empty());
}

#[test]
fn reads_the_chrome_profiles_in_the_state() {
    let state: State = read("state.json");
    assert_eq!(state.chrome.len(), 2);
    assert!(state.chrome[0].connected && state.chrome[0].outdated);
    assert!(state.chrome[1].paused.is_some());
    assert_eq!(state.chrome_pairing[0].label, "Side project");
    assert_eq!(state.chrome_latest.as_deref(), Some("1.0.0"));
}

#[test]
fn a_chrome_profile_without_its_pairing_time_still_reads() {
    let mut body = fixture("state.json");
    body["chrome"][0]["created"] = json!(null);
    let state: State = from_value(body);
    assert_eq!(state.chrome[0].created, None);
    assert!(state.chrome[1].created.is_some());
}
