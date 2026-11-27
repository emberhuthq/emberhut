//! The calendar: `GET /api/calendar` over a fake Mac (its events by day, kept for when the Mac
//! sleeps, their kinds), and the `calendar`, `reminders`, `add_event` and `add_reminder` tools.
#![expect(
    clippy::unwrap_used,
    reason = "test helpers: a failure here fails the test that called them"
)]

use emberhut_api::calendar::CalendarPage as CalendarView;
use emberhut_api::paths;
use emberhut_db::Database;
use jiff::{SignedDuration, Timestamp};
use serde_json::{Value, json};

use crate::common::{Call, TestApp, text};
use crate::fake_personal::Fakes;

/// An app whose clock reads Wednesday 2026-10-21, 10:00 UTC.
async fn app_on_a_wednesday(fakes: &Fakes) -> TestApp {
    let app = TestApp::with_fakes(fakes).await;
    let wednesday: Timestamp = "2026-10-21T10:00:00Z".parse().unwrap();
    app.clock.set(wednesday);
    app
}

fn week() -> Value {
    json!([
        {
            "title": "Standup", "start": "2026-10-21T09:00:00Z", "end": "2026-10-21T09:15:00Z",
            "allDay": false, "calendar": "Work", "people": 4
        },
        {
            "title": "Deep work: the report", "start": "2026-10-22T08:00:00Z",
            "end": "2026-10-22T10:00:00Z", "allDay": false, "calendar": "Work", "people": 1
        },
        {
            "title": "Trip", "start": "2026-10-23", "end": "2026-10-25", "allDay": true,
            "calendar": "Home", "people": 0, "location": "The coast"
        },
        { "title": "No end", "start": "2026-10-22T12:00:00Z", "calendar": "Home" }
    ])
}

async fn view(app: &TestApp, query: &str) -> crate::common::Answer {
    let session = app.sign_in().await;
    app.call(Call::get(paths::CALENDAR).token(&session).query(query))
        .await
}

#[tokio::test]
pub(crate) async fn the_calendar_shows_the_mac_s_events_by_day_and_keeps_them() {
    let fakes = Fakes::default();
    let app = app_on_a_wednesday(&fakes).await;
    fakes.mac.events(Some(week()));
    let answer = view(&app, "from=2026-10-19&to=2026-10-25").await;
    assert_eq!(answer.status, 200);
    let week: CalendarView = answer.read_as();
    assert_eq!(week.from.as_str(), "2026-10-19");
    assert_eq!(week.note, None);
    assert_eq!(week.items, []);
    let shown: Vec<(&str, &str, &str)> = week
        .events
        .iter()
        .map(|e| (e.title.as_str(), e.day.as_str(), e.kind.as_str()))
        .collect();
    assert_eq!(
        shown,
        [
            ("Standup", "2026-10-21", "meeting"),
            ("Deep work: the report", "2026-10-22", "focus"),
            ("Trip", "2026-10-23", "event"),
        ],
        "by start, each once, the one without an end left out"
    );
    assert_eq!(
        week.events[2].location.as_deref(),
        Some("The coast"),
        "as the Mac gave it"
    );
    assert_eq!(week.events[0].id.len(), 16);
    assert_eq!(
        fakes.mac.reads(),
        [(5, "2026-10-19".to_owned(), "2026-10-25".to_owned())]
    );

    // A day the trip covers, on its own: kept on every day of it.
    let saturday: CalendarView = view(&app, "from=2026-10-24&to=2026-10-24").await.read_as();
    assert_eq!(saturday.events[0].title, "Trip");
    let db = Database::connect(&app.url).await.unwrap();
    assert_eq!(
        db.calendar()
            .between("2026-10-23", "2026-10-24")
            .await
            .unwrap()
            .len(),
        2
    );

    // The Mac sleeps: what was last seen, with a note, once the minute's read is over.
    fakes.mac.events(None);
    let cached: CalendarView = view(&app, "from=2026-10-19&to=2026-10-25").await.read_as();
    assert_eq!(cached.events.len(), 3);
    assert_eq!(
        cached.note, None,
        "the same range within a minute is not read again"
    );
    app.clock.advance(SignedDuration::from_mins(2));
    let asleep: CalendarView = view(&app, "from=2026-10-19&to=2026-10-25").await.read_as();
    assert_eq!(asleep.events.len(), 3);
    assert_eq!(
        asleep.note.as_deref(),
        Some("Your Mac didn't answer (the Mac is asleep); showing what was last seen.")
    );
}

