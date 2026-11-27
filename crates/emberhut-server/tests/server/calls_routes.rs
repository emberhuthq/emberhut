//! Calls: recording one on the Mac (`POST /api/calls/record`), the tracks it sends
//! (`POST /api/calls/in`), their transcription into a `call` event, the latest calls and a
//! transcript; a live call's chunks (`POST /api/calls/live`), its lines and the copilot's hints
//! (`GET /api/calls/{id}/live`); the `mac_record` and `call_transcribe` tools; the sweep of old
//! audio; and the session every one of these routes, and the voice, calendar and health
//! routes, needs.
#![expect(
    clippy::unwrap_used,
    reason = "test helpers: a failure here fails the test that called them"
)]

use emberhut_api::calls::{CallList, CallLive, ReceivedTrack};
use emberhut_api::error::ErrorBody;
use emberhut_api::paths;
use emberhut_api::state::State;
use emberhut_core::RoomId;
use emberhut_core::event::Event;
use emberhut_core::message::{Author, HistoryQuery};
use emberhut_db::Database;
use emberhut_server::config::AuthMode;
use emberhut_server::services::mac::RecorderCommand;
use jiff::SignedDuration;
use serde_json::{Value, json};

use crate::common::{Call, TestApp, text};
use crate::fake_personal::Fakes;

/// The events waiting in Main.
async fn events(app: &TestApp) -> Vec<Event> {
    let db = Database::connect(&app.url).await.unwrap();
    db.events().waiting(RoomId::MAIN, 100).await.unwrap()
}

/// Main's latest messages.
async fn main_lines(app: &TestApp) -> Vec<(Author, String)> {
    let db = Database::connect(&app.url).await.unwrap();
    db.messages()
        .page(RoomId::MAIN, HistoryQuery::default())
        .await
        .unwrap()
        .into_iter()
        .map(|message| (message.author, message.text))
        .collect()
}

/// A new call's id as the Mac's recorder makes one: today's UTC day on the app's clock, then
/// `hex`.
fn new_call_id(app: &TestApp, hex: &str) -> String {
    format!("{}-{hex}", app.clock_now().strftime("%Y-%m-%d"))
}

async fn state(app: &TestApp, session: &emberhut_core::session::Session) -> State {
    let answer = app.call(Call::get(paths::STATE).token(session)).await;
    assert_eq!(answer.status, 200);
    answer.read_as()
}

