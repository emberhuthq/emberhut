//! The composer in a headless window: its keys, its drafts, words put in from elsewhere (the
//! empty chat's starters among them), a failed message sent again, dictation, and the
//! confirmation before recording a call.

use emberhut_api::calls::RecordCallRequest;
use emberhut_api::ids::RoomId;
use emberhut_api::talk::TalkRequest;
use gpui_kit::component::input::Enter;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, ElementId, TestAppContext, WindowHandle};
use serde_json::json;

use super::ChatView;
use super::reading::STARTERS;
use super::tests::open_chat;
use crate::backend::Error;
use crate::backend::fake::Call;
use crate::dictation::Dictation;
use crate::dictation::mic::fake::FakeMicrophone;
use crate::fixtures::state;

/// Presses Return in the focused field, with or without Shift or Cmd.
fn press_return(cx: &mut TestAppContext, window: WindowHandle<ChatView>, enter: Enter) {
    window
        .update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| input.focus(window, cx));
        })
        .unwrap();
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.dispatch_action(Box::new(enter), cx);
    })
    .unwrap();
    cx.run_until_parked();
}

fn typed(cx: &mut TestAppContext, window: WindowHandle<ChatView>, text: &str) {
    window
        .update(cx, |view, window, cx| {
            view.input
                .update(cx, |input, cx| input.set_value(text.to_owned(), window, cx));
        })
        .unwrap();
}

fn field(cx: &mut TestAppContext, window: WindowHandle<ChatView>) -> String {
    window
        .update(cx, |view, _, cx| view.input.read(cx).value().to_string())
        .unwrap()
}

fn click(cx: &mut TestAppContext, window: WindowHandle<ChatView>, id: impl Into<ElementId>) {
    let id = id.into();
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click(id, cx);
    })
    .unwrap();
    cx.run_until_parked();
}

const RETURN: Enter = Enter {
    secondary: false,
    shift: false,
};

#[gpui_kit::test]
fn shift_return_starts_a_line_and_return_sends(cx: &mut TestAppContext) {
    let (backend, _store, window) = open_chat(cx);
    typed(cx, window, "First line");
    window
        .update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.set_cursor_position(
                    gpui_kit::component::input::Position::new(0, 10),
                    window,
                    cx,
                );
            });
        })
        .unwrap();
    press_return(
        cx,
        window,
        Enter {
            secondary: false,
            shift: true,
        },
    );
    assert_eq!(field(cx, window), "First line\n");
    assert_eq!(backend.calls(), [], "a new line, nothing sent");
    press_return(cx, window, RETURN);
    assert_eq!(
        backend.calls(),
        [Call::Talk(TalkRequest::text(RoomId::MAIN, "First line"))]
    );
    assert_eq!(field(cx, window), "");
}

#[gpui_kit::test]
fn cmd_return_sends_too(cx: &mut TestAppContext) {
    let (backend, _store, window) = open_chat(cx);
    typed(cx, window, "Hello");
    press_return(
        cx,
        window,
        Enter {
            secondary: true,
            shift: false,
        },
    );
    assert_eq!(
        backend.calls(),
        [Call::Talk(TalkRequest::text(RoomId::MAIN, "Hello"))]
    );
}

#[gpui_kit::test]
fn each_room_keeps_its_draft_in_the_field(cx: &mut TestAppContext) {
    let (_backend, store, window) = open_chat(cx);
    typed(cx, window, "For Main");
    store.update(cx, |store, cx| store.switch_room(RoomId::new(2), cx));
    cx.run_until_parked();
    assert_eq!(field(cx, window), "");
    typed(cx, window, "For room 2");
    store.update(cx, |store, cx| store.switch_room(RoomId::MAIN, cx));
    cx.run_until_parked();
    assert_eq!(field(cx, window), "For Main");
    store.update(cx, |store, cx| store.switch_room(RoomId::new(2), cx));
    cx.run_until_parked();
    assert_eq!(field(cx, window), "For room 2");
}

#[gpui_kit::test]
fn words_from_elsewhere_land_in_the_field(cx: &mut TestAppContext) {
    let (_backend, store, window) = open_chat(cx);
    typed(cx, window, "Something else");
    store.update(cx, |store, cx| {
        store.start_message("What should I look at today?", cx);
    });
    cx.run_until_parked();
    assert_eq!(field(cx, window), "What should I look at today?");
}

