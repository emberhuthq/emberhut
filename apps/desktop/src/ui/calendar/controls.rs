//! The calendar's bar: Today (off while today shows), the arrows, the month or day as a big
//! title, Day | Week | Month, and the colours' legend.

use gpui_kit::TestSupportExt as _;
use gpui_kit::assets::IconName as AssetIcon;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{Disableable as _, Icon, Sizable as _};
use gpui_kit::prelude::*;
use gpui_kit::{Div, ElementId, FontWeight, div, px};

use super::{CalendarNext, CalendarPrevious, CalendarToday, CalendarView};
use crate::store::calendar::range::{self, Mode};
use crate::ui::a11y::Accessible as _;
use crate::ui::controls::{tab_stop, tip_for};
use crate::ui::palette::Pal;

impl CalendarView {
    /// The bar.
    pub(super) fn controls(&self, p: &Pal, cx: &mut Context<'_, Self>) -> Div {
        let today_shown = range::shows_today(self.mode, self.anchor, self.today());
        let arrow = |id: &'static str,
                     icon: AssetIcon,
                     tip: &'static str,
                     by: i64,
                     action: &dyn gpui_kit::Action,
                     cx: &mut Context<'_, Self>| {
            tip_for(
                tab_stop(
                    div()
                        .id(id)
                        .button_named(tip)
                        .flex()
                        .items_center()
                        .justify_center()
                        .size(px(26.))
                        .rounded(px(6.))
                        .border_2()
                        .cursor_pointer()
                        .hover(|d| d.bg(p.hover))
                        .child(Icon::new(icon).size(px(16.)).text_color(p.text)),
                    p,
                ),
                tip,
                action,
            )
            .on_click(cx.listener(move |this, _, _, cx| this.step(by, cx)))
            // Seen by the headless tests (a no-op outside them).
            .test_support()
        };
        let modes = self.mode_picker(p, cx);
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(10.))
            .px(px(12.))
            .py(px(8.))
            .child(
                Button::new("calendar-today")
                    .small()
                    .outline()
                    .label("Today")
                    .disabled(today_shown)
                    .tooltip_with_action("Today", &CalendarToday, Some(super::CONTEXT))
                    .on_click(cx.listener(|this, _, _, cx| this.go_today(cx))),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(2.))
                    .child(arrow(
                        "calendar-previous",
                        AssetIcon::ChevronLeft,
                        "Previous",
                        -1,
                        &CalendarPrevious,
                        cx,
                    ))
                    .child(arrow(
                        "calendar-next",
                        AssetIcon::ChevronRight,
                        "Next",
                        1,
                        &CalendarNext,
                        cx,
                    )),
            )
            .child(
                div()
                    // Said as the arrows, Today or a mode move it.
                    .id("calendar-title")
                    .announced(range::title(self.mode, self.anchor))
                    .min_w_0()
                    .truncate()
                    .text_size(px(20.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(range::title(self.mode, self.anchor)),
            )
            .child(div().flex_1())
            .child(modes)
            .child(
                Button::new("calendar-legend")
                    .ghost()
                    .icon(Icon::new(AssetIcon::Info).size(px(15.)))
                    .accessibility_label("What the colours mean")
                    .toggled(self.legend)
                    .tooltip("What the colours mean")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.legend = !this.legend;
                        cx.notify();
                    })),
            )
    }

    /// Day | Week | Month.
    fn mode_picker(&self, p: &Pal, cx: &mut Context<'_, Self>) -> Div {
        let mut modes = div()
            .flex()
            .items_center()
            .p(px(2.))
            .gap(px(2.))
            .rounded(px(7.))
            .bg(p.hover);
        for mode in Mode::ALL {
            let on = mode == self.mode;
            modes = modes.child(
                tab_stop(
                    div()
                        .id(ElementId::Name(
                            format!("calendar-mode-{}", mode.name()).into(),
                        ))
                        .button_named(mode.name())
                        .aria_selected(on)
                        .px(px(12.))
                        .py(px(3.))
                        .rounded(px(5.))
                        .border_1()
                        .text_size(px(12.))
                        .font_weight(if on {
                            FontWeight::SEMIBOLD
                        } else {
                            FontWeight::NORMAL
                        })
                        .cursor_pointer()
                        .when(on, |d| d.bg(p.surface_raised).shadow_sm())
                        .child(mode.name())
                        .on_click(cx.listener(move |this, _, _, cx| this.set_mode(mode, cx))),
                    p,
                )
                .test_support(),
            );
        }
        modes
    }
}
