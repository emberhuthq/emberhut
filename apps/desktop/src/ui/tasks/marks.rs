//! The small marks a task is drawn with: its state icon, its deadline chip, its progress bar,
//! a finding's state, and the colour of each state.

use emberhut_api::tasks::{Issue, Landed};
use gpui_kit::assets::IconName;
use gpui_kit::component::Icon;
use gpui_kit::prelude::*;
use gpui_kit::{Div, FontWeight, Hsla, div, px, relative};

use crate::store::tasks::counts::{Chip, ChipKind};
use crate::store::tasks::words::{self, Mark, Tone};
use crate::ui::palette::Pal;
use crate::ui::widgets::share;

/// A Lucide icon in a colour and size.
pub(crate) fn glyph(name: IconName, color: Hsla, size: f32) -> Icon {
    Icon::new(name).size(px(size)).text_color(color)
}

/// The mark of a finding's state.
pub(crate) fn landed_mark(p: &Pal, landed: &Landed) -> Icon {
    let icon = match landed {
        Landed::Landed => IconName::CircleCheck,
        Landed::Partly => IconName::CircleDashed,
        Landed::NotLanded => IconName::CircleX,
        Landed::Superseded => IconName::Redo2,
        _ => IconName::CircleQuestionMark,
    };
    glyph(
        icon,
        tone_color(p, crate::store::tasks::findings::tone(landed)),
        13.,
    )
}

/// A state's colour.
#[must_use]
pub(crate) fn tone_color(p: &Pal, tone: Tone) -> Hsla {
    match tone {
        Tone::Accent => p.accent,
        Tone::Warn => p.warn,
        Tone::Teal | Tone::Info => p.info,
        Tone::Success => p.success,
        Tone::Danger => p.danger,
        Tone::Muted => p.muted,
    }
}

/// The state icon: a ring in the state's colour with what is happening inside it (half filled
/// while running, a quarter queued, a pause when nothing runs, a clock when scheduled, a tick
/// when done).
pub(crate) fn status_icon(p: &Pal, task: &Issue) -> Div {
    let color = tone_color(p, words::tone(task));
    let ring = || {
        div()
            .size(px(16.))
            .flex_none()
            .rounded_full()
            .border_2()
            .border_color(color)
            .flex()
            .items_center()
            .justify_center()
    };
    match words::mark(task) {
        // The right half of a disc, then a quarter of one: the share of the turn done.
        Mark::Half => ring().child(
            div()
                .size(px(8.))
                .flex()
                .child(div().w(px(4.)))
                .child(div().w(px(4.)).h_full().rounded_r(px(4.)).bg(color)),
        ),
        Mark::Quarter => ring().child(
            div().size(px(8.)).flex().flex_col().child(
                div()
                    .h(px(4.))
                    .flex()
                    .child(div().w(px(4.)))
                    .child(div().size(px(4.)).rounded_tr(px(4.)).bg(color)),
            ),
        ),
        Mark::Soft => ring().child(div().size(px(8.)).rounded_full().bg(color.opacity(0.35))),
        Mark::Pause => ring().child(glyph(IconName::Pause, color, 8.)),
        Mark::Clock => ring().child(glyph(IconName::Clock, color, 9.)),
        Mark::Eye => ring().child(glyph(IconName::Eye, color, 9.)),
        Mark::Tick => div()
            .size(px(16.))
            .flex_none()
            .rounded_full()
            .bg(color)
            .flex()
            .items_center()
            .justify_center()
            .child(glyph(IconName::Check, p.on_accent, 10.)),
        Mark::Ring => ring(),
    }
}

/// A deadline chip: a missed one red, someone else's filled orange, its own outlined.
pub(crate) fn deadline_chip(p: &Pal, chip: Chip) -> Div {
    let base = div()
        .px(px(7.))
        .py(px(1.))
        .rounded(px(8.))
        .text_size(px(11.))
        .flex_none()
        .child(chip.text);
    match chip.kind {
        ChipKind::Late => base
            .bg(p.danger.opacity(0.18))
            .text_color(p.danger)
            .font_weight(FontWeight::SEMIBOLD),
        ChipKind::Hard => base
            .bg(p.warn.opacity(0.18))
            .text_color(p.warn)
            .font_weight(FontWeight::SEMIBOLD),
        ChipKind::Soft => base.border_1().border_color(p.border).text_color(p.muted),
    }
}

/// A thin bar filled to `fraction` (0 to 1).
pub(crate) fn progress(p: &Pal, fraction: f64, fill: Hsla) -> Div {
    div()
        .h(px(3.))
        .w_full()
        .mt(px(3.))
        .rounded_full()
        .bg(p.border)
        .child(
            div()
                .h_full()
                .rounded_full()
                .bg(fill)
                .w(relative(share(fraction))),
        )
}

/// A whole count with its thousands marked: `4,200`.
#[must_use]
pub(crate) fn thousands(value: f64) -> String {
    let whole = crate::format::whole(value);
    let digits = whole.unsigned_abs().to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3 + 1);
    if whole < 0 {
        out.push('-');
    }
    for (i, digit) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_mark_their_thousands() {
        assert_eq!(thousands(4200.0), "4,200");
        assert_eq!(thousands(1_234_567.0), "1,234,567");
        assert_eq!(thousands(950.4), "950");
        assert_eq!(thousands(-1000.0), "-1,000");
    }
}