#[gpui_kit::test]
fn a_failed_message_goes_again_with_a_click_on_its_note(cx: &mut TestAppContext) {
    let (backend, store, window) = open_chat(cx);
    backend.push_talk(Err(Error::Unreachable("refused".into())));
    typed(cx, window, "Hello");
    click(cx, window, "send");
    store.read_with(cx, |store, _| {
        assert!(
            store
                .pending()
                .is_some_and(crate::store::PendingMessage::failed)
        );
    });
    click(cx, window, "send-again");
    let hello = Call::Talk(TalkRequest::text(RoomId::MAIN, "Hello"));
    assert_eq!(backend.calls(), [hello.clone(), hello]);
    store.read_with(cx, |store, _| {
        assert_eq!(
            store
                .pending()
                .and_then(crate::store::PendingMessage::note)
                .as_deref(),
            Some("Emberhut is typing…")
        );
    });
}

#[gpui_kit::test]
fn dictated_words_come_after_what_is_typed(cx: &mut TestAppContext) {
    let (backend, _store, window) = open_chat(cx);
    window
        .update(cx, |view, _, _| {
            view.dictation = Dictation::new(Some(Box::new(FakeMicrophone::speaking(2))));
        })
        .unwrap();
    typed(cx, window, "Note:");
    click(cx, window, "dictate");
    window
        .update(cx, |view, _, _| assert!(view.dictation.is_listening()))
        .unwrap();
    click(cx, window, "done-dictation");
    assert_eq!(field(cx, window), "Note: hello there");
    let transcribed = backend.calls().into_iter().find_map(|call| match call {
        Call::Transcribe(name, words, size) => Some((name, words, size)),
        _ => None,
    });
    let (name, words, size) = transcribed.unwrap();
    assert_eq!(name, "dictation.wav");
    assert!(words.iter().any(|word| word == "Emberhut"));
    assert!(size > 44, "a WAV with samples");
}

#[gpui_kit::test]
fn the_x_drops_a_recording(cx: &mut TestAppContext) {
    let (backend, _store, window) = open_chat(cx);
    window
        .update(cx, |view, _, _| {
            view.dictation = Dictation::new(Some(Box::new(FakeMicrophone::speaking(2))));
        })
        .unwrap();
    click(cx, window, "dictate");
    click(cx, window, "cancel-dictation");
    window
        .update(cx, |view, _, _| assert!(!view.dictation.is_listening()))
        .unwrap();
    assert!(
        !backend
            .calls()
            .iter()
            .any(|call| matches!(call, Call::Transcribe(..)))
    );
    assert_eq!(field(cx, window), "");
}

#[gpui_kit::test]
fn a_failed_dictation_says_why_by_the_mic(cx: &mut TestAppContext) {
    let (backend, _store, window) = open_chat(cx);
    backend.push_transcript(Err(Error::Refused {
        status: Some(502),
        message: Some("the speech service is down".into()),
    }));
    window
        .update(cx, |view, _, _| {
            view.dictation = Dictation::new(Some(Box::new(FakeMicrophone::speaking(2))));
        })
        .unwrap();
    click(cx, window, "dictate");
    click(cx, window, "done-dictation");
    window
        .update(cx, |view, _, _| {
            assert_eq!(
                view.dictation.error(),
                Some("Dictation failed: the speech service is down")
            );
        })
        .unwrap();
    click(cx, window, "dictation-error");
    window
        .update(cx, |view, _, _| assert_eq!(view.dictation.error(), None))
        .unwrap();
    assert_eq!(field(cx, window), "");
}

#[gpui_kit::test]
fn option_return_starts_a_line_too(cx: &mut TestAppContext) {
    let (backend, _store, window) = open_chat(cx);
    cx.update(super::bind_keys);
    typed(cx, window, "One");
    window
        .update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.set_cursor_position(
                    gpui_kit::component::input::Position::new(0, 3),
                    window,
                    cx,
                );
            });
        })
        .unwrap();
    cx.update_window(window.into(), |_, window, cx| window.render_frame(cx))
        .unwrap();
    let mut visual = gpui_kit::VisualTestContext::from_window(window.into(), cx);
    visual.simulate_keystrokes("alt-enter");
    visual.run_until_parked();
    assert_eq!(field(cx, window), "One\n");
    assert_eq!(backend.calls(), []);
}

