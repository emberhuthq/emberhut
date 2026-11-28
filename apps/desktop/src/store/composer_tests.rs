//! The composer's way to the server, on GPUI's headless test platform with a scripted backend:
//! a failed message sent again, one lost on the way, the stop button, recording a call, and
//! words put in from elsewhere.

use std::sync::Arc;
use std::time::Instant;

use emberhut_api::calls::{RecordAction, RecordCallRequest};
use emberhut_api::headers::ETag;
use emberhut_api::ids::{MessageId, RoomId};
use emberhut_api::talk::{TalkReply, TalkRequest};
use gpui_kit::{AppContext as _, Entity, TestAppContext};
use serde_json::json;

use super::composer::CALL_SETTLES;
use super::talk::{LOST_AFTER, Stage};
use super::{Page, PendingMessage, Store, ThemeState};
use crate::backend::fake::{Call, FakeBackend};
use crate::backend::{Error, FreshState, StateReply};
use crate::fixtures::{message_at, state};

fn store(cx: &mut TestAppContext, backend: &Arc<FakeBackend>) -> Entity<Store> {
    let backend = Arc::clone(backend) as Arc<dyn crate::backend::Backend>;
    cx.new(|_| Store::idle(backend, None, ThemeState::load(None), RoomId::MAIN))
}

fn note(store: &Entity<Store>, cx: &mut TestAppContext) -> Option<String> {
    store.read_with(cx, |store, _| {
        store.pending().and_then(PendingMessage::note)
    })
}

fn stage(store: &Entity<Store>, cx: &mut TestAppContext) -> Option<Stage> {
    store.read_with(cx, |store, _| store.pending().map(|p| p.stage.clone()))
}

/// The store takes in a fresh state of the open room made of `body`.
fn take_state(store: &Entity<Store>, body: serde_json::Value, cx: &mut TestAppContext) {
    store.update(cx, |store, cx| {
        let reply = Ok(StateReply::Fresh(FreshState {
            state: Box::new(state(body)),
            etag: Some(ETag::new("\"v1\"")),
        }));
        store.apply(store.room(), reply, Instant::now(), cx);
    });
}

fn recordings(backend: &FakeBackend) -> Vec<Call> {
    backend
        .calls()
        .into_iter()
        .filter(|call| matches!(call, Call::Recording(_)))
        .collect()
}

fn talks(backend: &FakeBackend) -> Vec<TalkRequest> {
    backend
        .calls()
        .into_iter()
        .filter_map(|call| match call {
            Call::Talk(request) => Some(request),
            _ => None,
        })
        .collect()
}

#[gpui_kit::test]
fn a_failed_message_goes_again_with_a_click(cx: &mut TestAppContext) {
    let backend = Arc::new(FakeBackend::new([]));
    backend.push_talk(Err(Error::Unreachable("refused".into())));
    let store = store(cx, &backend);
    store.update(cx, |store, cx| {
        store.reply_to(&message_at(7, "2026-01-01T00:00:00Z", json!({})), cx);
        assert!(store.talk("Hello", cx));
    });
    cx.run_until_parked();
    assert_eq!(
        note(&store, cx).as_deref(),
        Some("Not sent: can't reach the server")
    );
    store.update(cx, Store::resend);
    assert_eq!(note(&store, cx).as_deref(), Some("Sending…"));
    cx.run_until_parked();
    assert_eq!(note(&store, cx).as_deref(), Some("Emberhut is typing…"));
    let sent = TalkRequest::text(RoomId::MAIN, "Hello").with_reply_to(MessageId::new(7));
    assert_eq!(talks(&backend), [sent.clone(), sent]);
    store.read_with(cx, |store, _| {
        assert_eq!(store.replying(), None, "the reply went with it");
    });
}

#[gpui_kit::test]
fn a_reply_goes_with_the_room(cx: &mut TestAppContext) {
    let backend = Arc::new(FakeBackend::new([]));
    let store = store(cx, &backend);
    store.update(cx, |store, cx| {
        store.reply_to(&message_at(7, "2026-01-01T00:00:00Z", json!({})), cx);
        assert!(store.replying().is_some());
        store.switch_draft(RoomId::MAIN, RoomId::new(2), "");
        assert_eq!(store.replying(), None);
    });
}

#[gpui_kit::test]
fn a_message_on_its_way_isnt_sent_again(cx: &mut TestAppContext) {
    let backend = Arc::new(FakeBackend::new([]));
    let store = store(cx, &backend);
    store.update(cx, |store, cx| assert!(store.talk("Hello", cx)));
    cx.run_until_parked();
    store.update(cx, Store::resend);
    assert_eq!(talks(&backend).len(), 1);
}

