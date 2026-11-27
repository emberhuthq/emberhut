//! The store's glue, on GPUI's headless test platform with a scripted backend: what a poll's
//! reply does to what the views show, a message's note, an answer's way to the server (on
//! quitting too), the live stream's end of a turn, what the server is told the user reads, how
//! often the state and the machines are asked for wherever the window is, and Later's fold on
//! Needs you.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use emberhut_api::headers::ETag;
use emberhut_api::ids::{RoomId, TaskId};
use emberhut_api::live::{Live, LiveEvent};
use emberhut_api::talk::TalkRequest;
use gpui_kit::{
    AppContext as _, Context, Empty, Entity, IntoElement, Render, TestAppContext, Window,
};
use serde_json::json;

use super::answers::UNDO_WINDOW;
use super::{Page, PendingMessage, Store, ThemeState};
use crate::backend::fake::{Call, FakeBackend};
use crate::backend::{Error, FreshState, StateReply};
use crate::fixtures::{message, message_at, state, waiting};

const OTHER: RoomId = RoomId::new(2);

fn store(cx: &mut TestAppContext, backend: &Arc<FakeBackend>) -> Entity<Store> {
    let backend = Arc::clone(backend) as Arc<dyn crate::backend::Backend>;
    cx.new(|_| Store::idle(backend, None, ThemeState::load(None), RoomId::MAIN))
}

fn fresh_state(extra: serde_json::Value) -> StateReply {
    StateReply::Fresh(FreshState {
        state: Box::new(state(extra)),
        etag: Some(ETag::new("\"v1\"")),
    })
}

fn unreachable() -> Result<StateReply, Error> {
    Err(Error::Unreachable("connection refused".into()))
}

#[gpui_kit::test]
fn a_reply_for_the_room_just_left_is_ignored(cx: &mut TestAppContext) {
    let backend = Arc::new(FakeBackend::new([]));
    let store = store(cx, &backend);
    store.update(cx, |store, cx| {
        store.switch_room(OTHER, cx);
        let failing = store.apply(
            RoomId::MAIN,
            Ok(fresh_state(json!({"on": true}))),
            Instant::now(),
            cx,
        );
        assert!(!failing);
        assert_eq!(store.state(), None, "Main's state doesn't show in room 2");
    });
}

#[gpui_kit::test]
fn a_fresh_state_settles_the_pending_message_and_the_answers(cx: &mut TestAppContext) {
    let backend = Arc::new(FakeBackend::new([]));
    let store = store(cx, &backend);
    store.update(cx, |store, cx| {
        store.pending = Some(PendingMessage {
            text: "Hello".into(),
            ..PendingMessage::default()
        });
        store.answers.answer(TaskId::new(5), "Yes".into());
        store.answers.flush();
        store.answers.answer(TaskId::new(6), "No".into());
        store.answers.flush();
        let mine = message_at(
            9,
            "2026-01-01T00:00:00Z",
            json!({"text": "Hello", "mine": true}),
        );
        let still_waiting = waiting(6, json!({}));
        let reply = Ok(fresh_state(
            json!({"chat": [mine], "working": [still_waiting]}),
        ));
        store.apply(RoomId::MAIN, reply, Instant::now(), cx);
        assert_eq!(store.pending(), None, "the chat has it now");
        assert!(
            !store.answers().gone().contains(&TaskId::new(5)),
            "the server agrees"
        );
        assert!(
            store.answers().gone().contains(&TaskId::new(6)),
            "still waiting there"
        );
        assert_eq!(store.chats().messages(RoomId::MAIN).len(), 1);
        assert_eq!(store.etag, Some(ETag::new("\"v1\"")));
    });
}

#[gpui_kit::test]
fn sign_in_shows_until_a_state_comes(cx: &mut TestAppContext) {
    let backend = Arc::new(FakeBackend::new([]));
    let store = store(cx, &backend);
    store.update(cx, |store, cx| {
        assert!(store.apply(
            RoomId::MAIN,
            Ok(StateReply::SignInRequired),
            Instant::now(),
            cx
        ));
        assert!(store.shows_sign_in());
        assert!(store.apply(RoomId::MAIN, Err(Error::SignInRequired), Instant::now(), cx));
        assert!(store.shows_sign_in());
        store.apply(RoomId::MAIN, Ok(fresh_state(json!({}))), Instant::now(), cx);
        assert!(!store.shows_sign_in());
    });
}

#[gpui_kit::test]
fn an_unchanged_state_after_a_failure_clears_the_problem(cx: &mut TestAppContext) {
    let backend = Arc::new(FakeBackend::new([]));
    let store = store(cx, &backend);
    store.update(cx, |store, cx| {
        assert!(store.apply(RoomId::MAIN, unreachable(), Instant::now(), cx));
        assert_eq!(store.problem(), Some("Can't reach the server · retrying"));
        assert!(!store.apply(RoomId::MAIN, Ok(StateReply::Unchanged), Instant::now(), cx));
        assert_eq!(store.problem(), None);
    });
}

