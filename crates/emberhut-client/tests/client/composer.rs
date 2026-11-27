//! The composer's other routes: the stop button, recording a call, and dictation.

use emberhut_api::calls::RecordCallRequest;
use emberhut_api::ids::RoomId;
use emberhut_api::transcribe::{MAX_SIZE, TranscribeQuery};
use emberhut_client::Error;
use serde_json::json;

use crate::support::{Reply, TestServer, fixture, fresh};

#[test]
fn the_stop_button_interrupts_the_rooms_turn() {
    let server = TestServer::start(|_| Reply::Json(200, fixture("answer_reply.json")));
    server
        .client(fresh("r1"))
        .interrupt(RoomId::new(2))
        .unwrap();
    assert_eq!(
        server.requests_to("/api/settings")[0].json(),
        json!({ "interrupt": true, "room": 2 })
    );
}

#[test]
fn recording_a_call_reads_a_refusal_sent_with_200() {
    let server = TestServer::start(|request| match request.json()["action"].as_str() {
        Some("start") => Reply::Json(200, fixture("record_call_reply.json")),
        _ => Reply::Json(200, fixture("record_call_refused.json")),
    });
    let client = server.client(fresh("r1"));
    let started = client
        .record_call(&RecordCallRequest::start(RoomId::new(2)))
        .unwrap();
    assert!(started.ok);
    let refused = client
        .record_call(&RecordCallRequest::stop(RoomId::new(2)))
        .unwrap();
    assert!(!refused.ok);
    assert_eq!(refused.error.as_deref(), Some("the Mac isn't reachable"));
    assert_eq!(
        server.requests_to("/api/calls/record")[0].json(),
        json!({ "action": "start", "room": 2 })
    );
}

#[test]
fn dictation_sends_the_recording_raw_with_its_words() {
    let server =
        TestServer::start(|_| Reply::Json(200, json!({ "ok": true, "text": " Hello \n" })));
    let audio = b"RIFF....WAVE".to_vec();
    let said = server
        .client(fresh("r1"))
        .transcribe(
            &TranscribeQuery::new("dictation.wav", &["Emberhut"]),
            &audio,
        )
        .unwrap();
    assert_eq!(said, "Hello");
    let sent = &server.requests_to("/api/transcribe")[0];
    assert_eq!(sent.query, "name=dictation.wav&words=Emberhut");
    assert_eq!(
        sent.header("content-type"),
        Some("application/octet-stream")
    );
    assert_eq!(sent.body, audio);
}

#[test]
fn a_refused_recording_says_why() {
    let server = TestServer::start(|_| Reply::Json(413, fixture("transcribe_refused.json")));
    let error = server
        .client(fresh("r1"))
        .transcribe(&TranscribeQuery::new("dictation.wav", &["Emberhut"]), b"x")
        .unwrap_err();
    assert!(
        matches!(&error, Error::Status { status: 413, message: Some(m) } if m == "recording too long"),
        "{error:?}"
    );
}

#[test]
fn a_recording_over_the_limit_is_never_sent() {
    let server = TestServer::start(|_| Reply::Json(200, fixture("transcribe_reply.json")));
    let audio = vec![0; usize::try_from(MAX_SIZE).unwrap() + 1];
    let error = server
        .client(fresh("r1"))
        .transcribe(
            &TranscribeQuery::new("dictation.wav", &["Emberhut"]),
            &audio,
        )
        .unwrap_err();
    assert!(matches!(error, Error::TooLarge { .. }), "{error:?}");
    assert!(server.requests_to("/api/transcribe").is_empty());
}