#[tokio::test]
pub(crate) async fn a_call_recorded_on_the_mac_is_transcribed_into_a_call_event() {
    let fakes = Fakes::default();
    let app = TestApp::with_fakes(&fakes).await;
    let session = app.sign_in().await;

    let started = app
        .call(
            Call::post(paths::CALLS_RECORD)
                .token(&session)
                .json(&json!({ "action": "start", "room": 1 })),
        )
        .await;
    assert_eq!(started.status, 200);
    assert_eq!(started.json(), json!({ "ok": true, "text": "Recording." }));
    let recorded = fakes.mac.recorded();
    assert_eq!(recorded[0].0, RecorderCommand::Start);
    let id = recorded[0].1["id"].as_str().unwrap().to_owned();
    assert!(
        recorded[0].1["title"]
            .as_str()
            .unwrap()
            .starts_with("Call on ")
    );

    let recording = state(&app, &session).await.recording.unwrap();
    assert_eq!(recording.id.as_str(), id);
    assert_eq!(recording.room, emberhut_api::ids::RoomId::MAIN);

    let mic = app
        .call(
            Call::post("/api/calls/in")
                .token(&session)
                .query(&format!("call={id}&track=mic"))
                .raw("MIC"),
        )
        .await;
    assert_eq!(mic.status, 200);
    let mic: ReceivedTrack = mic.read_as();
    assert_eq!(
        (mic.ok, mic.call.as_str(), mic.bytes),
        (true, id.as_str(), 3)
    );
    let sys = app
        .call(
            Call::post(paths::CALLS_IN)
                .token(&session)
                .query(&format!("call={id}&track=sys&last=1&seconds=61.5"))
                .raw("SYSTEM"),
        )
        .await;
    assert_eq!(sys.status, 200);
    app.state.calls().idle().await;

    let jobs = fakes.transcriber.jobs();
    assert_eq!(jobs.len(), 1);
    let speakers: Vec<Option<&str>> = jobs[0]
        .tracks
        .iter()
        .map(|(_, who)| who.as_deref())
        .collect();
    assert_eq!(speakers, [Some("Me"), Some("Others")]);
    let call = app
        .state
        .calls()
        .get(&emberhut_core::calls::CallId::parse(&id).unwrap())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(call.seconds, Some(61.5));

    let list = app.call(Call::get(paths::CALLS).token(&session)).await;
    assert_eq!(list.status, 200);
    let list: CallList = list.read_as();
    assert_eq!(list.calls[0].id, id);
    assert_eq!(list.calls[0].status, "done");
    assert!(
        state(&app, &session).await.recording.is_none(),
        "no longer recording"
    );

    let transcript = app
        .call(Call::get(&format!("/api/calls/{id}/transcript")).token(&session))
        .await;
    assert_eq!(transcript.status, 200);
    assert!(
        transcript
            .header("content-type")
            .starts_with("text/markdown")
    );
    assert!(transcript.text.contains("Me: we ship on Friday."));

    let news = events(&app).await;
    assert_eq!(news.len(), 1);
    assert_eq!(news[0].kind, "call");
    assert_eq!(news[0].key.as_deref(), Some(format!("call:{id}").as_str()));
    assert!(news[0].text.starts_with("Call transcript: \""));
    assert!(news[0].text.contains("Me: we ship on Friday."));

    let stopped = app
        .call(
            Call::post(paths::CALLS_RECORD)
                .token(&session)
                .json(&json!({ "action": "stop" })),
        )
        .await;
    assert_eq!(
        stopped.json(),
        json!({ "ok": true, "text": "Stopped; sending the tracks." })
    );
}

#[tokio::test]
pub(crate) async fn a_start_the_mac_refuses_leaves_no_call_and_a_bad_body_is_400() {
    let fakes = Fakes::default();
    let app = TestApp::with_fakes(&fakes).await;
    let session = app.sign_in().await;
    fakes.mac.refuses("The recorder has no microphone access");
    let refused = app
        .call(
            Call::post(paths::CALLS_RECORD)
                .token(&session)
                .json(&json!({})),
        )
        .await;
    assert_eq!(refused.status, 200);
    assert_eq!(
        refused.json(),
        json!({ "ok": false, "error": "The recorder has no microphone access" })
    );
    let list: CallList = app
        .call(Call::get(paths::CALLS).token(&session))
        .await
        .read_as();
    assert_eq!(list.calls, []);

    let bad = app
        .call(
            Call::post(paths::CALLS_RECORD)
                .token(&session)
                .json(&json!({ "room": 0 })),
        )
        .await;
    assert_eq!(bad.status, 400);
}

#[tokio::test]
pub(crate) async fn a_call_the_server_cant_keep_is_ok_false_not_500() {
    let fakes = Fakes::default();
    let app = TestApp::with_fakes(&fakes).await;
    let session = app.sign_in().await;
    // The calls' folder can't be made: a file is in its place.
    let dir = app.state.calls().dir().to_path_buf();
    if dir.is_dir() {
        std::fs::remove_dir_all(&dir).unwrap();
    }
    std::fs::write(&dir, "not a folder").unwrap();

    let answer = app
        .call(
            Call::post(paths::CALLS_RECORD)
                .token(&session)
                .json(&json!({ "action": "start" })),
        )
        .await;
    assert_eq!(answer.status, 200);
    assert_eq!(
        answer.json(),
        json!({ "ok": false, "error": "the call's files could not be written" })
    );
    assert!(fakes.mac.recorded().is_empty(), "the Mac was not asked");
    let list: CallList = app
        .call(Call::get(paths::CALLS).token(&session))
        .await
        .read_as();
    assert_eq!(list.calls, [], "no call is left recording");
}

