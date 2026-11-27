//! Coding threads in the state (`threads`, `followed`, `t3Running`), a followed thread's end,
//! the coding-thread app's polling, its activity route, and the assistant's thread tools.

use emberhut_api::error::ErrorBody;
use emberhut_api::ids::RoomId;
use emberhut_api::state::State;
use emberhut_api::threads::{ACTIVITY, ThreadActivity};
use emberhut_core::thread::{ACTIVITY_ITEMS, CodingThread, Ending, ThreadStatus};
use emberhut_server::services::coding_app::{CodingAppError, ListedThread};
use emberhut_server::services::threads::{STATE_THREADS, ThreadError};
use jiff::SignedDuration;
use serde_json::json;

use crate::common::{Call, TestApp, text};
use crate::fake_coding_app::{FakeThread, item};

/// A runner session `id`, last changed `minutes` after the app's clock started.
fn session(app: &TestApp, id: &str, status: ThreadStatus, minutes: i64) -> CodingThread {
    let at = app.clock_now() + SignedDuration::from_mins(minutes);
    let mut thread = CodingThread::new("runner", id, format!("Session {id}"), status, at);
    "Example".clone_into(&mut thread.project);
    thread
}

async fn state(app: &TestApp) -> State {
    let session = app.sign_in().await;
    let answer = app.call(Call::get("/api/state").token(&session)).await;
    assert_eq!(answer.status, 200);
    answer.read_as()
}

/// A thread of the coding-thread app as it lists it, last changed `minute` minutes past 21:00.
fn listed(id: &str, status: &str, minute: u32) -> ListedThread {
    ListedThread {
        id: id.to_owned(),
        title: format!("Thread {id}"),
        project: "Example".to_owned(),
        status: status.to_owned(),
        updated_at: format!("2026-11-24T21:{minute:02}:00.000Z"),
        parent: None,
    }
}

#[tokio::test]
async fn the_state_lists_the_newest_top_level_threads() {
    let app = TestApp::new().await;
    assert_eq!(state(&app).await.threads, Vec::new());

    let threads = app.state.threads();
    let mut newest = session(&app, "r-new", ThreadStatus::Running, 100);
    newest.ask = Some("Fix the build".to_owned());
    newest.since = Some(newest.updated_at);
    newest.direct = true;
    threads.record(&newest).await.unwrap();
    let mut helper = session(&app, "r-helper", ThreadStatus::Running, 200);
    helper.parent = Some("r-new".to_owned());
    threads.record(&helper).await.unwrap();
    for n in 0..STATE_THREADS {
        let minutes = i64::try_from(n).unwrap();
        threads
            .record(&session(
                &app,
                &format!("r-{n:02}"),
                ThreadStatus::Completed,
                minutes,
            ))
            .await
            .unwrap();
    }

    let listed = state(&app).await.threads;
    assert_eq!(listed.len(), 40, "the newest 40");
    assert_eq!(listed[0].id, "r-new", "most recently changed first");
    assert_eq!(listed[1].id, "r-39");
    assert_eq!(listed[39].id, "r-01", "the oldest one left out");
    assert!(
        listed.iter().all(|t| t.id != "r-helper"),
        "helpers left out"
    );
    let first = &listed[0];
    assert_eq!(first.machine.as_str(), "runner");
    assert_eq!(first.status, "running");
    assert_eq!(first.project, "Example");
    assert_eq!(first.ask.as_deref(), Some("Fix the build"));
    assert!(first.direct && first.parent.is_none());
    assert_eq!(first.since.as_ref(), Some(&first.updated_at));
}

