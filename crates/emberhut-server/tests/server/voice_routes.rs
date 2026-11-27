//! Voice: a call's key (`POST /api/voice/session`), the call asking the assistant
//! (`POST /api/voice/ask`), its end (`POST /api/voice/done`, the state's `lastCall`) and
//! dictation (`POST /api/transcribe`), over a fake speech provider; the state's `voice`.

use emberhut_api::paths;
use emberhut_api::state::State;
use emberhut_api::transcribe::TranscribeReply;
use emberhut_api::voice::{ASK_TOOL, NEUTRAL_PERSON_LABEL, PERSON_LABEL, VoiceSession};
use emberhut_core::RoomId;
use emberhut_db::Database;
use serde_json::json;

use crate::common::{Call, TestApp};
use crate::fake_personal::Fakes;

async fn state(app: &TestApp, session: &emberhut_core::session::Session) -> State {
    app.call(Call::get(paths::STATE).token(session))
        .await
        .read_as()
}

#[tokio::test]
pub(crate) async fn a_voice_call_gets_a_key_asks_the_assistant_and_reports_its_end() {
    let fakes = Fakes::default();
    let app = TestApp::with_fakes(&fakes).await;
    let session = app.sign_in().await;
    assert!(state(&app, &session).await.voice);

    let key = app
        .call(Call::post("/api/voice/session").token(&session))
        .await;
    assert_eq!(key.status, 200);
    let key: VoiceSession = key.read_as();
    assert_eq!(
        (key.ok, key.key.as_str(), key.model.as_str()),
        (true, "key-1", "realtime-test")
    );
    assert!(fakes.voice.instructions()[0].contains(ASK_TOOL));

    app.assistant
        .answers(&[], Some("Your next meeting is at three."));
    let asked = app
        .call(
            Call::post(paths::VOICE_ASK)
                .token(&session)
                .json(&json!({ "request": "what's next today?" })),
        )
        .await;
    assert_eq!(asked.status, 200);
    assert_eq!(
        asked.json(),
        json!({ "ok": true, "answer": "Your next meeting is at three." })
    );
    assert_eq!(
        app.assistant.turns()[0].said.text,
        "(by voice) what's next today?"
    );

    let done = app
        .call(Call::post(paths::VOICE_DONE).token(&session).json(&json!({
            "seconds": 125.4,
            "diag": "clean audio",
            "transcript": format!(
                "{PERSON_LABEL} what's next today?\nAssistant: Your next meeting is at three."
            ),
        })))
        .await;
    assert_eq!(done.status, 200);
    assert_eq!(done.json(), json!({ "ok": true }));
    let last = state(&app, &session).await.last_call.unwrap();
    assert!(
        last.report.ends_with(" call ended (125 s): clean audio"),
        "{}",
        last.report
    );
    assert!(last.transcript.ends_with("Your next meeting is at three."));
    let db = Database::connect(&app.url).await.unwrap();
    let news = db.events().waiting(RoomId::MAIN, 100).await.unwrap();
    assert_eq!(news.len(), 1);
    assert_eq!(news[0].kind, "voice");
    assert!(
        news[0]
            .text
            .starts_with("Voice call with the user (2 min).")
    );

    // Nothing the person said: nothing to follow up.
    let quiet = app
        .call(
            Call::post(paths::VOICE_DONE)
                .token(&session)
                .json(&json!({})),
        )
        .await;
    assert_eq!(quiet.status, 200);
    assert_eq!(
        db.events().waiting(RoomId::MAIN, 100).await.unwrap().len(),
        1
    );
    let last = state(&app, &session).await.last_call.unwrap();
    assert!(last.report.ends_with("call ended (0 s): no report"));

    // The neutral label is the person's too.
    let neutral = app
        .call(Call::post(paths::VOICE_DONE).token(&session).json(&json!({
            "seconds": 30,
            "transcript": format!("Assistant: Hello.\n{NEUTRAL_PERSON_LABEL} remind me at five"),
        })))
        .await;
    assert_eq!(neutral.status, 200);
    let news = db.events().waiting(RoomId::MAIN, 100).await.unwrap();
    assert_eq!(news.len(), 2);
    assert!(news[1].text.contains("remind me at five"));
}