#[tokio::test]
pub(crate) async fn a_track_needs_a_call_and_a_track_and_an_unknown_call_is_made_in_main() {
    let fakes = Fakes::default();
    let app = TestApp::with_fakes(&fakes).await;
    let session = app.sign_in().await;
    let id = new_call_id(&app, "a1b2c3");
    for query in ["track=mic", "call=abc&track=mic", "call=a1b2c3&track=both"] {
        let refused = app
            .call(
                Call::post(paths::CALLS_IN)
                    .token(&session)
                    .query(query)
                    .raw("x"),
            )
            .await;
        assert_eq!(refused.status, 400, "{query}");
        assert_eq!(
            refused.json(),
            json!({ "error": "call (its id) and track (mic or sys)" })
        );
    }

    // Only an id made as the server and the Mac's recorder make them, of today or yesterday.
    let made_up = new_call_id(&app, "zzzzzz").replace("zzzzzz", "mac-call-1");
    let old = format!(
        "{}-a1b2c3",
        (app.clock_now() - SignedDuration::from_hours(48)).strftime("%Y-%m-%d")
    );
    for call in ["mac-call-1", made_up.as_str(), old.as_str()] {
        let refused = app
            .call(
                Call::post(paths::CALLS_IN)
                    .token(&session)
                    .query(&format!("call={call}&track=mic"))
                    .raw("x"),
            )
            .await;
        assert_eq!(refused.status, 400, "{call}");
        assert_eq!(
            refused.json(),
            json!({ "error": "not a call being recorded" })
        );
    }

    fakes.transcriber.fails("Error: the model is missing");
    let taken = app
        .call(
            Call::post(paths::CALLS_IN)
                .token(&session)
                .query(&format!("call={id}&track=sys&title=Standup&last=1"))
                .raw("SYSTEM"),
        )
        .await;
    assert_eq!(taken.status, 200);
    // The call is no longer being recorded: nothing more is added to it.
    for again in [
        Call::post(paths::CALLS_IN).query(&format!("call={id}&track=mic")),
        Call::post(paths::CALLS_LIVE).query(&format!("call={id}&track=mic&t=1")),
    ] {
        let refused = app.call(again.token(&session).raw("MORE")).await;
        assert_eq!(refused.status, 400);
        assert_eq!(
            refused.json(),
            json!({ "error": "not a call being recorded" })
        );
    }
    app.state.calls().idle().await;
    let list: CallList = app
        .call(Call::get(paths::CALLS).token(&session))
        .await
        .read_as();
    assert_eq!(list.calls[0].title, "Standup");
    assert_eq!(list.calls[0].status, "failed");
    let news = events(&app).await;
    assert_eq!(
        news[0].key.as_deref(),
        Some(format!("call-failed:{id}").as_str())
    );
    assert!(
        news[0]
            .text
            .starts_with("Couldn't transcribe the call \"Standup\": Error: the model is missing.")
    );

    let none = app
        .call(Call::get(&format!("/api/calls/{id}/transcript")).token(&session))
        .await;
    assert_eq!(none.status, 404);
    assert_eq!(none.json(), json!({ "error": "no transcript" }));
}

