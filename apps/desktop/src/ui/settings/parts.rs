//! The pieces Settings' pages are built of: a section with its counted header and footer, a
//! row that opens a page, a tag, the bar back to the main page, and a sheet over a page.

use gpui_kit::assets::IconName;
use gpui_kit::component::Icon;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, Div, ElementId, FocusHandle, FontWeight, Hsla, SharedString, Stateful, actions,
    div, px,
};

use crate::store::settings::chrome::Health;
use crate::ui::a11y::{self, Accessible as _};
use crate::ui::controls::tab_stop;
use crate::ui::palette::Pal;

actions!(
    settings,
    [
        /// The sheet's default action: Done.
        SheetDone,
        /// Closes the sheet.
        SheetClose,
    ]
);

/// The key context of a sheet, where Return is Done and Esc closes it.
pub(crate) const SHEET_CONTEXT: &str = "SettingsSheet";

/// A section: its title (and count), its rows in a card, and a footer under it.
pub(super) fn section(
    p: &Pal,
    title: impl IntoElement,
    count: Option<usize>,
    rows: Vec<AnyElement>,
    footer: Option<String>,
) -> Div {
    let head = div()
        .flex()
        .items_baseline()
        .gap(px(6.))
        .px(px(4.))
        .text_size(px(12.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(p.muted)
        .child(title)
        .when_some(count, |d, count| {
            d.child(div().text_color(p.faint).child(count.to_string()))
        });
    let card = div()
        .flex()
        .flex_col()
        .rounded(px(12.))
        .border_1()
        .border_color(p.border)
        .bg(p.card)
        .children(rows.into_iter().enumerate().map(|(i, row)| {
            div()
                .px(px(14.))
                .py(px(8.))
                .when(i > 0, |d| d.border_t_1().border_color(p.border))
                .child(row)
        }));
    div()
        .flex()
        .flex_col()
        .gap(px(6.))
        .child(head)
        .child(card)
        .when_some(footer, |d, footer| d.child(note(p, footer).px(px(4.))))
}

/// A line of small secondary text.
pub(crate) fn note(p: &Pal, text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(11.5))
        .text_color(p.muted)
        .child(text.into())
}

/// A line of small text saying what went wrong.
pub(crate) fn problem_line(p: &Pal, text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(12.))
        .text_color(p.danger)
        .child(text.into())
}

/// The colour of a row's state.
pub(super) fn health_color(health: Health, p: &Pal) -> Hsla {
    match health {
        Health::Fine => p.success,
        Health::Warn => p.warn,
        Health::Down => p.danger,
        Health::Off => p.muted,
    }
}

/// A row that opens a page: its icon on a tinted square, its name, its state in its colour and
/// what it is, and a chevron.
pub(super) fn nav_row(
    id: impl Into<ElementId>,
    p: &Pal,
    (glyph, tint): (IconName, Hsla),
    name: impl Into<SharedString>,
    (status, health): (String, Health),
    detail: &str,
) -> Stateful<Div> {
    let name = name.into();
    let row = div()
        .id(id)
        .button_named(a11y::line(&[&name, &status, detail]))
        .accessibility_id(format!("{} row", name.to_lowercase()))
        .border_1()
        .rounded(px(6.))
        .flex()
        .items_center()
        .gap(px(10.))
        .cursor_pointer()
        .child(
            div()
                .size(px(20.))
                .rounded(px(5.))
                .bg(tint)
                .flex()
                .items_center()
                .justify_center()
                .child(
                    Icon::new(glyph)
                        .size(px(12.))
                        .text_color(Pal::readable_on(tint)),
                ),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .child(div().text_size(px(13.)).child(name))
                .child(
                    div()
                        .flex()
                        .gap(px(4.))
                        .text_size(px(12.))
                        .child(
                            div()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(health_color(health, p))
                                .child(status),
                        )
                        .when(!detail.is_empty(), |d| {
                            d.child(
                                div()
                                    .text_color(p.muted)
                                    .truncate()
                                    .child(format!("· {detail}")),
                            )
                        }),
                ),
        )
        .child(
            Icon::new(IconName::ChevronRight)
                .size(px(13.))
                .text_color(p.faint),
        );
    tab_stop(row, p)
}