#[gpui_kit::test]
fn the_poll_retries_quickly_and_recovers(cx: &mut TestAppContext) {
    let backend = Arc::new(FakeBackend::new([
        unreachable(),
        Ok(fresh_state(json!({"on": true}))),
    ]));
    let store = store(cx, &backend);
    store.update(cx, |store, cx| {
        store.window.active = true; // in front, where the retries are quick
        store.start_poll(cx);
    });
    cx.run_until_parked();
    store.read_with(cx, |store, _| assert!(store.problem().is_some()));
    cx.executor().advance_clock(super::poll::RETRY_EVERY);
    cx.run_until_parked();
    store.read_with(cx, |store, _| {
        assert_eq!(store.problem(), None);
        assert!(store.state().is_some_and(|state| state.on));
    });
    let polls: Vec<Call> = backend.calls();
    assert_eq!(
        polls,
        [
            Call::State(RoomId::MAIN, None, false),
            Call::State(RoomId::MAIN, None, false)
        ],
        "the failed poll had no fingerprint to send"
    );
}

#[gpui_kit::test]
fn a_message_gets_the_note_of_its_own_reply(cx: &mut TestAppContext) {
    let backend = Arc::new(FakeBackend::new([]));
    backend.push_talk(Err(Error::Refused {
        status: Some(500),
        message: Some("Too long".into()),
    }));
    let store = store(cx, &backend);
    store.update(cx, |store, cx| assert!(store.talk("  Hello  ", cx)));
    store.read_with(cx, |store, _| {
        assert_eq!(
            store.pending().and_then(PendingMessage::note).as_deref(),
            Some("Sending…")
        );
    });
    cx.run_until_parked();
    store.read_with(cx, |store, _| {
        assert_eq!(
            store.pending().and_then(PendingMessage::note).as_deref(),
            Some("Not sent: Too long")
        );
    });
    assert_eq!(
        backend.calls(),
        [Call::Talk(TalkRequest::text(RoomId::MAIN, "Hello"))]
    );
}

#[gpui_kit::test]
fn an_answer_goes_to_the_server_when_its_undo_window_ends(cx: &mut TestAppContext) {
    let backend = Arc::new(FakeBackend::new([]));
    let store = store(cx, &backend);
    store.update(cx, |store, cx| {
        store.decide(TaskId::new(4), "Yes".into(), cx);
    });
    cx.executor()
        .advance_clock(UNDO_WINDOW.saturating_sub(Duration::from_millis(1)));
    cx.run_until_parked();
    assert_eq!(backend.calls(), [], "still undoable");
    cx.executor().advance_clock(Duration::from_millis(1));
    cx.run_until_parked();
    assert_eq!(
        backend.calls(),
        [Call::Decide(TaskId::new(4), "Yes".into())]
    );
    store.read_with(cx, |store, _| assert_eq!(store.answers().toast(), None));
}

#[gpui_kit::test]
fn an_undone_answer_never_reaches_the_server(cx: &mut TestAppContext) {
    let backend = Arc::new(FakeBackend::new([]));
    let store = store(cx, &backend);
    store.update(cx, |store, cx| {
        store.decide(TaskId::new(4), "Yes".into(), cx);
        store.undo(1, cx);
    });
    cx.executor().advance_clock(UNDO_WINDOW * 2);
    cx.run_until_parked();
    assert_eq!(backend.calls(), []);
}

#[gpui_kit::test]
fn quitting_within_the_undo_window_sends_the_answer_once(cx: &mut TestAppContext) {
    // The test scheduler gives a blocking wait a random number of steps: enough for the send.
    cx.executor().set_block_on_ticks(1000..=1000);
    let backend = Arc::new(FakeBackend::new([]));
    let store = store(cx, &backend);
    cx.update(|cx| Store::send_held_on_quit(&store, cx));
    store.update(cx, |store, cx| {
        store.decide(TaskId::new(4), "Yes".into(), cx);
    });
    cx.quit();
    assert_eq!(
        backend.calls(),
        [Call::Decide(TaskId::new(4), "Yes".into())],
        "sent before the app is gone"
    );
    cx.executor().advance_clock(UNDO_WINDOW * 2);
    cx.run_until_parked();
    assert_eq!(
        backend.calls().len(),
        1,
        "the window ending sends nothing more"
    );
}

/// Stands in for the main window's views, which hold the store: closing the window drops them.
struct Holder {
    _store: Entity<Store>,
}

impl Render for Holder {
    fn render(&mut self, _: &mut Window, _: &mut Context<'_, Self>) -> impl IntoElement {
        Empty
    }
}

#[gpui_kit::test]
fn closing_the_window_within_the_undo_window_sends_the_answer_once(cx: &mut TestAppContext) {
    cx.executor().set_block_on_ticks(1000..=1000);
    let backend = Arc::new(FakeBackend::new([]));
    let store = store(cx, &backend);
    cx.update(|cx| Store::send_held_on_quit(&store, cx));
    store.update(cx, |store, cx| {
        store.decide(TaskId::new(4), "Yes".into(), cx);
    });
    let kept = store.downgrade();
    // From here on the window's views and the quit hook are the only ones holding the store.
    let window = cx.add_window(|_, _| Holder { _store: store });
    window
        .update(cx, |_, window, _| window.remove_window())
        .expect("the window is open");
    cx.run_until_parked();
    assert!(
        kept.upgrade().is_some(),
        "the store outlives the window it was in"
    );
    // Closing the window quits the app.
    cx.quit();
    assert_eq!(
        backend.calls(),
        [Call::Decide(TaskId::new(4), "Yes".into())],
        "sent before the app is gone"
    );
    cx.executor().advance_clock(UNDO_WINDOW * 2);
    cx.run_until_parked();
    assert_eq!(backend.calls().len(), 1, "sent once");
}

