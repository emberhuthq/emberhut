//! Approval requests in the store, on GPUI's headless test platform with the fake backend as
//! a small server: when the pending requests are asked for, an answer's round trip, a request
//! that changed under the user, the live stream's change, and the counts.

use std::sync::Arc;
use std::time::Duration;

use emberhut_api::approvals::{Approval, ApprovalState, ApprovalsEvent, Decision};
use emberhut_api::ids::{ApprovalId, ConfiguredAgentId, RoomId};
use gpui_kit::{AppContext as _, Entity, TestAppContext};
use serde_json::json;

use super::Ask;
use crate::backend::fake::{Call, FakeBackend};
use crate::store::{Page, Store, ThemeState};

fn store(cx: &mut TestAppContext, backend: &Arc<FakeBackend>) -> Entity<Store> {
    let backend = Arc::clone(backend) as Arc<dyn crate::backend::Backend>;
    cx.new(|_| Store::idle(backend, None, ThemeState::load(None), RoomId::MAIN))
}

/// Request `id` of agent `client-a`, waiting on the user, with `more` over its fields.
fn approval(id: i64, more: &serde_json::Value) -> Approval {
    let mut base = json!({
        "id": id, "agent": "client-a", "agentName": "Example helper", "kind": "post",
        "target": "example blog", "summary": "The launch post", "text": "We shipped it.",
        "files": [], "payloadHash": "ab".repeat(32), "approver": "owner",
        "waitsOnOwner": true, "state": "pending", "createdAt": "2026-11-27T10:00:00Z"
    });
    if let (Some(base), Some(more)) = (base.as_object_mut(), more.as_object()) {
        base.extend(more.clone());
    }
    serde_json::from_value(base).unwrap()
}

fn count(backend: &FakeBackend, wanted: impl Fn(&Call) -> bool) -> usize {
    backend.calls().iter().filter(|call| wanted(call)).count()
}

fn list_reads(backend: &FakeBackend) -> usize {
    count(backend, |call| {
        *call == Call::Approvals("state=pending".to_owned())
    })
}

#[gpui_kit::test]
fn the_pending_are_asked_for_while_their_list_or_a_page_shows(cx: &mut TestAppContext) {
    let backend = Arc::new(FakeBackend::new([]));
    backend.approvals.put(approval(7, &json!({})));
    let store = store(cx, &backend);
    store.update(cx, |store, cx| {
        store.window.active = true;
        store.start_approvals(cx);
    });
    cx.run_until_parked();
    assert_eq!(list_reads(&backend), 0, "nothing shows them");
    let key = store.update(cx, |store, cx| {
        let key = store.agent_key(ConfiguredAgentId::new("client-a"));
        store.show_in_test_column(Some(Page::Approvals(Some(key))), cx);
        key
    });
    cx.run_until_parked();
    assert_eq!(list_reads(&backend), 1, "the agent's list opened");
    store.read_with(cx, |store, _| {
        assert_eq!(
            store
                .approvals()
                .agent_of(key)
                .map(ConfiguredAgentId::as_str),
            Some("client-a")
        );
        assert!(store.approvals().has_read());
        assert_eq!(store.pending_approvals().len(), 1);
    });
    cx.executor().advance_clock(Duration::from_secs(10));
    cx.run_until_parked();
    assert_eq!(list_reads(&backend), 2, "every ten seconds");
    store.update(cx, |store, cx| {
        store.show_in_test_column(Some(Page::Approval(ApprovalId::new(7))), cx);
    });
    cx.run_until_parked();
    assert_eq!(list_reads(&backend), 3, "a request's page opened");
    assert_eq!(
        count(&backend, |call| *call == Call::Approval(ApprovalId::new(7))),
        1,
        "and it is read on its own"
    );
    store.update(cx, |store, cx| store.show_in_test_column(None, cx));
    cx.executor().advance_clock(Duration::from_mins(2));
    cx.run_until_parked();
    assert_eq!(list_reads(&backend), 3, "shut: no more");
}

