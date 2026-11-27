//! The composer: a call being recorded (or being started or stopped) and the last failed action
//! over it, the attachment chips, then the + menu, the field (it grows to six lines; Return
//! sends, Shift-Return starts a new line, Cmd-Return sends too) and at its end the mic and the
//! send button, or, while the assistant works and there is nothing to send, the stop button.
//! While dictating, the row is the recording instead. Option-Return starts a new line too, as in
//! the Mac's own text fields. Recording a call asks first: it records both sides of the call.

use emberhut_api::calls::RecordAction;
use gpui_kit::TestSupportExt as _;
use gpui_kit::assets::IconName;
use gpui_kit::component::Icon;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonCustomVariant, ButtonVariants as _};
use gpui_kit::component::input::{Enter, Position, Textarea, TextareaState};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::prelude::*;
use gpui_kit::{
    Anchor, Animation, AnimationExt as _, AnyElement, App, ClipboardItem, Div, Entity, Hsla,
    KeyBinding, PathPromptOptions, PromptLevel, Window, div, px, transparent_black,
};
use std::time::Duration;

use super::ChatView;
use super::attachments;
use super::paste::Pasted;
use crate::brand::APP_NAME;
use crate::store::Store;
use crate::store::composer::placeholder;
use crate::ui::a11y::Accessible as _;
use crate::ui::palette::Pal;

/// The composer's key context, for its own keys.
const CONTEXT: &str = "Composer";

/// The round buttons at the field's ends (the +, the mic, Send, Stop): their width and height.
pub(super) const BUTTON: f32 = 28.;

/// The composer's corner radius.
const RADIUS: f32 = 20.;

/// The space between the composer's edge and what it holds. A round button in a corner is then
/// concentric with the corner: `RADIUS == INSET + BUTTON / 2`.
const INSET: f32 = 6.;

/// The space above and below the field's text, besides the small field's own 2 px. A line of
/// the field's 14 px text is 21 px tall, so one line comes to [`BUTTON`]'s height: the buttons,
/// held at the row's foot, are centred on the line, and on the last line as the field grows,
/// where the send button of a chat app sits.
const FIELD_PAD_Y: f32 = 1.5;

/// The composer's keys: Option-Return starts a new line in its field, as Shift-Return does.
pub(crate) fn bind_keys(cx: &mut App) {
    cx.bind_keys([KeyBinding::new(
        "alt-enter",
        Enter {
            secondary: false,
            shift: true,
        },
        Some("Composer > Input"),
    )]);
}

/// What the confirmation before recording a call asks, says and offers.
const RECORD_QUESTION: &str = "Record the call on the Mac?";
const RECORD_DETAIL: &str = "This records the Mac's microphone and everything it plays, so \
     both sides of the call. The transcript comes to this room.";
const RECORD_ANSWERS: [&str; 2] = ["Record", "Cancel"];

impl ChatView {
    /// Keeps the field with the open room: its draft and its prompt, and words put in from
    /// elsewhere.
    pub(super) fn follow_room(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let room = self.store.read(cx).room();
        if room != self.composing_in {
            let typed = self.input.read(cx).value().to_string();
            let from = self.composing_in;
            let draft = self
                .store
                .update(cx, |store, _| store.switch_draft(from, room, &typed));
            self.composing_in = room;
            self.jump = None;
            self.discard_dictation(cx);
            fill(&self.input, &draft, window, cx);
        }
        let prompt = placeholder(self.store.read(cx).state(), room);
        if prompt != self.prompt {
            self.input.update(cx, |input, cx| {
                input.set_placeholder(prompt.clone(), window, cx);
            });
            self.prompt = prompt;
        }
        if let Some(started) = self.store.update(cx, |store, _| store.take_started()) {
            fill(&self.input, &started, window, cx);
        }
    }

