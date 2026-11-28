//! The huts page in a test window: a hut online and one asleep, their accounts' room said
//! honestly, the details fold, and "Add a hut" opening the pairing code.

use std::sync::Arc;

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AppContext as _, Context, Entity, Render, Role, ScrollHandle, TestAppContext, Window,
};
use serde_json::json;

use crate::backend::fake::FakeBackend;
use crate::store::settings::SettingsPane;
use crate::store::{Page, Store};
use crate::ui::column::{self, Column};
use crate::ui::palette::{Pal, pal};

/// The page alone in a window.
struct HutsView {
    store: Entity<Store>,
    scroll: ScrollHandle,
}

impl Render for HutsView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<'_, Self>) -> impl gpui_kit::IntoElement {
        super::page(&self.store, &self.scroll, &pal(cx), cx)
    }
}

fn store(cx: &mut TestAppContext) -> Entity<Store> {
    cx.update(|cx| {
        gpui_kit::init(cx);
        cx.set_global(Pal::new(&crate::theme::built_in(), false));
    });
    let backend = Arc::new(FakeBackend::new([])) as Arc<dyn crate::backend::Backend>;
    let store = cx.new(|_| Store::for_tests(backend));
    let state = crate::fixtures::state(json!({"accounts": [
        {"id": "a", "name": "Work", "machine": "studio", "loggedIn": true,
            "used5h": null, "used7d": 40.0},
        {"id": "b", "name": "Spare", "machine": "studio", "loggedIn": true},
        {"id": "c", "name": "Home", "machine": "runner", "loggedIn": true,
            "used5h": 28.0, "used7d": 10.0}
    ]}));
    let seen = jiff::Timestamp::now() - jiff::SignedDuration::from_hours(2);
    let page = serde_json::from_value(json!({"at": "2026-11-28T12:00:00Z", "machines": [
        {"id": "studio", "name": "Studio", "platform": "darwin", "state": "ok", "status": "ok",
            "battery": {"percent": 80.0},
            "sessions": [{"id": "r-1", "running": true, "title": "Example"}]},
        {"id": "runner", "name": "Tower", "platform": "linux", "state": "offline",
            "status": "no sample for 2 h", "seen": seen.to_string(), "placeable": false}
    ]}))
    .unwrap();
    store.update(cx, |store, cx| {
        store.show_state_for_tests(state, cx);
        store.show_machines_for_tests(page, cx);
    });
    store
}

#[gpui_kit::test]
fn a_hut_online_and_one_asleep_say_so_with_their_accounts_room(cx: &mut TestAppContext) {
    let store = store(cx);
    let (_, cx) = cx.add_window_view(|window, cx| {
        Column::new(store.clone(), window, cx);
        HutsView {
            store: store.clone(),
            scroll: ScrollHandle::new(),
        }
    });
    // Tall enough for both cards: an element out of sight isn't drawn.
    cx.simulate_resize(gpui_kit::size(gpui_kit::px(900.), gpui_kit::px(1400.)));
    cx.update(|window, cx| {
        window.render_frame(cx);
        let studio = window.find("hut-studio");
        assert_eq!(studio.role(), Some(Role::Group));
        assert_eq!(studio.label(), Some("Studio, Mac, Online"));
        let tower = window.find("hut-runner");
        assert_eq!(
            tower.label(),
            Some("Tower, Linux PC, Asleep · last seen 2 h ago")
        );
        assert_eq!(
            window.find("hut-account-studio-a").label(),
            Some("Work, 60% left, 5-hour not reported · week 40% used"),
            "an unknown 5-hour use is said, never read as 0%"
        );
        assert_eq!(
            window.find("hut-account-studio-b").label(),
            Some("Spare, Not checked yet"),
            "never 100% left"
        );
        assert_eq!(
            window.find("hut-account-runner-c").label(),
            Some("Home, 72% left, 5-hour 28% used · week 10% used")
        );
        let fold = window.find("hut-details-studio");
        assert_eq!(fold.label(), Some("Details of Studio"));
        assert_eq!(fold.expanded(), Some(false));
        window.click("hut-details-studio", cx);
        window.render_frame(cx);
        assert_eq!(window.find("hut-details-studio").expanded(), Some(true));
        window.click("huts-add", cx);
        let column = column::of(window, cx).unwrap();
        assert_eq!(
            column.read(cx).current(),
            Some(Page::SettingsPane(SettingsPane::Devices)),
            "Add a hut opens the pairing code"
        );
    });
}
