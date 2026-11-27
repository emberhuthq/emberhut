//! The assistant's standing wakes: new messages in the owner's chats reach Main (gathered for
//! the batch, or at once when one can't wait), and a meeting about an hour away gets a prep
//! task. Over the fake Mac and a moved clock; every name here is made up.
#![expect(
    clippy::unwrap_used,
    reason = "test helpers: a failure fails the test that called them"
)]

use emberhut_core::RoomId;
use emberhut_core::task::Owner;
use emberhut_db::Database;
use jiff::{SignedDuration, Timestamp};
use serde_json::{Value, json};

use crate::common::{TestApp, text};
use crate::fake_personal::Fakes;
use crate::messaging_fakes::Kit;

/// The Mac lists one chat, `wa:1` with Ann, whose messages are `messages` (text, from the
/// owner, minutes ago).
fn ann_writes(kit: &Kit, messages: &[(&str, bool, i64)]) {
    kit.mac.answers(
        "/changes",
        &json!([{
            "id": "wa:1", "name": "Ann", "source": "WhatsApp", "group": false,
            "incoming": messages.len(), "outgoing": 0
        }]),
    );
    let read: Vec<Value> = messages
        .iter()
        .map(|(said, mine, minutes)| {
            json!({
                "at": (kit.now() - SignedDuration::from_mins(*minutes)).to_string(),
                "fromMe": mine,
                "sender": if *mine { Value::Null } else { json!("Ann") },
                "text": said,
            })
        })
        .collect();
    kit.mac.answers("/read", &Value::Array(read));
}

/// Nothing new on the Mac.
fn quiet(kit: &Kit) {
    kit.mac.answers("/changes", &json!([]));
}

/// Main's waiting events, kind and text.
async fn waiting(kit: &Kit) -> Vec<(String, String)> {
    let db = Database::connect(&kit.app.url).await.unwrap();
    db.events()
        .waiting(RoomId::MAIN, 100)
        .await
        .unwrap()
        .into_iter()
        .map(|event| (event.kind, event.text))
        .collect()
}

#[tokio::test]
async fn new_messages_gather_for_the_batch_unless_one_cant_wait() {
    let kit = Kit::new().await;
    let news = kit.app.state.messaging().news().clone();

    // The first look reads the last six hours: older than the batch, so they go now.
    ann_writes(&kit, &[("see you at lunch", false, 30)]);
    news.tick().await;
    let events = waiting(&kit).await;
    assert_eq!(events.len(), 1);
    let (kind, said) = &events[0];
    assert_eq!(kind, "messages");
    assert!(
        said.contains("Ann (WhatsApp, chat wa:1)\nNo card yet.\n["),
        "{said}"
    );
    assert!(said.contains("] Ann: see you at lunch"), "{said}");
    assert!(said.contains("person_set"), "{said}");

    // Read up to now: the same messages don't come again.
    kit.later(120);
    quiet(&kit);
    news.tick().await;
    assert_eq!(waiting(&kit).await.len(), 1);

    // An ordinary message waits for the batch...
    kit.later(120);
    ann_writes(&kit, &[("the plan is in", false, 1)]);
    news.tick().await;
    assert_eq!(waiting(&kit).await.len(), 1);
    // ...and an urgent one doesn't: both go at once, as important.
    kit.later(120);
    ann_writes(
        &kit,
        &[("the plan is in", false, 3), ("call me asap", false, 1)],
    );
    news.tick().await;
    let events = waiting(&kit).await;
    assert_eq!(events.len(), 2);
    let (kind, said) = &events[1];
    assert_eq!(kind, "important");
    assert!(
        said.starts_with("Can't wait: Ann says it's urgent.\n"),
        "{said}"
    );
    assert!(said.contains("Ann: the plan is in") && said.contains("Ann: call me asap"));

    // Once the batch has passed, an ordinary one goes as messages.
    kit.later(120);
    ann_writes(&kit, &[("thanks", false, 1)]);
    news.tick().await;
    assert_eq!(waiting(&kit).await.len(), 2);
    kit.later(15 * 60);
    ann_writes(&kit, &[("thanks", false, 16)]);
    news.tick().await;
    let events = waiting(&kit).await;
    assert_eq!(events.len(), 3);
    assert_eq!(events[2].0, "messages");
}

