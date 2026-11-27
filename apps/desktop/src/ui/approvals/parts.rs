//! What the list's rows and a request's page share: the chips that say what a request is for,
//! who alone may approve it and where it stands, and the colour that marks it.

use emberhut_api::approvals::Approval;
use gpui_kit::assets::IconName as AssetIcon;
use gpui_kit::component::Icon;
use gpui_kit::prelude::*;
use gpui_kit::{Div, FontWeight, Hsla, div, px};

use crate::store::approvals::lists::waits_on_you;
use crate::store::approvals::words::{kind_name, state_name};
use crate::ui::palette::Pal;
use crate::ui::widgets::pill;

/// The words of the fixed core's mark.
pub(super) const ONLY_YOU: &str = "Only you can approve this";

/// A request's colour: the danger colour for the fixed core's kinds, the accent for the rest
/// that wait on the user, muted for what waits on others or is over.
#[must_use]
pub(super) fn tint(approval: &Approval, p: &Pal) -> Hsla {
    if !waits_on_you(approval) {
        p.muted
    } else if approval.kind.is_fixed_core() {
        p.danger
    } else {
        p.accent
    }
}

/// Its chips: what it is for; for the fixed core's kinds (payments, passwords and codes,
/// secrets, account security, anything in the user's name), that only the user can approve
/// it; and where it stands when it doesn't wait on the user.
pub(super) fn chips(approval: &Approval, p: &Pal) -> Div {
    let core = approval.kind.is_fixed_core();
    let kind_color = if core { p.danger } else { tint(approval, p) };
    div()
        .flex()
        .flex_wrap()
        .items_center()
        .gap(px(6.))
        .child(pill(
            kind_name(&approval.kind),
            kind_color.opacity(0.14),
            kind_color,
        ))
        .when(core, |d| d.child(only_you(p)))
        .when(!waits_on_you(approval), |d| {
            d.child(pill(state_name(&approval.state), p.hover, p.muted))
        })
}

/// The fixed core's mark: a lock and [`ONLY_YOU`], strong in the danger colour.
fn only_you(p: &Pal) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(4.))
        .px(px(6.))
        .py(px(1.))
        .rounded(px(8.))
        .bg(p.danger)
        .text_color(p.on_accent)
        .text_size(px(11.))
        .font_weight(FontWeight::SEMIBOLD)
        .flex_none()
        .child(
            Icon::new(AssetIcon::Lock)
                .size(px(11.))
                .text_color(p.on_accent),
        )
        .child(ONLY_YOU)
}