    /// The whole composer.
    pub(super) fn composer(&self, p: &Pal, cx: &mut Context<'_, Self>) -> Div {
        let row = if self.dictation.is_listening() {
            self.recording_row(p, cx)
        } else {
            self.field_row(p, cx)
        };
        let store = self.store.read(cx);
        let recording = store
            .state()
            .and_then(|state| state.recording.as_ref())
            .map(|recording| recording.title.clone());
        let changing = store.call_change();
        let notice = store.composer().notice().map(str::to_owned);
        div()
            .key_context(CONTEXT)
            .flex()
            .flex_col()
            .gap(px(8.))
            .p(px(INSET))
            .rounded(px(RADIUS))
            .border_1()
            .border_color(p.border)
            .bg(p.surface)
            .children(changing.map(|action| call_change_line(action, p)))
            .children(
                recording
                    .filter(|_| changing.is_none())
                    .map(|title| self.recording_line(&title, p)),
            )
            .children(notice.map(|notice| self.notice_line(notice, p)))
            .children(
                attachments::chips(&self.store, p, cx).map(|chips| div().pt(px(4.)).child(chips)),
            )
            .child(row)
    }

    /// The + menu, the field, and the buttons after it.
    fn field_row(&self, p: &Pal, cx: &mut Context<'_, Self>) -> Div {
        let store = self.store.clone();
        let field = Textarea::new(&self.input)
            .appearance(false)
            .small()
            .on_paste(move |item, _, cx| attach_pasted(&store, item, cx));
        div()
            .flex()
            .items_end()
            .gap(px(INSET))
            .child(self.attach_menu(p, cx))
            .child(div().flex_1().min_w_0().py(px(FIELD_PAD_Y)).child(field))
            .children(self.mic_button(p, cx))
            .children(self.send_or_stop(p, cx))
    }

