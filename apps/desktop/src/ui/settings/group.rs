//! A group page's frame: its title with how it stands at the right, then its blocks in a
//! scrolling column, each holding the settings whose entries anchor it. A setting opened from
//! the front door scrolls its block into view and washes it in the accent for a second; a
//! technical part sits under a disclosure that opens in place.

use std::time::Duration;

use gpui_kit::TestSupportExt as _;
use gpui_kit::assets::IconName;
use gpui_kit::component::Icon;
use gpui_kit::prelude::*;
use gpui_kit::{
    Animation, AnimationExt as _, AnyElement, Div, ElementId, FontWeight, SharedString, div, px,
};

use super::SettingsView;
use crate::store::settings::entries::{Fold, Group};
use crate::ui::a11y::{self, Accessible as _};
use crate::ui::controls::tab_stop;
use crate::ui::palette::Pal;

/// How long an opened setting's block stays washed in the accent.
pub(super) const FLASH: Duration = Duration::from_secs(1);

/// The id of a group page's scrolling column.
pub(super) const GROUP_SCROLL: &str = "settings-group-scroll";

/// A part of a group page, and the settings in it.
pub(super) struct Block {
    /// The ids of the entries whose rows are in it.
    anchors: Vec<&'static str>,
    /// What it draws.
    element: AnyElement,
}

/// A block holding the settings `anchors` (their entries' ids).
pub(super) fn block(anchors: &[&'static str], element: impl IntoElement) -> Block {
    Block {
        anchors: anchors.to_vec(),
        element: element.into_any_element(),
    }
}

/// A block of no setting of its own (a line of text, why a list didn't come).
pub(super) fn plain(element: impl IntoElement) -> Block {
    block(&[], element)
}

impl SettingsView {
    /// `group`'s page: its title and status, then `blocks` in the scrolling column, the one
    /// opened from the front door scrolled to and washed.
    pub(super) fn group_page(
        &mut self,
        group: Group,
        status: String,
        blocks: Vec<Block>,
        p: &Pal,
    ) -> Div {
        if self.shown_group != Some(group) {
            self.shown_group = Some(group);
            if self.scroll_to.is_none() {
                self.group_scroll
                    .set_offset(gpui_kit::point(px(0.), px(0.)));
            }
        }
        if let Some(anchor) = self.scroll_to.take()
            && let Some(index) = blocks
                .iter()
                .position(|block| block.anchors.contains(&anchor))
        {
            self.group_scroll.scroll_to_item(index);
        }
        let flashing = self.flash.filter(|(_, at)| at.elapsed() < FLASH);
        let body = div()
            .id(GROUP_SCROLL)
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&self.group_scroll)
            .flex()
            .flex_col()
            .gap(px(18.))
            .px(px(24.))
            .py(px(18.))
            .children(blocks.into_iter().map(|block| {
                let lit = flashing.filter(|(anchor, _)| block.anchors.contains(anchor));
                let holder = div().w_full().max_w(px(720.)).rounded(px(12.));
                match lit {
                    Some((anchor, at)) => {
                        let accent = p.accent;
                        let id = ElementId::Name(format!("flash-{anchor}-{at:?}").into());
                        holder
                            .child(block.element)
                            .with_animation(id, Animation::new(FLASH), move |d, delta| {
                                d.bg(accent.opacity(0.18 * (1. - delta)))
                            })
                            .into_any_element()
                    }
                    None => holder.child(block.element).into_any_element(),
                }
            }));
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(head(group.name(), status, p))
            .child(body)
    }

    /// A disclosure: its title and what is under it in a line, the chevron turning as it
    /// opens; open, `content` under it.
    pub(super) fn fold(
        &self,
        fold: Fold,
        (title, summary): (&'static str, String),
        content: Vec<AnyElement>,
        p: &Pal,
        cx: &Context<'_, Self>,
    ) -> Div {
        let open = self.folds.contains(&fold);
        let head = tab_stop(
            div()
                .id(fold.id())
                .button_named(a11y::line(&[title, &summary]))
                .aria_expanded(open)
                .flex()
                .items_center()
                .gap(px(10.))
                .px(px(14.))
                .py(px(10.))
                .rounded(px(12.))
                .border_1()
                .bg(p.card)
                .cursor_pointer()
                .hover(|d| d.bg(p.hover))
                .child(
                    Icon::new(if open {
                        IconName::ChevronDown
                    } else {
                        IconName::ChevronRight
                    })
                    .size(px(13.))
                    .text_color(p.muted),
                )
                .child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_size(px(13.5))
                        .child(title),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_size(px(12.))
                        .text_color(p.muted)
                        .child(summary),
                )
                .on_click(cx.listener(move |this, _, _, cx| this.toggle_fold(fold, cx))),
            p,
        )
        .border_color(p.border)
        .test_support();
        div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(head)
            .when(open, |d| d.children(content))
    }

    /// Opens `fold`, or folds it again.
    pub(super) fn toggle_fold(&mut self, fold: Fold, cx: &mut Context<'_, Self>) {
        if !self.folds.remove(&fold) {
            self.folds.insert(fold);
        }
        cx.notify();
    }

    /// Whether the block of `anchor` is washed now.
    #[cfg(test)]
    pub(super) fn flashing(&self) -> Option<&'static str> {
        self.flash
            .filter(|(_, at)| at.elapsed() < FLASH)
            .map(|(anchor, _)| anchor)
    }
}

/// A group page's head: its title, and how it stands at the right.
fn head(title: impl Into<SharedString>, status: String, p: &Pal) -> Div {
    div()
        .flex()
        .items_baseline()
        .gap(px(10.))
        .h(px(52.))
        .px(px(24.))
        .border_b_1()
        .border_color(p.border)
        .child(
            div()
                .flex_1()
                .min_w_0()
                .font_weight(FontWeight::SEMIBOLD)
                .text_size(px(15.))
                .pt(px(16.))
                .child(title.into()),
        )
        .child(
            div()
                .id("settings-group-status")
                .text_size(px(12.))
                .text_color(p.muted)
                .child(status),
        )
}
