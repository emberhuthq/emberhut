//! What the runner (`apps/runner`) sends, recorded by its own tests in `fixtures/runner/` with its
//! times, ids and folders made stable, read with the contract's types: a transcript with every
//! kind of step, the live stream, a new session, a reply, a settings change and the pickers.

use emberhut_api::ids::{SessionId, TurnId};
use emberhut_api::runner::actions::SentToSession;
use emberhut_api::runner::new_session::{BranchList, FolderList, OpenedSession};
use emberhut_api::runner::steps::{Sender, StepKind};
use emberhut_api::runner::transcript::{SessionHead, SessionLive, Transcript};

use crate::support::read;

/// The session the runner's tests open.
const ID: &str = "r-1a2b3c4d";

#[test]
fn the_runners_transcript_reads_with_every_kind_of_step() {
    let transcript: Transcript = read("runner/transcript.json");
    assert_eq!(transcript.session.id, SessionId::from(ID));
    assert!(transcript.session.is_direct());
    let kinds: Vec<&StepKind> = transcript.events.iter().map(|s| &s.kind).collect();
    for kind in [
        StepKind::User,
        StepKind::Thinking,
        StepKind::Text,
        StepKind::Tool,
        StepKind::Result,
        StepKind::Todos,
        StepKind::Question,
        StepKind::End,
        StepKind::System,
    ] {
        assert!(kinds.contains(&&kind), "a {kind:?} step");
    }
    let user = transcript
        .events
        .iter()
        .find(|s| s.kind == StepKind::User)
        .unwrap();
    assert_eq!(user.by, Some(Sender::Person));
    assert_eq!(user.turn.as_ref().and_then(TurnId::number), Some(1));
    let edit = transcript.events.iter().find(|s| s.diff.is_some()).unwrap();
    assert_eq!(edit.diff.as_deref(), Some("- old\n+ new"));
    assert!(
        transcript.events.windows(2).all(|w| w[0].seq < w[1].seq),
        "steps in order"
    );
}

#[test]
fn the_runners_live_stream_reads_event_by_event() {
    let events: Vec<SessionLive> = read("runner/live.json");
    assert!(
        matches!(&events[0], SessionLive::Head { session } if session.id == SessionId::from(ID))
    );
    assert!(events.iter().any(|e| matches!(
        e,
        SessionLive::Delta {
            start: Some(true),
            ..
        }
    )));
    assert!(
        events
            .iter()
            .any(|e| matches!(e, SessionLive::Think { since: Some(_), .. }))
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e, SessionLive::Tokens { .. }))
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e, SessionLive::Step { step } if step.kind == StepKind::End))
    );
}

#[test]
fn the_runners_answers_read_as_the_server_passes_them_on() {
    let opened: OpenedSession = read("runner/open_reply.json");
    assert_eq!(opened.id, SessionId::from(ID));
    assert!(opened.turn.is_none(), "opened with no message");
    assert_eq!(opened.session.unwrap().account.as_deref(), Some("a-2"));
    let sent: SentToSession = read("runner/send_reply.json");
    assert_eq!(sent.turn, Some(TurnId::from(format!("{ID}:1"))));
    assert!(!sent.queued);
    let settings: SessionHead = read("runner/settings_reply.json");
    assert_eq!(settings.effort.as_deref(), Some("max"));
    let roots: FolderList = read("runner/folders.json");
    assert_eq!(roots.recent[0].label(), "app");
    let inside: FolderList = read("runner/folders_inside.json");
    assert_eq!(inside.folders[0].label(), "docs");
    let branches: BranchList = read("runner/branches.json");
    assert_eq!(branches.current, "main");
}
