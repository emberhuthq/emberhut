//! Paused: over the composer, a line saying the assistant is paused with the one button that
//! undoes it; and the line saying a pause or resume didn't go through.

use gpui_kit::assets::IconName;
use gpui_kit::component::Icon;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::prelude::*;
use gpui_kit::{App, Div, Entity, FontWeight, div, px};

use crate::brand::APP_NAME;
use crate::store::Store;
use crate::ui::palette::Pal;

/// The paused line, while the assistant is paused; the failure line, after a failed switch.
pub(super) fn banner(store: &Entity<Store>, p: &Pal, cx: &App) -> Option<Div> {
    let held = store.read(cx);
    let paused = !held.is_on_duty();
    let problem = held.on_duty_problem().map(str::to_owned);
    if !paused && problem.is_none() {
        return None;
    }
    let (resume, dismiss) = (store.clone(), store.clone());
    Some(
        div()
            .mx(px(20.))
            .mb(px(6.))
            .flex()
            .flex_col()
            .gap(px(6.))
            .px(px(14.))
            .py(px(10.))
            .rounded(px(16.))
            .border_1()
            .border_color(p.border)
            .bg(p.card)
            .when(paused, |d| {
                d.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(12.))
                        .child(
                            Icon::new(IconName::CirclePause)
                                .size(px(20.))
                                .text_color(p.muted),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .flex_1()
                                .child(
                                    div()
                                        .text_size(px(13.))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child(format!("{APP_NAME} is paused")),
                                )
                                .child(div().text_size(px(12.)).text_color(p.muted).child(
                                    "It wakes for nothing until you resume it. It still answers \
                                     you here.",
                                )),
                        )
                        .child(Button::new("resume").primary().label("Resume").on_click(
                            move |_, _, cx| {
                                resume.update(cx, |store, cx| store.set_on_duty(true, cx));
                            },
                        )),
                )
            })
            .when_some(problem, |d, problem| {
                d.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .text_size(px(12.))
                        .text_color(p.danger)
                        .child(div().flex_1().child(problem))
                        .child(
                            div()
                                .id("dismiss-on-duty")
                                .cursor_pointer()
                                .text_color(p.muted)
                                .child("Dismiss")
                                .on_click(move |_, _, cx| {
                                    dismiss.update(cx, Store::dismiss_on_duty_problem);
                                }),
                        ),
                )
            }),
    )
}