#[tokio::test]
pub(crate) async fn a_range_the_calendar_doesn_t_take_is_400() {
    let fakes = Fakes::default();
    let app = app_on_a_wednesday(&fakes).await;
    for (query, error) in [
        (
            "from=2026-10-25&to=2026-10-19",
            "from and to must be dates like 2026-10-19, to on or after from",
        ),
        (
            "",
            "from and to must be dates like 2026-10-19, to on or after from",
        ),
        ("from=2026-01-01&to=2026-03-05", "at most 62 days at a time"),
    ] {
        let refused = view(&app, query).await;
        assert_eq!(refused.status, 400, "{query}");
        assert_eq!(refused.json(), json!({ "error": error }));
    }
    assert_eq!(fakes.mac.reads(), []);
}

#[tokio::test]
async fn days_before_today_are_read_only_from_a_mac_that_reads_them() {
    let fakes = Fakes::default();
    let app = app_on_a_wednesday(&fakes).await;
    fakes.mac.events(Some(json!([])));
    let past: CalendarView = view(&app, "from=2026-10-01&to=2026-10-07").await.read_as();
    assert_eq!((past.events.len(), past.note), (0, None));
    assert_eq!(
        fakes.mac.reads(),
        [],
        "an older helper reads from today on only"
    );

    // A helper that answers with an earlier day reads `from`: earlier ranges are read too.
    fakes.mac.events(Some(json!([{
        "title": "Review", "start": "2026-10-20T15:00:00Z", "end": "2026-10-20T16:00:00Z",
        "calendar": "Work", "people": 2
    }])));
    let this_week: CalendarView = view(&app, "from=2026-10-19&to=2026-10-25").await.read_as();
    assert_eq!(this_week.events[0].day.as_str(), "2026-10-20");
    let earlier = view(&app, "from=2026-10-12&to=2026-10-18").await;
    assert_eq!(earlier.status, 200);
    assert_eq!(fakes.mac.reads().len(), 2);
}

#[tokio::test]
async fn the_tools_read_the_calendar_and_reminders_and_add_to_them() {
    let fakes = Fakes::default();
    let app = app_on_a_wednesday(&fakes).await;

    let nothing = app.call_tool("calendar", json!({ "days": 3 })).await;
    assert_eq!(
        text(&nothing),
        "The Mac didn't answer and there is no earlier copy."
    );
    fakes.mac.events(Some(week()));
    let read = app.call_tool("calendar", json!({ "days": 3 })).await;
    assert_eq!(serde_json::from_str::<Value>(text(&read)).unwrap(), week());
    assert_eq!(
        fakes.mac.reads().last(),
        Some(&(3, "2026-10-21".to_owned(), "2026-10-24".to_owned()))
    );
    fakes.mac.events(None);
    let copy = app.call_tool("calendar", json!({ "days": 3 })).await;
    let copy: Value = serde_json::from_str(text(&copy)).unwrap();
    assert!(
        copy["note"]
            .as_str()
            .unwrap()
            .starts_with("The Mac is asleep; this is from 2026-10-21T10:00:00")
    );
    assert_eq!(copy["data"], week());
    let too_far = app.call_tool("calendar", json!({ "days": 61 })).await;
    assert_eq!(too_far.is_error, Some(true));

    fakes.mac.reminders(Some(
        json!([{ "title": "Milk", "due": null, "list": "Home" }]),
    ));
    let reminders = app.call_tool("reminders", json!({})).await;
    assert!(text(&reminders).contains("Milk"));

    let added = app
        .call_tool(
            "add_event",
            json!({
                "title": "Write the report", "start": "2026-10-21T14:00:00Z",
                "end": "2026-10-21T15:00:00Z", "location": null
            }),
        )
        .await;
    assert_eq!(text(&added), "Added to Home.");
    let reminder = app
        .call_tool(
            "add_reminder",
            json!({ "title": "Call the bank", "due": "2026-10-22T09:00:00+03:00" }),
        )
        .await;
    assert_eq!(text(&reminder), "true");
    let added = fakes.mac.added();
    assert_eq!(added[0].0, "event");
    assert_eq!(
        Value::Object(added[0].1.clone()),
        json!({ "title": "Write the report", "start": "2026-10-21T14:00:00Z", "end": "2026-10-21T15:00:00Z" })
    );
    assert_eq!(
        Value::Object(added[1].1.clone()),
        json!({ "title": "Call the bank", "due": "2026-10-22T09:00:00+03:00" })
    );
    let db = Database::connect(&app.url).await.unwrap();
    let log: Vec<String> = db
        .log()
        .recent(5)
        .await
        .unwrap()
        .into_iter()
        .map(|l| l.text)
        .collect();
    assert_eq!(log, ["Reminder: Call the bank", "Event: Write the report"]);

    // The block the assistant added shows as focus time.
    fakes.mac.events(Some(json!([{
        "title": "Write the report", "start": "2026-10-21T14:00:00.000Z",
        "end": "2026-10-21T15:00:00.000Z", "calendar": "Home", "people": 1
    }])));
    let today: CalendarView = view(&app, "from=2026-10-21&to=2026-10-21").await.read_as();
    assert_eq!(today.events[0].kind.as_str(), "focus");
}

