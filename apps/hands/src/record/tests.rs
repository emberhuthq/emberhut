use std::sync::atomic::Ordering;

use serde_json::json;

use super::*;
use crate::testing::{FakeAudio, FakeUploader, HandClock, eventually, server};

struct Bench {
    audio: Arc<FakeAudio>,
    uploader: Arc<FakeUploader>,
    clock: Arc<HandClock>,
    recorder: Recorder,
    calls: PathBuf,
    _dir: tempfile::TempDir,
}

fn bench(with_server: bool) -> Bench {
    let dir = tempfile::tempdir().unwrap();
    let calls = dir.path().join("calls");
    let audio = Arc::new(FakeAudio::default());
    let uploader = Arc::new(FakeUploader::default());
    let clock = Arc::new(HandClock::new());
    let recorder = Recorder::new(
        Arc::clone(&audio) as Arc<dyn AudioSource>,
        Arc::clone(&uploader) as Arc<dyn Uploader>,
        with_server.then(server),
        calls.clone(),
        Arc::clone(&clock) as Arc<dyn Clock>,
    );
    Bench {
        audio,
        uploader,
        clock,
        recorder,
        calls,
        _dir: dir,
    }
}

fn body(value: Value) -> Reply {
    let Value::Object(map) = value else {
        panic!("an object")
    };
    map
}

const ID: &str = "2026-11-26-abc123";

#[test]
fn call_ids_are_the_server_s() {
    assert!(is_call_id(ID));
    assert!(is_call_id("abc_12"));
    assert!(!is_call_id("abc"));
    assert!(!is_call_id("../../etc"));
    assert!(!is_call_id(&"a".repeat(41)));
    assert!(!is_call_id("call id"));
}

#[test]
fn a_recording_starts_stops_and_both_tracks_reach_the_server() {
    let b = bench(true);
    let started = b
        .recorder
        .start(&body(json!({ "id": ID, "title": "Weekly" })))
        .unwrap();
    assert_eq!(started.get("mic"), Some(&json!(true)));
    assert_eq!(
        b.recorder.current(),
        Some((ID.to_owned(), "Weekly".to_owned()))
    );
    let again = b.recorder.start(&body(json!({ "id": ID }))).unwrap_err();
    assert_eq!(again.status, 409);
    let status = b.recorder.status();
    assert_eq!(status.get("recording"), Some(&json!(ID)));
    b.clock.advance(std::time::Duration::from_secs(65));
    let stopped = b.recorder.stop("the assistant stopped it");
    assert_eq!(
        stopped.get("text").and_then(Value::as_str),
        Some(
            "Stopped \"Weekly\" after 1:05 (the assistant stopped it); sending it to the server \
             to be transcribed."
        )
    );
    assert!(b.audio.finished.load(Ordering::SeqCst));
    assert!(
        eventually(|| !b.calls.join(ID).exists()),
        "the call was not sent and deleted"
    );
    let sent = b.uploader.sent.lock().unwrap();
    let urls: Vec<&str> = sent.iter().map(|(url, ..)| url.as_str()).collect();
    assert_eq!(
        urls,
        [
            "http://emberhut.test:8791/api/calls/in?call=2026-11-26-abc123&track=mic&seconds=65&title=Weekly",
            "http://emberhut.test:8791/api/calls/in?call=2026-11-26-abc123&track=sys&seconds=65&title=Weekly&last=1",
        ]
    );
    assert!(sent.iter().all(|(_, token, _)| token == "device-token"));
    assert_eq!(sent[0].2, b"mic");
    assert_eq!(
        b.recorder.stop("again").get("text"),
        Some(&json!("Not recording."))
    );
}

#[test]
fn a_call_not_sent_waits_and_is_sent_again() {
    let b = bench(true);
    b.uploader.failing.store(true, Ordering::SeqCst);
    b.recorder.start(&body(json!({ "id": ID }))).unwrap();
    b.recorder.stop("done");
    std::thread::sleep(std::time::Duration::from_millis(100));
    assert!(b.calls.join(ID).join("call.json").is_file());
    let status = b.recorder.status();
    assert_eq!(status.get("waiting"), Some(&json!([ID])));
    assert!(
        status
            .get("text")
            .and_then(Value::as_str)
            .unwrap()
            .contains("Not sent to the server yet")
    );
    b.uploader.failing.store(false, Ordering::SeqCst);
    b.recorder.resend();
    assert!(eventually(|| !b.calls.join(ID).exists()));
}

#[test]
fn recording_is_refused_without_a_server_a_grant_or_a_call_id() {
    let none = bench(false);
    let refused = none.recorder.start(&body(json!({ "id": ID }))).unwrap_err();
    assert!(refused.text.contains("no server"));
    let b = bench(true);
    let bad = b
        .recorder
        .start(&body(json!({ "id": "../x" })))
        .unwrap_err();
    assert_eq!(bad.text, "id: the call's id from the server");
    b.audio.refused.store(true, Ordering::SeqCst);
    let denied = b.recorder.start(&body(json!({ "id": ID }))).unwrap_err();
    assert_eq!(denied.status, 403);
    assert_eq!(b.recorder.current(), None);
}