#[gpui_kit::test]
fn an_approve_asked_first_carries_the_hash_shown_and_takes_the_answer(cx: &mut TestAppContext) {
    let backend = Arc::new(FakeBackend::new([]));
    backend.approvals.put(approval(7, &json!({})));
    let store = store(cx, &backend);
    let id = ApprovalId::new(7);
    store.update(cx, |store, cx| {
        store.window.active = true;
        store.start_approvals(cx);
        store.show_in_test_column(Some(Page::Approval(id)), cx);
    });
    cx.run_until_parked();
    let shown = store.read_with(cx, |store, _| store.approval(id).cloned().unwrap());
    let ask = Ask::Approve {
        payload_hash: shown.payload_hash.clone(),
    };
    store.update(cx, |store, cx| {
        store.ask_approval(Some((id, ask.clone())), cx);
    });
    assert_eq!(
        store.read_with(cx, |store, _| store.approvals().asking(id).cloned()),
        Some(ask)
    );
    store.update(cx, |store, cx| {
        store.decide_approval(id, Decision::Approve, shown.payload_hash.clone(), cx);
    });
    cx.run_until_parked();
    assert!(backend.calls().contains(&Call::DecideApproval(
        id,
        Decision::Approve,
        "ab".repeat(32)
    )));
    store.read_with(cx, |store, _| {
        let feed = store.approvals();
        assert_eq!(feed.asking(id), None, "the question is answered");
        assert_eq!(feed.busy(), None);
        let notice = feed.notice(id).unwrap();
        assert!(!notice.failed);
        assert_eq!(
            store.approval(id).map(|a| a.state.clone()),
            Some(ApprovalState::Approved)
        );
        assert!(
            super::lists::Groups::of(store.pending_approvals(), None).is_empty(),
            "decided: gone from the list's groups"
        );
    });
    assert!(
        list_reads(&backend) >= 2,
        "the list is read again after the answer"
    );
}

#[gpui_kit::test]
fn a_request_that_changed_decides_nothing_and_shows_as_it_is_now(cx: &mut TestAppContext) {
    let backend = Arc::new(FakeBackend::new([]));
    backend.approvals.put(approval(7, &json!({})));
    let store = store(cx, &backend);
    let id = ApprovalId::new(7);
    store.update(cx, |store, cx| {
        store.show_in_test_column(Some(Page::Approval(id)), cx);
    });
    cx.run_until_parked();
    let old_hash = store.read_with(cx, |store, _| {
        store.approval(id).unwrap().payload_hash.clone()
    });
    // The agent asked again with other words: the server's request has another hash now.
    backend.approvals.put(approval(
        7,
        &json!({"text": "We shipped it!", "payloadHash": "ef".repeat(32)}),
    ));
    store.update(cx, |store, cx| {
        store.decide_approval(id, Decision::Approve, old_hash, cx);
    });
    cx.run_until_parked();
    store.read_with(cx, |store, _| {
        let notice = store.approvals().notice(id).unwrap();
        assert!(notice.failed);
        assert!(
            notice
                .text
                .starts_with("This request changed or was already decided"),
            "{}",
            notice.text
        );
        let now = store.approval(id).unwrap();
        assert_eq!(now.payload_hash, "ef".repeat(32), "read again");
        assert_eq!(now.text.as_deref(), Some("We shipped it!"));
        assert_eq!(now.state, ApprovalState::Pending, "nothing was decided");
    });
    assert_eq!(
        backend.approvals.served(id).map(|a| a.state),
        Some(ApprovalState::Pending)
    );
}

