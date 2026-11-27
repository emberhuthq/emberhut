//! Check-ins and watches through the MCP tools, and the wake rule that turns them (and other
//! events) into turns: `wake_me_at`, `list_checkins`, `cancel_checkin`, `watch`, `watch_note`,
//! `list_watches`, `stop_watch`.

use emberhut_api::state::State;
use emberhut_api::wake::{Checkin, Watch};
use emberhut_core::RoomId;
use emberhut_core::event::NewEvent;
use serde_json::{Value, json};

use crate::common::{TestApp, text};

#[expect(
    clippy::unwrap_used,
    reason = "a test helper: a failure fails the test"
)]
async fn state(app: &TestApp) -> State {
    app.state.state().state(RoomId::MAIN).await.unwrap()
}

/// An app whose routines are none, so a tick wakes only for what a test sets up.
#[expect(
    clippy::unwrap_used,
    reason = "a test helper: a failure fails the test"
)]
async fn quiet_app() -> TestApp {
    let app = TestApp::new().await;
    app.state.routines().set(Some(&[]), None).await.unwrap();
    app
}

#[tokio::test]
async fn a_check_in_is_scheduled_listed_and_cancelled() {
    let app = quiet_app().await;
    let scheduled = app
        .call_tool(
            "wake_me_at",
            json!({ "minutes": 30, "why": "is the build done?" }),
        )
        .await;
    assert_ne!(scheduled.is_error, Some(true));
    let at = text(&scheduled)
        .strip_prefix("Check-in at ")
        .and_then(|rest| rest.strip_suffix('.'))
        .unwrap()
        .to_owned();
    let when: jiff::Timestamp = at.parse().unwrap();
    let ahead = when.duration_since(app.clock_now()).as_secs();
    assert!((30 * 60 - 1..=30 * 60).contains(&ahead), "{ahead}");

    let listed = app.call_tool("list_checkins", json!({})).await;
    let checkins: Vec<Checkin> = serde_json::from_str(text(&listed)).unwrap();
    assert_eq!(checkins.len(), 1);
    assert_eq!(checkins[0].why, "is the build done?");
    assert_eq!(checkins[0].room, None, "Main's");
    assert_eq!(state(&app).await.checkins, checkins);

    let refused = app
        .call_tool("wake_me_at", json!({ "minutes": 0, "why": "x" }))
        .await;
    assert_eq!(refused.is_error, Some(true), "under a minute is refused");

    let missing = app
        .call_tool(
            "cancel_checkin",
            json!({ "at": "2000-01-01T00:00:00.000Z" }),
        )
        .await;
    assert_eq!(text(&missing), "No such check-in.");
    let cancelled = app.call_tool("cancel_checkin", json!({ "at": at })).await;
    assert_eq!(text(&cancelled), "Cancelled.");
    assert!(state(&app).await.checkins.is_empty(), "none");
}

#[tokio::test]
async fn a_check_in_that_comes_due_wakes_its_room_at_once() {
    let app = quiet_app().await;
    app.assistant.answers(&[], Some("Checked: it is done."));
    app.call_tool("wake_me_at", json!({ "minutes": 5, "why": "the deploy" }))
        .await;
    app.state.wake().tick().await.unwrap();
    app.settle().await;
    assert!(app.assistant.turns().is_empty(), "not due yet");

    app.advance_minutes(6);
    app.state.wake().tick().await.unwrap();
    app.settle().await;
    let turns = app.assistant.turns();
    assert_eq!(turns.len(), 1);
    assert_eq!(
        turns[0].prompt,
        "Event: - [checkin] Your check-in: the deploy"
    );
    assert!(!turns[0].from_owner);
    let state = state(&app).await;
    assert!(state.checkins.is_empty(), "a check-in that fired is gone");
    assert_eq!(state.pending, 0);
    let texts: Vec<&str> = state.chat.iter().map(|line| line.text.as_str()).collect();
    assert_eq!(
        texts,
        [
            "- [checkin] Your check-in: the deploy",
            "Checked: it is done."
        ]
    );
}