#[gpui_kit::test]
fn quitting_waits_for_an_answer_already_on_its_way(cx: &mut TestAppContext) {
    cx.executor().set_block_on_ticks(1000..=1000);
    let backend = Arc::new(FakeBackend::new([]));
    let store = store(cx, &backend);
    cx.update(|cx| Store::send_held_on_quit(&store, cx));
    store.update(cx, |store, cx| {
        store.decide(TaskId::new(4), "Yes".into(), cx);
        store.decide(TaskId::new(5), "No".into(), cx);
    });
    cx.quit();
    let calls = backend.calls();
    assert_eq!(calls.len(), 2, "each sent once: {calls:?}");
    assert!(calls.contains(&Call::Decide(TaskId::new(4), "Yes".into())));
    assert!(calls.contains(&Call::Decide(TaskId::new(5), "No".into())));
}

#[gpui_kit::test]
fn quitting_doesnt_send_an_undone_answer(cx: &mut TestAppContext) {
    let backend = Arc::new(FakeBackend::new([]));
    let store = store(cx, &backend);
    cx.update(|cx| Store::send_held_on_quit(&store, cx));
    store.update(cx, |store, cx| {
        store.decide(TaskId::new(4), "Yes".into(), cx);
        store.undo(1, cx);
    });
    cx.quit();
    cx.run_until_parked();
    assert_eq!(backend.calls(), []);
}

#[gpui_kit::test]
fn the_window_going_to_the_background_sends_the_answer_at_once(cx: &mut TestAppContext) {
    let backend = Arc::new(FakeBackend::new([]));
    let store = store(cx, &backend);
    let cx = cx.add_empty_window();
    cx.update(|window, cx| {
        Store::send_held_when_inactive(&store, window, cx);
        window.activate_window();
    });
    cx.run_until_parked();
    assert_eq!(backend.calls(), [], "coming to the front sends nothing");
    store.update(cx, |store, cx| {
        store.decide(TaskId::new(4), "Yes".into(), cx);
    });
    cx.deactivate_window();
    assert_eq!(
        backend.calls(),
        [Call::Decide(TaskId::new(4), "Yes".into())]
    );
    cx.executor().advance_clock(UNDO_WINDOW * 2);
    cx.run_until_parked();
    assert_eq!(backend.calls().len(), 1, "sent once");
}

#[gpui_kit::test]
fn the_end_of_a_turn_kicks_the_poll_and_other_rooms_are_ignored(cx: &mut TestAppContext) {
    let backend = Arc::new(FakeBackend::new([]));
    let store = store(cx, &backend);
    store.update(cx, Store::start_poll);
    cx.run_until_parked();
    assert_eq!(backend.calls().len(), 1, "the first poll");
    let step = Live {
        text: String::new(),
        doing: Some("Reading".into()),
    };
    store.update(cx, |store, cx| {
        store.take_live(OTHER, LiveEvent::Working(step.clone()), cx);
        assert_eq!(store.live(), None, "room 2's step isn't Main's");
        store.take_live(RoomId::MAIN, LiveEvent::Working(step.clone()), cx);
        assert_eq!(store.live(), Some(&step));
        store.take_live(RoomId::MAIN, LiveEvent::Idle, cx);
        assert_eq!(store.live(), None);
    });
    // The clock hasn't moved: only the kick can have polled again.
    cx.run_until_parked();
    assert_eq!(backend.calls().len(), 2, "the turn's end polled at once");
}

mod reading {
    use super::*;
    use crate::store::poll::POLL_EVERY;
    use crate::store::reading::AWAY_AFTER;

    /// Main's state with unread answers up to message `newest`.
    fn unread_up_to(newest: i64) -> StateReply {
        fresh_state(json!({"unread": true, "chat": [message(newest)]}))
    }

    fn seen_calls(backend: &FakeBackend) -> usize {
        backend
            .calls()
            .iter()
            .filter(|call| **call == Call::Seen(RoomId::MAIN))
            .count()
    }

    /// Each state poll's looking flag, in turn.
    fn looking(backend: &FakeBackend) -> Vec<bool> {
        backend
            .calls()
            .iter()
            .filter_map(|call| match call {
                Call::State(_, _, looking) => Some(*looking),
                _ => None,
            })
            .collect()
    }

    #[gpui_kit::test]
    fn the_poll_says_the_room_is_looked_at_only_while_it_is_read(cx: &mut TestAppContext) {
        let backend = Arc::new(FakeBackend::new([]));
        let store = store(cx, &backend);
        store.update(cx, Store::start_poll);
        cx.run_until_parked();
        // The window coming to the front polls at once, to say so.
        store.update(cx, |store, cx| store.set_window_active(true, cx));
        cx.run_until_parked();
        // Tasks opens in the column: Main's chat stays beside it, still read.
        store.update(cx, |store, cx| {
            store.show_in_test_column(Some(Page::Tasks), cx);
        });
        cx.executor().advance_clock(POLL_EVERY);
        cx.run_until_parked();
        store.update(cx, |store, cx| store.set_window_active(false, cx));
        cx.executor().advance_clock(POLL_EVERY);
        cx.run_until_parked();
        assert_eq!(
            looking(&backend),
            [false, true, true, false],
            "a background window, then Main in front, then Tasks beside it, then sent back"
        );
    }