#[tokio::test]
async fn someone_marked_important_wakes_at_once_and_a_loop_may_be_answered() {
    let kit = Kit::new().await;
    let news = kit.app.state.messaging().news().clone();
    quiet(&kit);
    news.tick().await;
    kit.app
        .call_tool(
            "person_set",
            json!({ "who": "Ann", "chat": "wa:1", "now": "waiting on the quote", "important": true }),
        )
        .await;
    kit.app
        .call_tool(
            "loop_set",
            json!({ "who": "Ann", "what": "send her the quote", "chat": "wa:1" }),
        )
        .await;

    kit.later(120);
    ann_writes(&kit, &[("here it is", true, 1), ("got it", false, 0)]);
    news.tick().await;
    let events = waiting(&kit).await;
    assert_eq!(events.len(), 1);
    let (kind, said) = &events[0];
    assert_eq!(kind, "important");
    assert!(
        said.starts_with("Can't wait: Ann wrote (marked important).\n"),
        "{said}"
    );
    assert!(said.contains("Now: waiting on the quote"), "{said}");
    assert!(said.contains("Owner: here it is"), "{said}");
    assert!(
        said.contains("The owner wrote here after loop #1 (Ann: send her the quote) opened"),
        "{said}"
    );
    let context = kit.app.call_tool("context", json!({})).await;
    assert!(
        text(&context).contains("maybeAnswered"),
        "{}",
        text(&context)
    );
}

#[tokio::test]
async fn the_mac_not_answering_keeps_its_mark_and_the_switch_turns_it_off() {
    let kit = Kit::new().await;
    let news = kit.app.state.messaging().news().clone();
    quiet(&kit);
    news.tick().await;
    let mark = kit.app.call_tool("context", json!({})).await;
    let mark: Value = serde_json::from_str(text(&mark)).unwrap();

    // The Mac doesn't answer: nothing goes, the mark stays.
    kit.later(120);
    kit.mac.raw("/changes", 503, "text/plain", b"asleep", None);
    news.tick().await;
    assert_eq!(waiting(&kit).await.len(), 0);
    let after = kit.app.call_tool("context", json!({})).await;
    let after: Value = serde_json::from_str(text(&after)).unwrap();
    assert_eq!(after["at"], mark["at"]);

    // Off: nothing is read at all.
    kit.app
        .call_tool("set_routines", json!({ "news": false }))
        .await;
    kit.later(30 * 60);
    ann_writes(&kit, &[("call me", false, 1)]);
    let asked = kit.mac.paths().len();
    news.tick().await;
    assert_eq!(waiting(&kit).await.len(), 0);
    assert_eq!(kit.mac.paths().len(), asked);
}

#[tokio::test]
async fn a_meeting_about_an_hour_away_gets_one_prep_task() {
    let fakes = Fakes::default();
    let app = TestApp::with_fakes(&fakes).await;
    let now: Timestamp = "2026-10-21T10:00:00Z".parse().unwrap();
    app.clock.set(now);
    fakes.mac.events(Some(json!([
        {
            "title": "Review with Ann", "start": "2026-10-21T11:00:00Z",
            "end": "2026-10-21T11:30:00Z", "allDay": false, "calendar": "Work", "people": 3,
            "location": "Room 2"
        },
        {
            "title": "Lunch", "start": "2026-10-21T13:00:00Z", "end": "2026-10-21T14:00:00Z",
            "allDay": false, "calendar": "Home", "people": 2
        },
        {
            "title": "Focus: the report", "start": "2026-10-21T11:00:00Z",
            "end": "2026-10-21T12:00:00Z", "allDay": false, "calendar": "Work", "people": 1
        },
        {
            "title": "Holiday", "start": "2026-10-21", "end": "2026-10-22", "allDay": true,
            "calendar": "Home", "people": 0
        }
    ])));
    let prep = app.state.meeting_prep().clone();
    assert_eq!(prep.tick().await.unwrap(), 1);
    assert_eq!(prep.tick().await.unwrap(), 0, "once per meeting");

    let db = Database::connect(&app.url).await.unwrap();
    let open = db.tasks().open().await.unwrap();
    assert_eq!(open.len(), 1);
    let task = &open[0];
    assert!(
        task.title.starts_with("Prep: Review with Ann at "),
        "{}",
        task.title
    );
    assert_eq!(task.owner, Owner::Assistant);
    assert_eq!(task.priority, 1);
    let brief = task.body.as_deref().unwrap_or_default();
    assert!(brief.contains(", Room 2, with 3 people."), "{brief}");

    // Lunch's turn comes two hours later; off, it gets none.
    app.call_tool("set_routines", json!({ "meeting_prep": false }))
        .await;
    app.clock.set(now + SignedDuration::from_hours(2));
    assert_eq!(prep.tick().await.unwrap(), 0);
}
