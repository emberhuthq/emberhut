//! A session page's box: the field (Return sends, Shift-Return a new line), the files added to
//! the message and Send. On the user's own session it takes pictures and files, and while a turn
//! runs it holds Stop this turn and says that what is sent now runs after it. A message that
//! didn't go comes back into the box, its files too.

use std::path::PathBuf;

use gpui_kit::TestSupportExt as _;
use gpui_kit::assets::IconName;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::input::{InputEvent, Textarea, TextareaState};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, Div, ElementId, Entity, PathPromptOptions, Subscription, Window, div, px,
};

use super::page::SessionPage;
use crate::store::sessions::files::SessionFiles;
use crate::ui::a11y::{self, Accessible as _};
use crate::ui::palette::Pal;
use crate::ui::tasks::marks::glyph;

/// The page's field.
pub(crate) struct Reply {
    field: Entity<TextareaState>,
    /// Whether the field is worded for the user's own session.
    direct: bool,
    /// The session's count of messages that went, as last seen.
    sent_seen: u64,
    _enter: Subscription,
}

impl Reply {
    /// A field for the user's own session (`direct`) or the assistant's.
    pub(crate) fn new(
        direct: bool,
        window: &mut Window,
        cx: &mut Context<'_, SessionPage>,
    ) -> Self {
        let field = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(1, 6)
                .submit_on_enter(true)
                .placeholder(prompt(direct))
        });
        let enter = cx.subscribe_in(&field, window, |page, _, event: &InputEvent, window, cx| {
            if let InputEvent::PressEnter { shift: false, .. } = event {
                page.send(window, cx);
            }
        });
        Self {
            field,
            direct,
            sent_seen: 0,
            _enter: enter,
        }
    }

    /// Words the field for the user's own session (`direct`) or the assistant's, as the session
    /// turns out to be once it has loaded.
    pub(crate) fn set_direct(&mut self, direct: bool, window: &mut Window, cx: &mut App) {
        if self.direct == direct {
            return;
        }
        self.direct = direct;
        self.field.update(cx, |field, cx| {
            field.set_placeholder(prompt(direct), window, cx);
        });
    }

    /// Whether the field is worded for the user's own session.
    #[cfg(test)]
    pub(crate) fn is_direct(&self) -> bool {
        self.direct
    }

    /// What the field holds.
    pub(crate) fn text(&self, cx: &App) -> String {
        self.field.read(cx).value().to_string()
    }

    /// The count of messages that went, as last seen.
    pub(crate) fn sent_seen(&self) -> u64 {
        self.sent_seen
    }

    /// A message went: the count moves on.
    pub(crate) fn sent(&mut self, count: u64, _: &mut Window, _: &mut App) {
        self.sent_seen = count;
    }

    /// A message is on its way: the field empties (it comes back if the message doesn't go).
    pub(crate) fn sending(&mut self, window: &mut Window, cx: &mut App) {
        self.clear(window, cx);
    }

    /// Empties the field.
    pub(crate) fn clear(&mut self, window: &mut Window, cx: &mut App) {
        self.field
            .update(cx, |field, cx| field.set_value("", window, cx));
    }

    /// Puts the words of a message that didn't go back in the field.
    pub(crate) fn restore(&mut self, text: &str, window: &mut Window, cx: &mut App) {
        self.field
            .update(cx, |field, cx| field.set_value(text.to_owned(), window, cx));
    }
}

/// The empty field's words: for the user's own session, or the assistant's.
fn prompt(direct: bool) -> &'static str {
    if direct {
        "Message Claude"
    } else {
        "Answer it, or tell it to carry on"
    }
}

/// Send (`ready`: with something to send), or a spinner while a message is on its way.
fn send_button(sending: bool, ready: bool, p: &Pal, cx: &Context<'_, SessionPage>) -> AnyElement {
    if sending {
        return a11y::said("session-sending", "Sending…")
            .p(px(4.))
            .child(Spinner::new().color(p.muted))
            .into_any_element();
    }
    div()
        .id("session-send")
        .button_named("Send")
        .accessibility_id("session send")
        .p(px(3.))
        .cursor_pointer()
        .child(glyph(
            IconName::CircleArrowUp,
            if ready { p.accent } else { p.faint },
            24.,
        ))
        .on_click(cx.listener(|page, _, window, cx| page.send(window, cx)))
        .test_support()
        .into_any_element()
}

