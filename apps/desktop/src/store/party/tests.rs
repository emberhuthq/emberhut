//! Party's glue, on GPUI's headless test platform with a scripted backend: the list read, a new
//! party started or refused, a party deleted, the controls, and the stream's thread: followed
//! again after an end, given up on a refusal, waited for longer after each failure.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use emberhut_api::ids::PartyId;
use emberhut_api::party::{Party, PartyAction, PartyEvent, PartySay, SayKind};
use gpui_kit::{AppContext as _, Entity, TestAppContext};

use super::roster::tests::{page, party};
use super::{gone_words, is_final, next_wait};
use crate::backend::fake::{Call, FakeBackend};
use crate::backend::{Backend, Cause, Error};
use crate::store::Store;

/// Runs the app's tasks, moving the clock on by `step` each round, until `done` says so. The
/// streams block threads of their own, beside the test's executor, so a test waits for them
/// (a few seconds at most), having let them wake its tasks
/// (`cx.executor().allow_parking()`) before it started any.
pub(crate) fn settle(
    cx: &mut TestAppContext,
    step: Duration,
    mut done: impl FnMut(&mut TestAppContext) -> bool,
) {
    for _ in 0..2_500 {
        cx.run_until_parked();
        if done(cx) {
            return;
        }
        cx.executor().advance_clock(step);
        std::thread::sleep(Duration::from_millis(2));
    }
    panic!("the store never got there");
}

fn follows(backend: &FakeBackend, id: i64) -> usize {
    backend
        .calls()
        .into_iter()
        .filter(|call| *call == Call::FollowParty(PartyId::new(id)))
        .count()
}

fn store(cx: &mut TestAppContext, backend: &Arc<FakeBackend>) -> Entity<Store> {
    let backend = Arc::clone(backend) as Arc<dyn Backend>;
    cx.new(|_| Store::for_tests(backend))
}

fn a_party(id: i64, status: &str) -> Party {
    serde_json::from_value(party(id, status)).unwrap()
}

#[gpui_kit::test]
fn the_list_is_read_and_a_failure_says_why(cx: &mut TestAppContext) {
    let backend = Arc::new(FakeBackend::new([]));
    let store = store(cx, &backend);
    store.update(cx, Store::load_parties);
    cx.run_until_parked();
    store.read_with(cx, |store, _| {
        let roster = store.party().roster();
        assert!(!roster.is_loaded());
        assert!(roster.problem().unwrap().contains("older code"), "a 404");
    });
    backend.party.push_parties(Ok(page()));
    store.update(cx, Store::load_parties);
    cx.run_until_parked();
    store.read_with(cx, |store, _| {
        assert_eq!(store.party().roster().parties().len(), 2);
        assert_eq!(store.party().roster().problem(), None);
    });
}

#[gpui_kit::test]
fn a_new_party_starts_with_its_cast_and_opens(cx: &mut TestAppContext) {
    // The stream blocks a thread of its own, which wakes this test's tasks from outside it.
    cx.executor().allow_parking();
    let backend = Arc::new(FakeBackend::new([]));
    backend.party.push_parties(Ok(page()));
    let store = store(cx, &backend);
    store.update(cx, |store, cx| store.show_new_party(true, cx));
    cx.run_until_parked();
    store.read_with(cx, |store, _| {
        assert_eq!(
            store.party().draft().unwrap().cast().len(),
            3,
            "the starting cast, once the presets came"
        );
    });
    backend.party.push_started(Ok(a_party(9, "running")));
    store.update(cx, |store, cx| store.start_party("  Tea or coffee ", cx));
    cx.run_until_parked();
    store.read_with(cx, |store, _| {
        assert!(store.party().draft().is_none(), "the form closed");
        assert_eq!(store.party().room().unwrap().id(), PartyId::new(9));
    });
    let started = backend.calls().into_iter().find_map(|call| match call {
        Call::StartParty(new) => Some(new),
        _ => None,
    });
    assert_eq!(started.unwrap().topic, "Tea or coffee");
    settle(cx, Duration::ZERO, |_| follows(&backend, 9) == 1);
    store.update(cx, |store, cx| store.open_party(None, cx));
}

#[gpui_kit::test]
fn a_refused_party_keeps_the_form_with_the_servers_reason(cx: &mut TestAppContext) {
    let backend = Arc::new(FakeBackend::new([]));
    backend.party.push_parties(Ok(page()));
    let store = store(cx, &backend);
    store.update(cx, |store, cx| store.show_new_party(true, cx));
    cx.run_until_parked();
    backend.party.push_started(Err(Error::Refused {
        status: Some(400),
        message: Some("each persona needs a name of its own".into()),
    }));
    store.update(cx, |store, cx| store.start_party("Tea", cx));
    cx.run_until_parked();
    store.read_with(cx, |store, _| {
        let draft = store.party().draft().unwrap();
        assert!(!draft.busy);
        assert_eq!(
            draft.problem.as_deref(),
            Some("each persona needs a name of its own")
        );
        assert!(store.party().room().is_none());
    });
}