#[tokio::test]
async fn ordinary_events_gather_into_one_wake() {
    let app = quiet_app().await;
    app.assistant.answers(&[], None);
    let events = emberhut_db::Database::connect(&app.url)
        .await
        .unwrap()
        .events();
    for text in ["first", "second"] {
        events
            .add(NewEvent::new(RoomId::MAIN, "note", text), app.clock_now())
            .await
            .unwrap();
    }
    app.state.wake().tick().await.unwrap();
    app.settle().await;
    assert!(app.assistant.turns().is_empty(), "none");
    assert_eq!(state(&app).await.pending, 2);
    app.advance_minutes(2);
    app.state.wake().tick().await.unwrap();
    app.settle().await;
    let turns = app.assistant.turns();
    assert_eq!(turns.len(), 1);
    assert_eq!(turns[0].prompt, "Event: - [note] first\n- [note] second");
}

#[tokio::test]
async fn a_wake_reads_ten_events_and_leaves_the_rest_for_the_next() {
    let app = quiet_app().await;
    app.assistant.answers(&[], None);
    let events = emberhut_db::Database::connect(&app.url)
        .await
        .unwrap()
        .events();
    for n in 1..=12 {
        events
            .add(
                NewEvent::new(RoomId::MAIN, "note", format!("event {n}")),
                app.clock_now(),
            )
            .await
            .unwrap();
    }
    app.state.wake().tick().await.unwrap();
    app.settle().await;
    let turns = app.assistant.turns();
    assert_eq!(turns.len(), 1, "ten waiting wake the room at once");
    let lines: Vec<&str> = turns[0].prompt.lines().collect();
    assert_eq!(lines.len(), 10);
    assert_eq!(lines[0], "Event: - [note] event 1");
    assert_eq!(lines[9], "- [note] event 10");
    assert_eq!(state(&app).await.pending, 2);

    app.advance_minutes(2);
    app.state.wake().tick().await.unwrap();
    app.settle().await;
    let turns = app.assistant.turns();
    assert_eq!(turns.len(), 2);
    assert_eq!(
        turns[1].prompt,
        "Event: - [note] event 11\n- [note] event 12"
    );
    assert_eq!(state(&app).await.pending, 0);
}

#[tokio::test]
async fn a_watch_reminds_its_room_to_look_until_it_is_stopped() {
    let app = quiet_app().await;
    app.assistant.answers(&[], None);
    let started = app
        .call_tool(
            "watch",
            json!({ "title": "The release build", "what": "the build on the laptop",
                "every_minutes": 10, "hours": 2 }),
        )
        .await;
    assert_eq!(
        text(&started),
        "Watch #1 started; it checks every 10 min for up to 2 h."
    );
    let listed = app.call_tool("list_watches", json!({})).await;
    let watches: Vec<Watch> = serde_json::from_str(text(&listed)).unwrap();
    assert_eq!(watches.len(), 1);
    assert_eq!(watches[0].title.as_deref(), Some("The release build"));
    assert_eq!(state(&app).await.watches, watches);

    let noted = app
        .call_tool("watch_note", json!({ "id": 1, "note": "Compiling, 40%." }))
        .await;
    assert_eq!(text(&noted), "Noted.");
    let nothing = app
        .call_tool("watch_note", json!({ "id": 9, "note": "x" }))
        .await;
    assert_eq!(nothing.is_error, Some(true));
    assert_eq!(text(&nothing), "no watch 9");

    app.advance_minutes(11);
    app.state.wake().tick().await.unwrap();
    // A watch's look is ordinary news: it gathers, then wakes.
    app.advance_minutes(2);
    app.state.wake().tick().await.unwrap();
    app.settle().await;
    let turns = app.assistant.turns();
    assert_eq!(turns.len(), 1);
    assert!(
        turns[0].prompt.starts_with(
            "Event: - [check] Watch #1 (The release build): look now. What to watch: the \
             build on the laptop. Last time ("
        ),
        "{}",
        turns[0].prompt
    );
    assert!(turns[0].prompt.contains("Compiling, 40%."));
    let watch = &state(&app).await.watches[0];
    assert!(watch.check_at.is_some());

    let stopped = app.call_tool("stop_watch", json!({ "id": 1 })).await;
    assert_eq!(text(&stopped), "Stopped.");
    assert!(state(&app).await.watches.is_empty(), "none");
    let did: Vec<String> = state(&app).await.did.into_iter().map(|d| d.text).collect();
    assert!(did.contains(&"Stopped watching The release build".to_owned()));
    assert!(did.contains(&"Started watching The release build".to_owned()));
}

