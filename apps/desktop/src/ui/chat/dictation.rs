//! Dictation in the composer, drawn and driven: the mic at the field's end (a spinner while the
//! words are on their way), the recording in the field's place (an X to drop it, a blinking
//! red dot and the time, the time left near the limit, the voice as a live waveform, and a
//! check to turn it into text), and why it failed or stopped, by the mic.
//! [`crate::dictation`] holds the state; the microphone is opened, and a recording finished
//! and encoded, on background threads.

use gpui_kit::TestSupportExt as _;
use gpui_kit::assets::IconName;
use gpui_kit::prelude::*;
use gpui_kit::{Animation, AnimationExt as _, Div, Transformation, Window, div, percentage, px};
use std::time::Duration;

use super::ChatView;
use super::composer::{BUTTON, fill, icon};
use crate::dictation::mic::Recording;
use crate::dictation::{TICK, append, clock};
use crate::ui::a11y::Accessible as _;
use crate::ui::palette::Pal;

/// The waveform's height, in pixels.
const WAVE_HEIGHT: f32 = 24.;

impl ChatView {
    /// The mic, where there is a microphone: it starts recording, or, while the words are
    /// turned into text, spins.
    pub(super) fn mic_button(&self, p: &Pal, cx: &mut Context<'_, Self>) -> Option<Div> {
        if !self.dictation.is_available() {
            return None;
        }
        let preparing = self.dictation.is_preparing();
        let error = self.dictation.error().map(str::to_owned);
        let glyph = if preparing {
            icon(IconName::LoaderCircle, p.muted, 15.)
                .with_animation(
                    "dictation-spin",
                    Animation::new(Duration::from_secs(1)).repeat(),
                    |icon, t| icon.transform(Transformation::rotate(percentage(t))),
                )
                .into_any_element()
        } else {
            icon(IconName::Mic, p.muted, 16.).into_any_element()
        };
        let tip = if preparing {
            "Turning your words into text…"
        } else {
            "Dictate: speak instead of typing (the recording goes to the server's speech-to-text)"
        };
        Some(
            div()
                .relative()
                .flex_none()
                .child(
                    div()
                        .id("dictate")
                        .button_named("Dictate")
                        .accessibility_id("dictate")
                        .cursor_pointer()
                        .size(px(BUTTON))
                        .rounded_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .hover(|d| d.bg(p.hover))
                        .active(|d| d.bg(p.pressed))
                        .tooltip(move |window, cx| {
                            gpui_kit::component::tooltip::Tooltip::new(tip).build(window, cx)
                        })
                        .child(glyph)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.toggle_dictation(window, cx);
                        }))
                        .test_support(),
                )
                .children(error.map(|error| Self::dictation_error(error, p, cx))),
        )
    }

    /// Why dictation failed, in a bubble over the mic; a click hides it.
    fn dictation_error(error: String, p: &Pal, cx: &mut Context<'_, Self>) -> Div {
        div().absolute().bottom(px(38.)).right_0().child(
            div()
                .id("dictation-error")
                .w(px(260.))
                .px(px(12.))
                .py(px(9.))
                .rounded(px(10.))
                .border_1()
                .border_color(p.border)
                .bg(p.surface_raised)
                .shadow_md()
                .text_size(px(12.5))
                .cursor_pointer()
                .child(error)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.dictation.dismiss_error();
                    cx.notify();
                }))
                .test_support(),
        )
    }

    /// The recording in the field's place: cancel, the red dot and time, the waveform, done.
    pub(super) fn recording_row(&self, p: &Pal, cx: &mut Context<'_, Self>) -> Div {
        let elapsed = self.dictation.elapsed();
        let lit = elapsed.as_secs().is_multiple_of(2);
        let bars = self.dictation.levels().iter().map(|&level| {
            div()
                .w(px(3.))
                .h(px((level * WAVE_HEIGHT).max(3.)))
                .rounded_full()
                .bg(p.danger.opacity(0.85))
        });
        div()
            .flex()
            .items_center()
            .gap(px(10.))
            .child(
                div()
                    .id("cancel-dictation")
                    .button_named("Cancel recording")
                    .cursor_pointer()
                    .size(px(BUTTON))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(icon(IconName::X, p.muted, 15.))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.discard_dictation(cx);
                        cx.notify();
                    }))
                    .test_support(),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(
                        div()
                            .size(px(8.))
                            .rounded_full()
                            .bg(p.danger)
                            .opacity(if lit { 1.0 } else { 0.35 }),
                    )
                    .child(
                        div()
                            .text_size(px(13.))
                            .text_color(p.muted)
                            .child(clock(elapsed)),
                    )
                    .children(self.dictation.time_left().map(|left| {
                        div()
                            .id("dictation-time-left")
                            .text_size(px(12.))
                            .text_color(p.danger)
                            .child(format!("{} left", clock(left)))
                            .test_support()
                    })),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h(px(WAVE_HEIGHT))
                    .overflow_hidden()
                    .flex()
                    .items_center()
                    .justify_end()
                    .gap(px(2.))
                    .children(bars),
            )
            .child(
                div()
                    .id("done-dictation")
                    .button_named("Done recording")
                    .cursor_pointer()
                    .size(px(BUTTON))
                    .rounded_full()
                    .bg(p.danger)
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(icon(IconName::Check, p.on_accent, 15.))
                    .on_click(cx.listener(|this, _, window, cx| this.stop_dictation(window, cx)))
                    .test_support(),
            )
    }

    /// The mic: starts recording, opening the microphone off the UI thread, or stops it
    /// (nothing while words are on their way).
    fn toggle_dictation(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.dictation.is_listening() {
            self.stop_dictation(window, cx);
            return;
        }
        let Some(opening) = self.dictation.begin() else {
            return;
        };
        let session = opening.session;
        let microphone = opening.microphone;
        let opened = cx.background_spawn(async move { microphone.start() });
        cx.spawn_in(window, async move |this, cx| {
            let opened = opened.await;
            let updated = this.update_in(cx, |this, window, cx| {
                let unwanted = this.dictation.opened(session, opened);
                let_go(unwanted, cx);
                if this.dictation.is_listening() {
                    Self::follow_levels(session, window, cx);
                }
                cx.notify();
            });
            if let Err(error) = updated {
                log::debug!("the chat closed while the microphone opened: {error}");
            }
        })
        .detach();
        cx.notify();
    }

    /// Takes a loudness reading every [`TICK`] while recording `session`, and redraws; stops
    /// the recording at the longest the server takes. Another recording has its own loop.
    fn follow_levels(session: u64, window: &Window, cx: &mut Context<'_, Self>) {
        cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor().timer(TICK).await;
                let going = this.update_in(cx, |this, window, cx| {
                    if !this.dictation.is_recording(session) {
                        return false;
                    }
                    this.dictation.tick();
                    if this.dictation.is_full() {
                        this.stop_dictation(window, cx);
                    }
                    cx.notify();
                    this.dictation.is_recording(session)
                });
                if !matches!(going, Ok(true)) {
                    break;
                }
            }
        })
        .detach();
    }

    /// Drops the recording, if any, unused (see [`crate::dictation::Dictation::discard`]).
    pub(super) fn discard_dictation(&mut self, cx: &mut Context<'_, Self>) {
        let_go(self.dictation.discard(), cx);
    }

    /// Stops recording; the recording is finished and turned into text off the UI thread, and
    /// the words, once the server has them, come after what is typed.
    pub(super) fn stop_dictation(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let Some(stopped) = self.dictation.stop() else {
            cx.notify();
            return;
        };
        let session = stopped.session;
        let said = self
            .store
            .update(cx, |store, cx| store.transcribe(stopped.recording, cx));
        cx.spawn_in(window, async move |this, cx| {
            let said = said.await;
            let updated = this.update_in(cx, |this, window, cx| {
                if let Some(said) = this.dictation.finish(session, said) {
                    let typed = this.input.read(cx).value().to_string();
                    fill(&this.input, &append(&typed, &said), window, cx);
                }
                cx.notify();
            });
            if let Err(error) = updated {
                log::debug!("the chat closed before the dictation's words came: {error}");
            }
        })
        .detach();
        cx.notify();
    }
}

/// Lets go of a recording no longer wanted on a background thread: stopping the device waits
/// for its audio thread.
fn let_go(recording: Option<Box<dyn Recording>>, cx: &mut Context<'_, ChatView>) {
    if let Some(recording) = recording {
        cx.background_spawn(async move { drop(recording) }).detach();
    }
}
