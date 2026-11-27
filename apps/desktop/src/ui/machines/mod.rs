//! Machines: each machine the assistant works on, with its processor, memory, graphics,
//! disks, battery, session runner, last 24 hours and sessions.

use gpui_kit::component::IconName;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::prelude::*;
use gpui_kit::{Div, FontWeight, Hsla, ScrollHandle, div, px};

mod bars;
mod history;
mod series;
pub(crate) mod summary;

use super::palette::{Pal, pal};
use super::widgets::{dot, icon, page_head, pill, scroller};
use super::window::MainWindow;
use emberhut_api::machines::{Machine, MachineState, WarningLevel};

/// The page.
pub(crate) fn page(
    window: &mut MainWindow,
    scroll: &ScrollHandle,
    cx: &mut Context<'_, MainWindow>,
) -> Div {
    let p = pal(cx);
    let feed = window.store.read(cx).machines();
    let mut body = div()
        .flex()
        .flex_col()
        .gap(px(18.))
        .px(px(24.))
        .py(px(18.))
        .max_w(px(820.));
    match feed.page() {
        None => {
            let problem = feed.problem().map(str::to_owned);
            body = body.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .text_color(p.muted)
                    .when(problem.is_none(), |d| {
                        d.child(Spinner::new().color(p.muted))
                    })
                    .child(problem.unwrap_or_else(|| "Looking at your machines…".to_owned())),
            );
        }
        Some(page) => {
            if page.machines.is_empty() {
                body = body.child(
                    div()
                        .text_color(p.muted)
                        .child("No machines have reported yet."),
                );
            }
            for machine in &page.machines {
                body = body.child(card(&p, machine));
            }
            if let Some(problem) = feed.problem() {
                body = body.child(
                    div()
                        .text_color(p.danger)
                        .text_size(px(12.))
                        .child(problem.to_owned()),
                );
            }
        }
    }
    body = body.child(super::working::jobs::link_row(&p));
    div()
        .size_full()
        .flex()
        .flex_col()
        .child(page_head(&p, "Machines", Some("live, every 10 s".into())))
        .child(scroller("machines-scroll", scroll).child(body))
}

fn state_color(p: &Pal, state: &MachineState) -> Hsla {
    match state {
        MachineState::Ok => p.success,
        MachineState::Warn => p.warn,
        MachineState::Low | MachineState::Offline => p.danger,
        // Stale, unknown, and a state this app doesn't know yet.
        _ => p.faint,
    }
}

fn state_word(state: &MachineState) -> &'static str {
    match state {
        MachineState::Ok => "OK",
        MachineState::Warn => "Warning",
        MachineState::Low => "Low",
        MachineState::Stale => "Stale",
        MachineState::Offline => "Offline",
        // Unknown, and a state this app doesn't know yet.
        _ => "Unknown",
    }
}

/// A machine's card.
fn card(p: &Pal, machine: &Machine) -> Div {
    let color = state_color(p, &machine.state);
    let kind = if machine.platform.as_deref() == Some("darwin") {
        IconName::Frame
    } else {
        IconName::HardDrive
    };
    let skip = (machine.placeable == Some(false)).then(|| {
        format!(
            "Not taking new sessions: {}",
            machine.skip_why.as_deref().unwrap_or("it isn't reachable")
        )
    });
    let mut card = div()
        .flex()
        .flex_col()
        .gap(px(8.))
        .p(px(16.))
        .rounded(px(12.))
        .border_1()
        .border_color(p.border)
        .bg(p.card)
        .child(
            div()
                .flex()
                .items_start()
                .gap(px(12.))
                .child(div().pt(px(2.)).child(icon(kind, color, 22.)))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(3.))
                        .flex_1()
                        .child(
                            div()
                                .flex()
                                .items_baseline()
                                .gap(px(6.))
                                .child(
                                    div()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_size(px(14.))
                                        .child(machine.name.clone()),
                                )
                                .when_some(machine.host.clone(), |d, host| {
                                    d.child(
                                        div().text_size(px(11.5)).text_color(p.muted).child(host),
                                    )
                                }),
                        )
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(5.))
                                .child(dot(color, 8.))
                                .child(
                                    div()
                                        .text_size(px(12.5))
                                        .text_color(p.muted)
                                        .child(machine.status.clone()),
                                ),
                        )
                        .when_some(skip, |d, skip| {
                            d.child(div().text_size(px(11.5)).text_color(p.danger).child(skip))
                        }),
                )
                .child(pill(state_word(&machine.state), color.opacity(0.15), color)),
        );
    for warning in &machine.warnings {
        let tone = if warning.level == WarningLevel::Warn {
            p.warn
        } else {
            p.danger
        };
        card = card.child(
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .text_size(px(12.5))
                .text_color(tone)
                .child(icon(IconName::TriangleAlert, tone, 13.))
                .child(warning.text.clone()),
        );
    }
    card = bars::resources(p, machine, card);
    if machine
        .history
        .as_ref()
        .is_some_and(|history| history.len() > 1)
    {
        card = card.child(history::trend(p, machine));
    }
    history::sessions(p, machine, card)
}
