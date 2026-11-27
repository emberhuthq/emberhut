//! The morning brief and the other routines: at its time a routine wakes Main once that day
//! with what to do; the brief also carries what the rooms said since the day before.

use emberhut_core::RoomId;
use emberhut_server::services::routines::Routine;
use jiff::tz::TimeZone;
use serde_json::json;

use crate::common::{TestApp, text};

/// `HH:MM` a few minutes before or after now, in the server's zone.
#[expect(
    clippy::unwrap_used,
    reason = "a test helper: a failure fails the test"
)]
fn local(app: &TestApp, minutes: i64) -> String {
    let at = app
        .clock_now()
        .checked_add(jiff::SignedDuration::from_mins(minutes))
        .unwrap()
        .to_zoned_datetime(TimeZone::system());
    at.strftime("%H:%M").to_string()
}

#[tokio::test]
async fn a_routine_due_wakes_main_once_that_day_and_a_brief_carries_the_rooms() {
    let app = TestApp::new().await;
    // A day's start is midnight: keep the test's times inside one day.
    let now = app.clock_now().to_zoned_datetime(TimeZone::system());
    if now.hour() == 0 || now.hour() == 23 {
        app.advance_minutes(120);
    }
    app.assistant.answers(&[], Some("Good morning."));
    app.state
        .talk()
        .talk(RoomId::MAIN, "Remember the dentist".to_owned(), None)
        .await
        .unwrap();
    app.settle().await;
    let brief = format!("Morning brief: {}", "keep it short.");
    let saved = app
        .call_tool(
            "set_routines",
            json!({ "routines": [
                { "at": local(&app, -5), "what": brief },
                { "at": local(&app, 30), "what": "Later: not yet." },
            ] }),
        )
        .await;
    assert_eq!(text(&saved), "Saved.");

    let wake = app.state.wake();
    wake.tick().await.unwrap();
    app.settle().await;
    let turns = app.assistant.turns();
    assert_eq!(turns.len(), 2, "the message and the brief");
    let prompt = &turns[1].prompt;
    assert!(
        prompt.starts_with("Event: - [routine] Morning brief: keep it short.\nWhat was said in the rooms since yesterday:\n## Main\n"),
        "{prompt}"
    );
    assert!(prompt.contains("Owner: Remember the dentist"), "{prompt}");
    assert!(prompt.contains("Assistant: Good morning."), "{prompt}");

    // Once that day: the next ticks don't send it again.
    app.advance_minutes(5);
    wake.tick().await.unwrap();
    app.settle().await;
    assert_eq!(app.assistant.turns().len(), 2);
}

#[tokio::test]
async fn a_weekly_routine_waits_for_its_day_and_the_defaults_are_the_brief_and_the_wrap() {
    let app = TestApp::new().await;
    let routines = app.state.routines();
    let defaults = routines.routines().await.unwrap();
    assert_eq!(defaults.len(), 2);
    assert!(emberhut_core::briefs::is_morning_brief(&defaults[0].what));
    assert!(defaults[1].what.starts_with("Evening wrap"));

    let today = app
        .clock_now()
        .to_zoned_datetime(TimeZone::system())
        .weekday()
        .to_sunday_zero_offset();
    let other_day = u8::try_from((today + 1) % 7).unwrap();
    routines
        .set(
            Some(&[Routine {
                at: "00:00".to_owned(),
                what: "Weekly review".to_owned(),
                day: Some(other_day),
            }]),
            None,
        )
        .await
        .unwrap();
    app.state.wake().tick().await.unwrap();
    app.settle().await;
    assert!(app.assistant.turns().is_empty(), "not its day");
}