    #[gpui_kit::test]
    fn a_window_sent_to_the_back_stops_being_read(cx: &mut TestAppContext) {
        let backend = Arc::new(FakeBackend::new([]));
        let store = store(cx, &backend);
        store.update(cx, Store::start_poll);
        cx.run_until_parked();
        store.update(cx, |store, cx| store.set_window_active(true, cx));
        cx.run_until_parked();
        store.update(cx, |store, cx| store.set_window_active(false, cx));
        cx.executor().advance_clock(POLL_EVERY);
        cx.run_until_parked();
        assert_eq!(
            looking(&backend),
            [false, true, false],
            "in the background, in front, in the background again"
        );
        store.update(cx, |store, cx| {
            store.apply(RoomId::MAIN, Ok(unread_up_to(9)), Instant::now(), cx);
        });
        cx.run_until_parked();
        assert_eq!(seen_calls(&backend), 0, "no one reads it there");
    }

    #[gpui_kit::test]
    fn an_active_window_no_one_can_see_isnt_read_until_it_is_shown(cx: &mut TestAppContext) {
        let backend = Arc::new(FakeBackend::new([]));
        let store = store(cx, &backend);
        store.update(cx, Store::start_poll);
        cx.run_until_parked();
        store.update(cx, |store, cx| store.set_window_active(true, cx));
        cx.run_until_parked();
        // The screen locks, or another app's full-screen window covers it without taking the
        // focus: still the active window, with input a moment ago, but out of sight.
        store.update(cx, |store, _| store.set_window_visible(false));
        // The end of a turn kicks the poll (the live stream does): it goes, but no one looks.
        store.update(cx, |store, _| store.kicker.kick());
        cx.run_until_parked();
        assert_eq!(
            looking(&backend),
            [false, true, false],
            "in the background, in front, out of sight"
        );
        store.update(cx, |store, cx| {
            store.apply(RoomId::MAIN, Ok(unread_up_to(9)), Instant::now(), cx);
        });
        cx.run_until_parked();
        assert_eq!(seen_calls(&backend), 0, "the new answer waits for them");
        // The clock doesn't move: shown again, the poll goes at once and the answer is read.
        store.update(cx, |store, _| store.set_window_visible(true));
        cx.run_until_parked();
        assert_eq!(seen_calls(&backend), 1);
        assert_eq!(looking(&backend).get(3), Some(&true), "in sight again");
    }

    #[gpui_kit::test]
    fn a_window_no_one_used_for_five_minutes_isnt_read_until_they_come_back(
        cx: &mut TestAppContext,
    ) {
        let backend = Arc::new(FakeBackend::new([]));
        let store = store(cx, &backend);
        store.update(cx, Store::start_poll);
        cx.run_until_parked();
        store.update(cx, |store, cx| store.set_window_active(true, cx));
        cx.run_until_parked();
        // Still in front, but the screen locked or the user left the desk. The clock passes
        // each poll on the way: looked at until the five minutes are up, then not.
        cx.executor().advance_clock(AWAY_AFTER);
        cx.run_until_parked();
        let away = looking(&backend);
        let (last, before) = away.split_last().expect("polls");
        assert!(!last, "five minutes without input");
        assert!(before.len() > 2, "{before:?}");
        assert!(!before[0], "the background window");
        assert!(before[1..].iter().all(|looked| *looked), "{before:?}");
        store.update(cx, |store, cx| {
            store.apply(RoomId::MAIN, Ok(unread_up_to(9)), Instant::now(), cx);
        });
        cx.run_until_parked();
        assert_eq!(seen_calls(&backend), 0, "the new answer waits for them");
        // Back at the keyboard: read at once, and the poll says so without waiting its turn.
        store.update(cx, Store::note_input);
        cx.run_until_parked();
        assert_eq!(seen_calls(&backend), 1);
        // The clock hasn't moved: these polls are the coming back's and the mark's.
        let back = &looking(&backend)[away.len()..];
        assert!(!back.is_empty(), "polled at once");
        assert!(back.iter().all(|looked| *looked), "{back:?}");
    }

    #[gpui_kit::test]
    fn a_mark_that_didnt_reach_the_server_is_sent_with_the_next_poll(cx: &mut TestAppContext) {
        let backend = Arc::new(FakeBackend::new([]));
        backend.push_seen(Err(Error::Unreachable("connection reset".into())));
        let store = store(cx, &backend);
        store.update(cx, |store, cx| {
            store.set_window_active(true, cx);
            store.apply(RoomId::MAIN, Ok(unread_up_to(9)), Instant::now(), cx);
        });
        cx.run_until_parked();
        assert_eq!(seen_calls(&backend), 1, "the first try failed");
        store.update(cx, |store, cx| {
            store.apply(RoomId::MAIN, Ok(StateReply::Unchanged), Instant::now(), cx);
        });
        cx.run_until_parked();
        assert_eq!(seen_calls(&backend), 2, "tried again, and it went through");
        store.update(cx, |store, cx| {
            store.apply(RoomId::MAIN, Ok(StateReply::Unchanged), Instant::now(), cx);
        });
        cx.run_until_parked();
        assert_eq!(seen_calls(&backend), 2, "told now");
    }

