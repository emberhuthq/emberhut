//! The approvals' pages in a window, on GPUI's headless test platform with the fake backend: a
//! row opens its request, Approve asks first and then sends the hash of the payload on screen,
//! and Needs you's row opens the list.

use std::sync::Arc;

use emberhut_api::approvals::{Approval, ApprovalState, Decision};
use emberhut_api::ids::ApprovalId;
use gpui_kit::TestSupportExt as _;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AppContext as _, Context, Entity, IntoElement, Render, ScrollHandle, TestAppContext, Window,
};
use serde_json::json;

use super::{detail, list, needs_row};
use crate::backend::fake::{Call, FakeBackend};
use crate::store::approvals::Ask;
use crate::store::{Page, Store};
use crate::ui::column::Column;
use crate::ui::palette::{Pal, pal};

/// What the probe window shows.
#[derive(Clone, Copy)]
enum Shows {
    List,
    Detail(ApprovalId),
    NeedsRow,
}

/// One of the approvals' pages alone in a window.
struct Probe {
    store: Entity<Store>,
    shows: Shows,
    scroll: ScrollHandle,
}

impl Render for Probe {
    fn render(&mut self, _: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let p = pal(cx);
        match self.shows {
            Shows::List => list::page(&self.store, None, &self.scroll, &p, cx).into_any_element(),
            Shows::Detail(id) => {
                detail::page(&self.store, id, &self.scroll, &p, cx).into_any_element()
            }
            Shows::NeedsRow => {
                let summary = self.store.read(cx).approvals_summary();
                needs_row(summary, &p).test_support().into_any_element()
            }
        }
    }
}

/// A payment waiting on the user.
fn payment() -> Approval {
    serde_json::from_value(json!({
        "id": 7, "agent": "client-a", "agentName": "Example helper", "kind": "payment",
        "target": "example-hosting.test", "summary": "The hosting bill",
        "text": "Invoice  EX-1\n  two spaces kept", "files": [
            {"name": "invoice.pdf", "size": 2048, "sha256": "3f2a9c0b1d4e".to_owned() + &"0".repeat(52)}
        ],
        "payloadHash": "ab".repeat(32), "approver": "owner", "waitsOnOwner": true,
        "amountCents": 4_900, "amountCurrency": "EUR", "state": "pending",
        "createdAt": "2026-11-27T10:00:00Z"
    }))
    .unwrap()
}

/// A window showing `shows` over a store whose state has `approvals`, with its column.
fn open(
    cx: &mut TestAppContext,
    shows: Shows,
    approvals: &[Approval],
) -> (
    Arc<FakeBackend>,
    Entity<Store>,
    Entity<Column>,
    gpui_kit::WindowHandle<Probe>,
) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        cx.set_global(Pal::new(&crate::theme::built_in(), false));
    });
    let backend = Arc::new(FakeBackend::new([]));
    for approval in approvals {
        backend.approvals.put(approval.clone());
    }
    let shared = Arc::clone(&backend) as Arc<dyn crate::backend::Backend>;
    let store = cx.new(|_| Store::for_tests(shared));
    store.update(cx, |store, cx| {
        let state = crate::fixtures::state(json!({ "approvals": approvals }));
        store.show_state_for_tests(state, cx);
    });
    let mut column = None;
    let window = cx.add_window(|window, cx| {
        column = Some(Column::new(store.clone(), window, cx));
        Probe {
            store: store.clone(),
            shows,
            scroll: ScrollHandle::new(),
        }
    });
    (backend, store, column.unwrap(), window)
}

#[gpui_kit::test]
fn a_row_opens_its_request_and_approve_opens_it_with_the_question(cx: &mut TestAppContext) {
    let (_backend, store, column, window) = open(cx, Shows::List, &[payment()]);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("approval-7", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(
        column.read_with(cx, |column, _| column.current()),
        Some(Page::Approval(ApprovalId::new(7)))
    );
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("approval-7-approve", cx);
    })
    .unwrap();
    cx.run_until_parked();
    let asked = store.read_with(cx, |store, _| {
        store.approvals().asking(ApprovalId::new(7)).cloned()
    });
    assert_eq!(
        asked,
        Some(Ask::Approve {
            payload_hash: "ab".repeat(32)
        }),
        "the question waits under the payload, on the hash the row showed"
    );
}

#[gpui_kit::test]
fn approve_asks_first_then_sends_the_hash_shown(cx: &mut TestAppContext) {
    let id = ApprovalId::new(7);
    let (backend, store, _column, window) = open(cx, Shows::Detail(id), &[payment()]);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(
            window.try_find("approval-question").is_none(),
            "nothing is asked yet"
        );
        window.click("approval-approve", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert!(
        !backend
            .calls()
            .iter()
            .any(|call| matches!(call, Call::DecideApproval(..))),
        "a first click only asks"
    );
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("approval-question").is_some());
        window.click("approval-confirm", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert!(backend.calls().contains(&Call::DecideApproval(
        id,
        Decision::Approve,
        "ab".repeat(32)
    )));
    store.read_with(cx, |store, _| {
        assert_eq!(
            store.approval(id).map(|approval| approval.state.clone()),
            Some(ApprovalState::Approved)
        );
    });
}

#[gpui_kit::test]
fn needs_yous_row_opens_the_list(cx: &mut TestAppContext) {
    let (_backend, _store, column, window) = open(cx, Shows::NeedsRow, &[payment()]);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("approvals-row", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(
        column.read_with(cx, |column, _| column.current()),
        Some(Page::Approvals(None))
    );
}

#[gpui_kit::test]
fn the_question_goes_when_the_payload_on_screen_changes(cx: &mut TestAppContext) {
    let id = ApprovalId::new(7);
    let (backend, store, _column, window) = open(cx, Shows::Detail(id), &[payment()]);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("approval-approve", cx);
    })
    .unwrap();
    cx.run_until_parked();
    // The request is read again with another payload while the question is up.
    let mut changed = payment();
    changed.payload_hash = "ef".repeat(32);
    changed.text = Some("Invoice EX-2".to_owned());
    backend.approvals.put(changed);
    store.update(cx, |store, cx| store.retry_approval(id, cx));
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(
            window.try_find("approval-question").is_none(),
            "a question asked on another payload is gone"
        );
        assert!(window.try_find("approval-approve").is_some());
    })
    .unwrap();
    assert!(
        !backend
            .calls()
            .iter()
            .any(|call| matches!(call, Call::DecideApproval(..))),
        "nothing was sent"
    );
}

#[gpui_kit::test]
fn a_request_waiting_on_someone_else_has_no_buttons(cx: &mut TestAppContext) {
    let id = ApprovalId::new(7);
    let mut theirs = payment();
    theirs.waits_on_owner = false;
    theirs.approver = "example-lead".to_owned();
    let (_backend, _store, _column, window) = open(cx, Shows::Detail(id), &[theirs]);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("approval-approve").is_none());
        assert!(window.try_find("approval-reject").is_none());
        assert!(
            window.try_find("approval-withdraw").is_some(),
            "a waiting request can still be withdrawn"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_list_row_has_no_reject(cx: &mut TestAppContext) {
    let (_backend, _store, _column, window) = open(cx, Shows::List, &[payment()]);
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("approval-7-approve").is_some());
        assert!(
            window.try_find("approval-7-reject").is_none(),
            "a decision is made only on the request's page"
        );
    })
    .unwrap();
}
