//! The calls store (`calls`, `call_lines`, `call_hints`), the calendar's days (`calendar_days`)
//! and the health and screen-time days (`health`, `screentime`), against a real SQLite file.

use emberhut_core::RoomId;
use emberhut_core::calls::{CallId, CallStatus, HeardLine, LiveLine, NewCall, Track, TrackKind};
use emberhut_db::DayEvent;

use crate::common::{at, database};

#[expect(
    clippy::unwrap_used,
    reason = "a test helper: a failure here fails the test that called it"
)]
fn id(text: &str) -> CallId {
    CallId::parse(text).unwrap()
}

fn new_call(text: &str, status: CallStatus) -> NewCall {
    NewCall {
        id: id(text),
        title: format!("Call {text}"),
        task: None,
        room: RoomId::MAIN,
        status,
    }
}

fn heard(track: TrackKind, t: f64, text: &str, lang: &str, sure: bool) -> HeardLine {
    HeardLine {
        line: LiveLine {
            track,
            t,
            text: text.to_owned(),
        },
        lang: Some(lang.to_owned()),
        sure,
    }
}

#[tokio::test]
async fn a_call_moves_from_recording_to_done_with_its_tracks() {
    let (_dir, db) = database().await;
    let calls = db.calls();
    calls
        .create(new_call("call-0001", CallStatus::Recording), at(0))
        .await
        .unwrap();
    assert!(
        !calls
            .create_if_missing(new_call("call-0001", CallStatus::Recording), at(1))
            .await
            .unwrap(),
        "a second start of the same call keeps the first"
    );
    let recording = calls.recording_since(at(-60)).await.unwrap().unwrap();
    assert_eq!(recording.id, id("call-0001"));
    assert!(
        calls.recording_since(at(0)).await.unwrap().is_none(),
        "only newer ones"
    );

    let tracks = vec![
        Track {
            file: "calls/call-0001/mic.m4a".to_owned(),
            who: Some("Me".to_owned()),
        },
        Track {
            file: "calls/call-0001/sys.m4a".to_owned(),
            who: Some("Others".to_owned()),
        },
    ];
    calls
        .set_tracks(&id("call-0001"), &tracks, Some(61.5))
        .await
        .unwrap();
    calls
        .set_tracks(&id("call-0001"), &tracks, None)
        .await
        .unwrap();
    calls
        .set_status(&id("call-0001"), CallStatus::Queued)
        .await
        .unwrap();
    let queued = calls.next_queued().await.unwrap().unwrap();
    assert_eq!(queued.tracks, tracks);
    assert_eq!(
        queued.seconds,
        Some(61.5),
        "a track without a length keeps the one known"
    );

    calls
        .set_status(&id("call-0001"), CallStatus::Transcribing)
        .await
        .unwrap();
    assert_eq!(calls.requeue_transcribing().await.unwrap(), 1);
    calls
        .mark_done(&id("call-0001"), "calls/call-0001/transcript.md", at(5))
        .await
        .unwrap();
    let done = calls.get(&id("call-0001")).await.unwrap().unwrap();
    assert_eq!(done.status, CallStatus::Done);
    assert_eq!(
        done.transcript.as_deref(),
        Some("calls/call-0001/transcript.md")
    );
    assert_eq!(done.done_at, Some(at(5)));
    assert!(calls.next_queued().await.unwrap().is_none());
}

#[tokio::test]
async fn old_audio_is_found_once_and_failed_calls_keep_their_reason() {
    let (_dir, db) = database().await;
    let calls = db.calls();
    for (n, text) in ["call-0001", "call-0002", "call-0003"]
        .into_iter()
        .enumerate()
    {
        let n = i64::try_from(n).unwrap_or_default();
        calls
            .create(new_call(text, CallStatus::Queued), at(n))
            .await
            .unwrap();
    }
    calls
        .mark_done(&id("call-0001"), "t.md", at(10))
        .await
        .unwrap();
    calls
        .mark_failed(&id("call-0002"), "no model", at(20))
        .await
        .unwrap();
    let failed = calls.get(&id("call-0002")).await.unwrap().unwrap();
    assert_eq!(failed.error.as_deref(), Some("no model"));

    let old: Vec<CallId> = calls
        .audio_older_than(at(30))
        .await
        .unwrap()
        .into_iter()
        .map(|call| call.id)
        .collect();
    assert_eq!(
        old,
        [id("call-0001"), id("call-0002")],
        "a queued one keeps its audio"
    );
    calls.mark_audio_gone(&id("call-0001")).await.unwrap();
    assert_eq!(calls.audio_older_than(at(30)).await.unwrap().len(), 1);

    let latest: Vec<CallId> = calls
        .latest(2)
        .await
        .unwrap()
        .into_iter()
        .map(|call| call.id)
        .collect();
    assert_eq!(latest, [id("call-0003"), id("call-0002")]);
    assert!(calls.delete(&id("call-0003")).await.unwrap());
    assert!(calls.get(&id("call-0003")).await.unwrap().is_none());
}