#[tokio::test]
pub(crate) async fn dictation_turns_a_recording_into_text_up_to_25_mib() {
    let fakes = Fakes::default();
    let app = TestApp::with_fakes(&fakes).await;
    let session = app.sign_in().await;
    let heard = app
        .call(
            Call::post("/api/transcribe")
                .token(&session)
                .query("name=memo.m4a&words=Ada,%20Example")
                .raw("AUDIO"),
        )
        .await;
    assert_eq!(heard.status, 200);
    let heard: TranscribeReply = heard.read_as();
    assert_eq!(heard.text, "heard: Names that may come up: Ada, Example.");
    assert_eq!(fakes.voice.heard(), [(5, "memo.m4a".to_owned())]);

    let long = "a".repeat(usize::try_from(emberhut_api::transcribe::MAX_SIZE).unwrap() + 1);
    let refused = app
        .call(Call::post(paths::TRANSCRIBE).token(&session).raw(&long))
        .await;
    assert_eq!(refused.status, 413);
    assert_eq!(
        refused.json(),
        json!({ "ok": false, "error": "recording too long" })
    );
}

#[tokio::test]
pub(crate) async fn without_a_speech_provider_voice_is_off_and_its_routes_say_why() {
    let fakes = Fakes::default();
    fakes.voice.off();
    let app = TestApp::with_fakes(&fakes).await;
    let session = app.sign_in().await;
    assert!(!state(&app, &session).await.voice);
    for call in [
        Call::post(paths::VOICE_SESSION),
        Call::post(paths::TRANSCRIBE).raw("AUDIO"),
    ] {
        let failed = app.call(call.token(&session)).await;
        assert_eq!(failed.status, 502);
        assert_eq!(
            failed.json(),
            json!({ "ok": false, "error": "no speech provider is set up" })
        );
    }
}

#[tokio::test]
pub(crate) async fn a_voice_request_the_room_cant_take_is_502_and_a_bad_body_is_400() {
    let app = TestApp::with_fakes(&Fakes::default()).await;
    let session = app.sign_in().await;
    for path in [paths::VOICE_ASK, paths::VOICE_DONE] {
        let bad = app
            .call(Call::post(path).token(&session).json(&json!({ "room": 0 })))
            .await;
        assert_eq!(bad.status, 400, "{path}");
    }
    app.state.stopping().cancel();
    let refused = app
        .call(
            Call::post(paths::VOICE_ASK)
                .token(&session)
                .json(&json!({ "request": "hello" })),
        )
        .await;
    assert_eq!(refused.status, 502);
    assert_eq!(refused.json()["ok"], false);
}

#[tokio::test]
async fn a_voice_call_s_follow_up_wakes_main_by_the_wake_rule() {
    let fakes = Fakes::default();
    let app = TestApp::with_fakes(&fakes).await;
    app.state.routines().set(Some(&[]), None).await.unwrap();
    app.assistant.answers(&[], Some("NO_REPLY"));
    let session = app.sign_in().await;
    let done = app
        .call(Call::post(paths::VOICE_DONE).token(&session).json(&json!({
            "seconds": 61,
            "transcript": format!("{PERSON_LABEL} remind me to call the bank"),
        })))
        .await;
    assert_eq!(done.status, 200);

    // A call's news gathers like any event's: not at once...
    app.state.wake().tick().await.unwrap();
    app.settle().await;
    assert!(app.assistant.turns().is_empty(), "it waits for the batch");
    // ...but once the batch's time is up, Main wakes for it.
    app.advance_minutes(2);
    app.state.wake().tick().await.unwrap();
    app.settle().await;
    let turns = app.assistant.turns();
    assert_eq!(turns.len(), 1);
    assert_eq!(turns[0].said.room, RoomId::MAIN);
    assert!(turns[0].prompt.starts_with("Event:"), "{}", turns[0].prompt);
    assert!(
        turns[0]
            .prompt
            .contains("[voice] Voice call with the user (1 min)")
    );
    assert!(turns[0].prompt.contains("remind me to call the bank"));
    let db = Database::connect(&app.url).await.unwrap();
    assert_eq!(db.events().waiting(RoomId::MAIN, 100).await.unwrap(), []);
}