#[tokio::test]
async fn a_followed_thread_shows_its_look_and_its_end_is_news_once() {
    let app = TestApp::new().await;
    let threads = app.state.threads();
    let why = "w".repeat(100);
    let follow = threads.follow("runner", "r-1", &why).await.unwrap();
    assert_eq!(follow.why.chars().count(), 80, "the reason cut short");
    assert_eq!(follow.at.as_millisecond(), app.clock_now().as_millisecond());

    let followed = state(&app).await.followed;
    assert_eq!(followed.len(), 1);
    assert_eq!(
        (followed[0].id.as_str(), followed[0].machine.as_str()),
        ("r-1", "runner")
    );
    assert!(followed[0].thread.is_none(), "not seen yet");

    let running = session(&app, "r-1", ThreadStatus::Running, 1);
    assert_eq!(threads.record(&running).await.unwrap(), None);
    let followed = state(&app).await.followed;
    let look = followed[0].thread.as_ref().unwrap();
    assert_eq!((look.id.as_str(), look.status.as_str()), ("r-1", "running"));

    let waiting = session(&app, "r-1", ThreadStatus::Waiting, 2);
    let news = threads.record(&waiting).await.unwrap().unwrap();
    assert_eq!(news.ending, Ending::Waiting);
    assert_eq!(news.followed, follow);
    assert_eq!(
        state(&app).await.followed.len(),
        1,
        "waiting: still followed"
    );

    let done = session(&app, "r-1", ThreadStatus::Completed, 3);
    let news = threads.record(&done).await.unwrap().unwrap();
    assert_eq!((news.ending, news.thread), (Ending::Ended, done.clone()));
    let after = state(&app).await;
    assert!(after.followed.is_empty(), "ended: no longer followed");
    assert_eq!(after.threads[0].status, "completed");
    assert_eq!(threads.record(&done).await.unwrap(), None, "news once");
}

#[tokio::test]
async fn unfollowing_takes_it_off_the_state() {
    let app = TestApp::new().await;
    let threads = app.state.threads();
    threads
        .follow("runner", "r-1", "Fix the build")
        .await
        .unwrap();
    threads.follow("runner", "r-2", "Review it").await.unwrap();
    assert!(threads.unfollow("r-1").await.unwrap());
    assert!(!threads.unfollow("r-1").await.unwrap());
    let followed = state(&app).await.followed;
    assert_eq!(followed.len(), 1);
    assert_eq!(followed[0].id, "r-2");
    assert_eq!(followed[0].why, "Review it");
}

#[tokio::test]
async fn polling_stores_the_app_s_threads_and_reads_what_changed() {
    let app = TestApp::new().await;
    let fake = &app.coding_app;
    fake.on(&["laptop", "desk"]);
    fake.thread(
        "laptop",
        listed("mcp:a", "running", 5),
        vec![
            item("user_message", "Fix   the build"),
            item("assistant_message", "On it"),
            item("user_message", "And the tests"),
            item("assistant_message", "All green"),
        ],
    );
    let mut helper = listed("mcp:h", "running", 6);
    helper.parent = Some("mcp:a".to_owned());
    fake.thread("laptop", helper, vec![item("user_message", "Lane work")]);
    fake.take_down("desk", "asleep");

    assert_eq!(app.state.threads().poll().await.unwrap(), Vec::new());
    let listed_now = app.state.threads().list(None, true).await.unwrap();
    let a = listed_now.threads.iter().find(|t| t.id == "mcp:a").unwrap();
    assert_eq!(a.ask.as_deref(), Some("Fix the build"));
    assert_eq!(a.last_ask.as_deref(), Some("And the tests"));
    assert_eq!(a.last.as_deref(), Some("All green"));
    assert_eq!(a.project, "Example");
    let h = listed_now.threads.iter().find(|t| t.id == "mcp:h").unwrap();
    assert!(h.ask.is_none(), "a helper's thread isn't read");
    assert_eq!(
        listed_now.unreachable.get("desk").map(String::as_str),
        Some("desk: asleep")
    );
    let errors = app.state.threads().machine_errors();
    assert_eq!(errors.get("laptop"), Some(&None));
    let reads = fake.reads().len();
    assert_eq!(reads, 2, "the head, then the tail");

    // Unchanged: not read again. Gone: forgotten.
    app.state.threads().poll().await.unwrap();
    assert_eq!(fake.reads().len(), reads);
    fake.forget("laptop", "mcp:h");
    app.state.threads().poll().await.unwrap();
    let all = app.state.threads().list(None, true).await.unwrap().threads;
    assert_eq!(
        all.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(),
        ["mcp:a"]
    );
}