/// How many of the elements called `id` the window draws now.
fn drawn(
    cx: &mut TestAppContext,
    window: WindowHandle<ChatView>,
    id: impl Into<ElementId>,
) -> usize {
    let id = id.into();
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.find_all(id).len()
    })
    .unwrap()
}

#[gpui_kit::test]
fn a_starter_of_the_empty_chat_fills_the_field(cx: &mut TestAppContext) {
    let (backend, store, window) = open_chat(cx);
    store.update(cx, |store, cx| {
        store.take_state_for_tests(state(json!({})), cx);
    });
    cx.run_until_parked();
    click(cx, window, ("starter", 1_usize));
    assert_eq!(field(cx, window), STARTERS[1]);
    assert_eq!(backend.calls(), [], "put in the field, not sent");
}

#[gpui_kit::test]
fn an_empty_chat_with_a_message_on_its_way_shows_only_that_message(cx: &mut TestAppContext) {
    let (_backend, store, window) = open_chat(cx);
    assert_eq!(drawn(cx, window, "empty-chat"), 1, "loading");
    store.update(cx, |store, cx| {
        store.take_state_for_tests(state(json!({})), cx);
    });
    cx.run_until_parked();
    assert_eq!(drawn(cx, window, ("starter", 0_usize)), 1, "the welcome");
    typed(cx, window, "Hello");
    click(cx, window, "send");
    assert!(store.read_with(cx, |store, _| store.pending().is_some()));
    assert_eq!(drawn(cx, window, ("starter", 0_usize)), 0, "no welcome");
    assert_eq!(drawn(cx, window, "empty-chat"), 0, "not loading either");
}

#[gpui_kit::test]
fn recording_a_call_asks_first(cx: &mut TestAppContext) {
    let (backend, _store, window) = open_chat(cx);
    window.update(cx, ChatView::ask_to_record_call).unwrap();
    assert!(cx.has_pending_prompt());
    cx.simulate_prompt_answer("Cancel");
    cx.run_until_parked();
    assert_eq!(backend.calls(), [], "nothing recorded");
    window.update(cx, ChatView::ask_to_record_call).unwrap();
    cx.simulate_prompt_answer("Record");
    cx.run_until_parked();
    assert_eq!(
        backend.calls(),
        [Call::Recording(RecordCallRequest::start(RoomId::MAIN))]
    );
    assert_eq!(drawn(cx, window, "call-change"), 1, "starting, at once");
    window.update(cx, ChatView::ask_to_record_call).unwrap();
    assert!(!cx.has_pending_prompt(), "not while one is on its way");
}

#[gpui_kit::test]
fn a_dictation_left_on_stops_at_the_limit_and_keeps_its_words(cx: &mut TestAppContext) {
    let (backend, _store, window) = open_chat(cx);
    let longest = u32::try_from(crate::dictation::wav::max_seconds()).unwrap();
    window
        .update(cx, |view, _, _| {
            view.dictation = Dictation::new(Some(Box::new(FakeMicrophone::speaking(longest + 1))));
        })
        .unwrap();
    click(cx, window, "dictate");
    cx.executor().advance_clock(crate::dictation::TICK);
    cx.run_until_parked();
    window
        .update(cx, |view, _, _| {
            assert!(!view.dictation.is_listening(), "stopped by itself");
            assert!(
                view.dictation
                    .error()
                    .is_some_and(|note| note.starts_with("Dictation stopped at"))
            );
        })
        .unwrap();
    assert_eq!(field(cx, window), "hello there");
    assert!(
        backend
            .calls()
            .iter()
            .any(|call| matches!(call, Call::Transcribe(..)))
    );
}

#[gpui_kit::test]
fn stop_shows_as_soon_as_a_turn_is_live(cx: &mut TestAppContext) {
    let (_backend, store, window) = open_chat(cx);
    let has_stop = |cx: &mut TestAppContext| {
        cx.update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
            window.try_find("stop").is_some()
        })
        .unwrap()
    };
    assert!(!has_stop(cx), "nothing live: no Stop");
    store.update(cx, |store, cx| {
        let live = json!({"live": {"text": "", "doing": null}});
        store.take_state_for_tests(state(live), cx);
    });
    cx.run_until_parked();
    assert!(has_stop(cx), "a turn started, no step yet: Stop already");
}