#[gpui_kit::test]
fn a_deleted_party_goes_at_once(cx: &mut TestAppContext) {
    let backend = Arc::new(FakeBackend::new([]));
    backend.party.push_parties(Ok(page()));
    let store = store(cx, &backend);
    store.update(cx, Store::load_parties);
    cx.run_until_parked();
    store.update(cx, |store, cx| {
        store.delete_party(PartyId::new(2), cx);
        assert_eq!(store.party().roster().parties().len(), 1);
    });
    cx.run_until_parked();
    assert!(
        backend
            .calls()
            .contains(&Call::ActOnParty(PartyId::new(2), PartyAction::Delete))
    );
}

#[gpui_kit::test]
fn the_controls_act_on_the_party_open(cx: &mut TestAppContext) {
    // The stream blocks a thread of its own, which wakes this test's tasks from outside it.
    cx.executor().allow_parking();
    let backend = Arc::new(FakeBackend::new([]));
    let store = store(cx, &backend);
    store.update(cx, |store, cx| store.open_party(Some(PartyId::new(3)), cx));
    backend.party.push_act(Ok(Some(a_party(3, "paused"))));
    store.update(cx, |store, cx| store.act_on_party(PartyAction::Pause, cx));
    cx.run_until_parked();
    store.read_with(cx, |store, _| {
        let room = store.party().room().unwrap();
        assert_eq!(room.party().unwrap().status.as_str(), "paused");
    });
    backend.party.push_act(Err(Error::Refused {
        status: Some(400),
        message: Some("say something".into()),
    }));
    store.update(cx, |store, cx| {
        store.say_in_party("   ", false, cx);
        store.say_in_party(" Now the other side ", true, cx);
    });
    cx.run_until_parked();
    store.read_with(cx, |store, _| {
        assert_eq!(
            store.party().room().unwrap().problem(),
            Some("say something")
        );
    });
    let steer = PartyAction::Say(PartySay {
        text: "Now the other side".into(),
        kind: SayKind::Steer,
    });
    let said: Vec<_> = backend
        .calls()
        .into_iter()
        .filter(|call| matches!(call, Call::ActOnParty(..)))
        .collect();
    assert_eq!(
        said,
        [
            Call::ActOnParty(PartyId::new(3), PartyAction::Pause),
            Call::ActOnParty(PartyId::new(3), steer)
        ],
        "a blank line says nothing"
    );
    store.update(cx, |store, cx| store.open_party(None, cx));
}

#[gpui_kit::test]
fn leaving_stops_the_party_and_drops_the_form(cx: &mut TestAppContext) {
    // The stream blocks a thread of its own, which wakes this test's tasks from outside it.
    cx.executor().allow_parking();
    let backend = Arc::new(FakeBackend::new([]));
    let store = store(cx, &backend);
    store.update(cx, |store, cx| {
        store.open_party(Some(PartyId::new(4)), cx);
        store.show_new_party(true, cx);
        let generation = store.party.generation.load(Ordering::Relaxed);
        store.leave_parties(cx);
        assert!(store.party().room().is_none() && store.party().draft().is_none());
        assert_ne!(store.party.generation.load(Ordering::Relaxed), generation);
    });
}

#[gpui_kit::test]
fn a_party_whose_window_closed_before_it_started_isnt_opened(cx: &mut TestAppContext) {
    let backend = Arc::new(FakeBackend::new([]));
    backend.party.push_parties(Ok(page()));
    let store = store(cx, &backend);
    store.update(cx, |store, cx| store.show_new_party(true, cx));
    cx.run_until_parked();
    backend.party.push_started(Ok(a_party(9, "running")));
    store.update(cx, |store, cx| {
        store.start_party("Tea or coffee", cx);
        store.leave_parties(cx);
    });
    cx.run_until_parked();
    store.read_with(cx, |store, _| {
        assert!(store.party().room().is_none() && store.party().draft().is_none());
    });
    assert_eq!(
        follows(&backend, 9),
        0,
        "nothing would ever stop its stream"
    );
}

