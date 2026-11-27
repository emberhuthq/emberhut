//! The task store's glue on GPUI's headless test platform with a scripted backend: the pages
//! the windows' columns hold and how often they are asked for, the actions held with Undo and the changes sent at
//! once, the search and the archived list, a message on a task, and Talk.

use std::sync::Arc;
use std::time::{Duration, Instant};

use emberhut_api::headers::ETag;
use emberhut_api::ids::{RoomId, TaskId};
use emberhut_api::task_actions::{Edit, TaskAction, TaskChange};
use emberhut_api::task_message::{MessageTo, TaskMessageReply};
use emberhut_api::tasks::TaskSize;
use gpui_kit::{AppContext as _, Entity, TestAppContext, WindowId};
use serde_json::json;

use super::filter::{OwnerPick, TaskFilter};
use super::pages::{LIVE_EVERY, PageStatus};
use super::search::{Found, TYPING_PAUSE, TaskLookup};
use crate::backend::fake::{Call, FakeBackend};
use crate::backend::{Error, FreshState, StateReply};
use crate::fixtures::{issue, room, state, task_page};
use crate::store::answers::UNDO_WINDOW;
use crate::store::{Page, Store, ThemeState};

const ONE: TaskId = TaskId::new(1);
const TWO: TaskId = TaskId::new(2);

fn store(cx: &mut TestAppContext, backend: &Arc<FakeBackend>) -> Entity<Store> {
    let backend = Arc::clone(backend) as Arc<dyn crate::backend::Backend>;
    cx.new(|_| Store::idle(backend, None, ThemeState::load(None), RoomId::MAIN))
}

/// One window's column holds `pages` now, the shown one last.
fn column(store: &Entity<Store>, cx: &mut TestAppContext, pages: &[TaskId]) {
    let pages = pages.iter().copied().map(Page::Task).collect();
    store.update(cx, |store, cx| {
        store.column_changed(WindowId::from(1), pages, cx);
    });
}

fn take_state(store: &Entity<Store>, cx: &mut TestAppContext, extra: serde_json::Value) {
    store.update(cx, |store, cx| {
        let reply = StateReply::Fresh(FreshState {
            state: Box::new(state(extra)),
            etag: Some(ETag::new("\"v\"")),
        });
        store.apply(RoomId::MAIN, Ok(reply), Instant::now(), cx);
    });
}

fn task_calls(backend: &FakeBackend) -> Vec<Call> {
    backend
        .calls()
        .into_iter()
        .filter(|call| !matches!(call, Call::State(..) | Call::File(_)))
        .collect()
}

#[gpui_kit::test]
fn a_task_shown_in_a_column_is_fetched_and_forgotten_off_it(cx: &mut TestAppContext) {
    let backend = Arc::new(FakeBackend::new([]));
    backend.put_task(task_page(1, json!({"title": "Example"})));
    let store = store(cx, &backend);
    column(&store, cx, &[ONE]);
    store.read_with(cx, |store, _| {
        assert_eq!(store.tasks().page_status(ONE), PageStatus::Loading);
    });
    cx.run_until_parked();
    store.read_with(cx, |store, _| {
        assert!(matches!(
            store.tasks().page_status(ONE),
            PageStatus::Ready(_)
        ));
        assert_eq!(
            store.task_of(ONE).map(|t| t.title.as_str()),
            Some("Example")
        );
    });
    column(&store, cx, &[ONE, TWO]);
    cx.run_until_parked();
    store.read_with(cx, |store, _| {
        assert_eq!(store.tasks().page_status(TWO), PageStatus::NotFound);
    });
    column(&store, cx, &[ONE]);
    cx.run_until_parked();
    assert_eq!(
        task_calls(&backend),
        [Call::Task(ONE), Call::Task(TWO)],
        "back on a page held under: not asked again"
    );
    column(&store, cx, &[]);
    store.read_with(cx, |store, _| {
        assert_eq!(
            store.tasks().page_status(ONE),
            PageStatus::Loading,
            "off every column: forgotten"
        );
    });
}

