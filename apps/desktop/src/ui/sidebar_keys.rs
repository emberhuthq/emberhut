//! ↑ and ↓ through the sidebar: it keeps the keyboard after a click on one of its rows, and
//! the arrows move through the places it lists (your ember, Needs you, Tasks, the rooms, the
//! agents where they are shown, Sessions under More on a server that keeps plans, Machines,
//! Settings), opening each one they land on.

use gpui_kit::prelude::*;
use gpui_kit::{App, Div, KeyBinding, MouseButton, Window, actions};

use super::window::MainWindow;
use crate::store::{AgentsPlace, Page, Store, lookup};

actions!(
    sidebar,
    [
        /// Opens the place above in the sidebar.
        SidebarUp,
        /// Opens the place below in the sidebar.
        SidebarDown,
    ]
);

/// The key context the sidebar's arrows work in.
const CONTEXT: &str = "Sidebar";

/// Binds the arrows inside the sidebar.
pub(crate) fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("up", SidebarUp, Some(CONTEXT)),
        KeyBinding::new("down", SidebarDown, Some(CONTEXT)),
    ]);
}

/// The places the sidebar lists, top to bottom, as it shows them now.
#[must_use]
pub(crate) fn places(store: &Store, more_open: bool) -> Vec<Page> {
    let switcher = store.switcher();
    let mut places = Vec::new();
    places.extend(switcher.main().map(|main| Page::Room(main.id)));
    places.push(Page::Needs);
    if store.state().is_none_or(lookup::lists_tasks) {
        places.push(Page::Tasks);
    }
    places.push(Page::Scheduled);
    let agents: Vec<Page> = if store.agents_shown() {
        store
            .agent_rooms()
            .iter()
            .map(|own| Page::Room(own.agent.room))
            .collect()
    } else {
        Vec::new()
    };
    let (above, below) = match store.agents_place() {
        AgentsPlace::Above => (agents, Vec::new()),
        AgentsPlace::Below => (Vec::new(), agents),
    };
    places.extend(above);
    if store.rooms_open() {
        places.extend(switcher.listed().iter().map(|room| Page::Room(room.id)));
        if store.paused_open() {
            places.extend(switcher.paused().iter().map(|room| Page::Room(room.id)));
        }
    }
    places.extend(below);
    if more_open {
        if store.keeps_plans() {
            places.push(Page::Sessions);
        }
        places.extend([Page::Machines, Page::Settings]);
    }
    places
}

/// The place one step `down` (or up) from `current`; none past either end. From a place not
/// listed (a room folded away), the first or the last.
#[must_use]
pub(crate) fn step(places: &[Page], current: Page, down: bool) -> Option<Page> {
    let Some(at) = places.iter().position(|place| *place == current) else {
        return if down {
            places.first().copied()
        } else {
            places.last().copied()
        };
    };
    let next = if down {
        at.checked_add(1)?
    } else {
        at.checked_sub(1)?
    };
    places.get(next).copied()
}

/// The sidebar with its arrows: it takes the keyboard when clicked.
pub(crate) fn with_keys(
    sidebar: Div,
    window: &MainWindow,
    cx: &mut Context<'_, MainWindow>,
) -> Div {
    sidebar
        .track_focus(&window.sidebar_focus)
        .key_context(CONTEXT)
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _, window, cx| window.focus(&this.sidebar_focus, cx)),
        )
        .on_action(cx.listener(|this, _: &SidebarUp, window, cx| move_by(this, false, window, cx)))
        .on_action(cx.listener(|this, _: &SidebarDown, window, cx| move_by(this, true, window, cx)))
}

