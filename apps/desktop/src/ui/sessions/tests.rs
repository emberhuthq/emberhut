//! The Sessions screen and a session's page in a test window: a row opens its page in the
//! column, a finished row opens to its last answer, and the page loads its session and sends
//! what its box holds.

use std::sync::Arc;
use std::time::{Duration, Instant};

use emberhut_api::ids::SessionId;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, Entity, TestAppContext, WindowHandle};
use serde_json::json;

use super::{SessionPage, SessionsView};
use crate::backend::fake::{Call, FakeBackend};
use crate::store::sessions::places::SessionPlace;
use crate::store::sessions::view::SessionRef;
use crate::store::{Page, Store};
use crate::ui::column::Column;
use crate::ui::palette::Pal;

fn session(id: &str, status: &str, extra: serde_json::Value) -> serde_json::Value {
    let mut base = json!({"id": id, "machine": "runner", "title": format!("Session {id}"),
        "project": "", "status": status, "updatedAt": "2026-11-25T08:00:00Z", "place": "Linux PC",
        "doing": null, "account": null, "accountName": null, "ask": null, "last": null,
        "direct": true, "model": null, "tasks": [], "room": 1});
    if let (Some(base), serde_json::Value::Object(extra)) = (base.as_object_mut(), extra) {
        base.extend(extra);
    }
    base
}

fn setup(cx: &mut TestAppContext) -> (Arc<FakeBackend>, Entity<Store>) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        cx.set_global(Pal::new(&crate::theme::built_in(), false));
    });
    let backend = Arc::new(FakeBackend::new([]));
    let shared = Arc::clone(&backend) as Arc<dyn crate::backend::Backend>;
    let store = cx.new(|_| Store::for_tests(shared));
    let state = crate::fixtures::state(json!({
        "sessions": [session("r-1", "running", json!({"direct": false}))],
        "mySessions": [session("r-2", "completed", json!({"last": "All done."}))]
    }));
    store.update(cx, |store, cx| store.show_state_for_tests(state, cx));
    (backend, store)
}

fn click<V: 'static>(cx: &mut TestAppContext, window: WindowHandle<V>, id: &'static str) {
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click(id, cx);
    })
    .unwrap();
    cx.run_until_parked();
}

#[gpui_kit::test]
fn a_row_opens_its_page_and_a_finished_one_its_last_answer(cx: &mut TestAppContext) {
    let (_backend, store) = setup(cx);
    let mut column = None;
    let window = cx.add_window(|window, cx| {
        column = Some(Column::new(store.clone(), window, cx));
        SessionsView::new(store.clone(), cx)
    });
    let column = column.unwrap();
    click(cx, window, "session-fold-r-2");
    window
        .update(cx, |view, _, _| {
            assert!(view.open_rows.contains(&SessionId::from("r-2")));
        })
        .unwrap();
    click(cx, window, "session-r-1");
    let page = column.read_with(cx, |column, _| column.current());
    let Some(Page::Session(key)) = page else {
        panic!("the session's page opens, not {page:?}");
    };
    store.read_with(cx, |store, _| {
        assert_eq!(
            store.session_place(key),
            Some(&SessionPlace::Session(SessionRef::whole(SessionId::from(
                "r-1"
            ))))
        );
    });
}