#[tokio::test]
async fn a_watch_past_its_time_ends_and_says_so() {
    let app = quiet_app().await;
    app.assistant.answers(&[], None);
    app.call_tool(
        "watch",
        json!({ "title": "Short", "what": "the queue", "every_minutes": 5, "hours": 0.25 }),
    )
    .await;
    app.advance_minutes(16);
    app.state.wake().tick().await.unwrap();
    app.state.wake().tick().await.unwrap();
    app.settle().await;
    assert!(state(&app).await.watches.is_empty(), "none");
    let turns = app.assistant.turns();
    assert_eq!(turns.len(), 1);
    assert_eq!(
        turns[0].prompt,
        "Event: - [watch] Watch #1 ended (time limit): the queue. Last seen: nothing"
    );
}

#[tokio::test]
async fn paused_nothing_wakes() {
    let app = quiet_app().await;
    app.assistant.answers(&[], None);
    app.state.wake().set_on(false).await.unwrap();
    app.call_tool("wake_me_at", json!({ "minutes": 1, "why": "x" }))
        .await;
    app.advance_minutes(5);
    app.state.wake().tick().await.unwrap();
    app.settle().await;
    assert!(app.assistant.turns().is_empty(), "none");
    let state = state(&app).await;
    assert!(!state.on);
    assert_eq!(state.checkins.len(), 1, "kept for when it is on again");
    let value: Value = serde_json::to_value(&state).unwrap();
    assert_eq!(value["on"], false);
}

#[tokio::test]
async fn a_followed_thread_that_ends_wakes_main() {
    let app = quiet_app().await;
    app.assistant.answers(&[], None);
    let listed = |status: &str, minute: u32| emberhut_server::services::coding_app::ListedThread {
        id: "mcp:a".to_owned(),
        title: "Fix the build".to_owned(),
        project: "Example".to_owned(),
        status: status.to_owned(),
        updated_at: format!("2026-11-24T21:{minute:02}:00.000Z"),
        parent: None,
    };
    app.coding_app.on(&["laptop"]);
    app.coding_app
        .thread("laptop", listed("running", 1), Vec::new());
    app.state
        .threads()
        .follow("laptop", "mcp:a", "the build fix")
        .await
        .unwrap();
    assert_eq!(app.state.wake().follow_threads().await.unwrap(), 0);
    app.coding_app
        .thread("laptop", listed("completed", 2), Vec::new());
    assert_eq!(app.state.wake().follow_threads().await.unwrap(), 1);
    app.state.wake().tick().await.unwrap();
    app.settle().await;
    let turns = app.assistant.turns();
    assert_eq!(turns.len(), 1);
    assert!(
        turns[0].prompt.starts_with(
            "Event: - [thread] The thread \"Fix the build\" on laptop you followed (the build \
             fix) is completed."
        ),
        "{}",
        turns[0].prompt
    );
}

#[tokio::test]
async fn an_unanswered_message_is_sent_again_once_and_a_stopped_one_never() {
    let app = quiet_app().await;
    app.assistant.answers(&[], None);
    app.assistant.fails();
    let talk = app.state.talk();
    talk.talk(RoomId::MAIN, "Book the table".to_owned(), None)
        .await
        .unwrap();
    app.settle().await;
    app.state.wake().tick().await.unwrap();
    app.settle().await;
    assert_eq!(app.assistant.turns().len(), 1, "not a minute old yet");

    app.assistant.answers(&[], Some("Booked."));
    app.advance_minutes(2);
    app.state.wake().tick().await.unwrap();
    app.settle().await;
    let turns = app.assistant.turns();
    assert_eq!(turns.len(), 2);
    assert_eq!(turns[1].prompt, "Book the table");
    assert!(turns[1].from_owner);
    app.advance_minutes(2);
    app.state.wake().tick().await.unwrap();
    app.settle().await;
    assert_eq!(
        app.assistant.turns().len(),
        2,
        "answered now, and never twice"
    );

    // A message the owner stopped is settled.
    app.assistant.answers(&[], None);
    talk.talk(RoomId::MAIN, "Wrong message".to_owned(), None)
        .await
        .unwrap();
    app.settle().await;
    talk.interrupt(RoomId::MAIN).await.unwrap();
    app.advance_minutes(2);
    app.state.wake().tick().await.unwrap();
    app.settle().await;
    assert_eq!(app.assistant.turns().len(), 3);
}