fn move_by(
    this: &mut MainWindow,
    down: bool,
    window: &mut Window,
    cx: &mut Context<'_, MainWindow>,
) {
    let store = this.store.read(cx);
    let places = places(store, this.more_open);
    // Where the window is: the page its column shows, else the chat's room.
    let here = this
        .column
        .read(cx)
        .current()
        .unwrap_or(Page::Room(store.room()));
    if let Some(page) = step(&places, here, down) {
        this.open(page, window, cx);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use emberhut_api::ids::RoomId;
    use gpui_kit::TestAppContext;
    use serde_json::json;

    use super::*;
    use crate::backend::fake::FakeBackend;
    use crate::fixtures::{room, state};

    /// The places over Main, two active rooms and a paused one, in the order the sidebar draws
    /// its rows: Main, Needs you, Tasks, Scheduled, the rooms, Paused's rooms, More's pages.
    #[gpui_kit::test]
    fn the_places_follow_the_sidebar_and_its_folds(cx: &mut TestAppContext) {
        let backend = Arc::new(FakeBackend::new([])) as Arc<dyn crate::backend::Backend>;
        let store = cx.new(|_| Store::for_tests(backend));
        let rooms = json!([
            room(1, json!({})),
            room(2, json!({})),
            room(3, json!({"active": false, "status": "paused"})),
            room(4, json!({})),
        ]);
        let r = |id| Page::Room(RoomId::new(id));
        let more = [Page::Machines, Page::Settings];
        store.update(cx, |store, cx| {
            store.take_state_for_tests(state(json!({ "rooms": rooms })), cx);
            let top = [r(1), Page::Needs, Page::Tasks, Page::Scheduled];
            assert_eq!(
                places(store, true),
                [&top[..], &[r(2), r(4)], &more].concat()
            );
            store.toggle_paused(cx);
            assert_eq!(
                places(store, true),
                [&top[..], &[r(2), r(4), r(3)], &more].concat(),
                "Paused open"
            );
            assert_eq!(
                places(store, false),
                [&top[..], &[r(2), r(4), r(3)]].concat(),
                "More folded"
            );
            store.toggle_rooms(cx);
            assert_eq!(
                places(store, true),
                [&top[..], &more].concat(),
                "Rooms folded"
            );
            store.take_state_for_tests(
                state(json!({ "rooms": rooms, "ui": {"hideProjects": false} })),
                cx,
            );
            assert_eq!(
                places(store, false),
                [r(1), Page::Needs, Page::Scheduled],
                "no Tasks while the server hides them"
            );
            store.take_state_for_tests(
                state(json!({ "rooms": rooms, "ui": {"hideProjects": false, "taskPlans": true} })),
                cx,
            );
            assert_eq!(
                places(store, true),
                [&top[..], &[Page::Sessions, Page::Machines, Page::Settings]].concat(),
                "with plans: Tasks always, and Sessions under More"
            );
        });
    }

    /// The agents' rooms come after the rooms by default, right under the pages when Settings
    /// puts them above, and nowhere while they are hidden.
    #[gpui_kit::test]
    fn the_agents_follow_the_sidebars_place_for_them(cx: &mut TestAppContext) {
        let backend = Arc::new(FakeBackend::new([])) as Arc<dyn crate::backend::Backend>;
        let store = cx.new(|_| Store::for_tests(backend));
        let r = |id| Page::Room(RoomId::new(id));
        store.update(cx, |store, cx| {
            store.take_state_for_tests(
                state(json!({
                    "rooms": [room(1, json!({})), room(2, json!({})), room(9, json!({}))],
                    "agents": [
                        {"id": "main", "name": "Main", "principal": "the owner", "room": 1,
                         "enabled": true, "pendingApprovals": 0},
                        {"id": "helper", "name": "Example helper", "principal": "Example Ltd",
                         "room": 9, "enabled": true, "pendingApprovals": 0}
                    ]
                })),
                cx,
            );
            let top = [r(1), Page::Needs, Page::Tasks, Page::Scheduled];
            let rooms: Vec<Page> = store
                .switcher()
                .listed()
                .iter()
                .map(|listed| Page::Room(listed.id))
                .collect();
            assert_eq!(
                places(store, false),
                [&top[..], &rooms, &[r(9)]].concat(),
                "below the rooms, by default"
            );
            store.place_agents(AgentsPlace::Above, cx);
            assert_eq!(
                places(store, false),
                [&top[..], &[r(9)], &rooms].concat(),
                "above the rooms"
            );
            store.show_agents(false, cx);
            assert_eq!(places(store, false), [&top[..], &rooms].concat(), "hidden");
        });
    }

    #[test]
    fn the_arrows_step_through_the_places_and_stop_at_the_ends() {
        let places = [
            Page::Room(RoomId::MAIN),
            Page::Needs,
            Page::Room(RoomId::new(2)),
            Page::Settings,
        ];
        assert_eq!(
            step(&places, Page::Needs, true),
            Some(Page::Room(RoomId::new(2)))
        );
        assert_eq!(
            step(&places, Page::Needs, false),
            Some(Page::Room(RoomId::MAIN))
        );
        assert_eq!(step(&places, Page::Room(RoomId::MAIN), false), None);
        assert_eq!(step(&places, Page::Settings, true), None);
        assert_eq!(
            step(&places, Page::Room(RoomId::new(9)), true),
            Some(Page::Room(RoomId::MAIN)),
            "from a room folded away"
        );
        assert_eq!(step(&[], Page::Needs, true), None);
    }
}