/// A session's page in a test window, its session (`r-2`, the user's own) loaded.
fn loaded_page(
    cx: &mut TestAppContext,
) -> (Arc<FakeBackend>, Entity<Store>, WindowHandle<SessionPage>) {
    cx.executor().allow_parking();
    let (backend, store) = setup(cx);
    backend.sessions.put_transcript(
        serde_json::from_value(json!({
            "session": {"id": "r-2", "title": "Session r-2", "status": "completed",
                "meta": {"direct": true}},
            "events": [{"seq": 1, "turn": "r-2:1", "kind": "text", "text": "All done."}]
        }))
        .unwrap(),
    );
    let mut column = None;
    let window = cx.add_window(|window, cx| {
        let made = Column::new(store.clone(), window, cx);
        let page = SessionPage::new(store.clone(), &made, window, cx);
        column = Some(made);
        page
    });
    let column = column.unwrap();
    let key = store.update(cx, |store, _| {
        store.session_key(SessionPlace::Session(SessionRef::whole(SessionId::from(
            "r-2",
        ))))
    });
    column.update(cx, |column, cx| column.open(Page::Session(key), cx));
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        cx.run_until_parked();
        let loaded = window
            .update(cx, |page, _, cx| {
                page.session()
                    .is_some_and(|watch| watch.read(cx).view().is_loaded())
            })
            .unwrap();
        if loaded || Instant::now() > deadline {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    (backend, store, window)
}

#[gpui_kit::test]
fn the_page_loads_its_session_and_sends_its_box(cx: &mut TestAppContext) {
    let (backend, _store, window) = loaded_page(cx);
    window
        .update(cx, |page, window, cx| {
            let head = page.summary(cx).unwrap();
            assert_eq!(head.title, "Session r-2");
            assert!(head.direct && !head.running);
            page.reply.restore("Carry on", window, cx);
        })
        .unwrap();
    click(cx, window, "session-send");
    assert!(backend.calls().contains(&Call::SendToSession(
        SessionId::from("r-2"),
        "Carry on".into()
    )));
}

#[gpui_kit::test]
fn enter_while_a_message_is_on_its_way_keeps_the_next_one(cx: &mut TestAppContext) {
    let (backend, _store, window) = loaded_page(cx);
    window
        .update(cx, |page, window, cx| {
            page.reply.restore("First", window, cx);
            page.send(window, cx);
            assert_eq!(page.reply.text(cx), "", "the first is taken");
            page.reply.restore("Second", window, cx);
            // Enter again before the first's answer.
            page.send(window, cx);
            assert_eq!(page.reply.text(cx), "Second", "kept, not thrown away");
        })
        .unwrap();
    cx.run_until_parked();
    let sent: Vec<String> = backend
        .calls()
        .into_iter()
        .filter_map(|call| match call {
            Call::SendToSession(_, text) => Some(text),
            _ => None,
        })
        .collect();
    assert_eq!(
        sent,
        ["First"],
        "the second waits for the user's next Enter"
    );
    window.update(cx, SessionPage::send).unwrap();
    cx.run_until_parked();
    assert!(backend.calls().contains(&Call::SendToSession(
        SessionId::from("r-2"),
        "Second".into()
    )));
}

#[gpui_kit::test]
fn the_box_is_worded_for_the_session_once_it_loads(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let (backend, store) = setup(cx);
    backend.sessions.put_transcript(
        serde_json::from_value(json!({
            "session": {"id": "r-3", "title": "Session r-3", "status": "completed",
                "meta": {"direct": true}},
            "events": [{"seq": 1, "turn": "r-3:1", "kind": "text", "text": "Done."}]
        }))
        .unwrap(),
    );
    let mut column = None;
    let window = cx.add_window(|window, cx| {
        let made = Column::new(store.clone(), window, cx);
        let page = SessionPage::new(store.clone(), &made, window, cx);
        column = Some(made);
        page
    });
    let column = column.unwrap();
    let key = store.update(cx, |store, _| {
        store.session_key(SessionPlace::Session(SessionRef::whole(SessionId::from(
            "r-3",
        ))))
    });
    column.update(cx, |column, cx| column.open(Page::Session(key), cx));
    let opened_as = window
        .update(cx, |page, _, _| page.reply.is_direct())
        .unwrap();
    assert!(!opened_as, "not in the state yet: the assistant's wording");
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        cx.run_until_parked();
        let direct = window
            .update(cx, |page, _, _| page.reply.is_direct())
            .unwrap();
        if direct {
            break;
        }
        assert!(Instant::now() < deadline, "the box follows the loaded head");
        std::thread::sleep(Duration::from_millis(5));
    }
}