/// A small coloured tag: `Update`, `Paused`, `Needs you`.
pub(crate) fn tag(text: impl Into<SharedString>, color: Hsla) -> Div {
    div()
        .px(px(6.))
        .py(px(1.))
        .rounded(px(8.))
        .bg(color.opacity(0.14))
        .text_color(color)
        .text_size(px(10.5))
        .font_weight(FontWeight::BOLD)
        .flex_none()
        .child(text.into())
}

/// A sheet over the page: a dimmed backdrop and a card with its title, its body and its
/// closing button (`close`: Done, Cancel) at its foot. It holds the keyboard focus (`focus`):
/// Return is that button and Esc closes it the same way.
///
/// Return in one of the sheet's fields is that field's own (Rename, Save, Get link): a
/// single-line field lets the key go on to this binding before it reports Return itself, so
/// the button runs only when the sheet, not something in it, has the focus.
pub(super) fn sheet(
    p: &Pal,
    focus: &FocusHandle,
    title: impl Into<SharedString>,
    body: impl IntoElement,
    (close, done): (
        &'static str,
        impl Fn(&mut gpui_kit::Window, &mut gpui_kit::App) + 'static,
    ),
) -> Div {
    let done = std::rc::Rc::new(done);
    let (on_button, on_key, on_escape) = (done.clone(), done.clone(), done);
    let own = focus.clone();
    div()
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .bg(p.text.opacity(0.18))
        .child(
            div()
                .id("settings-sheet")
                .key_context(SHEET_CONTEXT)
                .track_focus(focus)
                .on_action(move |_: &SheetDone, window, cx| {
                    if own.is_focused(window) {
                        on_key(window, cx);
                    } else {
                        cx.propagate();
                    }
                })
                .on_action(move |_: &SheetClose, window, cx| on_escape(window, cx))
                .w(px(520.))
                .max_h(px(560.))
                .flex()
                .flex_col()
                .rounded(px(14.))
                .border_1()
                .border_color(p.border)
                .bg(p.surface)
                .shadow_lg()
                .child(
                    div()
                        .px(px(18.))
                        .pt(px(16.))
                        .pb(px(8.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_size(px(15.))
                        .child(title.into()),
                )
                .child(
                    div()
                        .id("settings-sheet-body")
                        .flex_1()
                        .overflow_y_scroll()
                        .px(px(18.))
                        .pb(px(12.))
                        .child(body),
                )
                .child(
                    div()
                        .flex()
                        .justify_end()
                        .px(px(14.))
                        .py(px(10.))
                        .border_t_1()
                        .border_color(p.border)
                        .child(
                            Button::new("sheet-done")
                                .primary()
                                .label(close)
                                .on_click(move |_, window, cx| on_button(window, cx)),
                        ),
                ),
        )
}

/// Binds the sheets' keys: Return is Done, Esc closes.
pub(crate) fn bind_keys(cx: &mut gpui_kit::App) {
    cx.bind_keys([
        gpui_kit::KeyBinding::new("enter", SheetDone, Some(SHEET_CONTEXT)),
        gpui_kit::KeyBinding::new("escape", SheetClose, Some(SHEET_CONTEXT)),
    ]);
}

/// Text the user can select and copy (a server's whole error, a link): shown as it is, every
/// ASCII punctuation mark escaped so nothing in it reads as Markdown.
pub(crate) fn selectable(
    id: impl Into<ElementId>,
    text: &str,
) -> gpui_kit::component::text::TextView {
    gpui_kit::component::text::TextView::markdown(id, escape_markdown(text)).selectable(true)
}

/// `text` with every ASCII punctuation mark backslash-escaped (`CommonMark` reads each as itself).
fn escape_markdown(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for c in text.chars() {
        if c.is_ascii_punctuation() {
            escaped.push('\\');
        }
        escaped.push(c);
    }
    escaped
}

/// A page's scrolling body, its content in a column as wide as Settings reads well.
pub(crate) fn page_body(id: &'static str, content: impl IntoElement) -> Stateful<Div> {
    div().id(id).flex_1().overflow_y_scroll().child(
        div()
            .flex()
            .flex_col()
            .gap(px(18.))
            .px(px(24.))
            .py(px(18.))
            .max_w(px(720.))
            .child(content),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selectable_text_reads_as_it_is() {
        assert_eq!(escape_markdown("a *b* [c](d)"), r"a \*b\* \[c\]\(d\)");
        assert_eq!(escape_markdown("plain"), "plain");
    }
}