    #[gpui_kit::test]
    fn a_refused_mark_isnt_sent_with_every_poll(cx: &mut TestAppContext) {
        let backend = Arc::new(FakeBackend::new([]));
        backend.push_seen(Err(Error::Refused {
            status: Some(404),
            message: None,
        }));
        let store = store(cx, &backend);
        store.update(cx, |store, cx| {
            store.set_window_active(true, cx);
            store.apply(RoomId::MAIN, Ok(unread_up_to(9)), Instant::now(), cx);
        });
        cx.run_until_parked();
        store.update(cx, |store, cx| {
            store.apply(RoomId::MAIN, Ok(StateReply::Unchanged), Instant::now(), cx);
            store.apply(RoomId::MAIN, Ok(unread_up_to(9)), Instant::now(), cx);
        });
        cx.run_until_parked();
        assert_eq!(seen_calls(&backend), 1, "the server said no to these");
        store.update(cx, |store, cx| {
            store.apply(RoomId::MAIN, Ok(unread_up_to(10)), Instant::now(), cx);
        });
        cx.run_until_parked();
        assert_eq!(seen_calls(&backend), 2, "a new answer is tried");
    }

    #[gpui_kit::test]
    fn a_room_read_is_marked_seen_once_for_each_new_answer(cx: &mut TestAppContext) {
        let backend = Arc::new(FakeBackend::new([]));
        let store = store(cx, &backend);
        store.update(cx, |store, cx| {
            store.set_window_active(true, cx);
            store.apply(RoomId::MAIN, Ok(unread_up_to(9)), Instant::now(), cx);
            // Polls that left before the mark arrived still say unread.
            store.apply(RoomId::MAIN, Ok(unread_up_to(9)), Instant::now(), cx);
            store.apply(RoomId::MAIN, Ok(StateReply::Unchanged), Instant::now(), cx);
        });
        cx.run_until_parked();
        assert_eq!(seen_calls(&backend), 1, "one mark for answer 9");
        store.update(cx, |store, cx| {
            store.apply(RoomId::MAIN, Ok(unread_up_to(10)), Instant::now(), cx);
        });
        cx.run_until_parked();
        assert_eq!(seen_calls(&backend), 2, "answer 10 came in while reading");
    }

    #[gpui_kit::test]
    fn an_unread_room_is_marked_seen_when_its_window_comes_to_the_front(cx: &mut TestAppContext) {
        let backend = Arc::new(FakeBackend::new([]));
        let store = store(cx, &backend);
        store.update(cx, |store, cx| {
            store.apply(RoomId::MAIN, Ok(unread_up_to(9)), Instant::now(), cx);
        });
        cx.run_until_parked();
        assert_eq!(seen_calls(&backend), 0, "the window is in the background");
        store.update(cx, |store, cx| store.set_window_active(true, cx));
        cx.run_until_parked();
        assert_eq!(seen_calls(&backend), 1);
    }

    #[gpui_kit::test]
    fn an_unread_room_beside_a_page_in_the_column_is_marked_seen(cx: &mut TestAppContext) {
        let backend = Arc::new(FakeBackend::new([]));
        let store = store(cx, &backend);
        store.update(cx, |store, cx| {
            store.show_in_test_column(Some(Page::Tasks), cx);
            store.set_window_active(true, cx);
            store.apply(RoomId::MAIN, Ok(unread_up_to(9)), Instant::now(), cx);
        });
        cx.run_until_parked();
        assert_eq!(seen_calls(&backend), 1, "the chat is beside Tasks");
        store.update(cx, |store, cx| store.show_in_test_column(None, cx));
        cx.run_until_parked();
        assert_eq!(seen_calls(&backend), 1, "marked once");
    }

    #[gpui_kit::test]
    fn a_mark_seen_polls_at_once_for_the_cleared_unread(cx: &mut TestAppContext) {
        let backend = Arc::new(FakeBackend::new([Ok(unread_up_to(9))]));
        let store = store(cx, &backend);
        // In front from the start, so only the mark can wake the poll.
        store.update(cx, |store, cx| {
            store.window.active = true;
            store.last_input.input(cx.background_executor().now());
            store.start_poll(cx);
        });
        // The clock doesn't move: the second poll is the mark's.
        cx.run_until_parked();
        assert_eq!(
            backend.calls(),
            [
                Call::State(RoomId::MAIN, None, true),
                Call::Seen(RoomId::MAIN),
                Call::State(RoomId::MAIN, Some("\"v1\"".into()), true),
            ]
        );
    }
}

mod presence {
    use super::*;
    use crate::store::poll::{BEHIND_EVERY, POLL_EVERY};
    use crate::store::presence::Presence;

    /// Long enough for dozens of polls in front.
    const A_WHILE: Duration = Duration::from_mins(10);

    fn polls(backend: &FakeBackend) -> usize {
        backend
            .calls()
            .iter()
            .filter(|call| matches!(call, Call::State(..)))
            .count()
    }

    fn machines_fetched(backend: &FakeBackend) -> usize {
        backend
            .calls()
            .iter()
            .filter(|call| **call == Call::Machines)
            .count()
    }