#[tokio::test]
pub(crate) async fn a_live_call_s_chunks_become_lines_and_the_copilot_gives_hints() {
    let fakes = Fakes::default();
    let app = TestApp::with_fakes(&fakes).await;
    let session = app.sign_in().await;
    let id = new_call_id(&app, "111111");
    fakes.live.says(&[
        "We can ship it on Friday if the review is done.",
        "we can ship it on friday if the review is done",
    ]);
    for (track, t) in [("sys", "10"), ("mic", "12.04")] {
        let taken = app
            .call(
                Call::post("/api/calls/live")
                    .token(&session)
                    .query(&format!("call={id}&track={track}&t={t}&title=Weekly"))
                    .raw("WAV"),
            )
            .await;
        assert_eq!(taken.status, 200);
        assert_eq!(taken.json(), json!({ "ok": true }));
        app.state.live_calls().idle().await;
    }
    let chunks = fakes.live.chunks();
    assert!(chunks[1].path.ends_with("live/mic-120.wav"));
    assert_eq!(chunks[1].lang.as_deref(), Some("en"));
    assert_eq!(
        chunks[1].prompt,
        "We can ship it on Friday if the review is done."
    );
    assert!(!chunks[0].path.exists(), "a chunk's audio goes once heard");

    let live = app
        .call(Call::get(&format!("/api/calls/{id}/live")).token(&session))
        .await;
    assert_eq!(live.status, 200);
    let live: CallLive = live.read_as();
    assert_eq!(live.title.as_deref(), Some("Weekly"));
    assert_eq!(live.status.as_deref(), Some("recording"));
    assert_eq!(live.copilot, "waiting");
    assert_eq!(live.lines.len(), 1, "the microphone's echo is left out");
    assert_eq!(live.lines[0].who, "Others");

    fakes
        .copilot
        .replies(&["\"Promise: they ship on Friday once the review is done\""]);
    app.state.live_calls().tick().await.unwrap();
    app.state.live_calls().idle().await;
    let opened = fakes.copilot.opened();
    assert_eq!(opened[0].0, id);
    assert!(opened[0].1.contains("silent copilot") && opened[0].1.contains("Main room"));
    assert_eq!(
        fakes.copilot.said()[0],
        "[0:10–0:10]\nOthers: We can ship it on Friday if the review is done."
    );
    let live: CallLive = app
        .call(Call::get(&format!("/api/calls/{id}/live")).token(&session))
        .await
        .read_as();
    assert_eq!(live.copilot, "on");
    assert_eq!(
        live.hints[0].text,
        "Promise: they ship on Friday once the review is done"
    );
    assert_eq!(live.hints[0].t, Some(10.0));
    let lines = main_lines(&app).await;
    assert!(lines.contains(&(
        Author::Event,
        "💡 Weekly (0:10): Promise: they ship on Friday once the review is done".to_owned()
    )));

    let off = app
        .call(
            Call::post(paths::CALLS_LIVE)
                .token(&session)
                .query(&format!("call={id}&track=mic&t=20&copilot=0"))
                .raw("WAV"),
        )
        .await;
    assert_eq!(off.status, 200);
    app.state.live_calls().idle().await;
    assert_eq!(fakes.copilot.closed(), std::slice::from_ref(&id));
    let live: CallLive = app
        .call(Call::get(&format!("/api/calls/{id}/live")).token(&session))
        .await
        .read_as();
    assert_eq!(live.copilot, "off: turned off on the Mac");

    let bad = app
        .call(
            Call::post(paths::CALLS_LIVE)
                .token(&session)
                .query(&format!("call={id}&track=mic&t=soon"))
                .raw("WAV"),
        )
        .await;
    assert_eq!(bad.status, 400);
    assert_eq!(
        bad.json(),
        json!({ "error": "call, track (mic or sys) and t" })
    );
    let not_a_call = app
        .call(Call::get("/api/calls/x/live").token(&session))
        .await;
    assert_eq!(not_a_call.status, 404);
}

#[tokio::test]
async fn a_transcriber_that_cant_start_is_said_once_and_a_failing_copilot_stops() {
    let fakes = Fakes::default();
    let app = TestApp::with_fakes(&fakes).await;
    let session = app.sign_in().await;
    let id = new_call_id(&app, "222222");
    fakes
        .live
        .says(&["Hello there, everyone, thanks for joining this call today."]);
    let first = app
        .call(
            Call::post(paths::CALLS_LIVE)
                .token(&session)
                .query(&format!("call={id}&track=sys&t=1"))
                .raw("WAV"),
        )
        .await;
    assert_eq!(first.status, 200);
    app.state.live_calls().idle().await;

    fakes.copilot.fails("the account is out of credit");
    app.state.live_calls().tick().await.unwrap();
    app.state.live_calls().idle().await;
    let live: CallLive = app
        .call(Call::get(&format!("/api/calls/{id}/live")).token(&session))
        .await
        .read_as();
    assert_eq!(live.copilot, "off: the account is out of credit");
    assert!(main_lines(&app).await.iter().any(|(author, line)| {
        *author == Author::Event
            && line.starts_with("The live copilot for Call on ")
            && line.ends_with(
                "stopped (the account is out of credit); the live transcript carries on.",
            )
    }));

    fakes.live.cant_start("the model isn't there");
    for t in ["11", "21"] {
        app.call(
            Call::post(paths::CALLS_LIVE)
                .token(&session)
                .query(&format!("call={id}&track=sys&t={t}"))
                .raw("WAV"),
        )
        .await;
        app.state.live_calls().idle().await;
    }
    let down: Vec<Event> = events(&app)
        .await
        .into_iter()
        .filter(|event| event.key.as_deref() == Some(format!("live-down:{id}").as_str()))
        .collect();
    assert_eq!(down.len(), 1, "said once per call");
    assert!(down[0].text.contains("the model isn't there"));
}