#[gpui_kit::test]
fn a_page_is_asked_for_again_when_its_task_changes(cx: &mut TestAppContext) {
    let backend = Arc::new(FakeBackend::new([]));
    backend.put_task(task_page(1, json!({})));
    let store = store(cx, &backend);
    take_state(
        &store,
        cx,
        json!({"issues": [issue(1, "todo", json!({"updated": "2026-11-25T08:00:00Z"}))]}),
    );
    column(&store, cx, &[ONE]);
    cx.run_until_parked();
    take_state(
        &store,
        cx,
        json!({"issues": [issue(1, "todo", json!({"updated": "2026-11-25T08:00:00Z"}))]}),
    );
    cx.run_until_parked();
    assert_eq!(task_calls(&backend).len(), 1, "nothing changed");
    take_state(
        &store,
        cx,
        json!({"issues": [issue(1, "working", json!({"updated": "2026-11-25T09:00:00Z"}))]}),
    );
    cx.run_until_parked();
    assert_eq!(task_calls(&backend), [Call::Task(ONE), Call::Task(ONE)]);
}

#[gpui_kit::test]
fn a_page_with_running_agents_is_asked_for_every_few_seconds(cx: &mut TestAppContext) {
    let backend = Arc::new(FakeBackend::new([]));
    backend.put_task(task_page(
        1,
        json!({"ultraRun": {"on": true, "state": "running", "parallel": 2, "budget": 1.0,
            "spent": 0.0, "done": 0, "total": 2, "running": 1, "agents": []}}),
    ));
    let store = store(cx, &backend);
    column(&store, cx, &[ONE]);
    cx.run_until_parked();
    cx.executor().advance_clock(LIVE_EVERY);
    cx.run_until_parked();
    cx.executor().advance_clock(LIVE_EVERY);
    cx.run_until_parked();
    assert_eq!(task_calls(&backend).len(), 3);
    column(&store, cx, &[]);
    cx.executor().advance_clock(LIVE_EVERY * 3);
    cx.run_until_parked();
    assert_eq!(task_calls(&backend).len(), 3, "closed: no more");
}

#[gpui_kit::test]
fn archive_is_held_with_undo_and_a_refusal_says_why(cx: &mut TestAppContext) {
    let backend = Arc::new(FakeBackend::new([]));
    let store = store(cx, &backend);
    store.update(cx, |store, cx| store.archive(ONE, cx));
    store.read_with(cx, |store, _| {
        assert!(store.is_gone(ONE));
        assert_eq!(
            store.answers().toast().map(|t| t.text.as_str()),
            Some("Archived")
        );
    });
    cx.executor().advance_clock(UNDO_WINDOW);
    cx.run_until_parked();
    assert_eq!(task_calls(&backend), [Call::Act(TaskAction::archive(ONE))]);

    backend.push_action(Err(Error::Refused {
        status: Some(400),
        message: Some("no task T-2".into()),
    }));
    store.update(cx, |store, cx| store.mark_done(TWO, cx));
    cx.executor().advance_clock(UNDO_WINDOW);
    cx.run_until_parked();
    store.read_with(cx, |store, _| {
        assert!(!store.is_gone(TWO), "refused: it comes back");
        assert_eq!(
            store.answers().toast().map(|t| t.text.as_str()),
            Some("Not marked done: no task T-2")
        );
    });
}

#[gpui_kit::test]
fn not_now_puts_a_task_off_a_week_and_undo_keeps_it(cx: &mut TestAppContext) {
    let backend = Arc::new(FakeBackend::new([]));
    let store = store(cx, &backend);
    store.update(cx, |store, cx| {
        store.not_now(ONE, cx);
        let serial = store.answers().toast().map(|toast| toast.serial).unwrap();
        store.undo(serial, cx);
        store.not_now(TWO, cx);
    });
    cx.executor().advance_clock(UNDO_WINDOW);
    cx.run_until_parked();
    let calls = task_calls(&backend);
    assert_eq!(calls.len(), 1);
    assert!(matches!(&calls[0], Call::Act(TaskAction::Park(park)) if park.todo_park == TWO));
}

#[gpui_kit::test]
fn a_change_goes_at_once_and_a_refusal_shows_as_a_note(cx: &mut TestAppContext) {
    let backend = Arc::new(FakeBackend::new([]));
    let store = store(cx, &backend);
    let change = TaskChange {
        size: Edit::Set(TaskSize::Large),
        ..TaskChange::of(ONE)
    };
    backend.push_action(Err(Error::Refused {
        status: Some(400),
        message: Some("no such task".into()),
    }));
    store.update(cx, |store, cx| {
        store.change_task(change.clone(), cx);
        assert!(!store.create_task("  ", cx), "a blank title adds nothing");
    });
    cx.run_until_parked();
    assert_eq!(task_calls(&backend), [Call::Act(change.into())]);
    store.read_with(cx, |store, _| {
        assert_eq!(store.tasks().notice(), Some("Task not saved: no such task"));
    });
    cx.executor().advance_clock(Duration::from_secs(6));
    cx.run_until_parked();
    store.read_with(cx, |store, _| assert_eq!(store.tasks().notice(), None));
    store.update(cx, |store, cx| {
        assert!(store.create_task(" Call the bank ", cx));
        store.stop_worker(TWO, cx);
    });
    cx.run_until_parked();
    // Sent side by side: in either order.
    let later = &task_calls(&backend)[1..];
    assert_eq!(later.len(), 2);
    assert!(later.contains(&Call::Act(TaskAction::create("Call the bank"))));
    assert!(later.contains(&Call::Act(TaskAction::stop_worker(TWO))));
}