    /// A store in front, its poll started.
    fn polling_in_front(cx: &mut TestAppContext, backend: &Arc<FakeBackend>) -> Entity<Store> {
        let store = store(cx, backend);
        store.update(cx, |store, cx| {
            store.window.active = true;
            store.start_poll(cx);
        });
        cx.run_until_parked();
        store
    }

    #[gpui_kit::test]
    fn a_hidden_window_makes_no_polls_and_polls_at_once_when_shown(cx: &mut TestAppContext) {
        let backend = Arc::new(FakeBackend::new([]));
        let store = polling_in_front(cx, &backend);
        assert_eq!(polls(&backend), 1, "the first poll");

        store.update(cx, |store, _| store.set_window_visible(false));
        assert_eq!(
            store.read_with(cx, |store, _| store.presence()),
            Presence::Hidden
        );
        // The poll's next turn was due in a few seconds; it doesn't come while hidden.
        cx.executor().advance_clock(A_WHILE);
        cx.run_until_parked();
        assert_eq!(polls(&backend), 1, "minimised for ten minutes: no poll");

        // The clock doesn't move: only the window coming back can poll.
        store.update(cx, |store, _| store.set_window_visible(true));
        cx.run_until_parked();
        assert_eq!(polls(&backend), 2, "shown again: a poll at once");
        cx.executor().advance_clock(POLL_EVERY);
        cx.run_until_parked();
        assert_eq!(polls(&backend), 3, "and every few seconds again");
    }

    #[gpui_kit::test]
    fn a_failing_poll_isnt_retried_while_hidden(cx: &mut TestAppContext) {
        let backend = Arc::new(FakeBackend::new([unreachable()]));
        let store = polling_in_front(cx, &backend);
        store.read_with(cx, |store, _| assert!(store.problem().is_some()));
        store.update(cx, |store, _| store.set_window_visible(false));
        cx.executor().advance_clock(A_WHILE);
        cx.run_until_parked();
        assert_eq!(polls(&backend), 1, "no quiet retries out of sight");
    }

    #[gpui_kit::test]
    fn a_window_behind_another_app_polls_slowly_and_at_once_in_front(cx: &mut TestAppContext) {
        let backend = Arc::new(FakeBackend::new([]));
        let store = store(cx, &backend);
        store.update(cx, Store::start_poll);
        cx.run_until_parked();
        assert_eq!(
            store.read_with(cx, |store, _| store.presence()),
            Presence::Behind
        );
        assert_eq!(polls(&backend), 1, "the first poll");
        cx.executor()
            .advance_clock(BEHIND_EVERY.saturating_sub(Duration::from_millis(1)));
        cx.run_until_parked();
        assert_eq!(polls(&backend), 1, "behind: not every few seconds");
        cx.executor().advance_clock(Duration::from_millis(1));
        cx.run_until_parked();
        assert_eq!(polls(&backend), 2, "a slow heartbeat");

        store.update(cx, |store, cx| store.set_window_active(true, cx));
        cx.run_until_parked();
        assert_eq!(polls(&backend), 3, "brought to the front: a poll at once");
    }

    #[gpui_kit::test]
    fn the_machines_page_refreshes_only_in_front_and_at_once_back_there(cx: &mut TestAppContext) {
        let backend = Arc::new(FakeBackend::new([]));
        let store = store(cx, &backend);
        store.update(cx, |store, cx| {
            // open at launch
            store
                .columns
                .set(gpui_kit::WindowId::from(1), vec![Page::Machines]);
            store.window.active = true;
            store.start_machines(cx);
        });
        cx.run_until_parked();
        assert_eq!(machines_fetched(&backend), 1, "at launch");
        cx.executor().advance_clock(Duration::from_secs(10));
        cx.run_until_parked();
        assert_eq!(
            machines_fetched(&backend),
            2,
            "every ten seconds on their page"
        );

        store.update(cx, |store, _| store.set_window_visible(false));
        cx.executor().advance_clock(A_WHILE);
        cx.run_until_parked();
        assert_eq!(machines_fetched(&backend), 2, "not while minimised");

        // Shown, but the user is in another app: still no one watches them.
        store.update(cx, |store, cx| {
            store.set_window_active(false, cx);
            store.set_window_visible(true);
        });
        cx.executor().advance_clock(A_WHILE);
        cx.run_until_parked();
        assert_eq!(
            machines_fetched(&backend),
            2,
            "not behind another app's window"
        );

        // The clock doesn't move: only coming to the front can fetch them.
        store.update(cx, |store, cx| store.set_window_active(true, cx));
        cx.run_until_parked();
        assert_eq!(machines_fetched(&backend), 3, "in front again: at once");
    }

