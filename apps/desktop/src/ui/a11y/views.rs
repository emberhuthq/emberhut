//! What a screen reader is told on a few of the app's pages, read back from the last frame in a
//! test window: a row's one line, its fold's state, the chosen mode, an icon button's name.

use std::sync::Arc;

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, Entity, Role, TestAppContext};
use serde_json::json;

use crate::backend::fake::FakeBackend;
use crate::store::Store;
use crate::ui::calendar::CalendarView;
use crate::ui::column::Column;
use crate::ui::palette::Pal;
use crate::ui::sessions::SessionsView;

fn store(cx: &mut TestAppContext, state: serde_json::Value) -> Entity<Store> {
    cx.update(|cx| {
        gpui_kit::init(cx);
        cx.set_global(Pal::new(&crate::theme::built_in(), false));
    });
    let backend = Arc::new(FakeBackend::new([])) as Arc<dyn crate::backend::Backend>;
    let store = cx.new(|_| Store::for_tests(backend));
    let state = crate::fixtures::state(state);
    store.update(cx, |store, cx| store.show_state_for_tests(state, cx));
    store
}

fn session(id: &str, status: &str, last: Option<&str>) -> serde_json::Value {
    json!({"id": id, "machine": "runner", "title": format!("Session {id}"), "project": "",
        "status": status, "updatedAt": "2026-11-25T08:00:00Z", "place": "Linux PC",
        "doing": "Reading the logs", "account": null, "accountName": null, "ask": null,
        "last": last, "direct": true, "model": null, "tasks": [], "room": 1})
}

#[gpui_kit::test]
fn a_session_reads_as_one_line_and_its_answer_folds(cx: &mut TestAppContext) {
    let store = store(
        cx,
        json!({
            "sessions": [session("r-1", "running", None)],
            "mySessions": [session("r-2", "completed", Some("All done."))]
        }),
    );
    let window = cx.add_window(|window, cx| {
        Column::new(store.clone(), window, cx);
        SessionsView::new(store.clone(), cx)
    });
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        let running = window.find("session-r-1");
        assert_eq!(running.role(), Some(Role::Button));
        let label = running.label().unwrap_or_default();
        assert!(
            label.starts_with("Session r-1, running") && label.ends_with("Reading the logs"),
            "{label}"
        );
        let fold = window.find("session-fold-r-2");
        assert_eq!(fold.label(), Some("Show its last answer"));
        assert_eq!(fold.expanded(), Some(false));
        window.click("session-fold-r-2", cx);
        window.render_frame(cx);
        let fold = window.find("session-fold-r-2");
        assert_eq!(fold.label(), Some("Hide its last answer"));
        assert_eq!(fold.expanded(), Some(true));
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_calendar_names_its_arrows_and_selects_its_mode(cx: &mut TestAppContext) {
    let store = store(cx, json!({}));
    let window = cx.add_window(|window, cx| CalendarView::new(store.clone(), window, cx));
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(window.find("calendar-previous").label(), Some("Previous"));
        assert_eq!(window.find("calendar-next").label(), Some("Next"));
        let week = window.find("calendar-mode-Week");
        assert_eq!((week.label(), week.selected()), (Some("Week"), Some(true)));
        let month = window.find("calendar-mode-Month");
        assert_ne!(month.selected(), Some(true));
        window.click("calendar-mode-Month", cx);
        window.render_frame(cx);
        assert_eq!(
            window.find("calendar-mode-Month").selected(),
            Some(true),
            "the chosen mode moves"
        );
    })
    .unwrap();
}