#[tokio::test]
async fn a_followed_app_thread_s_end_comes_from_the_poll() {
    let app = TestApp::new().await;
    let fake = &app.coding_app;
    fake.on(&["laptop"]);
    fake.thread("laptop", listed("mcp:a", "running", 1), vec![]);
    let threads = app.state.threads();
    threads
        .follow("laptop", "mcp:a", "Fix the build")
        .await
        .unwrap();
    assert_eq!(threads.poll().await.unwrap(), Vec::new());
    fake.thread("laptop", listed("mcp:a", "completed", 2), vec![]);
    let ends = threads.poll().await.unwrap();
    assert_eq!(ends.len(), 1);
    assert_eq!(ends[0].ending, Ending::Ended);
    assert_eq!(ends[0].followed.why, "Fix the build");
    assert_eq!(state(&app).await.followed, Vec::new());
}

#[tokio::test]
async fn the_state_lists_the_app_s_threads_still_running() {
    let app = TestApp::new().await;
    assert_eq!(state(&app).await.t3_running, Vec::new());
    let fake = &app.coding_app;
    fake.on(&["laptop"]);
    fake.thread(
        "laptop",
        listed("mcp:run", "running", 3),
        vec![
            item("user_message", "Fix it"),
            item("assistant_message", "Done half"),
        ],
    );
    fake.thread("laptop", listed("mcp:wait", "waiting", 5), vec![]);
    fake.thread("laptop", listed("mcp:queue", "queued", 4), vec![]);
    fake.thread("laptop", listed("mcp:done", "completed", 9), vec![]);
    let mut helper = listed("mcp:lane", "running", 8);
    helper.parent = Some("mcp:run".to_owned());
    fake.thread("laptop", helper, vec![]);
    app.state.threads().poll().await.unwrap();
    // A runner's session is not the app's.
    let runner = session(&app, "r-1", ThreadStatus::Running, 600);
    app.state.threads().record(&runner).await.unwrap();

    let running = state(&app).await.t3_running;
    let ids: Vec<&str> = running.iter().map(|t| t.id.as_str()).collect();
    assert_eq!(ids, ["mcp:wait", "mcp:queue", "mcp:run"]);
    let run = &running[2];
    assert_eq!(run.machine.as_str(), "laptop");
    assert_eq!(run.status, "running");
    assert_eq!(run.title, "Thread mcp:run");
    assert_eq!(run.updated_at.as_str(), "2026-11-24T21:03:00.000Z");
    assert_eq!(run.ask.as_deref(), Some("Fix it"));
    assert_eq!(run.last.as_deref(), Some("Done half"));
    assert_eq!(run.room, RoomId::MAIN);

    // On the wire it is `t3Running`, every field written.
    let session = app.sign_in().await;
    let raw = app
        .call(Call::get("/api/state").token(&session))
        .await
        .json();
    assert_eq!(raw["t3Running"][0]["statusLine"], json!(null));
    assert_eq!(raw["t3Running"][0]["tasks"], json!([]));

    // Main lists every room's; another room, only its own (none has a project yet).
    let threads = app.state.threads();
    assert_eq!(
        threads
            .running(emberhut_core::RoomId::MAIN)
            .await
            .unwrap()
            .len(),
        3
    );
    let other_room = threads
        .running(emberhut_core::RoomId::new(2))
        .await
        .unwrap();
    assert_eq!(other_room, Vec::new());
}