#[gpui_kit::test]
fn a_withdrawal_and_a_reject_go_one_at_a_time(cx: &mut TestAppContext) {
    let backend = Arc::new(FakeBackend::new([]));
    backend.approvals.put(approval(7, &json!({})));
    backend.approvals.put(approval(8, &json!({})));
    let store = store(cx, &backend);
    store.update(cx, |store, cx| {
        store.ask_approval(Some((ApprovalId::new(7), Ask::Withdraw)), cx);
        store.withdraw_approval(ApprovalId::new(7), cx);
        store.decide_approval(ApprovalId::new(8), Decision::Reject, "ab".repeat(32), cx);
    });
    cx.run_until_parked();
    assert_eq!(
        backend
            .approvals
            .served(ApprovalId::new(7))
            .map(|a| a.state),
        Some(ApprovalState::Withdrawn)
    );
    assert_eq!(
        backend
            .approvals
            .served(ApprovalId::new(8))
            .map(|a| a.state),
        Some(ApprovalState::Pending),
        "the reject came while the withdrawal was on its way: not sent"
    );
    assert_eq!(
        count(&backend, |call| matches!(call, Call::DecideApproval(..))),
        0
    );
}

#[gpui_kit::test]
fn the_live_streams_change_reads_what_shows_again(cx: &mut TestAppContext) {
    let backend = Arc::new(FakeBackend::new([]));
    backend.approvals.put(approval(7, &json!({})));
    let store = store(cx, &backend);
    let id = ApprovalId::new(7);
    store.update(cx, |store, cx| {
        store.window.active = true;
        store.start_approvals(cx);
        store.show_in_test_column(Some(Page::Approval(id)), cx);
    });
    cx.run_until_parked();
    let (lists, ones) = (
        list_reads(&backend),
        count(&backend, |call| *call == Call::Approval(id)),
    );
    backend.approvals.put(approval(
        7,
        &json!({"state": "expired", "waitsOnOwner": false}),
    ));
    store.update(cx, |store, cx| {
        let changed = ApprovalsEvent {
            id,
            state: ApprovalState::Expired,
            pending: 0,
        };
        store.approvals_changed(&changed, cx);
    });
    cx.run_until_parked();
    assert_eq!(list_reads(&backend), lists + 1);
    assert_eq!(
        count(&backend, |call| *call == Call::Approval(id)),
        ones + 1
    );
    assert_eq!(
        store.read_with(cx, |store, _| store.approval(id).map(|a| a.state.clone())),
        Some(ApprovalState::Expired)
    );
}

#[gpui_kit::test]
fn needs_you_counts_the_requests_waiting_on_you_and_agents_have_rooms(cx: &mut TestAppContext) {
    let backend = Arc::new(FakeBackend::new([]));
    let store = store(cx, &backend);
    store.update(cx, |store, cx| {
        let state = crate::fixtures::state(json!({
            "rooms": [crate::fixtures::room(1, json!({})), crate::fixtures::room(9, json!({}))],
            "agents": [
                {"id": "main", "name": "Main", "principal": "the owner", "room": 1,
                 "enabled": true, "pendingApprovals": 0},
                {"id": "client-a", "name": "Example helper", "principal": "Example Ltd",
                 "room": 9, "enabled": false, "pendingApprovals": 2}
            ],
            "approvals": [
                approval(7, &json!({"kind": "payment"})),
                approval(8, &json!({"waitsOnOwner": false, "approver": "example-lead"}))
            ]
        }));
        store.show_state_for_tests(state, cx);
    });
    store.read_with(cx, |store, _| {
        let summary = store.approvals_summary();
        assert_eq!((summary.yours, summary.only_you, summary.others), (1, 1, 1));
        assert_eq!(
            store.needs_count(),
            1,
            "no asks, one request waiting on you"
        );
        assert_eq!(
            store.pending_approvals().len(),
            2,
            "the state's, before a read"
        );
        let own = store.agent_rooms();
        assert_eq!(own.len(), 1);
        assert_eq!(
            own[0].waiting, 1,
            "the request on an approver counts nowhere here"
        );
        assert_eq!(store.waiting_on_you(&ConfiguredAgentId::new("client-a")), 1);
        assert_eq!(
            store.agent_of_room(RoomId::new(9)).map(|a| a.name.as_str()),
            Some("Example helper")
        );
        assert!(
            store.switcher().listed().is_empty(),
            "an agent's room is listed under Agents, not Rooms"
        );
    });
}

