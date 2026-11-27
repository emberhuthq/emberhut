//! The person's changes (`/api/settings`), the switches, what the assistant knows, and the
//! Chrome profiles.

use emberhut_api::chrome::{ChromeAction, ChromeControl, ChromeReply};
use emberhut_api::facts::{FactCategory, FactsPage};
use emberhut_api::ids::{ChromeCode, ChromeProfileId, FactId, ProjectId, RoomId, TaskId, TrialId};
use emberhut_api::patch::Patch;
use emberhut_api::settings::{FactChange, SettingsAction, SettingsRequest, TaskChange};
use emberhut_api::switches::{McpState, SkillSource, SwitchAction, SwitchPage};
use emberhut_api::time::Day;
use serde_json::json;

use crate::support::{
    assert_round_trip, assert_writes_shape, fixture, from_value, to_value, with_unknown_fields,
};

#[test]
fn a_fact_correction_is_its_own_key() {
    let request: SettingsRequest = assert_round_trip("settings_fact_set.json");
    let change = request.fact_set.clone().unwrap();
    assert_eq!(change.category, FactCategory::People);
    let action = SettingsAction::FactSet(FactChange {
        id: FactId::new("people/example-person"),
        category: FactCategory::People,
        title: "Example Person".into(),
        text: "Works on the example project.".into(),
    });
    assert_eq!(SettingsRequest::from(action), request);
}

#[test]
fn a_task_change_sends_only_what_changes() {
    let request: SettingsRequest = assert_round_trip("settings_task_set.json");
    assert_eq!(request.room, Some(RoomId::new(2)));
    let change = request.task_set.unwrap();
    assert_eq!(change.deadline, Patch::Clear);
    assert_eq!(change.project, Patch::Set(ProjectId::new(2)));
    assert_eq!(change.decide_by, Patch::Set(Day::new("2026-11-28")));
    assert_eq!(change.size, Patch::Keep);
    let mut built = TaskChange::new(TaskId::new(12));
    built.deadline = Patch::Clear;
    built.hard = Some(true);
    built.project = Patch::Set(ProjectId::new(2));
    built.decide_by = Patch::Set(Day::new("2026-11-28"));
    assert_eq!(
        to_value(&SettingsRequest::from(SettingsAction::TaskSet(built)))["taskSet"],
        fixture("settings_task_set.json")["taskSet"]
    );
}

#[test]
fn every_action_writes_the_keys_the_server_reads() {
    let cases = [
        (SettingsAction::SetOn(true), json!({ "on": true })),
        (
            SettingsAction::TrialDrop(TrialId::new(252)),
            json!({ "trialDrop": 252 }),
        ),
        (
            SettingsAction::TrialKeep(TrialId::new(252)),
            json!({ "trialKeep": 252 }),
        ),
        (
            SettingsAction::FactDrop(FactId::new("work/example")),
            json!({ "factDrop": "work/example" }),
        ),
        (
            SettingsAction::TodoDone {
                task: TaskId::new(3),
                done: false,
            },
            json!({ "todoDone": 3, "done": false }),
        ),
        (
            SettingsAction::TaskCreate {
                title: "Write it".into(),
                goal: None,
                project: Some(ProjectId::new(4)),
            },
            json!({ "taskCreate": "Write it", "project": 4 }),
        ),
    ];
    for (action, wire) in cases {
        assert_eq!(to_value(&SettingsRequest::from(action)), wire);
    }
}

#[test]
fn the_switches_page_round_trips() {
    let page: SwitchPage = assert_round_trip("switches.json");
    assert_eq!(page.account.as_deref(), Some("main"));
    let states: Vec<_> = page.mcp.iter().map(|server| server.state.clone()).collect();
    assert_eq!(
        states,
        [
            McpState::Connected,
            McpState::Failed,
            McpState::NeedsAuth,
            McpState::Off
        ]
    );
    assert!(page.mcp[0].fixed && !page.mcp[0].can_reconnect());
    assert!(page.mcp[1].can_reconnect(), "on, and failed");
    assert!(!page.mcp[3].can_reconnect(), "off");
    assert_eq!(page.skills[2].source, SkillSource::BuiltIn);
    let mut body = fixture("switches.json");
    with_unknown_fields(&mut body);
    assert_eq!(from_value::<SwitchPage>(body), page);
}

#[test]
fn a_switch_request_round_trips() {
    let action: SwitchAction = assert_round_trip("switch_request.json");
    assert_eq!(
        action,
        SwitchAction::Mcp {
            name: "files".into(),
            on: false
        }
    );
}

#[test]
fn the_facts_page_writes_the_servers_shape() {
    let page: FactsPage = assert_writes_shape("facts.json", &[]);
    assert!(page.facts[0].is_important());
    assert_eq!(page.facts[1].category, FactCategory::Preferences);
    assert_eq!(page.facts[1].now, None);
    assert_eq!(page.trials[0].id, TrialId::new(252));
    assert_eq!(
        page.trials[0].lesson.as_deref(),
        Some("I say which file a change is in.")
    );
    assert_eq!(page.lessons.len(), 1);
    let older: FactsPage = from_value(json!({ "facts": [] }));
    assert!(older.trials.is_empty(), "a server without trials");
    let unworded: FactsPage = from_value(json!({ "facts": [], "trials": [{
        "id": 7, "text": "Say which file a change is in.", "at": "2026-11-20T10:00:00.000Z",
        "ends": "2026-12-04T10:00:00.000Z", "said": "Which file was that in?"
    }] }));
    assert_eq!(
        unworded.trials[0].lesson, None,
        "a server without the person's line"
    );
    assert!(
        to_value(&unworded.trials[0]).get("lesson").is_none(),
        "none is left out"
    );
}

#[test]
fn a_request_reads_trial_keep() {
    let request: SettingsRequest = from_value(json!({ "trialKeep": 252 }));
    assert_eq!(request.trial_keep, Some(TrialId::new(252)));
    assert_eq!(
        request,
        SettingsRequest::from(SettingsAction::TrialKeep(TrialId::new(252)))
    );
}

#[test]
fn a_chrome_request_round_trips() {
    let action: ChromeAction = assert_round_trip("chrome_request.json");
    assert_eq!(
        action,
        ChromeAction::Rename {
            id: ChromeProfileId::new(2),
            label: "Work laptop".into()
        }
    );
    assert_eq!(
        to_value(&ChromeAction::Cancel {
            code: ChromeCode::new("ABCD-1234")
        }),
        json!({ "kind": "cancel", "code": "ABCD-1234" })
    );
}

#[test]
fn the_chrome_reply_round_trips() {
    let reply: ChromeReply = assert_writes_shape("chrome_reply.json", &[]);
    assert!(reply.ok);
    assert_eq!(reply.chrome[0].chrome_major(), Some("131"));
    assert_eq!(reply.chrome[1].control, ChromeControl::Person);
    assert_eq!(reply.chrome[1].task, Some(TaskId::new(13)));
    assert_eq!(
        reply.pairing.map(|pairing| pairing.code),
        Some(ChromeCode::new("ABCD-1234"))
    );
}