#[gpui_kit::test]
fn a_search_waits_for_the_typing_and_keeps_to_the_filter(cx: &mut TestAppContext) {
    let backend = Arc::new(FakeBackend::new([]));
    backend.push_found(Ok(vec![issue(7, "done", json!({}))]));
    let store = store(cx, &backend);
    store.update(cx, |store, cx| {
        store.set_task_filter(
            TaskFilter {
                owner: Some(OwnerPick::Yours),
                ..TaskFilter::default()
            },
            cx,
        );
        store.search_tasks("rep", cx);
        store.search_tasks("report", cx);
    });
    store.read_with(cx, |store, _| {
        assert_eq!(store.tasks().search().lookup().found(), Found::Loading);
    });
    cx.executor().advance_clock(TYPING_PAUSE);
    cx.run_until_parked();
    assert_eq!(
        task_calls(&backend),
        [Call::FindTasks("q=report&limit=50&owner=owner".into())],
        "only the last words are asked for"
    );
    store.read_with(cx, |store, _| {
        assert!(matches!(store.tasks().search().lookup().found(), Found::Tasks(tasks) if tasks.len() == 1));
    });
    store.update(cx, Store::toggle_archived_tasks);
    cx.run_until_parked();
    assert_eq!(
        task_calls(&backend)[1],
        Call::FindTasks("q=&limit=200&archived=1&owner=owner".into())
    );
}

#[gpui_kit::test]
fn an_unarchived_task_leaves_the_archived_list_and_it_is_asked_again(cx: &mut TestAppContext) {
    let backend = Arc::new(FakeBackend::new([]));
    backend.push_found(Ok(vec![
        issue(1, "canceled", json!({})),
        issue(2, "canceled", json!({})),
    ]));
    let store = store(cx, &backend);
    take_state(&store, cx, json!({"issues": [issue(3, "todo", json!({}))]}));
    store.update(cx, Store::toggle_archived_tasks);
    cx.run_until_parked();
    store.update(cx, |store, cx| store.unarchive(ONE, cx));
    cx.executor().advance_clock(UNDO_WINDOW);
    cx.run_until_parked();
    let archived = |store: &Store| match store.tasks().archived().map(TaskLookup::found) {
        Some(Found::Tasks(tasks)) => tasks.iter().map(|task| task.id).collect::<Vec<_>>(),
        other => panic!("the archived list shows: {other:?}"),
    };
    store.read_with(cx, |store, _| {
        assert!(store.is_gone(ONE), "until the next state");
        assert_eq!(archived(store), [ONE, TWO]);
    });

    backend.push_found(Ok(vec![issue(2, "canceled", json!({}))]));
    take_state(
        &store,
        cx,
        json!({"issues": [issue(1, "todo", json!({})), issue(3, "todo", json!({}))]}),
    );
    store.read_with(cx, |store, _| {
        assert!(!store.is_gone(ONE), "back in to do");
        assert_eq!(
            archived(store),
            [TWO],
            "out of the archived list at once, though its answer hasn't come"
        );
    });
    cx.run_until_parked();
    let finds = |backend: &FakeBackend| {
        task_calls(backend)
            .into_iter()
            .filter(|call| matches!(call, Call::FindTasks(_)))
            .count()
    };
    assert_eq!(finds(&backend), 2, "the archived list is asked again");
    store.read_with(cx, |store, _| assert_eq!(archived(store), [TWO]));

    take_state(
        &store,
        cx,
        json!({"issues": [issue(1, "working", json!({})), issue(3, "todo", json!({}))]}),
    );
    cx.run_until_parked();
    assert_eq!(finds(&backend), 2, "the same tasks: not asked again");
    store.update(cx, Store::toggle_archived_tasks);
    take_state(&store, cx, json!({"issues": [issue(3, "todo", json!({}))]}));
    cx.run_until_parked();
    assert_eq!(finds(&backend), 2, "folded: not asked");
}