    /// Send once there is something to send (dimmed while an upload runs); else, while the
    /// assistant works, Stop; else nothing.
    fn send_or_stop(&self, p: &Pal, cx: &mut Context<'_, Self>) -> Option<AnyElement> {
        let text = self.input.read(cx).value();
        let store = self.store.read(cx);
        let attachments = store.attachments();
        if !text.trim().is_empty() || !attachments.items().is_empty() {
            let can_send = attachments.can_send(&text);
            return Some(
                round_button("send", p.accent)
                    .button_named("Send")
                    .accessibility_id("send")
                    .when(!can_send, |d| d.opacity(0.4).cursor_default())
                    .child(icon(IconName::ArrowUp, p.on_accent, 15.))
                    .on_click(cx.listener(|this, _, window, cx| this.send(window, cx)))
                    // Seen by the headless tests (a no-op outside them).
                    .test_support()
                    .into_any_element(),
            );
        }
        if !is_working(store) {
            return None;
        }
        Some(
            round_button("stop", p.text)
                .button_named(format!("Stop {APP_NAME}"))
                .accessibility_id("stop")
                .tooltip(|window, cx| Tooltip::new(format!("Stop {APP_NAME}")).build(window, cx))
                .child(div().size(px(9.)).rounded(px(2.)).bg(p.surface))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.store.update(cx, Store::interrupt);
                }))
                .test_support()
                .into_any_element(),
        )
    }

    /// The + menu: photos or videos, files, what the clipboard holds, and recording a call on
    /// the Mac (after a confirmation), or stopping it; that item is off while a start or stop
    /// is on its way. The + is a round button the size of Send, outlined like the composer:
    /// tinted under the pointer, deeper while held down or while its menu is open.
    fn attach_menu(&self, p: &Pal, cx: &Context<'_, Self>) -> impl IntoElement {
        let store = self.store.read(cx);
        let recording = store.state().is_some_and(|state| state.recording.is_some());
        let changing = store.call_change().is_some();
        let store = self.store.clone();
        let view = cx.entity();
        let look = ButtonCustomVariant::new(cx)
            .color(transparent_black())
            .foreground(p.muted)
            .hover(p.hover)
            .active(p.pressed)
            .shadow(false);
        Button::new("attach")
            .custom(look)
            .size(px(BUTTON))
            .rounded(px(BUTTON / 2.))
            .border_1()
            .border_color(p.border)
            .icon(Icon::new(IconName::Plus))
            .tooltip("Attach pictures, videos or files")
            .accessibility_label("Attach")
            .accessibility_id("attach")
            // Above the +: the composer sits at the window's foot.
            .dropdown_menu_with_anchor(Anchor::BottomLeft, move |menu, _, _| {
                let (photos, files, paste, call) =
                    (store.clone(), store.clone(), store.clone(), store.clone());
                let view = view.clone();
                let record = if recording {
                    PopupMenuItem::new("Stop recording the call")
                        .icon(Icon::new(IconName::CircleStop))
                        .on_click(move |_, _, cx| {
                            call.update(cx, |store, cx| store.record_call(RecordAction::Stop, cx));
                        })
                } else {
                    PopupMenuItem::new("Record a call on the Mac (both sides)…")
                        .icon(Icon::new(IconName::Disc))
                        .on_click(move |_, window, cx| {
                            view.update(cx, |view, cx| view.ask_to_record_call(window, cx));
                        })
                };
                menu.item(
                    PopupMenuItem::new("Photos or videos…")
                        .icon(Icon::new(IconName::Images))
                        .on_click(move |_, _, cx| {
                            pick_files(&photos, "Attach photos or videos", cx);
                        }),
                )
                .item(
                    PopupMenuItem::new("Files…")
                        .icon(Icon::new(IconName::File))
                        .on_click(move |_, _, cx| pick_files(&files, "Attach", cx)),
                )
                .item(
                    PopupMenuItem::new("Paste image")
                        .icon(Icon::new(IconName::Clipboard))
                        .on_click(move |_, _, cx| {
                            if let Some(item) = cx.read_from_clipboard() {
                                attach_pasted(&paste, &item, cx);
                            }
                        }),
                )
                .separator()
                .item(record.disabled(changing))
            })
    }

    /// Asks before recording a call on the Mac, which records the other people on it too; a
    /// yes starts it in the open room.
    pub(super) fn ask_to_record_call(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.store.read(cx).call_change().is_some() {
            return;
        }
        let answer = window.prompt(
            PromptLevel::Warning,
            RECORD_QUESTION,
            Some(RECORD_DETAIL),
            &RECORD_ANSWERS,
            cx,
        );
        let store = self.store.clone();
        cx.spawn(async move |_, cx| {
            if matches!(answer.await, Ok(0)) {
                store.update(cx, |store, cx| store.record_call(RecordAction::Start, cx));
            }
        })
        .detach();
    }

    /// A call being recorded on the Mac: a pulsing red mark, its title, and Stop.
    fn recording_line(&self, title: &str, p: &Pal) -> Div {
        let store = self.store.clone();
        let mark = icon(IconName::Disc, p.danger, 14.).with_animation(
            "recording-pulse",
            Animation::new(Duration::from_millis(1400)).repeat(),
            |icon, t| icon.opacity(pulse(t)),
        );
        div()
            .flex()
            .items_center()
            .gap(px(8.))
            .px(px(8.))
            .pt(px(4.))
            .child(mark)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(px(13.))
                    .child(format!("Recording on the Mac: {title}")),
            )
            .child(
                Button::new("stop-recording")
                    .small()
                    .danger()
                    .label("Stop")
                    .tooltip("Stop recording the call")
                    .on_click(move |_, _, cx| {
                        store.update(cx, |store, cx| store.record_call(RecordAction::Stop, cx));
                    }),
            )
    }

    /// The last failed action, with an X to hide it.
    fn notice_line(&self, notice: String, p: &Pal) -> Div {
        let store = self.store.clone();
        div()
            .flex()
            .items_center()
            .gap(px(8.))
            .px(px(8.))
            .pt(px(4.))
            .text_size(px(12.5))
            .text_color(p.danger)
            .child(icon(IconName::TriangleAlert, p.danger, 13.))
            .child(div().flex_1().min_w_0().child(notice))
            .child(
                div()
                    .id("dismiss-notice")
                    .cursor_pointer()
                    .child(icon(IconName::X, p.muted, 13.))
                    .on_click(move |_, _, cx| store.update(cx, Store::dismiss_notice)),
            )
    }
}

