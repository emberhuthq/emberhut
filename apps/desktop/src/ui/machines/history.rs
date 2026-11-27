//! A machine's last 24 hours and the sessions on it.

use gpui_kit::prelude::*;
use gpui_kit::{
    Bounds, Div, FontWeight, Hsla, PathBuilder, Pixels, Window, canvas, div, point, px,
};

use super::series::{self, Series};
use crate::format::{capitalized, gigabytes, whole};
use crate::ui::palette::Pal;
use crate::ui::widgets::dot;
use emberhut_api::machines::{Machine, MachineSession};

/// The last 24 hours: free memory (green) and processor busy (blue) as two lines on a time
/// axis, and the least memory that was free.
pub(super) fn trend(p: &Pal, machine: &Machine) -> Div {
    let series = Series::of(machine, jiff::Timestamp::now());
    let (memory_color, cpu_color) = (p.success, p.info);
    let legend = |color: Hsla, label: &'static str| {
        div()
            .flex()
            .items_center()
            .gap(px(4.))
            .child(div().w(px(10.)).h(px(3.)).rounded_full().bg(color))
            .child(label)
    };
    let lines = [
        (series::runs(&series.memory), memory_color, 1.5),
        (series::runs(&series.cpu), cpu_color, 1.2),
    ];
    let chart = canvas(
        |_, _, _| {},
        move |bounds, (), window, _| {
            for (runs, color, width) in &lines {
                for run in runs {
                    paint_run(run, bounds, *width, *color, window);
                }
            }
        },
    )
    .w_full()
    .h(px(34.));
    div()
        .flex()
        .flex_col()
        .gap(px(4.))
        .pt(px(4.))
        .child(
            div()
                .flex()
                .gap(px(10.))
                .text_size(px(11.))
                .text_color(p.muted)
                .child(div().font_weight(FontWeight::MEDIUM).child("Last 24 h"))
                .child(div().flex_1())
                .child(legend(memory_color, "Memory free"))
                .child(legend(cpu_color, "CPU")),
        )
        .child(
            div()
                .w_full()
                .rounded(px(4.))
                .bg(p.faint.opacity(0.08))
                .child(chart),
        )
        .when_some(series.lowest, |d, lowest| {
            d.child(
                div()
                    .text_size(px(11.5))
                    .text_color(p.muted)
                    .child(format!("Least free memory: {}", gigabytes(Some(lowest)))),
            )
        })
}

/// Strokes one run of a line, its points from 0 to 1 (`y` upwards) inside `bounds`.
#[expect(
    clippy::cast_possible_truncation,
    reason = "shares from 0 to 1 scale a few hundred pixels: well inside f32"
)]
fn paint_run(
    run: &[(f64, f64)],
    bounds: Bounds<Pixels>,
    width: f32,
    color: Hsla,
    window: &mut Window,
) {
    let at = |(x, y): (f64, f64)| {
        point(
            bounds.origin.x + bounds.size.width * x as f32,
            bounds.origin.y + bounds.size.height * (1. - y as f32),
        )
    };
    let [first, rest @ ..] = run else {
        return;
    };
    let mut path = PathBuilder::stroke(px(width));
    path.move_to(at(*first));
    if rest.is_empty() {
        // A lone point: a dot's worth of line, so it shows.
        path.line_to(at((first.0, first.1)) + point(px(1.), px(0.)));
    }
    for &next in rest {
        path.line_to(at(next));
    }
    match path.build() {
        Ok(path) => window.paint_path(path, color),
        Err(error) => log::debug!("the trend's line didn't build: {error}"),
    }
}

/// The sessions on the machine.
pub(super) fn sessions(p: &Pal, machine: &Machine, mut card: Div) -> Div {
    if machine.sessions.is_empty() {
        return card;
    }
    card = card.child(
        div()
            .pt(px(4.))
            .text_size(px(11.5))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(p.muted)
            .child(format!("Sessions here · {}", machine.sessions.len())),
    );
    for session in &machine.sessions {
        let running = session.running;
        let mut line = vec![if running {
            "Running".to_owned()
        } else {
            session
                .status
                .as_deref()
                .map_or_else(|| "Idle".to_owned(), capitalized)
        }];
        if let Some(doing) = session.doing.clone().filter(|doing| !doing.is_empty()) {
            line.push(doing);
        }
        let account = account_line(session);
        card = card.child(
            div()
                .flex()
                .items_start()
                .gap(px(8.))
                .child(
                    div()
                        .pt(px(6.))
                        .child(dot(if running { p.info } else { p.faint }, 7.)),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(1.))
                        .min_w_0()
                        .flex_1()
                        .child(
                            div().text_size(px(13.)).child(
                                session
                                    .title
                                    .clone()
                                    .unwrap_or_else(|| session.id.to_string()),
                            ),
                        )
                        .child(
                            div()
                                .truncate()
                                .text_size(px(11.5))
                                .text_color(if running { p.info } else { p.muted })
                                .child(line.join(" · ")),
                        )
                        .when_some(account, |d, account| {
                            d.child(div().text_size(px(11.5)).text_color(p.faint).child(account))
                        }),
                ),
        );
    }
    card
}

/// A session's account and its use: `Example account · 5h 40% · 7d 12% · limited until 18:00`.
fn account_line(session: &MachineSession) -> Option<String> {
    let name = session
        .account_name
        .clone()
        .or_else(|| session.account.clone())?;
    let mut parts = vec![name];
    if let Some(used) = session.used5h {
        parts.push(format!("5h {}%", whole(used)));
    }
    if let Some(used) = session.used7d {
        parts.push(format!("7d {}%", whole(used)));
    }
    let until = session
        .limited_until
        .as_ref()
        .and_then(|until| until.as_str().parse::<jiff::Timestamp>().ok())
        .map(|until| until.to_zoned_datetime(jiff::tz::TimeZone::system()));
    if let Some(until) = until {
        parts.push(format!("limited until {}", until.strftime("%H:%M")));
    }
    Some(parts.join(" · "))
}