/// A morning brief every day and a standing job on Saturdays.
async fn two_routines(app: &TestApp) -> Result<(), emberhut_db::DbError> {
    use emberhut_server::services::routines::Routine;

    let routines = [
        Routine {
            at: "08:00".to_owned(),
            what: "Morning brief: the day ahead".to_owned(),
            day: None,
        },
        Routine {
            at: "20:00".to_owned(),
            what: "Standing job, Saturday: plan the week".to_owned(),
            day: Some(6),
        },
    ];
    app.state.routines().set(Some(&routines), None).await
}

#[tokio::test]
async fn the_scheduled_page_lists_the_watches_check_ins_and_routines() {
    use emberhut_api::scheduled::ScheduledPage;
    use emberhut_core::task::Who;
    use emberhut_server::services::wake::NewWatch;

    use crate::common::Call;

    let app = TestApp::new().await;
    two_routines(&app).await.unwrap();
    let task = app
        .state
        .tasks()
        .create(
            emberhut_db::NewTask {
                title: "Ship the build".to_owned(),
                ..emberhut_db::NewTask::default()
            },
            Who::Assistant,
        )
        .await
        .unwrap();
    let wake = app.state.wake();
    let watch = wake
        .add_watch(
            NewWatch {
                title: "The build".to_owned(),
                thread: None,
                what: "whether the build passed".to_owned(),
                every_minutes: 30.0,
                hours: 4.0,
                note: "running".to_owned(),
            },
            RoomId::MAIN,
        )
        .await
        .unwrap();
    let checkin = wake
        .wake_me_at(
            90,
            &format!("look at {} again", task.id.key()),
            RoomId::MAIN,
        )
        .await
        .unwrap();

    let session = app.sign_in().await;
    let page: ScheduledPage = app
        .call(Call::get("/api/scheduled").token(&session))
        .await
        .read_as();
    let watched = page
        .items
        .iter()
        .find(|i| i.kind.as_str() == "watch")
        .unwrap();
    assert_eq!(
        watched.watch.map(emberhut_api::ids::WatchId::get),
        Some(watch)
    );
    assert_eq!(watched.repeat.as_deref(), Some("every 30 min"));
    assert_eq!(watched.note.as_deref(), Some("running"));
    let checked = page
        .items
        .iter()
        .find(|i| i.kind.as_str() == "checkin")
        .unwrap();
    assert_eq!(checked.checkin.as_ref(), Some(&checkin));
    assert_eq!(checked.next_at.as_ref(), Some(&checkin));
    assert_eq!(checked.tasks.len(), 1, "the task it names");
    assert_eq!(checked.tasks[0].title, "Ship the build");
    let brief = page
        .items
        .iter()
        .find(|i| i.title == "Morning brief")
        .unwrap();
    assert_eq!(brief.kind.as_str(), "routine");
    assert_eq!(brief.repeat.as_deref(), Some("daily 08:00"));
    let weekly = page
        .items
        .iter()
        .find(|i| i.title == "Saturday: plan the week")
        .unwrap();
    assert_eq!(weekly.repeat.as_deref(), Some("weekly, Sat 20:00"));
    let now = emberhut_api::time::Timestamp::new(
        app.clock_now()
            .strftime("%Y-%m-%dT%H:%M:%S%.3fZ")
            .to_string(),
    );
    for item in &page.items {
        assert!(
            item.next_at
                .as_ref()
                .is_some_and(|at| at.as_str() > now.as_str()),
            "{} runs next later on",
            item.id
        );
    }
}
