//! The Dark room's own controls, drawn in its dark palette whatever the app's appearance: the
//! widget library takes its colours from the one app-wide theme, so in a light app its field,
//! buttons, tooltips, menus, prompts and code blocks would be light. The room draws its own
//! instead, and gives the field its colours each frame.

use gpui_kit::assets::IconName;
use gpui_kit::base::input::{DiagnosticColors, InputEditorStyle};
use gpui_kit::component::Icon;
use gpui_kit::component::highlighter::HighlightTheme;
use gpui_kit::component::input::TextareaState;
use gpui_kit::component::text::TextViewStyle;
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, AnyView, App, Div, ElementId, Entity, FontWeight, HighlightStyle, SharedString,
    Stateful, StyleRefinement, Window, canvas, div, px,
};

use crate::ui::a11y::Accessible as _;
use crate::ui::palette::Pal;

/// The field's colours in the palette `p`: its text, placeholder, selection and caret.
#[must_use]
fn field_style(p: &Pal) -> InputEditorStyle {
    InputEditorStyle {
        foreground: p.text,
        muted_foreground: p.faint,
        background: p.surface_raised,
        border: p.border,
        selection: p.accent.opacity(0.35),
        caret: p.text,
        diagnostics: DiagnosticColors {
            error: p.danger,
            warning: p.warn,
            info: p.info,
            hint: p.muted,
        },
        ..InputEditorStyle::default()
    }
}

/// Gives `field` the palette's colours for this frame. The field takes the app's theme as it
/// lays out and reads its colours as it prepaints, so this goes before it, in the same parent:
/// its prepaint runs after the field has laid out and before the field prepaints.
pub(super) fn colour_field(field: &Entity<TextareaState>, p: &Pal) -> impl IntoElement {
    let (field, style) = (field.clone(), field_style(p));
    canvas(
        move |_, _, cx| field.update(cx, |state, _| state.set_editor_style(style)),
        |_, (), _, _| {},
    )
    .absolute()
    .size_0()
}

/// How an answer's markdown draws in the palette `p`: dark code blocks and inline code.
#[must_use]
pub(super) fn answer_style(p: &Pal) -> TextViewStyle {
    TextViewStyle {
        highlight_theme: HighlightTheme::default_dark(),
        code_block: StyleRefinement::default()
            .bg(p.surface_raised)
            .border_color(p.border),
        inline_code: HighlightStyle {
            background_color: Some(p.surface_raised),
            ..HighlightStyle::default()
        },
        is_dark: true,
        ..TextViewStyle::default()
    }
}

/// A button showing `icon`, called `tip` on its tooltip and for the screen reader, lit under
/// the pointer while `enabled`.
pub(super) fn icon_button(
    id: &'static str,
    icon: IconName,
    tip: &'static str,
    enabled: bool,
    p: &Pal,
) -> Stateful<Div> {
    let (hover, tint) = (p.hover, if enabled { p.text } else { p.faint });
    let (surface, border, text) = (p.surface_raised, p.border, p.text);
    div()
        .id(id)
        .button_named(tip)
        .flex()
        .items_center()
        .justify_center()
        .size(px(28.))
        .rounded(px(7.))
        .when(enabled, |d| d.cursor_pointer().hover(move |d| d.bg(hover)))
        .child(Icon::new(icon).size(px(15.)).text_color(tint))
        .tooltip(move |_, cx| tooltip(tip, surface, border, text, cx))
}

/// A tooltip in the room's colours.
fn tooltip(
    text: &'static str,
    surface: gpui_kit::Hsla,
    border: gpui_kit::Hsla,
    ink: gpui_kit::Hsla,
    cx: &mut App,
) -> AnyView {
    cx.new(|_| Tip {
        text,
        surface,
        border,
        ink,
    })
    .into()
}

/// A tooltip's view.
struct Tip {
    text: &'static str,
    surface: gpui_kit::Hsla,
    border: gpui_kit::Hsla,
    ink: gpui_kit::Hsla,
}

impl Render for Tip {
    fn render(&mut self, _: &mut Window, _: &mut Context<'_, Self>) -> impl IntoElement {
        div()
            .px(px(8.))
            .py(px(4.))
            .rounded(px(6.))
            .bg(self.surface)
            .border_1()
            .border_color(self.border)
            .text_color(self.ink)
            .text_size(px(12.))
            .child(self.text)
    }
}

/// A menu item: `icon` and `label`, lit under the pointer.
pub(super) fn menu_item(
    id: impl Into<ElementId>,
    icon: IconName,
    label: &'static str,
    p: &Pal,
) -> Stateful<Div> {
    let hover = p.hover;
    div()
        .id(id)
        .flex()
        .items_center()
        .gap(px(8.))
        .px(px(10.))
        .py(px(5.))
        .rounded(px(6.))
        .cursor_pointer()
        .hover(move |d| d.bg(hover))
        .child(Icon::new(icon).size(px(14.)).text_color(p.muted))
        .child(label)
}

/// A menu's panel holding `items`.
pub(super) fn menu_panel(p: &Pal, items: impl IntoIterator<Item = AnyElement>) -> Div {
    div()
        .min_w(px(140.))
        .p(px(4.))
        .rounded(px(9.))
        .bg(p.surface_raised)
        .border_1()
        .border_color(p.border)
        .shadow_lg()
        .text_color(p.text)
        .text_size(px(13.))
        .children(items)
}

/// A question over the whole room: `title`, `detail`, then its buttons (the first one does
/// it, in `danger`'s colour; the last one cancels).
pub(super) fn confirm(
    title: &'static str,
    detail: &'static str,
    buttons: [Stateful<Div>; 2],
    p: &Pal,
) -> Div {
    let [act, cancel] = buttons;
    div()
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .bg(gpui_kit::black().opacity(0.5))
        .occlude()
        .child(
            div()
                .w(px(340.))
                .flex()
                .flex_col()
                .gap(px(10.))
                .p(px(18.))
                .rounded(px(14.))
                .bg(p.surface_raised)
                .border_1()
                .border_color(p.border)
                .shadow_lg()
                .text_color(p.text)
                .child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_size(px(14.))
                        .child(title),
                )
                .child(div().text_size(px(12.5)).text_color(p.muted).child(detail))
                .child(
                    div()
                        .pt(px(6.))
                        .flex()
                        .justify_end()
                        .gap(px(8.))
                        .child(cancel)
                        .child(act),
                ),
        )
}

/// A button of a question: `label`, filled with `fill` (none: outlined).
pub(super) fn dialog_button(
    id: &'static str,
    label: impl Into<SharedString>,
    fill: Option<gpui_kit::Hsla>,
    p: &Pal,
) -> Stateful<Div> {
    let hover = p.hover;
    div()
        .id(id)
        .px(px(12.))
        .py(px(5.))
        .rounded(px(7.))
        .cursor_pointer()
        .text_size(px(13.))
        .font_weight(FontWeight::MEDIUM)
        .map(|d| match fill {
            Some(fill) => d.bg(fill).text_color(gpui_kit::white()),
            None => d
                .border_1()
                .border_color(p.border)
                .hover(move |d| d.bg(hover)),
        })
        .child(label.into())
}
