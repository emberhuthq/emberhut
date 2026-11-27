//! The runner sessions: a transcript, the live stream, a reply, a new session with its folders
//! and branches, the state's agents, sessions, external threads and accounts, and an external thread's steps.

use emberhut_api::chat::FileKind;
use emberhut_api::ids::{MachineId, SessionId, TaskId, ToolCallId, TurnId};
use emberhut_api::runner::actions::{Effort, SendToSession, SentToSession};
use emberhut_api::runner::agents::AgentWho;
use emberhut_api::runner::new_session::{BranchList, FolderList, OpenSession, OpenedSession};
use emberhut_api::runner::steps::{Sender, StepKind, TodoStatus};
use emberhut_api::runner::transcript::{SessionLive, Transcript};
use emberhut_api::state::State;
use emberhut_api::threads::ThreadActivity;

use crate::support::{assert_round_trip, assert_writes_shape, from_value, read};

#[test]
fn a_transcript_reads_its_head_and_every_kind_of_step() {
    let transcript: Transcript = assert_writes_shape("runner_transcript.json", &["session.claude"]);
    let head = &transcript.session;
    assert_eq!(head.id, SessionId::from("r-1a"));
    assert!(head.is_direct() && head.is_running());
    assert_eq!(head.account_name.as_deref(), Some("Example account"));
    assert_eq!(head.tasks, [TaskId::new(12)]);
    assert!(transcript.earlier);

    let steps = &transcript.events;
    let user = &steps[0];
    assert_eq!(user.kind, StepKind::User);
    assert_eq!(user.by, Some(Sender::Person));
    assert_eq!(user.files[0].kind, FileKind::File);
    assert_eq!(user.turn.as_ref().and_then(TurnId::number), Some(3));
    assert_eq!(steps[1].ms, Some(6200));
    let tool = &steps[3];
    assert_eq!(tool.call, Some(ToolCallId::from("call-1")));
    assert_eq!(tool.summary.as_deref(), Some("cargo test"));
    assert!(steps[4].failed());
    assert_eq!(steps[5].parent, Some(ToolCallId::from("call-0")));
    assert_eq!(steps[5].diff.as_deref(), Some("- old\n+ new"));
    assert_eq!(steps[6].todos[0].status, TodoStatus::InProgress);
    let question = &steps[7].questions[0];
    assert_eq!(question.options[0].label, "The first");
    assert_eq!(question.options[1].description, None);
    assert_eq!(
        (steps[8].n, steps[8].outcome.as_deref()),
        (Some(3), Some("completed"))
    );
    assert_eq!(steps[9].kind, StepKind::System);
}

#[test]
fn a_workers_transcript_has_a_shorter_head() {
    let transcript: Transcript = read("runner_worker_transcript.json");
    let head = &transcript.session;
    assert!(head.is_running() && !head.is_direct());
    assert_eq!(head.place.as_deref(), Some("Linux PC"));
    assert!(head.account.is_none() && head.meta.is_none());
}

#[test]
fn the_live_stream_has_each_kind_of_event() {
    let events: Vec<SessionLive> = assert_writes_shape("runner_live.json", &[]);
    let SessionLive::Head { session } = &events[0] else {
        panic!("a head first");
    };
    assert_eq!(session.queued, Some(1));
    assert_eq!(session.agents[0].tools, Some(4));
    assert!(matches!(&events[1], SessionLive::Step { step } if step.seq == 17));
    assert!(matches!(
        &events[2],
        SessionLive::Delta {
            start: Some(true),
            ..
        }
    ));
    assert!(matches!(&events[3], SessionLive::Delta { text, start: None, .. } if text == "Hel"));
    assert!(matches!(
        &events[4],
        SessionLive::Think { since: Some(_), .. }
    ));
    assert!(
        matches!(&events[6], SessionLive::Tokens { tokens, .. } if (*tokens - 6000.).abs() < f64::EPSILON)
    );
    let unknown = serde_json::json!({ "type": "something new", "text": "" });
    assert!(serde_json::from_value::<SessionLive>(unknown).is_err());
}

#[test]
fn a_reply_and_its_answer_keep_their_shape() {
    let message: SendToSession = assert_round_trip("runner_send_request.json");
    assert_eq!(message.files[0].kind, FileKind::Image);
    let answer: SentToSession = assert_round_trip("runner_send_reply.json");
    assert!(answer.ok && answer.queued);
    let worker: SentToSession = from_value(serde_json::json!({ "ok": true, "queued": 2 }));
    assert!(worker.queued, "a worker counts what waits");
    let none: SentToSession = from_value(serde_json::json!({ "ok": true }));
    assert!(!none.queued && none.turn.is_none());
}

#[test]
fn a_new_session_and_its_sheet() {
    let open: OpenSession = assert_round_trip("runner_open_request.json");
    assert_eq!(open.machine, MachineId::from("runner-b"));
    assert_eq!(open.effort, Some(Effort::ExtraHigh));
    assert_eq!(
        open.worktree.unwrap().branch.as_deref(),
        Some("example-fix")
    );
    let opened: OpenedSession = read("runner_open_reply.json");
    assert_eq!(opened.id, SessionId::from("r-3c"));
    assert_eq!(
        opened.session.unwrap().branch.as_deref(),
        Some("example-fix")
    );

    let roots: FolderList = read("runner_folders.json");
    assert_eq!(roots.recent[0].label(), "work");
    assert_eq!(roots.projects[0].label(), "Example project");
    assert_eq!(roots.documents.as_deref(), Some("/home/example/Documents"));
    let inside: FolderList = read("runner_folders_inside.json");
    assert_eq!(inside.folders[0].git, Some(false));
    let branches: BranchList = assert_round_trip("runner_branches.json");
    assert_eq!(branches.current, "main");
}

#[test]
fn the_state_has_the_agents_sessions_threads_and_accounts() {
    let state: State = read("state.json");
    let lead = &state.agents_now[0];
    assert_eq!(lead.who(), AgentWho::Worker);
    assert_eq!(lead.viewer, Some(SessionId::from("w-12")));
    assert_eq!(state.agents_now[1].parent, Some(ToolCallId::from("call-1")));
    let mine = &state.my_sessions[0];
    assert!(mine.direct && mine.is_waiting() && !mine.is_running());
    assert_eq!(mine.question.as_deref(), Some("Which one should it be?"));
    assert_eq!(state.sessions[0].cwd.as_deref(), Some("/work/example"));
    let thread = &state.t3_running[0];
    assert!(thread.on_mac());
    assert_eq!(thread.place(), "Coding app on the Mac");
    let account = &state.accounts[0];
    assert!(account.is_on(&MachineId::from("runner")) && account.can_be_picked());
    assert_eq!(account.used_7d, Some(40.));
    assert!(state.usage[0].best);
    let assistant = state.assistant_account.as_ref().unwrap();
    assert_eq!(
        (assistant.on.as_deref(), assistant.mode.as_str()),
        (Some("a-1"), "auto")
    );
}

#[test]
fn a_t3_threads_steps_keep_their_shape() {
    let activity: ThreadActivity = assert_round_trip("t3_activity.json");
    assert_eq!(activity.items[0].kind_name(), "Asked");
    assert!(activity.items[1].is_command());
}