#[tokio::test]
pub(crate) async fn the_activity_is_a_thread_s_latest_steps() {
    let app = TestApp::new().await;
    let fake = &app.coding_app;
    fake.on(&["laptop"]);
    let mut items: Vec<_> = (0..50)
        .map(|n| item("assistant_message", &format!("Step {n}")))
        .collect();
    items[45] = item("checkpoint", "saved");
    items[46] = item("tool_call", "   ");
    items[47].text = "x".repeat(2000);
    fake.thread("laptop", listed("mcp:a", "running", 1), items);
    let session = app.sign_in().await;
    let answer = app
        .call(
            Call::get(ACTIVITY)
                .query("machine=laptop&thread=mcp:a")
                .token(&session),
        )
        .await;
    assert_eq!(answer.status, 200);
    let activity: ThreadActivity = answer.read_as();
    assert_eq!(activity.title, "Thread mcp:a");
    assert_eq!(activity.status, "running");
    assert_eq!(
        activity.updated_at.as_deref(),
        Some("2026-11-24T21:01:00.000Z")
    );
    assert_eq!(
        activity.items.len(),
        ACTIVITY_ITEMS - 2,
        "the newest 40, two left out"
    );
    assert_eq!(activity.items[0].text, "Step 10");
    assert_eq!(activity.items.last().unwrap().text, "Step 49");
    assert_eq!(
        activity.items[35].text.chars().count(),
        1200,
        "Step 47, cut"
    );
    assert_eq!(activity.items[0].kind, "assistant_message");
    let raw = answer.json();
    assert_eq!(raw["items"][0]["type"], "assistant_message");
    let tail = fake.reads().last().unwrap().2;
    assert_eq!(
        (tail.after, tail.limit, tail.activity),
        (Some(10), 40, true)
    );
}

#[tokio::test]
pub(crate) async fn the_activity_of_an_unknown_machine_is_400() {
    let app = TestApp::new().await;
    app.coding_app.on(&["laptop"]);
    let session = app.sign_in().await;
    for query in ["machine=desk&thread=mcp:a", "thread=mcp:a", ""] {
        let answer = app
            .call(Call::get("/api/t3/activity").query(query).token(&session))
            .await;
        assert_eq!(answer.status, 400, "{query}");
        let body: ErrorBody = answer.read_as();
        assert_eq!(body.ok, Some(false));
        assert_eq!(body.error, "No coding-thread app on that machine.");
    }
}

#[tokio::test]
pub(crate) async fn the_activity_the_app_cannot_give_is_502_with_its_error() {
    let app = TestApp::new().await;
    app.coding_app.on(&["laptop", "desk"]);
    app.coding_app.take_down("desk", "connection refused");
    let session = app.sign_in().await;
    let down = app
        .call(
            Call::get(ACTIVITY)
                .query("machine=desk&thread=mcp:a")
                .token(&session),
        )
        .await;
    assert_eq!(down.status, 502);
    let body: ErrorBody = down.read_as();
    assert_eq!(
        (body.ok, body.error.as_str()),
        (Some(false), "desk: connection refused")
    );
    let unknown = app
        .call(
            Call::get(ACTIVITY)
                .query("machine=laptop&thread=mcp:zz")
                .token(&session),
        )
        .await;
    assert_eq!(unknown.status, 502);
    assert_eq!(
        unknown.read_as::<ErrorBody>().error,
        "laptop: thread_not_found: No such thread"
    );
}

#[tokio::test]
pub(crate) async fn the_activity_needs_a_session() {
    let app = TestApp::new().await;
    app.coding_app.on(&["laptop"]);
    let answer = app
        .call(Call::get(ACTIVITY).query("machine=laptop&thread=mcp:a"))
        .await;
    assert_eq!(answer.status, 401);
    assert_eq!(app.coding_app.reads(), Vec::new());
}