#[gpui_kit::test]
fn a_server_without_the_list_says_so_and_keeps_the_states(cx: &mut TestAppContext) {
    let backend = Arc::new(FakeBackend::new([]));
    backend.approvals.refuse_reads(Some(404));
    let store = store(cx, &backend);
    store.update(cx, |store, cx| {
        store.window.active = true;
        store.show_state_for_tests(
            crate::fixtures::state(json!({ "approvals": [approval(7, &json!({}))] })),
            cx,
        );
        store.start_approvals(cx);
        store.show_in_test_column(Some(Page::Approvals(None)), cx);
    });
    cx.run_until_parked();
    store.read_with(cx, |store, _| {
        assert_eq!(
            store.approvals().problem(),
            Some("Emberhut's server is on older code: approvals come with its next update")
        );
        assert!(!store.approvals().has_read());
        assert_eq!(
            store.pending_approvals().len(),
            1,
            "the state's pending requests still show"
        );
    });
}

#[gpui_kit::test]
fn a_list_read_older_than_the_answer_never_brings_the_buttons_back(cx: &mut TestAppContext) {
    let backend = Arc::new(FakeBackend::new([]));
    backend.approvals.put(approval(7, &json!({})));
    let store = store(cx, &backend);
    let id = ApprovalId::new(7);
    store.update(cx, |store, cx| {
        store.show_in_test_column(Some(Page::Approval(id)), cx);
    });
    cx.run_until_parked();
    // A list read starts now, with request 7 pending...
    let stale = backend.approvals.page(&super::pending_query()).unwrap();
    store.update(cx, |store, cx| {
        store.decide_approval(id, Decision::Approve, "ab".repeat(32), cx);
    });
    cx.run_until_parked();
    // ...and lands after the answer.
    store.update(cx, |store, _| {
        store.take_approvals(Ok(stale));
    });
    store.read_with(cx, |store, _| {
        assert_eq!(
            store.approval(id).map(|a| a.state.clone()),
            Some(ApprovalState::Approved),
            "the page keeps the answer"
        );
        assert!(
            super::lists::Groups::of(store.pending_approvals(), None).is_empty(),
            "and the list doesn't wait on it again"
        );
    });
    // A single read that started before the answer is left out too.
    store.update(cx, |store, _| {
        store.approvals.take_one(approval(7, &json!({})));
    });
    assert_eq!(
        store.read_with(cx, |store, _| store.approval(id).map(|a| a.state.clone())),
        Some(ApprovalState::Approved)
    );
}

#[gpui_kit::test]
fn a_request_the_server_doesnt_have_says_so(cx: &mut TestAppContext) {
    let backend = Arc::new(FakeBackend::new([]));
    let store = store(cx, &backend);
    let id = ApprovalId::new(999);
    store.update(cx, |store, cx| {
        store.show_in_test_column(Some(Page::Approval(id)), cx);
    });
    cx.run_until_parked();
    store.read_with(cx, |store, _| {
        assert_eq!(store.approvals().unread(id), Some(&super::Unread::Gone));
        assert!(store.approval(id).is_none());
    });
}

#[gpui_kit::test]
fn an_answer_lost_on_the_way_reads_the_request_again(cx: &mut TestAppContext) {
    let backend = Arc::new(FakeBackend::new([]));
    backend.approvals.put(approval(7, &json!({})));
    backend.approvals.lose_answers();
    let store = store(cx, &backend);
    let id = ApprovalId::new(7);
    store.update(cx, |store, cx| {
        store.show_in_test_column(Some(Page::Approval(id)), cx);
    });
    cx.run_until_parked();
    store.update(cx, |store, cx| {
        store.decide_approval(id, Decision::Approve, "ab".repeat(32), cx);
    });
    cx.run_until_parked();
    store.read_with(cx, |store, _| {
        let notice = store.approvals().notice(id).unwrap();
        assert!(notice.text.contains("may or may not have gone through"));
        assert_eq!(
            store.approval(id).map(|a| a.state.clone()),
            Some(ApprovalState::Approved),
            "read again: it went through"
        );
    });
}
