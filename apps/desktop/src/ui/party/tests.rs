//! The Party window drawn in a test window over a scripted backend: the list, the New party
//! form (⌥⌘N) and a persona's editor, a party live with its talk, the sheet over the party's
//! controls, and a link in the talk.

use std::sync::Arc;
use std::time::Duration;

use emberhut_api::ids::PartyId;
use emberhut_api::party::{PartyEvent, Speaking};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext as _, Entity, Modifiers, TestAppContext, VisualTestContext};

use super::{NewParty, PartyWindow};
use crate::backend::Backend;
use crate::backend::fake::{Call, FakeBackend};
use crate::store::Store;
use crate::store::party::roster::tests::{page, party};
use crate::store::party::tests::settle;
use crate::ui::palette::Pal;

fn window(
    backend: FakeBackend,
    cx: &mut TestAppContext,
) -> (Entity<Store>, Entity<PartyWindow>, &mut VisualTestContext) {
    shared_window(Arc::new(backend), cx)
}

fn shared_window(
    backend: Arc<FakeBackend>,
    cx: &mut TestAppContext,
) -> (Entity<Store>, Entity<PartyWindow>, &mut VisualTestContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        cx.set_global(Pal::new(&crate::theme::built_in(), false));
    });
    let backend = backend as Arc<dyn Backend>;
    let store = cx.new(|_| Store::for_tests(backend));
    let (view, cx) = cx.add_window_view(|window, cx| PartyWindow::new(store.clone(), window, cx));
    cx.run_until_parked();
    cx.update(TestWindowExt::render_frame);
    (store, view, cx)
}

#[gpui_kit::test]
fn the_list_then_a_new_party_and_a_persona(cx: &mut TestAppContext) {
    let backend = FakeBackend::new([]);
    backend.party.push_parties(Ok(page()));
    let (store, view, cx) = window(backend, cx);
    assert_eq!(
        cx.read(|cx| store.read(cx).party().roster().parties().len()),
        2
    );
    cx.dispatch_action(NewParty);
    cx.run_until_parked();
    cx.update(TestWindowExt::render_frame);
    assert_eq!(
        cx.read(|cx| store.read(cx).party().draft().map(|d| d.cast().len())),
        Some(3),
        "the form, with its starting cast"
    );
    view.update_in(cx, |view, window, cx| view.edit_persona(None, window, cx));
    cx.update(TestWindowExt::render_frame);
    assert!(cx.read(|cx| {
        store
            .read(cx)
            .party()
            .draft()
            .is_some_and(|draft| draft.editing.is_some())
    }));
    // Esc is Cancel: the editor first, then the sheet.
    cx.simulate_keystrokes("escape");
    cx.update(TestWindowExt::render_frame);
    assert!(cx.read(|cx| {
        store
            .read(cx)
            .party()
            .draft()
            .is_some_and(|draft| draft.editing.is_none())
    }));
    cx.simulate_keystrokes("escape");
    assert!(
        cx.read(|cx| store.read(cx).party().draft().is_none()),
        "the sheet closed"
    );
}