#[tokio::test]
async fn the_tools_record_on_the_mac_and_transcribe_a_shared_recording() {
    let fakes = Fakes::default();
    let app = TestApp::with_fakes(&fakes).await;

    let started = app
        .call_tool(
            "mac_record",
            json!({ "action": "start", "title": " Weekly " }),
        )
        .await;
    assert_eq!(text(&started), "Recording.");
    assert_eq!(fakes.mac.recorded()[0].1["title"], "Weekly");
    let status = app
        .call_tool("mac_record", json!({ "action": "status" }))
        .await;
    let status = text(&status);
    assert!(
        status.starts_with("Not recording.\nLast calls:\n- "),
        "{status}"
    );
    assert!(status.contains("\"Weekly\" recording"), "{status}");

    fakes.mac.refuses("The Mac is locked");
    let refused = app
        .call_tool("mac_record", json!({ "action": "stop" }))
        .await;
    assert_eq!(text(&refused), "Calls: The Mac is locked");

    let uploads = app.state.uploads().dir().to_path_buf();
    tokio::fs::create_dir_all(&uploads).await.unwrap();
    let memo = uploads.join("abc-Memo.M4A");
    tokio::fs::write(&memo, b"A VOICE MEMO").await.unwrap();
    let path = memo.display().to_string();
    let queued = app
        .call_tool(
            "call_transcribe",
            json!({ "path": path, "title": "Memo with the bank" }),
        )
        .await;
    let queued = text(&queued);
    assert!(
        queued.starts_with("Transcribing Memo with the bank (0.0 MB) on this server, call "),
        "{queued}"
    );
    app.state.calls().idle().await;
    let job = fakes.transcriber.jobs().pop().unwrap();
    assert_eq!(job.tracks.len(), 1);
    assert!(job.tracks[0].0.ends_with("call.m4a"));
    assert_eq!(job.tracks[0].1, None, "one file has no speakers");
    assert!(
        events(&app)
            .await
            .iter()
            .any(|e| e.text.contains("\"Memo with the bank\""))
    );

    for outside in ["/etc/passwd", "relative/memo.m4a"] {
        let none = app
            .call_tool("call_transcribe", json!({ "path": outside, "title": "x" }))
            .await;
        assert_eq!(text(&none), format!("No audio file at {outside}."));
    }

    // The audio goes after the days it is kept; the transcript stays.
    let copied = job.tracks[0].0.clone();
    assert!(copied.exists() && memo.exists());
    app.clock.advance(SignedDuration::from_hours(24 * 15));
    assert_eq!(app.state.calls().sweep().await.unwrap(), 1);
    assert!(!copied.exists(), "the copy's audio is gone");
    assert!(!memo.exists(), "and the upload it came from");
    assert!(job.out.exists(), "the transcript is kept");
    assert_eq!(app.state.calls().sweep().await.unwrap(), 0);
}