/// A call's recording being started or stopped, until the state shows it.
fn call_change_line(action: RecordAction, p: &Pal) -> impl IntoElement {
    let line = match action {
        RecordAction::Start => "Starting to record the call on the Mac…",
        RecordAction::Stop => "Stopping the call's recording…",
    };
    div()
        .id("call-change")
        .flex()
        .items_center()
        .gap(px(8.))
        .px(px(8.))
        .pt(px(4.))
        .text_size(px(13.))
        .text_color(p.muted)
        .child(icon(IconName::Disc, p.danger, 14.))
        .child(line)
        .test_support()
}

/// Whether the assistant works in the open room now: a turn is live there (with or without a
/// step yet), or the room's state says so.
pub(super) fn is_working(store: &Store) -> bool {
    store.live().is_some()
        || (store.is_state_here() && store.state().is_some_and(|state| state.busy))
}

/// A round button at the field's end.
fn round_button(id: &'static str, color: Hsla) -> gpui_kit::Stateful<Div> {
    div()
        .id(id)
        .cursor_pointer()
        .flex_none()
        .size(px(BUTTON))
        .rounded_full()
        .bg(color)
        .flex()
        .items_center()
        .justify_center()
}

/// An icon of the full set, in `color`, `size` pixels wide.
pub(super) fn icon(name: IconName, color: Hsla, size: f32) -> Icon {
    Icon::new(name).size(px(size)).text_color(color)
}

/// The opacity of a pulse at `t` (`0..1`): full at its middle, faint at its ends.
fn pulse(t: f32) -> f32 {
    0.35 + 0.65 * (1.0 - (2.0 * t - 1.0).abs())
}

/// Puts `text` in the field, the cursor at its end.
pub(super) fn fill(
    input: &Entity<TextareaState>,
    text: &str,
    window: &mut Window,
    cx: &mut Context<'_, ChatView>,
) {
    let line = u32::try_from(text.matches('\n').count()).unwrap_or(u32::MAX);
    let last = text.rsplit('\n').next().unwrap_or_default();
    let character = u32::try_from(last.chars().count()).unwrap_or(u32::MAX);
    input.update(cx, |input, cx| {
        input.set_value(text.to_owned(), window, cx);
        input.set_cursor_position(Position::new(line, character), window, cx);
    });
    cx.notify();
}

/// Attaches what the clipboard holds when it is files or a picture; whether it did (text is
/// left to the field).
fn attach_pasted(store: &Entity<Store>, item: &ClipboardItem, cx: &mut App) -> bool {
    match Pasted::of(item) {
        Pasted::Files(paths) => store.update(cx, |store, cx| store.attach_paths(&paths, cx)),
        Pasted::Picture(picture) => store.update(cx, |store, cx| store.attach_picture(picture, cx)),
        Pasted::Text => return false,
    }
    true
}

/// Asks for files to attach, in the system's open panel (its sidebar reaches the photo
/// library on macOS), its button reading `prompt`.
fn pick_files(store: &Entity<Store>, prompt: &'static str, cx: &mut App) {
    let picked = cx.prompt_for_paths(PathPromptOptions {
        files: true,
        directories: false,
        multiple: true,
        prompt: Some(prompt.into()),
    });
    let store = store.clone();
    cx.spawn(async move |cx| {
        let paths = match picked.await {
            Ok(Ok(Some(paths))) => paths,
            Ok(Ok(None)) => return,
            Ok(Err(error)) => {
                log::warn!("the file picker failed: {error}");
                return;
            }
            Err(error) => {
                log::warn!("the file picker went away: {error}");
                return;
            }
        };
        store.update(cx, |store, cx| store.attach_paths(&paths, cx));
    })
    .detach();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pulse_is_full_in_its_middle() {
        assert!((pulse(0.5) - 1.0).abs() < f32::EPSILON);
        assert!((pulse(0.0) - 0.35).abs() < f32::EPSILON);
        assert!((pulse(1.0) - 0.35).abs() < f32::EPSILON);
    }
}
