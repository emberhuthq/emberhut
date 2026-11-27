//! The composer's other routes: the stop button, recording a call, and dictation.

use emberhut_api::calls::{CallRecording, RecordAction, RecordCallReply, RecordCallRequest};
use emberhut_api::error::{ErrorBody, OkReply};
use emberhut_api::ids::{CallId, MessageId, RoomId};
use emberhut_api::interrupt::InterruptRequest;
use emberhut_api::state::State;
use emberhut_api::talk::TalkRequest;
use emberhut_api::transcribe::{TranscribeQuery, TranscribeReply};
use serde_json::json;

use crate::support::{assert_round_trip, fixture, from_value, read, to_value};

#[test]
fn the_stop_button_round_trips_and_reads_as_ok() {
    let request: InterruptRequest = assert_round_trip("interrupt_request.json");
    assert_eq!(request, InterruptRequest::new(RoomId::new(2)));
    let reply: OkReply = read("answer_reply.json");
    assert!(reply.ok, "the state spread into the answer is skipped");
}

#[test]
fn recording_a_call_round_trips() {
    let request: RecordCallRequest = assert_round_trip("record_call_request.json");
    assert_eq!(request, RecordCallRequest::start(RoomId::new(2)));
    assert_eq!(
        to_value(&RecordCallRequest::stop(RoomId::MAIN)),
        json!({ "action": "stop", "room": 1 })
    );
    let done: RecordCallReply = assert_round_trip("record_call_reply.json");
    assert!(done.ok && done.error.is_none());
    let refused: RecordCallReply = assert_round_trip("record_call_refused.json");
    assert!(!refused.ok);
    assert_eq!(refused.error.as_deref(), Some("the Mac isn't reachable"));
    assert_eq!(
        serde_json::from_str::<RecordCallRequest>(r#"{"action":"stop"}"#).unwrap(),
        RecordCallRequest {
            action: RecordAction::Stop,
            room: RoomId::MAIN
        }
    );
}

#[test]
fn the_state_says_when_a_call_is_being_recorded() {
    let state: State = read("state.json");
    assert!(state.recording.is_none(), "the fixture has none");
    let mut body = fixture("state.json");
    body["recording"] = fixture("recording.json");
    let state: State = from_value(body);
    let recording = state.recording.unwrap();
    assert_eq!(recording.title, "Call 25 Nov 10:00");
    assert_eq!(recording.room, RoomId::new(2));
    assert_eq!(recording.id, CallId::new("c-1a2b3c"));
    let _: CallRecording = assert_round_trip("recording.json");
}

#[test]
fn dictation_reads_its_text_or_the_refusal() {
    let reply: TranscribeReply = assert_round_trip("transcribe_reply.json");
    assert_eq!(reply.text, "Look at the example files please");
    let refused: ErrorBody = assert_round_trip("transcribe_refused.json");
    assert_eq!(refused.error, "recording too long");
    assert_eq!(
        TranscribeQuery::new("dictation.wav", &["Emberhut", "Claude Code"]).to_query(),
        "name=dictation.wav&words=Emberhut%2C%20Claude%20Code"
    );
    assert_eq!(
        TranscribeQuery::new("dictation.wav", &[] as &[&str]).to_query(),
        "name=dictation.wav&words="
    );
}

#[test]
fn a_reply_names_the_message_it_answers() {
    let request = TalkRequest::text(RoomId::new(2), "Yes").with_reply_to(MessageId::new(39));
    assert_eq!(
        to_value(&request),
        json!({ "text": "Yes", "files": [], "replyTo": 39, "room": 2 })
    );
}
