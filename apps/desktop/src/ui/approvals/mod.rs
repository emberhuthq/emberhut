//! Agents' approval requests: the pending list (every agent's, or one agent's from its room's
//! bar), a request's page with its exact payload and the user's answer, and Needs you's row
//! that opens the list.

pub(crate) mod detail;
pub(crate) mod list;
mod parts;

use gpui_kit::assets::IconName as AssetIcon;
use gpui_kit::component::{Icon, IconName};
use gpui_kit::prelude::*;
use gpui_kit::{Div, FontWeight, Stateful, div, px};

use crate::store::Page;
use crate::store::approvals::lists::Summary;
use crate::ui::a11y::{self, Accessible as _};
use crate::ui::column;
use crate::ui::controls::tab_stop;
use crate::ui::palette::Pal;

/// Needs you's row of the pending requests: how many wait on the user, how many of those only
/// they can approve, and how many wait on others. It opens the list over Needs you (Back
/// returns there).
pub(crate) fn needs_row(summary: Summary, p: &Pal) -> Stateful<Div> {
    let title = summary.title();
    let subtitle = summary.subtitle();
    let strong = summary.yours > 0;
    let color = if summary.only_you > 0 {
        p.danger
    } else if strong {
        p.accent
    } else {
        p.muted
    };
    tab_stop(
        div()
            .id("approvals-row")
            .button_named(a11y::line(&[
                "Approvals",
                &title,
                subtitle.as_deref().unwrap_or_default(),
            ]))
            .hint("Opens the pending approval requests")
            .border_1()
            .rounded(px(10.))
            .flex()
            .items_center()
            .gap(px(10.))
            .p(px(12.))
            .bg(p.card)
            .border_color(if strong { color.opacity(0.6) } else { p.border })
            .cursor_pointer()
            .hover(|d| d.bg(p.hover))
            .child(
                Icon::new(AssetIcon::BadgeCheck)
                    .size(px(18.))
                    .text_color(color),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .text_size(px(13.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(format!("Approvals · {title}")),
                    )
                    .when_some(subtitle, |d, subtitle| {
                        d.child(
                            div()
                                .text_size(px(12.))
                                .text_color(p.muted)
                                .truncate()
                                .child(subtitle),
                        )
                    }),
            )
            .child(
                Icon::new(IconName::ChevronRight)
                    .size(px(13.))
                    .text_color(p.faint),
            )
            .on_click(|_, window, cx| column::push(Page::Approvals(None), window, cx)),
        p,
    )
}

#[cfg(test)]
mod tests;