/// A week of the owner's work: project Launch due Saturday (hard) with one open task, a task
/// due Friday, an ask to decide by Thursday, and a task pinned to Today.
async fn a_week_of_work(tasks: &emberhut_server::services::tasks::TaskService) {
    use emberhut_core::task::ask::OptionIn;
    use emberhut_core::task::board::ProjectChange;
    use emberhut_core::task::{Owner, Who};
    use emberhut_db::NewTask;
    use emberhut_server::services::tasks::AskInput;

    let day = |text: &str| text.parse::<jiff::civil::Date>().unwrap();
    let launch = tasks
        .set_project(
            None,
            ProjectChange {
                name: Some("Launch".to_owned()),
                deadline: Some(Some(day("2026-10-24"))),
                hard: Some(true),
                ..ProjectChange::default()
            },
        )
        .await
        .unwrap();
    let mine = |title: &str| NewTask {
        title: title.to_owned(),
        owner: Some(Owner::Person),
        ..NewTask::default()
    };
    tasks
        .create(
            NewTask {
                pid: Some(launch),
                ..mine("Book the hall")
            },
            Who::Person,
        )
        .await
        .unwrap();
    tasks
        .create(
            NewTask {
                due: Some(day("2026-10-23")),
                ..mine("Send the deck")
            },
            Who::Person,
        )
        .await
        .unwrap();
    let venue = tasks
        .create(mine("Pick a venue"), Who::Person)
        .await
        .unwrap();
    tasks
        .ask(
            venue.id,
            AskInput {
                question: "Which venue?".to_owned(),
                options: vec![OptionIn::Plain("The hall".to_owned())],
                action: Some("Pick the venue".to_owned()),
                decide_by: Some("2026-10-22".to_owned()),
                ..AskInput::default()
            },
            Who::Assistant,
        )
        .await
        .unwrap();
    tasks
        .create(
            NewTask {
                today: true,
                ..mine("Write the notes")
            },
            Who::Person,
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn the_calendar_has_the_tasks_deadlines_decide_by_days_and_today_s_plan() {
    let fakes = Fakes::default();
    let app = app_on_a_wednesday(&fakes).await;
    fakes.mac.events(Some(json!([])));
    a_week_of_work(app.state.tasks()).await;
    let week: CalendarView = view(&app, "from=2026-10-19&to=2026-10-25").await.read_as();
    let mut items: Vec<(String, String, String, bool)> = week
        .items
        .iter()
        .map(|i| {
            (
                i.kind.as_str().to_owned(),
                i.day.as_str().to_owned(),
                i.title.clone(),
                i.hard,
            )
        })
        .collect();
    items.sort();
    assert_eq!(
        items,
        [
            (
                "deadline".to_owned(),
                "2026-10-23".to_owned(),
                "Send the deck".to_owned(),
                false
            ),
            (
                "deadline".to_owned(),
                "2026-10-24".to_owned(),
                "Launch".to_owned(),
                true
            ),
            (
                "decide".to_owned(),
                "2026-10-22".to_owned(),
                "Pick a venue".to_owned(),
                false
            ),
            (
                "today".to_owned(),
                "2026-10-21".to_owned(),
                "Book the hall".to_owned(),
                false
            ),
            (
                "today".to_owned(),
                "2026-10-21".to_owned(),
                "Send the deck".to_owned(),
                false
            ),
            (
                "today".to_owned(),
                "2026-10-21".to_owned(),
                "Write the notes".to_owned(),
                false
            ),
        ],
        "today's plan is the board's Today, the ask left out of it"
    );
    let project = week.items.iter().find(|i| i.title == "Launch").unwrap();
    assert_eq!(
        project.count,
        Some(1),
        "one open task takes the project's deadline"
    );
    assert_eq!(project.task, None);
    let ask = week
        .items
        .iter()
        .find(|i| i.kind.as_str() == "decide")
        .unwrap();
    assert_eq!(
        ask.action.as_ref().and_then(Option::as_deref),
        Some("Pick the venue")
    );
    assert_eq!(ask.later, Some(false));
    let next_week: CalendarView = view(&app, "from=2026-10-26&to=2026-11-01").await.read_as();
    assert_eq!(next_week.items, [], "only the range's days");
}