#[gpui_kit::test]
fn the_party_open_is_followed_live_and_again_after_its_stream_ends(cx: &mut TestAppContext) {
    // The stream blocks a thread of its own, which wakes this test's tasks from outside it.
    cx.executor().allow_parking();
    let backend = Arc::new(FakeBackend::new([]));
    backend.party.push_live(vec![PartyEvent::Snapshot {
        party: Some(a_party(3, "running")),
        messages: Vec::new(),
    }]);
    backend.party.push_live(vec![PartyEvent::Party {
        party: Some(a_party(3, "paused")),
    }]);
    let store = store(cx, &backend);
    store.update(cx, |store, cx| store.open_party(Some(PartyId::new(3)), cx));
    let status = |cx: &mut TestAppContext| {
        store.read_with(cx, |store, _| {
            let room = store.party().room()?;
            Some(room.party()?.status.as_str().to_owned())
        })
    };
    settle(cx, Duration::ZERO, |cx| {
        status(cx).as_deref() == Some("running")
    });
    settle(cx, Duration::from_secs(2), |cx| {
        status(cx).as_deref() == Some("paused")
    });
    assert_eq!(follows(&backend, 3), 2, "followed again");
    store.update(cx, |store, cx| store.open_party(None, cx));
    for _ in 0..20 {
        cx.executor().advance_clock(Duration::from_secs(2));
        cx.run_until_parked();
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(
        follows(&backend, 3),
        2,
        "not followed once the party closed"
    );
}

#[gpui_kit::test]
fn a_party_the_server_doesnt_have_is_gone_and_not_asked_for_again(cx: &mut TestAppContext) {
    // The stream blocks a thread of its own, which wakes this test's tasks from outside it.
    cx.executor().allow_parking();
    let backend = Arc::new(FakeBackend::new([]));
    backend.party.push_live_end(Err(Error::Refused {
        status: Some(404),
        message: None,
    }));
    let store = store(cx, &backend);
    store.update(cx, |store, cx| store.open_party(Some(PartyId::new(5)), cx));
    let problem = |cx: &mut TestAppContext| {
        store.read_with(cx, |store, _| {
            store.party().room()?.problem().map(str::to_owned)
        })
    };
    settle(cx, Duration::ZERO, |cx| problem(cx).is_some());
    assert_eq!(problem(cx).as_deref(), Some("This party is gone."));
    for _ in 0..20 {
        cx.executor().advance_clock(Duration::from_secs(30));
        cx.run_until_parked();
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(follows(&backend, 5), 1, "one request, no retry");
    store.update(cx, |store, cx| store.open_party(None, cx));
}

#[gpui_kit::test]
fn a_stream_that_cant_reach_the_server_waits_longer_each_time(cx: &mut TestAppContext) {
    // The stream blocks a thread of its own, which wakes this test's tasks from outside it.
    cx.executor().allow_parking();
    let backend = Arc::new(FakeBackend::new([]));
    for _ in 0..3 {
        backend
            .party
            .push_live_end(Err(Error::Unreachable(Cause::from("connection refused"))));
    }
    let store = store(cx, &backend);
    store.update(cx, |store, cx| store.open_party(Some(PartyId::new(6)), cx));
    let problem = |cx: &mut TestAppContext| {
        store.read_with(cx, |store, _| {
            store.party().room()?.problem().map(str::to_owned)
        })
    };
    settle(cx, Duration::ZERO, |cx| {
        problem(cx).as_deref() == Some("Reconnecting…")
    });
    assert_eq!(follows(&backend, 6), 1);
    // 2 s, then 4 s: the third request waits for the clock to pass the second wait whole.
    settle(cx, Duration::from_secs(2), |_| follows(&backend, 6) == 2);
    for _ in 0..30 {
        cx.executor().advance_clock(Duration::from_millis(100));
        cx.run_until_parked();
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(follows(&backend, 6), 2, "not yet: the wait doubled");
    settle(cx, Duration::from_secs(4), |_| follows(&backend, 6) == 3);
    store.update(cx, |store, cx| store.open_party(None, cx));
}

#[test]
fn the_wait_doubles_after_each_failure_up_to_half_a_minute() {
    let waits: Vec<u64> = (0..8)
        .map(|failures| next_wait(failures).as_secs())
        .collect();
    assert_eq!(waits, [2, 2, 4, 8, 16, 30, 30, 30]);
}

#[test]
fn a_refusal_is_final_and_says_why() {
    let refused = |status, message: Option<&str>| Error::Refused {
        status: Some(status),
        message: message.map(str::to_owned),
    };
    assert!(is_final(&refused(404, None)));
    assert!(is_final(&refused(403, None)));
    assert!(is_final(&Error::SignInRequired));
    assert!(!is_final(&refused(502, None)));
    assert!(!is_final(&Error::Unreachable(Cause::from("timed out"))));
    assert_eq!(gone_words(&refused(404, None)), "This party is gone.");
    assert_eq!(
        gone_words(&refused(403, Some("not yours"))),
        "not yours",
        "the server's own words"
    );
}