    #[gpui_kit::test]
    fn the_machines_are_fetched_at_once_when_their_page_opens(cx: &mut TestAppContext) {
        let backend = Arc::new(FakeBackend::new([]));
        let store = store(cx, &backend);
        store.update(cx, |store, cx| {
            store.window.active = true;
            store.start_machines(cx);
        });
        cx.run_until_parked();
        assert_eq!(machines_fetched(&backend), 1, "at launch");
        cx.executor().advance_clock(A_WHILE);
        cx.run_until_parked();
        assert_eq!(machines_fetched(&backend), 1, "not off their page");

        // The clock doesn't move: only opening their page can fetch them.
        store.update(cx, |store, cx| {
            store.show_in_test_column(Some(Page::Machines), cx);
        });
        cx.run_until_parked();
        assert_eq!(machines_fetched(&backend), 2, "their page opened: at once");
        cx.executor().advance_clock(Duration::from_secs(10));
        cx.run_until_parked();
        assert_eq!(machines_fetched(&backend), 3, "then every ten seconds");

        store.update(cx, |store, cx| {
            store.show_in_test_column(Some(Page::Tasks), cx);
        });
        cx.executor().advance_clock(A_WHILE);
        cx.run_until_parked();
        assert_eq!(machines_fetched(&backend), 3, "another page again");
    }
}

#[gpui_kit::test]
fn later_starts_folded_and_stays_as_left_for_the_session(cx: &mut TestAppContext) {
    let backend = Arc::new(FakeBackend::new([]));
    let store = store(cx, &backend);
    let redraws = Rc::new(Cell::new(0_u32));
    let _observing = cx.update(|cx| {
        let redraws = Rc::clone(&redraws);
        cx.observe(&store, move |_, _| redraws.set(redraws.get() + 1))
    });
    // Each toggle flips the fold and redraws the views once, by itself.
    let toggle = |cx: &mut TestAppContext| {
        let before = redraws.get();
        store.update(cx, Store::toggle_later);
        cx.run_until_parked();
        assert_eq!(redraws.get(), before + 1, "a toggle redraws the views");
        store.read_with(cx, |store, _| store.is_later_open())
    };
    store.update(cx, |store, cx| {
        store.show_in_test_column(Some(Page::Needs), cx);
    });
    cx.run_until_parked();
    store.read_with(cx, |store, _| assert!(!store.is_later_open()));
    assert!(toggle(cx), "a toggle opens it");
    store.update(cx, |store, cx| {
        store.show_in_test_column(Some(Page::Tasks), cx);
        store.show_in_test_column(Some(Page::Needs), cx);
    });
    cx.run_until_parked();
    store.read_with(cx, |store, _| {
        assert!(store.is_later_open(), "leaving the page keeps it open");
    });
    assert!(!toggle(cx), "a toggle folds it again");
}

mod theme {
    use super::*;
    use emberhut_api::theme::{Appearance, ThemePage};

    fn page(preset: &str) -> ThemePage {
        let mut page = crate::theme::built_in();
        page.choice.preset = preset.into();
        page
    }

    #[gpui_kit::test]
    fn of_two_quick_picks_the_second_stays(cx: &mut TestAppContext) {
        let backend = Arc::new(FakeBackend::new([]));
        let store = store(cx, &backend);
        store.update(cx, |store, cx| {
            store.pick_theme(None, Some(Appearance::Dark), cx);
            store.pick_theme(None, Some(Appearance::Light), cx);
        });
        cx.run_until_parked();
        store.read_with(cx, |store, _| {
            assert_eq!(store.theme().page().choice.appearance, Appearance::Light);
        });
        assert_eq!(backend.calls().len(), 2, "both saves went out");
    }

    #[gpui_kit::test]
    fn a_reply_to_an_older_save_is_ignored_whenever_it_comes(cx: &mut TestAppContext) {
        let backend = Arc::new(FakeBackend::new([]));
        let store = store(cx, &backend);
        store.update(cx, |store, cx| {
            store.theme.sent = 2;
            store.take_saved(2, true, Ok(page("blue")), cx);
            store.take_saved(1, true, Ok(page("green")), cx);
            assert_eq!(
                store.theme().page().choice.preset,
                "blue",
                "save 2 is the newest"
            );
            assert_eq!(store.theme.answered, 2);
        });
    }

    #[gpui_kit::test]
    fn a_pending_pick_isnt_sent_again_while_a_save_is_out(cx: &mut TestAppContext) {
        let backend = Arc::new(FakeBackend::new([]));
        let store = store(cx, &backend);
        store.update(cx, |store, cx| {
            store.pick_theme(Some("blue".into()), None, cx);
            assert_eq!(store.theme.sent, 1);
            let brief = store.theme().page().choice.clone();
            store.theme_seen(&brief, cx);
            assert_eq!(store.theme.sent, 1, "the first save is still out");
            store.take_saved(1, true, Err(Error::Unreachable("down".into())), cx);
            store.theme_seen(&brief, cx);
            assert_eq!(store.theme.sent, 2, "sent again once the first came back");
        });
    }

    #[gpui_kit::test]
    fn a_preview_that_comes_after_a_save_is_dropped(cx: &mut TestAppContext) {
        let backend = Arc::new(FakeBackend::new([]));
        let store = store(cx, &backend);
        store.update(cx, |store, cx| {
            store.custom_accent(Some("#336699".into()), true, cx);
            let asked = store.theme.previews;
            store.custom_accent(Some("#336699".into()), false, cx);
            store.take_preview(asked, Ok(page("blue")), cx);
            assert_eq!(
                store.theme().preview(),
                None,
                "the saved theme shows, not the preview"
            );
        });
    }
}

mod pictures {
    use gpui_kit::{Empty, ImageSource, RenderImage, VisualTestContext, img, point, px, size};

    use super::*;
    use crate::store::fetched::RETRY_AFTER;
    use crate::store::retention::Limits;