#[gpui_kit::test]
fn a_search_hit_follows_the_state_and_is_asked_again_when_its_tasks_change(
    cx: &mut TestAppContext,
) {
    let backend = Arc::new(FakeBackend::new([]));
    backend.push_found(Ok(vec![
        issue(1, "todo", json!({})),
        issue(2, "todo", json!({})),
    ]));
    let store = store(cx, &backend);
    take_state(
        &store,
        cx,
        json!({"issues": [issue(1, "todo", json!({})), issue(2, "todo", json!({}))]}),
    );
    store.update(cx, |store, cx| store.search_tasks("report", cx));
    cx.executor().advance_clock(TYPING_PAUSE);
    cx.run_until_parked();
    let statuses = |store: &Store| match store.tasks().search().lookup().found() {
        Found::Tasks(tasks) => tasks
            .iter()
            .map(|task| task.status.as_str().to_owned())
            .collect::<Vec<_>>(),
        other => panic!("the search shows: {other:?}"),
    };
    take_state(
        &store,
        cx,
        json!({"issues": [issue(1, "done", json!({})), issue(2, "todo", json!({}))]}),
    );
    store.read_with(cx, |store, _| {
        assert_eq!(statuses(store), ["done", "todo"], "as the state has it now");
    });
    cx.run_until_parked();
    assert_eq!(
        task_calls(&backend).len(),
        1,
        "the same tasks: not asked again"
    );

    backend.push_found(Ok(vec![
        issue(1, "done", json!({})),
        issue(2, "canceled", json!({})),
    ]));
    take_state(&store, cx, json!({"issues": [issue(1, "done", json!({}))]}));
    cx.run_until_parked();
    assert_eq!(
        task_calls(&backend),
        [
            Call::FindTasks("q=report&limit=50".into()),
            Call::FindTasks("q=report&limit=50".into())
        ],
        "task 2 left the state: asked again, without the typing's pause"
    );
    store.read_with(cx, |store, _| {
        assert_eq!(statuses(store), ["done", "canceled"]);
    });
}

#[gpui_kit::test]
fn a_message_on_a_task_says_where_it_went(cx: &mut TestAppContext) {
    let backend = Arc::new(FakeBackend::new([]));
    backend.put_task(task_page(1, json!({})));
    backend.push_task_message(Ok(TaskMessageReply {
        ok: true,
        to: MessageTo::Session,
        session: None,
        queued: Some(true),
    }));
    let store = store(cx, &backend);
    column(&store, cx, &[ONE]);
    store.update(cx, |store, cx| {
        assert!(!store.send_task_message(ONE, "   ", cx), "nothing to send");
        assert!(store.send_task_message(ONE, " Shorter, please ", cx));
        assert!(store.tasks().reply(ONE).sending);
    });
    cx.run_until_parked();
    store.read_with(cx, |store, _| {
        let reply = store.tasks().reply(ONE);
        assert!(!reply.sending);
        assert_eq!(reply.sent, 1);
        assert_eq!(
            reply.note.as_deref(),
            Some("Sent. The session takes it after the turn it's on.")
        );
    });
    cx.executor().advance_clock(Duration::from_secs(1));
    cx.run_until_parked();
    assert_eq!(
        task_calls(&backend),
        [
            Call::Task(ONE),
            Call::MessageTask(ONE, "Shorter, please".into()),
            Call::Task(ONE)
        ],
        "its page is asked for again a moment later"
    );
}

#[gpui_kit::test]
fn talk_opens_the_tasks_project_room_with_the_composer_started(cx: &mut TestAppContext) {
    let backend = Arc::new(FakeBackend::new([]));
    let store = store(cx, &backend);
    take_state(
        &store,
        cx,
        json!({"rooms": [room(1, json!({})), room(3, json!({}))],
               "issues": [issue(1, "todo", json!({"key": "T-1", "title": "Example", "pid": 3}))]}),
    );
    store.update(cx, |store, cx| {
        store.talk_about_task(ONE, cx);
        assert_eq!(store.room(), RoomId::new(3));
        assert_eq!(
            store.take_composer_start().as_deref(),
            Some("About T-1 \"Example\": ")
        );
        assert_eq!(store.take_composer_start(), None, "taken once");
    });
}
