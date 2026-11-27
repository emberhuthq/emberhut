//! Where the assistant works and what it uses: the environments page and its actions, the
//! background jobs, and the Claude accounts' use.

use emberhut_api::environments::{EnvActionKind, EnvActionRequest, EnvGroupId, EnvPage, EnvState};
use emberhut_api::ids::{EnvironmentId, MachineJobId, RoomId, TaskId};
use emberhut_api::jobs::{JobActionRequest, JobKind, JobState, JobsPage};
use emberhut_api::state::State;
use emberhut_api::usage::{UsageCheckReply, UseAccountReply, UseAccountRequest};

use crate::support::{
    assert_round_trip, assert_writes_shape, fixture, from_value, read, with_unknown_fields,
};

#[test]
fn the_environments_page_round_trips() {
    let page: EnvPage = assert_round_trip("environments.json");
    assert_eq!(page.summary.down, 1);
    assert_eq!(page.groups[0].id, EnvGroupId::Runners);
    let runner = page.row(&EnvironmentId::new("runner")).unwrap();
    assert_eq!(runner.state, EnvState::Ok);
    assert_eq!(runner.facts[1].state, Some(EnvState::Ok));
    assert_eq!(runner.lists[1].empty.as_deref(), Some("No folders yet"));
    assert!(runner.last_error.is_some());
    let t3 = page.row(&EnvironmentId::new("t3-mac")).unwrap();
    assert_eq!(t3.legacy, Some(true));
    let gmail = page.row(&EnvironmentId::new("gmail:work")).unwrap();
    assert!(gmail.group.is_data_source());
    assert_eq!(gmail.actions[2].id, EnvActionKind::Unlink);
    assert_eq!(page.rows().count(), 5);
    assert!(page.pairing.is_none() && page.link.is_none());
}

#[test]
fn an_action_answers_with_the_page_and_what_it_made() {
    let page: EnvPage = assert_round_trip("environments_check_reply.json");
    assert_eq!(page.pairing.unwrap().label, "Work");
    assert!(page.note.is_some());
    assert_eq!(page.link.as_deref(), Some("https://example.com/once/abc"));
    let request: EnvActionRequest = assert_round_trip("environments_check_request.json");
    assert_eq!(request.kind, EnvActionKind::Link);
}

#[test]
fn the_environments_page_takes_unknown_fields() {
    let mut body = fixture("environments.json");
    with_unknown_fields(&mut body);
    let page: EnvPage = from_value(body);
    assert_eq!(page, read::<EnvPage>("environments.json"));
}

#[test]
fn the_jobs_round_trip() {
    let page: JobsPage = assert_writes_shape("jobs.json", &[]);
    let [codex, files, old] = page.jobs.as_slice() else {
        panic!("three jobs")
    };
    assert_eq!(codex.task.as_ref().unwrap().id, TaskId::new(12));
    assert_eq!(codex.memory, Some(4_900_000_000));
    assert!(codex.is_live() && codex.file.as_ref().unwrap().is_file());
    assert_eq!(files.kind, JobKind::Server);
    assert!(files.is_quiet());
    assert_eq!(files.ports, [8080]);
    assert_eq!(old.state, JobState::Failed);
    assert!(old.clear && old.last.as_ref().unwrap().is_log());
    assert!(!page.mac.ok);
    let request: JobActionRequest = assert_round_trip("job_request.json");
    assert_eq!(request.id, MachineJobId::new("unit:ex12-pictures"));
}

#[test]
fn the_usage_card_reads_from_the_state() {
    let state: State = read("state.json");
    let [first, second] = state.usage.as_slice() else {
        panic!("two accounts")
    };
    assert!(first.best && first.can_be_pinned());
    assert_eq!(first.used_7d, Some(82.0));
    assert_eq!(first.places, ["Linux PC", "Mac"]);
    assert!(!second.logged_in && !second.has_numbers());
    let account = state.assistant_account.unwrap();
    assert_eq!(account.on.as_deref(), Some("a-1"));
    assert!(!account.is_pinned());
    assert_eq!(account.rooms[0].room, RoomId::MAIN);
}

#[test]
fn pinning_an_account_answers_with_the_choice_and_the_use() {
    let request: UseAccountRequest = assert_round_trip("use_account_request.json");
    assert_eq!(request, UseAccountRequest::pin("a-2"));
    let reply: UseAccountReply = assert_writes_shape("use_account_reply.json", &[]);
    assert!(reply.account.is_pinned());
    assert_eq!(reply.account.waits.as_ref().map(Vec::len), Some(1));
    assert_eq!(reply.usage.len(), 1);
    let check: UsageCheckReply = assert_round_trip("usage_check_reply.json");
    assert!(check.ok && !check.asked);
}
