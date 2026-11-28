//! What makes a control usable from the keyboard and explained on hover: a place in the Tab
//! order with a focus ring (Enter or Space then clicks it, as GPUI does for a focused element
//! with a click handler), and a tooltip, with the shortcut of its action when it has one.

use gpui_kit::component::IconName;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::prelude::*;
use gpui_kit::{
    Action, Div, ElementId, FontWeight, SharedString, Stateful, div, px, transparent_black,
};

use super::a11y::Accessible as _;
use super::palette::Pal;
use super::widgets::icon;

/// Puts `element` in the window's Tab order, its border coloured as a ring while the keyboard
/// has it. The element keeps a clear border of its own width, so the ring moves nothing.
pub(crate) fn tab_stop(element: Stateful<Div>, p: &Pal) -> Stateful<Div> {
    let ring = p.accent;
    element
        .border_color(transparent_black())
        .tab_index(0)
        .focus_visible(move |style| style.border_color(ring))
}

/// One segment of a segmented switch (System, Light, Dark), raised while it is the one `on`.
pub(crate) fn segment(id: &'static str, label: &'static str, on: bool, p: &Pal) -> Stateful<Div> {
    div()
        .id(id)
        .button_named(label)
        .aria_selected(on)
        .px(px(12.))
        .py(px(5.))
        .rounded(px(7.))
        .cursor_pointer()
        .text_size(px(12.5))
        .when(on, |d| {
            d.bg(p.surface)
                .border_1()
                .border_color(p.border)
                .font_weight(FontWeight::SEMIBOLD)
        })
        .when(!on, |d| d.text_color(p.muted))
        .child(label)
}

/// Shows `text` on hover over `element`.
pub(crate) fn tip(element: Stateful<Div>, text: impl Into<SharedString>) -> Stateful<Div> {
    let text = text.into();
    element.tooltip(move |window, cx| Tooltip::new(text.clone()).build(window, cx))
}

/// Shows `text` on hover over `element`, with the shortcut bound to `action` (if any).
pub(crate) fn tip_for(
    element: Stateful<Div>,
    text: impl Into<SharedString>,
    action: &dyn Action,
) -> Stateful<Div> {
    let text = text.into();
    let action = action.boxed_clone();
    element.tooltip(move |window, cx| {
        Tooltip::new(text.clone())
            .action(action.as_ref(), None)
            .build(window, cx)
    })
}

/// A small square button with an icon: in the Tab order, with a tooltip.
pub(crate) fn icon_button(
    id: impl Into<ElementId>,
    name: IconName,
    text: impl Into<SharedString>,
    action: &dyn Action,
    p: &Pal,
) -> Stateful<Div> {
    let text = text.into();
    let button = div()
        .id(id)
        // The icon says nothing to a screen reader: it hears the tip's words.
        .button_named(text.clone())
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(26.))
        .rounded(px(6.))
        .border_2()
        .cursor_pointer()
        .hover(|d| d.bg(p.hover))
        .child(icon(name, p.muted, 15.));
    tip_for(tab_stop(button, p), text, action)
}