#[test]
fn without_the_microphone_only_the_mac_s_sound_is_recorded() {
    let b = bench(true);
    b.audio.no_mic.store(true, Ordering::SeqCst);
    let started = b.recorder.start(&body(json!({ "id": ID }))).unwrap();
    assert_eq!(started.get("mic"), Some(&json!(false)));
    assert!(
        started
            .get("text")
            .and_then(Value::as_str)
            .unwrap()
            .contains("the Mac's sound only")
    );
}

#[test]
fn the_call_s_folder_is_the_owner_s_only() {
    use std::os::unix::fs::PermissionsExt as _;
    let b = bench(true);
    b.uploader.failing.store(true, Ordering::SeqCst);
    b.recorder.start(&body(json!({ "id": ID }))).unwrap();
    b.recorder.stop("done");
    let mode = |path: &Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&b.calls), 0o700);
    assert_eq!(mode(&b.calls.join(ID)), 0o700);
    assert_eq!(mode(&b.calls.join(ID).join("sys.m4a")), 0o600);
    assert_eq!(mode(&b.calls.join(ID).join("call.json")), 0o600);
}

#[test]
fn ten_loud_seconds_of_a_track_go_to_the_live_transcript() {
    let b = bench(true);
    b.recorder.start(&body(json!({ "id": ID }))).unwrap();
    let sink = b.audio.sinks.lock().unwrap()[0].clone();
    sink(Track::Sys, 0.0, 100.0, &[0.5; 999]);
    sink(Track::Sys, 9.99, 100.0, &[0.5; 1]);
    assert!(eventually(|| !b.uploader.sent.lock().unwrap().is_empty()));
    let sent = b.uploader.sent.lock().unwrap();
    assert_eq!(
        sent[0].0,
        "http://emberhut.test:8791/api/calls/live?call=2026-11-26-abc123&track=sys&t=0.0"
    );
    assert_eq!(&sent[0].2[0..4], b"RIFF");
}

#[test]
fn a_call_the_server_refuses_is_set_aside() {
    let b = bench(true);
    b.uploader.failing.store(true, Ordering::SeqCst);
    *b.uploader.status.lock().unwrap() = Some(400);
    b.recorder.start(&body(json!({ "id": ID }))).unwrap();
    b.recorder.stop("done");
    let aside = b.calls.with_file_name("calls-unsent").join(ID);
    assert!(
        eventually(|| aside.join("call.json").is_file()),
        "not set aside"
    );
    assert!(!b.calls.join(ID).exists());
    let status = b.recorder.status();
    assert_eq!(status.get("waiting"), Some(&json!([])));
    assert!(
        status
            .get("text")
            .and_then(Value::as_str)
            .unwrap()
            .contains("Set aside")
    );
}

#[test]
fn a_second_call_of_the_same_id_set_aside_gets_a_name_of_its_own() {
    let b = bench(true);
    b.uploader.failing.store(true, Ordering::SeqCst);
    *b.uploader.status.lock().unwrap() = Some(400);
    let aside = b.calls.with_file_name("calls-unsent");
    for _ in 0..2 {
        b.recorder.start(&body(json!({ "id": ID }))).unwrap();
        b.recorder.stop("done");
        assert!(eventually(|| !b.calls.join(ID).exists()));
    }
    assert!(aside.join(ID).is_dir());
    assert!(aside.join(format!("{ID}-2")).is_dir());
}

#[test]
fn a_call_that_keeps_failing_counts_its_tries_and_is_set_aside() {
    let b = bench(true);
    b.uploader.failing.store(true, Ordering::SeqCst);
    *b.uploader.status.lock().unwrap() = Some(502);
    b.recorder.start(&body(json!({ "id": ID }))).unwrap();
    b.recorder.stop("done");
    let file = b.calls.join(ID).join("call.json");
    let tries = || {
        fs::read(&file)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
            .and_then(|meta| meta.get("tries").and_then(Value::as_u64))
    };
    assert!(eventually(|| tries() == Some(1)));
    let mut meta: Value = serde_json::from_slice(&fs::read(&file).unwrap()).unwrap();
    meta["tries"] = json!(MAX_TRIES - 1);
    fs::write(&file, meta.to_string()).unwrap();
    b.recorder.resend();
    let aside = b.calls.with_file_name("calls-unsent").join(ID);
    assert!(eventually(|| aside.is_dir()));
}

#[test]
fn a_network_failure_or_a_busy_server_is_not_permanent() {
    let error = |status| UploadError {
        status,
        text: String::new(),
    };
    assert!(!error(None).permanent());
    assert!(!error(Some(502)).permanent());
    assert!(!error(Some(429)).permanent());
    assert!(!error(Some(408)).permanent());
    assert!(error(Some(400)).permanent());
    assert!(error(Some(404)).permanent());
    // A sign-in the owner can still fix.
    assert!(!error(Some(401)).permanent());
    assert!(!error(Some(403)).permanent());
}

#[test]
fn a_lost_capture_is_reported_to_whoever_listens() {
    let b = bench(true);
    let told = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag = Arc::clone(&told);
    b.recorder
        .when_lost(move |number| flag.store(number == 1, Ordering::SeqCst));
    b.recorder.start(&body(json!({ "id": ID }))).unwrap();
    let lost = b.audio.losts.lock().unwrap()[0].clone();
    lost();
    assert!(told.load(Ordering::SeqCst));
}