#[tokio::test]
async fn call_lines_and_call_hints_belong_to_their_call() {
    let (_dir, db) = database().await;
    let calls = db.calls();
    calls
        .create(new_call("call-0001", CallStatus::Recording), at(0))
        .await
        .unwrap();
    let call = id("call-0001");
    calls
        .add_line(
            &call,
            heard(TrackKind::Mic, 10.0, "hello", "en", true),
            at(1),
        )
        .await
        .unwrap();
    calls
        .add_line(
            &call,
            heard(TrackKind::Sys, 20.0, "marhaba", "ar", true),
            at(2),
        )
        .await
        .unwrap();
    calls
        .add_line(
            &call,
            heard(TrackKind::Sys, 30.0, "ahlan", "ar", true),
            at(3),
        )
        .await
        .unwrap();
    calls
        .add_line(&call, heard(TrackKind::Mic, 40.0, "ok", "en", false), at(4))
        .await
        .unwrap();
    assert_eq!(calls.call_lang(&call).await.unwrap().as_deref(), Some("ar"));
    let last: Vec<f64> = calls
        .last_lines(&call, 2)
        .await
        .unwrap()
        .iter()
        .map(|l| l.t)
        .collect();
    assert_eq!(last, [30.0, 40.0], "the latest two, oldest first");
    assert_eq!(calls.lines_after(&call, 20.0).await.unwrap().len(), 2);

    let live = calls.live_since(at(3)).await.unwrap();
    assert_eq!(live.len(), 1);
    assert_eq!(live[0].id, call);
    assert_eq!(calls.live_since(at(4)).await.unwrap(), []);

    calls
        .add_hint(&call, Some(30.0), "Ask: by when?", at(5))
        .await
        .unwrap();
    let hints = calls.hints(&call).await.unwrap();
    assert_eq!(hints[0].text, "Ask: by when?");
    assert_eq!(hints[0].at, at(5));

    calls.delete(&call).await.unwrap();
    assert!(
        calls.hints(&call).await.unwrap().is_empty(),
        "removed with the call"
    );
    assert_eq!(calls.last_lines(&call, 10).await.unwrap(), []);
}

fn event(day: &str, id: &str, title: &str) -> DayEvent {
    DayEvent {
        day: day.to_owned(),
        id: id.to_owned(),
        event: format!(r#"{{"title":"{title}"}}"#),
    }
}

#[tokio::test]
async fn calendar_days_are_replaced_only_where_the_mac_looked() {
    let (_dir, db) = database().await;
    let calendar = db.calendar();
    let both = ["2026-10-19".to_owned(), "2026-10-20".to_owned()];
    calendar
        .replace(
            &both,
            vec![
                event("2026-10-19", "a", "Standup"),
                event("2026-10-20", "b", "Review"),
                event("2026-10-21", "c", "Outside the days"),
            ],
            at(0),
        )
        .await
        .unwrap();
    assert_eq!(
        calendar
            .between("2026-10-19", "2026-10-21")
            .await
            .unwrap()
            .len(),
        2
    );

    calendar
        .replace(
            &["2026-10-20".to_owned()],
            vec![event("2026-10-20", "d", "Moved")],
            at(1),
        )
        .await
        .unwrap();
    let seen = calendar.between("2026-10-19", "2026-10-20").await.unwrap();
    assert_eq!(seen, [r#"{"title":"Standup"}"#, r#"{"title":"Moved"}"#]);
}

#[tokio::test]
async fn health_and_screentime_days_are_kept_once_per_day() {
    let (_dir, db) = database().await;
    let store = db.health_data();
    store
        .put_health(
            vec![
                ("2026-10-11".to_owned(), r#"{"steps":1}"#.to_owned()),
                ("2026-10-12".to_owned(), r#"{"steps":2}"#.to_owned()),
            ],
            at(0),
        )
        .await
        .unwrap();
    store
        .put_health(
            vec![("2026-10-12".to_owned(), r#"{"steps":3}"#.to_owned())],
            at(1),
        )
        .await
        .unwrap();
    assert_eq!(
        store.recent_health(5).await.unwrap(),
        [r#"{"steps":3}"#, r#"{"steps":1}"#]
    );
    assert_eq!(
        store.health_day("2026-10-11").await.unwrap().as_deref(),
        Some(r#"{"steps":1}"#)
    );
    assert!(store.health_day("2026-10-13").await.unwrap().is_none());

    let day = |date: &str, device: &str, minutes: u32| {
        (
            date.to_owned(),
            device.to_owned(),
            format!(r#"{{"device":"{device}","minutes":{minutes}}}"#),
        )
    };
    store
        .put_screentime(
            vec![
                day("2026-10-11", "iPhone", 100),
                day("2026-10-12", "Mac", 50),
                day("2026-10-12", "iPhone", 60),
            ],
            at(0),
        )
        .await
        .unwrap();
    store
        .put_screentime(vec![day("2026-10-12", "iPhone", 70)], at(1))
        .await
        .unwrap();
    assert_eq!(
        store.screentime_since("2026-10-12").await.unwrap(),
        [
            r#"{"device":"Mac","minutes":50}"#,
            r#"{"device":"iPhone","minutes":70}"#
        ]
    );
}