#[tokio::test]
async fn the_threads_tool_filters_by_status_and_helpers() {
    let app = TestApp::new().await;
    let threads = app.state.threads();
    threads
        .record(&session(&app, "r-1", ThreadStatus::Waiting, 1))
        .await
        .unwrap();
    threads
        .record(&session(&app, "r-2", ThreadStatus::Running, 2))
        .await
        .unwrap();
    let mut helper = session(&app, "r-3", ThreadStatus::Waiting, 3);
    helper.parent = Some("r-1".to_owned());
    threads.record(&helper).await.unwrap();
    let ids = |list: Vec<emberhut_api::threads::CodingThread>| {
        list.into_iter().map(|t| t.id).collect::<Vec<_>>()
    };
    assert_eq!(
        ids(threads.list(None, false).await.unwrap().threads),
        ["r-2", "r-1"]
    );
    assert_eq!(
        ids(threads.list(Some("waiting"), false).await.unwrap().threads),
        ["r-1"]
    );
    assert_eq!(
        ids(threads.list(Some("waiting"), true).await.unwrap().threads),
        ["r-3", "r-1"]
    );
    assert_eq!(
        ids(threads.list(Some(""), false).await.unwrap().threads),
        ["r-2", "r-1"]
    );
    assert!(
        threads
            .list(None, false)
            .await
            .unwrap()
            .unreachable
            .is_empty()
    );
}

#[tokio::test]
async fn thread_send_completes_a_short_id_and_queues_the_message() {
    let app = TestApp::new().await;
    let fake = &app.coding_app;
    fake.on(&["laptop"]);
    fake.thread("laptop", listed("mcp:1234-abcd", "running", 1), vec![]);
    let threads = app.state.threads();
    threads.poll().await.unwrap();

    let sent = threads
        .send("laptop", "1234-abcd", "Run the tests")
        .await
        .unwrap();
    assert_eq!(sent.thread, "mcp:1234-abcd");
    assert!(sent.sent.queued);
    assert_eq!(sent.sent.run_id.as_deref(), Some("run-1"));
    let exact = threads
        .send("laptop", "mcp:1234-abcd", "Again")
        .await
        .unwrap();
    assert_eq!(exact.thread, "mcp:1234-abcd");
    assert_eq!(
        fake.sent(),
        [
            (
                "laptop".to_owned(),
                "mcp:1234-abcd".to_owned(),
                "Run the tests".to_owned()
            ),
            (
                "laptop".to_owned(),
                "mcp:1234-abcd".to_owned(),
                "Again".to_owned()
            ),
        ]
    );

    fake.refuse_sends_to("mcp:gone");
    let refused = threads
        .send("laptop", "mcp:gone", "Hello")
        .await
        .unwrap_err();
    assert!(matches!(
        refused,
        ThreadError::App(CodingAppError::Refused { .. })
    ));
    let unknown = threads.send("desk", "mcp:1", "Hello").await.unwrap_err();
    assert!(matches!(
        unknown,
        ThreadError::App(CodingAppError::UnknownMachine)
    ));
    assert_eq!(fake.sent().len(), 2);
}

#[tokio::test]
async fn an_unlisted_thread_s_activity_can_still_be_read() {
    let app = TestApp::new().await;
    app.coding_app.on(&["laptop"]);
    app.coding_app.unlisted(
        "laptop",
        "mcp:old",
        FakeThread {
            status: Some("completed".to_owned()),
            items: vec![item("assistant_message", "Finished")],
            ..FakeThread::default()
        },
    );
    let activity = app
        .state
        .threads()
        .activity("laptop", "mcp:old")
        .await
        .unwrap();
    assert_eq!(
        (activity.title.as_str(), activity.status.as_str()),
        ("", "completed")
    );
    assert_eq!(activity.updated_at, None);
    assert_eq!(activity.items.len(), 1);
}