#[gpui_kit::test]
fn a_party_live_shows_its_talk_and_who_speaks(cx: &mut TestAppContext) {
    // The stream blocks a thread of its own, which wakes this test's tasks from outside it.
    cx.executor().allow_parking();
    let backend = FakeBackend::new([]);
    backend.party.push_live(vec![
        PartyEvent::Snapshot {
            party: Some(serde_json::from_value(party(3, "running")).unwrap()),
            messages: vec![
                serde_json::from_value(serde_json::json!({
                    "id": 1, "party": 3, "who": "owner", "name": "Emberhut", "kind": "steer",
                    "text": "Argue the other side", "model": null, "tokens": 0, "cost": 0,
                    "cut": false, "at": "2026-11-25T10:00:00.000Z"
                }))
                .unwrap(),
                serde_json::from_value(serde_json::json!({
                    "id": 2, "party": 3, "who": "p1", "name": "Nora", "kind": "turn",
                    "text": "Tea.", "model": "claude-opus-5-5", "tokens": 10, "cost": 0.01,
                    "cut": true, "at": "2026-11-25T10:01:00.000Z"
                }))
                .unwrap(),
            ],
        },
        PartyEvent::Speaking {
            speaking: Some(Speaking {
                who: "p2".into(),
                name: "Victor".into(),
                text: String::new(),
            }),
        },
    ]);
    let (store, view, cx) = window(backend, cx);
    store.update(cx, |store, cx| store.open_party(Some(PartyId::new(3)), cx));
    settle(cx, Duration::ZERO, |cx| {
        store.read_with(cx, |store, _| {
            store
                .party()
                .room()
                .is_some_and(|room| room.speaking().is_some())
        })
    });
    cx.update(TestWindowExt::render_frame);
    let room = cx
        .read(|cx| store.read(cx).party().room().cloned())
        .unwrap();
    assert_eq!(room.lines().len(), 2);
    assert_eq!(room.speaking().unwrap().name, "Victor");
    assert_eq!(view.read_with(cx, |view, _| view.title.clone()), "Party 3");
    store.update(cx, |store, cx| store.open_party(None, cx));
    cx.run_until_parked();
    cx.update(TestWindowExt::render_frame);
    assert_eq!(view.read_with(cx, |view, _| view.title.clone()), "Party");
}

/// A running party open, followed until its stream has said who is in it.
fn a_running_party<'a>(
    backend: &Arc<FakeBackend>,
    cx: &'a mut TestAppContext,
) -> (Entity<Store>, &'a mut VisualTestContext) {
    backend.party.push_live(vec![PartyEvent::Snapshot {
        party: Some(serde_json::from_value(party(3, "running")).unwrap()),
        messages: vec![
            serde_json::from_value(serde_json::json!({
                "id": 1, "party": 3, "who": "p1", "name": "Nora", "kind": "turn",
                "text": "Tea.", "model": "claude-opus-5-5", "tokens": 10, "cost": 0.01,
                "cut": false, "at": "2026-11-25T10:01:00.000Z"
            }))
            .unwrap(),
        ],
    }]);
    let (store, _view, cx) = shared_window(Arc::clone(backend), cx);
    // The stream blocks a thread of its own, which wakes this test's tasks from outside it.
    cx.executor().allow_parking();
    store.update(cx, |store, cx| store.open_party(Some(PartyId::new(3)), cx));
    settle(cx, Duration::ZERO, |cx| {
        store.read_with(cx, |store, _| {
            store
                .party()
                .room()
                .is_some_and(|room| room.party().is_some())
        })
    });
    cx.update(TestWindowExt::render_frame);
    (store, cx)
}

#[gpui_kit::test]
fn a_click_on_the_new_party_sheet_never_reaches_the_controls_under_it(cx: &mut TestAppContext) {
    let backend = Arc::new(FakeBackend::new([]));
    let (store, cx) = a_running_party(&backend, cx);
    let stop = cx.update(|window, _| window.find("party-stop").bounds().center());
    let stops = || {
        backend
            .calls()
            .into_iter()
            .filter(|call| matches!(call, Call::ActOnParty(..)))
            .count()
    };
    cx.dispatch_action(NewParty);
    cx.run_until_parked();
    cx.update(TestWindowExt::render_frame);
    assert!(cx.read(|cx| store.read(cx).party().draft().is_some()));
    cx.simulate_click(stop, Modifiers::none());
    cx.run_until_parked();
    assert_eq!(stops(), 0, "the backdrop took the click");
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    cx.update(TestWindowExt::render_frame);
    cx.simulate_click(stop, Modifiers::none());
    cx.run_until_parked();
    assert_eq!(stops(), 1, "with the sheet gone, the same click is Stop");
    store.update(cx, |store, cx| store.open_party(None, cx));
}