/// The box: files, the field, Send (and Stop this turn while the user's own session runs).
pub(crate) fn composer(
    page: &SessionPage,
    direct: bool,
    running: bool,
    note: &str,
    p: &Pal,
    cx: &Context<'_, SessionPage>,
) -> Div {
    let files = page.files.read(cx);
    let ready = !page.reply.text(cx).trim().is_empty() || !files.files().is_empty();
    let sending = page.is_busy(cx);
    let mut column = div()
        .flex()
        .flex_col()
        .gap(px(4.))
        .px(px(12.))
        .py(px(8.))
        .border_t_1()
        .border_color(p.border)
        .bg(p.surface);
    if running || !note.is_empty() {
        let line = if running {
            "Sent now, it runs after this turn."
        } else {
            note
        };
        column = column.child(
            div()
                .text_size(px(11.5))
                .text_color(p.faint)
                .child(line.to_owned()),
        );
    }
    if direct {
        column = attached(column, &page.files, files, p);
    }
    let mut row = div().flex().items_end().gap(px(8.));
    if direct {
        let box_ = page.files.clone();
        row = row.child(
            div()
                .id("session-attach")
                .button_named("Attach files")
                .p(px(5.))
                .rounded(px(6.))
                .cursor_pointer()
                .hover(|d| d.bg(p.hover))
                .child(glyph(IconName::Paperclip, p.muted, 17.))
                .on_click(move |_, _, cx| pick_files(&box_, cx)),
        );
    }
    if direct && page.dictation.is_listening() {
        row = row.child(recording(page, p, cx));
    } else {
        row = row.child(
            div()
                .flex_1()
                .min_w_0()
                .child(Textarea::new(&page.reply.field)),
        );
        if direct && page.dictation.is_available() {
            row = row.child(mic(page, p, cx));
        }
    }
    if direct && running {
        row = row.child(
            div()
                .id("session-stop-turn")
                .button_named("Stop this turn")
                .p(px(3.))
                .cursor_pointer()
                .child(glyph(IconName::CircleStop, p.danger, 24.))
                .on_click(cx.listener(|page, _, _, cx| page.stop_turn(cx))),
        );
    }
    row = row.child(send_button(sending, ready, p, cx));
    column = column.child(row);
    if direct && let Some(error) = page.dictation.error() {
        column = column.child(
            a11y::said("session-dictation-problem", error.to_owned())
                .text_size(px(11.5))
                .text_color(p.warn)
                .child(error.to_owned()),
        );
    }
    column
}

/// The mic: it starts dictating (a spinner while the words are on their way).
fn mic(page: &SessionPage, p: &Pal, cx: &Context<'_, SessionPage>) -> impl IntoElement {
    let preparing = page.dictation.is_preparing();
    div()
        .id("session-dictate")
        .button_named("Dictate")
        .accessibility_id("session dictate")
        .p(px(5.))
        .rounded(px(6.))
        .cursor_pointer()
        .hover(|d| d.bg(p.hover))
        .child(if preparing {
            super::marks::spinner(p, "session-dictation-spin", 16., cx)
        } else {
            glyph(IconName::Mic, p.muted, 17.).into_any_element()
        })
        .when(!preparing, |d| {
            d.on_click(cx.listener(|page, _, window, cx| page.start_dictation(window, cx)))
        })
}

/// A recording in the field's place: ✕ drops it, the time, ✓ turns it into words.
fn recording(page: &SessionPage, p: &Pal, cx: &Context<'_, SessionPage>) -> impl IntoElement {
    div()
        .flex_1()
        .flex()
        .items_center()
        .gap(px(10.))
        .px(px(10.))
        .py(px(6.))
        .rounded(px(10.))
        .bg(p.hover)
        .child(
            div()
                .id("session-drop-dictation")
                .button_named("Cancel recording")
                .cursor_pointer()
                .child(glyph(IconName::X, p.muted, 15.))
                .on_click(cx.listener(|page, _, _, cx| page.drop_dictation(cx))),
        )
        .child(crate::ui::widgets::dot(p.danger, 8.))
        .child(
            div()
                .flex_1()
                .text_size(px(13.))
                .child(crate::dictation::clock(page.dictation.elapsed())),
        )
        .child(
            div()
                .id("session-done-dictation")
                .button_named("Done recording")
                .cursor_pointer()
                .child(glyph(IconName::Check, p.accent, 17.))
                .on_click(cx.listener(|page, _, window, cx| page.stop_dictation(window, cx))),
        )
}

/// The files added to the message, each with its ✕, and a spinner while one uploads.
fn attached(mut column: Div, box_: &Entity<SessionFiles>, files: &SessionFiles, p: &Pal) -> Div {
    if files.files().is_empty() && !files.is_uploading() && files.note().is_none() {
        return column;
    }
    let mut chips = div().flex().flex_wrap().gap(px(6.));
    for (i, file) in files.files().iter().enumerate() {
        let box_ = box_.clone();
        chips = chips.child(
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .px(px(8.))
                .py(px(4.))
                .rounded(px(8.))
                .border_1()
                .border_color(p.border)
                .text_size(px(12.))
                .child(glyph(IconName::File, p.muted, 13.))
                .child(file.name.clone())
                .child(
                    div()
                        .id(ElementId::NamedInteger(
                            "session-file-remove".into(),
                            i as u64,
                        ))
                        .button_named(format!("Remove {}", file.name))
                        .cursor_pointer()
                        .child(glyph(IconName::X, p.muted, 12.))
                        .on_click(move |_, _, cx| box_.update(cx, |b, cx| b.remove(i, cx))),
                ),
        );
    }
    if files.is_uploading() {
        chips = chips.child(Spinner::new().small().color(p.muted));
    }
    column = column.child(chips);
    if let Some(note) = files.note() {
        column = column.child(
            div()
                .text_size(px(11.5))
                .text_color(p.warn)
                .child(note.to_owned()),
        );
    }
    column
}

/// Asks for files to add to the message.
fn pick_files(box_: &Entity<SessionFiles>, cx: &mut App) {
    let picked = cx.prompt_for_paths(PathPromptOptions {
        files: true,
        directories: false,
        multiple: true,
        prompt: Some("Add".into()),
    });
    let box_ = box_.clone();
    cx.spawn(async move |cx| {
        let paths: Vec<PathBuf> = match picked.await {
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
        box_.update(cx, |box_, cx| box_.add(paths, cx));
    })
    .detach();
}