#[gpui_kit::test]
fn a_message_never_answered_nor_shown_says_so_after_a_while(cx: &mut TestAppContext) {
    let backend = Arc::new(FakeBackend::new([]));
    let store = store(cx, &backend);
    // Send 1's answer hasn't come when its timer goes off.
    store.update(cx, |store, cx| {
        store.pending = Some(PendingMessage {
            text: "Hello".into(),
            attempt: 1,
            ..PendingMessage::default()
        });
        store.check_lost(1, cx);
    });
    assert_eq!(stage(&store, cx), Some(Stage::Lost));
    assert_eq!(
        note(&store, cx).as_deref(),
        Some("Not sent: Emberhut never got it")
    );
    assert!(talks(&backend).is_empty(), "never sent again by itself");
}

#[gpui_kit::test]
fn a_message_the_server_confirmed_is_never_called_lost(cx: &mut TestAppContext) {
    let backend = Arc::new(FakeBackend::new([]));
    backend.push_talk(Ok(TalkReply {
        ok: true,
        started: true,
        id: None,
    }));
    let store = store(cx, &backend);
    store.update(cx, |store, cx| assert!(store.talk("Hello", cx)));
    cx.run_until_parked();
    assert_eq!(stage(&store, cx), Some(Stage::Typing));
    // No chat shows it (the polls fail, say), and the timer goes off.
    cx.executor().advance_clock(LOST_AFTER);
    cx.run_until_parked();
    assert_eq!(stage(&store, cx), Some(Stage::Typing), "the server has it");
    store.read_with(cx, |store, _| {
        assert!(!store.pending().is_some_and(PendingMessage::failed));
    });
}

#[gpui_kit::test]
fn a_message_the_server_took_without_a_turn_keeps_its_bubble_with_no_note(cx: &mut TestAppContext) {
    let backend = Arc::new(FakeBackend::new([]));
    backend.push_talk(Ok(TalkReply {
        ok: true,
        started: false,
        id: None,
    }));
    let store = store(cx, &backend);
    store.update(cx, |store, cx| assert!(store.talk("Hello", cx)));
    cx.run_until_parked();
    let shown = |store: &Entity<Store>, cx: &mut TestAppContext| {
        store.read_with(cx, |store, _| {
            store
                .pending()
                .map(|pending| (pending.text.clone(), pending.note()))
        })
    };
    assert_eq!(
        shown(&store, cx),
        Some(("Hello".to_owned(), None)),
        "the bubble stays, without a note"
    );
    cx.executor().advance_clock(LOST_AFTER);
    cx.run_until_parked();
    assert_eq!(stage(&store, cx), Some(Stage::Taken), "never called lost");
    assert_eq!(shown(&store, cx), Some(("Hello".to_owned(), None)));
    take_state(
        &store,
        json!({"chat": [{"id": 1, "mine": true, "text": "Hello", "time": "",
            "at": "2026-01-01T00:00:00Z"}]}),
        cx,
    );
    assert_eq!(shown(&store, cx), None, "it goes once the chat has it");
}

#[gpui_kit::test]
fn the_timer_of_an_earlier_send_leaves_a_later_one_alone(cx: &mut TestAppContext) {
    let backend = Arc::new(FakeBackend::new([]));
    let store = store(cx, &backend);
    store.update(cx, |store, cx| assert!(store.talk("First", cx)));
    cx.run_until_parked();
    cx.executor().advance_clock(LOST_AFTER / 2);
    store.update(cx, |store, cx| {
        let mine = message_at(
            9,
            "2026-01-01T00:00:00Z",
            json!({"text": "First", "mine": true}),
        );
        let reply = Ok(StateReply::Fresh(FreshState {
            state: Box::new(state(json!({"chat": [mine]}))),
            etag: Some(ETag::new("\"v1\"")),
        }));
        store.apply(RoomId::MAIN, reply, Instant::now(), cx);
        assert!(store.talk("Second", cx));
    });
    cx.run_until_parked();
    cx.executor().advance_clock(LOST_AFTER / 2);
    cx.run_until_parked();
    assert_eq!(note(&store, cx).as_deref(), Some("Emberhut is typing…"));
}