#[tokio::test]
async fn the_threads_tool_lists_them_through_the_endpoint() {
    let app = TestApp::new().await;
    let threads = app.state.threads();
    threads
        .record(&session(&app, "r-1", ThreadStatus::Waiting, 1))
        .await
        .unwrap();
    threads
        .record(&session(&app, "r-2", ThreadStatus::Running, 2))
        .await
        .unwrap();
    app.coding_app.on(&["laptop"]);
    app.coding_app.take_down("laptop", "connection refused");
    threads.poll().await.unwrap();

    let all = app.call_tool("threads", json!({})).await;
    assert_ne!(all.is_error, Some(true), "{all:?}");
    let listed: serde_json::Value = serde_json::from_str(text(&all)).unwrap();
    let ids: Vec<&str> = listed["threads"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["r-2", "r-1"]);
    assert!(
        listed["unreachable"]["laptop"]
            .as_str()
            .unwrap()
            .contains("connection refused"),
        "{listed}"
    );
    let waiting = app
        .call_tool("threads", json!({ "status": "waiting" }))
        .await;
    let waiting: serde_json::Value = serde_json::from_str(text(&waiting)).unwrap();
    assert_eq!(waiting["threads"].as_array().unwrap().len(), 1);
    assert_eq!(waiting["threads"][0]["id"], "r-1");
}

#[tokio::test]
async fn thread_send_sends_through_the_endpoint_and_says_what_it_did() {
    let app = TestApp::new().await;
    let fake = &app.coding_app;
    fake.on(&["laptop"]);
    fake.thread("laptop", listed("mcp:1234-abcd", "running", 1), vec![]);
    app.state.threads().poll().await.unwrap();

    let sent = app
        .call_tool(
            "thread_send",
            json!({ "machine": "laptop", "thread": "1234-abcd", "message": "Run the tests" }),
        )
        .await;
    assert_eq!(
        text(&sent),
        "Sent to mcp:1234-abcd (that's the thread's full id; you wrote 1234-abcd) as its next \
         turn (queued behind the running one)."
    );
    assert_eq!(
        fake.sent(),
        [(
            "laptop".to_owned(),
            "mcp:1234-abcd".to_owned(),
            "Run the tests".to_owned()
        )]
    );

    let unknown = app
        .call_tool(
            "thread_send",
            json!({ "machine": "desk", "thread": "mcp:1", "message": "Hello" }),
        )
        .await;
    assert_ne!(unknown.is_error, Some(true));
    assert_eq!(
        text(&unknown),
        "Not sent: No coding-thread app on that machine. Check the thread's id and machine \
         with the threads tool."
    );
    let blank = app
        .call_tool(
            "thread_send",
            json!({ "machine": "laptop", "thread": "mcp:1234-abcd", "message": "" }),
        )
        .await;
    assert_eq!(blank.is_error, Some(true));
    let too_long = app
        .call_tool(
            "thread_send",
            json!({ "machine": "laptop", "thread": "mcp:1234-abcd", "message": "x".repeat(100_001) }),
        )
        .await;
    assert_eq!(
        too_long.is_error,
        Some(true),
        "a message to a room is as long as it gets"
    );
    assert_eq!(fake.sent().len(), 1);
}

#[tokio::test]
async fn follow_through_the_endpoint_puts_the_thread_in_the_state() {
    let app = TestApp::new().await;
    let followed = app
        .call_tool(
            "follow",
            json!({ "machine": "runner", "thread": "r-1", "why": "Fixes the login page" }),
        )
        .await;
    assert_eq!(text(&followed), "Following it.");
    let now = state(&app).await;
    assert_eq!(now.followed.len(), 1);
    assert_eq!(now.followed[0].id, "r-1");
    assert_eq!(now.followed[0].why, "Fixes the login page");

    let too_long = app
        .call_tool(
            "follow",
            json!({ "machine": "runner", "thread": "r-2", "why": "x".repeat(81) }),
        )
        .await;
    assert_eq!(too_long.is_error, Some(true));
    assert_eq!(state(&app).await.followed.len(), 1);
}