#[tokio::test]
pub(crate) async fn the_calls_voice_calendar_and_health_routes_need_a_session() {
    let app = TestApp::new().await;
    let calls: Vec<(Call, Option<Value>)> = vec![
        (Call::post(paths::CALLS_RECORD), Some(json!({}))),
        (
            Call::post(paths::CALLS_IN).query("call=call-0001&track=mic"),
            None,
        ),
        (
            Call::post(paths::CALLS_LIVE).query("call=call-0001&track=mic&t=1"),
            None,
        ),
        (Call::get(paths::CALLS), None),
        (Call::get("/api/calls/call-0001/transcript"), None),
        (Call::get("/api/calls/call-0001/live"), None),
        (Call::post(paths::VOICE_SESSION), None),
        (
            Call::post(paths::VOICE_ASK),
            Some(json!({ "request": "hi" })),
        ),
        (Call::post(paths::VOICE_DONE), Some(json!({}))),
        (Call::post(paths::TRANSCRIBE), None),
        (
            Call::get(paths::CALENDAR).query("from=2026-10-19&to=2026-10-25"),
            None,
        ),
        (Call::post(paths::HEALTH_DAYS), Some(json!({ "days": [] }))),
        (Call::post(paths::SCREENTIME), Some(json!({ "days": [] }))),
    ];
    for (call, body) in calls {
        let call = match body {
            Some(body) => call.json(&body),
            None => call,
        };
        let refused = app.call(call).await;
        assert_eq!(refused.status, 401);
        assert_eq!(refused.json()["signIn"], true);
    }
}

#[tokio::test]
async fn in_report_mode_a_request_without_a_session_never_records_spends_or_stores() {
    let fakes = Fakes::default();
    let mac = std::sync::Arc::clone(&fakes.mac);
    let personal = fakes.dependencies();
    let app = TestApp::with_dependencies(AuthMode::Report, move |dependencies, _| {
        emberhut_server::app::Dependencies {
            mac: mac as _,
            personal,
            ..dependencies
        }
    })
    .await;
    let id = new_call_id(&app, "333333");
    let calls: Vec<(Call, Option<Value>)> = vec![
        (
            Call::post(paths::CALLS_RECORD),
            Some(json!({ "action": "start" })),
        ),
        (
            Call::post(paths::CALLS_IN).query(&format!("call={id}&track=mic&last=1")),
            None,
        ),
        (
            Call::post(paths::CALLS_LIVE).query(&format!("call={id}&track=mic&t=1")),
            None,
        ),
        (Call::post(paths::VOICE_SESSION), None),
        (
            Call::post(paths::VOICE_ASK),
            Some(json!({ "request": "send my messages" })),
        ),
        (
            Call::post(paths::VOICE_DONE),
            Some(json!({ "transcript": "Me: call my sister" })),
        ),
        (Call::post(paths::TRANSCRIBE), None),
        (
            Call::post(paths::HEALTH_DAYS),
            Some(json!({ "days": [{ "date": "2026-11-25", "steps": 1 }] })),
        ),
        (
            Call::post(paths::SCREENTIME),
            Some(json!({ "days": [{ "date": "2026-11-25", "device": "mac" }] })),
        ),
    ];
    for (call, body) in calls {
        let call = match body {
            Some(body) => call.json(&body),
            None => call.raw("AUDIO"),
        };
        let refused = app.call(call).await;
        assert_eq!(refused.status, 401, "{}", refused.text);
        assert_eq!(
            refused.read_as::<ErrorBody>(),
            ErrorBody::sign_in_required()
        );
    }
    assert!(fakes.mac.recorded().is_empty(), "the microphone stays off");
    assert!(fakes.live.chunks().is_empty(), "no chunk was heard");
    assert!(
        fakes.transcriber.jobs().is_empty(),
        "nothing was transcribed"
    );

    // Reading still passes report mode's gate, and nothing was kept.
    let list = app.call(Call::get(paths::CALLS)).await;
    assert_eq!(list.status, 200, "{}", list.text);
    assert_eq!(list.read_as::<CallList>().calls, []);
    let calendar = app
        .call(Call::get(paths::CALENDAR).query("from=2026-10-19&to=2026-10-25"))
        .await;
    assert_eq!(calendar.status, 200, "{}", calendar.text);
}