#[gpui_kit::test]
fn the_stop_button_stops_the_rooms_turn(cx: &mut TestAppContext) {
    let backend = Arc::new(FakeBackend::new([]));
    backend.push_interrupt(Err(Error::Unreachable("refused".into())));
    let store = store(cx, &backend);
    store.update(cx, |store, cx| {
        assert!(store.talk("Hello", cx));
        store.interrupt(cx);
        assert_eq!(store.pending(), None, "its answer won't come");
    });
    cx.run_until_parked();
    assert!(backend.calls().contains(&Call::Interrupt(RoomId::MAIN)));
    store.update(cx, |store, cx| {
        assert_eq!(
            store.composer().notice(),
            Some("Couldn't stop Your ember: can't reach the server")
        );
        store.dismiss_notice(cx);
        assert_eq!(store.composer().notice(), None);
    });
}

#[gpui_kit::test]
fn the_stop_button_keeps_a_failed_message(cx: &mut TestAppContext) {
    let backend = Arc::new(FakeBackend::new([]));
    backend.push_talk(Err(Error::Unreachable("refused".into())));
    let store = store(cx, &backend);
    store.update(cx, |store, cx| assert!(store.talk("Hello", cx)));
    cx.run_until_parked();
    store.update(cx, Store::interrupt);
    cx.run_until_parked();
    assert_eq!(
        note(&store, cx).as_deref(),
        Some("Not sent: can't reach the server"),
        "still there to be sent again"
    );
}

#[gpui_kit::test]
fn recording_a_call_starts_and_stops_in_the_open_room(cx: &mut TestAppContext) {
    let backend = Arc::new(FakeBackend::new([]));
    backend.push_record_call(Ok(()));
    backend.push_record_call(Err(Error::Refused {
        status: None,
        message: Some("nothing is being recorded".into()),
    }));
    let store = store(cx, &backend);
    let room = RoomId::new(2);
    store.update(cx, |store, cx| {
        store.switch_room(room, cx);
        store.record_call(RecordAction::Start, cx);
    });
    cx.run_until_parked();
    store.read_with(cx, |store, _| {
        assert_eq!(store.composer().notice(), None);
        assert_eq!(
            store.call_change(),
            Some(RecordAction::Start),
            "starting, until the state shows it"
        );
    });
    take_state(
        &store,
        json!({"room": 2, "recording": {"id": "c-1", "title": "Call", "room": 2,
            "at": "2026-11-25T10:00:00.000Z"}}),
        cx,
    );
    store.update(cx, |store, cx| {
        assert_eq!(store.call_change(), None, "the state shows it");
        store.record_call(RecordAction::Stop, cx);
    });
    cx.run_until_parked();
    assert_eq!(
        recordings(&backend),
        [
            Call::Recording(RecordCallRequest::start(room)),
            Call::Recording(RecordCallRequest::stop(room))
        ]
    );
    store.read_with(cx, |store, _| {
        assert_eq!(
            store.composer().notice(),
            Some("Couldn't stop recording: nothing is being recorded")
        );
        assert_eq!(store.call_change(), None, "a failure ends the wait");
    });
}

#[gpui_kit::test]
fn a_second_click_while_recording_starts_sends_nothing(cx: &mut TestAppContext) {
    let backend = Arc::new(FakeBackend::new([]));
    let store = store(cx, &backend);
    store.update(cx, |store, cx| {
        store.record_call(RecordAction::Start, cx);
        store.record_call(RecordAction::Start, cx);
        store.record_call(RecordAction::Stop, cx);
    });
    cx.run_until_parked();
    // Answered, but the state doesn't show it yet: still nothing.
    store.update(cx, |store, cx| store.record_call(RecordAction::Start, cx));
    cx.run_until_parked();
    assert_eq!(
        recordings(&backend),
        [Call::Recording(RecordCallRequest::start(RoomId::MAIN))]
    );
    // A state that never shows it: the composer stops waiting after a while.
    cx.executor().advance_clock(CALL_SETTLES);
    cx.run_until_parked();
    store.update(cx, |store, cx| {
        assert_eq!(store.call_change(), None);
        store.record_call(RecordAction::Start, cx);
    });
    cx.run_until_parked();
    assert_eq!(recordings(&backend).len(), 2);
}

#[gpui_kit::test]
fn words_from_elsewhere_go_in_the_composer_beside_the_column(cx: &mut TestAppContext) {
    let backend = Arc::new(FakeBackend::new([]));
    let store = store(cx, &backend);
    store.update(cx, |store, cx| {
        store.show_in_test_column(Some(Page::Tasks), cx);
        store.start_message("Check on the build in an hour", cx);
        assert!(
            store.shows_page(Page::Tasks),
            "the page stays beside the chat"
        );
        assert_eq!(
            store.take_started().as_deref(),
            Some("Check on the build in an hour")
        );
        assert_eq!(store.take_started(), None);
    });
}