    /// A 1 × 1 PNG.
    const PNG: &[u8] = &[
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F,
        0x15, 0xC4, 0x89, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0xF8,
        0xCF, 0xC0, 0xF0, 0x1F, 0x00, 0x05, 0x00, 0x01, 0xFF, 0x89, 0x99, 0x3D, 0x1D, 0x00, 0x00,
        0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
    ];

    fn path(room: i64) -> String {
        format!("/uploads/room{room}@2x.png")
    }

    /// Opens `room`, whose chat shows its one picture, and lets the picture come.
    fn visit(cx: &mut TestAppContext, store: &Entity<Store>, room: i64) {
        let picture = json!({"name": "shot.png", "url": path(room), "kind": "image"});
        let chat = [message_at(
            room,
            "2026-01-01T00:00:00Z",
            json!({"files": [picture]}),
        )];
        store.update(cx, |store, cx| {
            store.switch_room(RoomId::new(room), cx);
            store.apply(
                RoomId::new(room),
                Ok(fresh_state(json!({"room": room, "chat": chat}))),
                Instant::now(),
                cx,
            );
        });
        cx.run_until_parked();
    }

    fn picture(cx: &TestAppContext, store: &Entity<Store>, room: i64) -> Option<Arc<RenderImage>> {
        store.read_with(cx, |store, _| store.picture(&path(room)))
    }

    /// A window that has drawn `picture`, as the chat does.
    fn drawn(cx: &mut TestAppContext, picture: &Arc<RenderImage>) -> VisualTestContext {
        let window = cx.add_window(|_, _| Empty);
        let mut window = VisualTestContext::from_window(window.into(), cx);
        let source = ImageSource::Render(Arc::clone(picture));
        window.draw(point(px(0.), px(0.)), size(px(10.), px(10.)), |_, _| {
            img(source)
        });
        window
    }

    /// Whether `window`'s sprite atlas holds `picture`.
    fn in_atlas(window: &mut VisualTestContext, picture: &RenderImage) -> bool {
        window.update(|window, _| window.has_image_atlas_entry(picture))
    }

    /// The files the store fetched.
    fn fetched(backend: &FakeBackend) -> Vec<String> {
        let calls = backend.calls().into_iter();
        calls
            .filter_map(|call| match call {
                Call::File(path) => Some(path),
                _ => None,
            })
            .collect()
    }

    #[gpui_kit::test]
    fn a_file_that_isnt_a_picture_is_fetched_once(cx: &mut TestAppContext) {
        let backend = Arc::new(FakeBackend::new([]).with_file(&path(1), b"not a picture"));
        let store = store(cx, &backend);
        visit(cx, &store, 1);
        visit(cx, &store, 1);
        assert!(picture(cx, &store, 1).is_none(), "not a picture");
        assert_eq!(fetched(&backend), [path(1)]);
        let later = Instant::now() + RETRY_AFTER * 2;
        let again = store.update(cx, |store, _| {
            store.images.begin(RoomId::MAIN, &path(1), later)
        });
        assert!(!again, "never fetched again: it would fail the same way");
    }

    #[gpui_kit::test]
    fn a_picture_that_didnt_come_is_fetched_again_later(cx: &mut TestAppContext) {
        let backend = Arc::new(FakeBackend::new([]));
        let store = store(cx, &backend);
        visit(cx, &store, 1);
        assert_eq!(fetched(&backend), [path(1)]);
        let later = Instant::now() + RETRY_AFTER;
        let again = store.update(cx, |store, _| {
            store.images.begin(RoomId::MAIN, &path(1), later)
        });
        assert!(again, "a fetch that failed may pass next time");
    }

    #[gpui_kit::test]
    fn other_rooms_pictures_are_freed_past_the_last_few(cx: &mut TestAppContext) {
        let rooms = 1..=i64::try_from(Limits::PICTURES.rooms).unwrap() + 2;
        let backend = rooms.clone().fold(FakeBackend::new([]), |backend, room| {
            backend.with_file(&path(room), PNG)
        });
        let backend = Arc::new(backend);
        let store = store(cx, &backend);

        visit(cx, &store, 1);
        let main = picture(cx, &store, 1).expect("Main's picture came");
        let bytes = store.read_with(cx, |store, _| store.images.bytes());
        assert_eq!(bytes, 4, "1 × 1 BGRA");
        let mut window = drawn(cx, &main);
        assert!(in_atlas(&mut window, &main), "drawn");
        for room in rooms.clone().skip(1) {
            let kept = picture(cx, &store, 1).is_some();
            assert!(kept, "Main's is still kept before room {room}");
            visit(cx, &store, room);
            let shown = picture(cx, &store, room).is_some();
            assert!(shown, "the open room's, on screen");
        }

        assert!(
            picture(cx, &store, 1).is_none(),
            "Main is past the last rooms"
        );
        assert!(!in_atlas(&mut window, &main), "its tiles are freed");
        let freed = Arc::downgrade(&main);
        drop(main);
        assert!(
            freed.upgrade().is_none(),
            "its pixels are freed: nothing else holds them"
        );
        let held = store.read_with(cx, |store, _| (store.images.len(), store.images.bytes()));
        let rooms_held = Limits::PICTURES.rooms + 1;
        assert_eq!(held, (rooms_held, 4 * rooms_held));
    }
}
