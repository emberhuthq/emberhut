//! Huts: each computer the assistant works on (a Mac, a Linux PC, a server) as a card with its
//! kind, whether it is online or asleep and when it was last seen, what runs there now, and
//! its accounts with the room each has left. Its processor, memory, graphics, disks, battery,
//! session runner, last 24 hours and sessions fold under it. "Add a hut" opens the pairing
//! code in Settings › Devices.

use emberhut_api::machines::{Machine, MachineState, WarningLevel};
use emberhut_api::runner::accounts::ClaudeAccount;
use gpui_kit::assets::IconName;
use gpui_kit::component::button::Button;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{Icon, Sizable as _};
use gpui_kit::prelude::*;
use gpui_kit::{
    App, Div, Entity, FontWeight, Hsla, Role, ScrollHandle, TestSupportExt as _, div, px, relative,
};
use jiff::Timestamp;
use jiff::tz::TimeZone;

mod bars;
mod history;
mod series;
pub(crate) mod summary;
#[cfg(test)]
mod tests;
pub(crate) mod words;

use self::words::{HutKind, Presence, Room, RoomTone};
use super::a11y::{self, Accessible as _};
use super::column;
use super::controls::tab_stop;
use super::palette::Pal;
use super::widgets::{dot, page_head, pill, scroller};
use crate::store::settings::SettingsPane;
use crate::store::{Page, Store};

/// The page.
pub(crate) fn page(store: &Entity<Store>, scroll: &ScrollHandle, p: &Pal, cx: &App) -> Div {
    let read = store.read(cx);
    let feed = read.machines();
    let accounts = read
        .state()
        .map_or(&[][..], |state| state.accounts.as_slice());
    let now = (Timestamp::now(), TimeZone::system());
    let mut body = div()
        .flex()
        .flex_col()
        .gap(px(14.))
        .px(px(24.))
        .py(px(18.))
        .max_w(px(820.));
    body = body.child(top_row(
        feed.page().map(|page| words::count_line(&page.machines)),
        p,
    ));
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
                    .child(problem.unwrap_or_else(|| "Looking at your huts…".to_owned())),
            );
        }
        Some(page) => {
            if page.machines.is_empty() {
                body = body.child(
                    div()
                        .text_color(p.muted)
                        .child("No huts have reported yet."),
                );
            }
            for machine in &page.machines {
                let on_it = words::accounts_on(accounts, machine);
                let open = feed.is_open(&machine.id);
                body = body.child(card(store, machine, &on_it, open, &now, p));
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
    body = body.child(super::working::jobs::link_row(p));
    div()
        .size_full()
        .flex()
        .flex_col()
        .child(page_head(p, "Huts", Some("live, every 10 s".into())))
        .child(scroller("machines-scroll", scroll).child(body))
}

/// The line over the cards: how many huts and how many asleep, and "Add a hut", which opens
/// the pairing code (Settings › Devices) over this page: Back returns here.
fn top_row(count: Option<String>, p: &Pal) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(10.))
        .child(
            div()
                .flex_1()
                .text_size(px(12.5))
                .text_color(p.muted)
                .child(count.unwrap_or_default()),
        )
        .child(
            Button::new("huts-add")
                .small()
                .icon(gpui_kit::component::IconName::Plus)
                .label("Add a hut")
                .tooltip("Pair another computer with a code or QR code")
                .on_click(|_, window, cx| {
                    column::push(Page::SettingsPane(SettingsPane::Devices), window, cx);
                }),
        )
}

/// The dot's colour for `state`: how the hut is, beside whether it is there.
fn state_color(p: &Pal, state: &MachineState) -> Hsla {
    match state {
        MachineState::Ok => p.success,
        MachineState::Warn | MachineState::Stale => p.warn,
        MachineState::Low => p.danger,
        // Asleep, unknown, and a state this app doesn't know yet.
        _ => p.faint,
    }
}

/// A hut's card: its head, its facts as rows, its accounts, and its details folded.
fn card(
    store: &Entity<Store>,
    machine: &Machine,
    accounts: &[&ClaudeAccount],
    open: bool,
    (now, zone): &(Timestamp, TimeZone),
    p: &Pal,
) -> impl IntoElement {
    let presence = words::presence_line(machine, *now);
    let running = words::running_line(machine);
    let mut card = div()
        .id(gpui_kit::ElementId::Name(
            format!("hut-{}", machine.id).into(),
        ))
        .read_as(
            Role::Group,
            a11y::line(&[&machine.name, HutKind::of(machine).word(), &presence]),
        )
        .flex()
        .flex_col()
        .rounded(px(12.))
        .border_1()
        .border_color(p.border)
        .bg(p.card)
        .overflow_hidden()
        .child(head(machine, p))
        .child(fact_row(
            p,
            "Status",
            &presence,
            Some(state_color(p, &machine.state)),
        ))
        .child(fact_row(p, "Running now", &running, None))
        .when_some(words::resources_line(machine), |d, line| {
            d.child(fact_row(p, "Resources", &line, None))
        });
    let skip = (machine.placeable == Some(false)).then(|| {
        format!(
            "Not taking new sessions: {}",
            machine.skip_why.as_deref().unwrap_or("it isn't reachable")
        )
    });
    let mut notes = div().flex().flex_col().gap(px(4.)).px(px(16.));
    let has_notes = skip.is_some() || !machine.warnings.is_empty();
    if let Some(skip) = skip {
        notes = notes.child(div().text_size(px(12.)).text_color(p.danger).child(skip));
    }
    for warning in &machine.warnings {
        let tone = if warning.level == WarningLevel::Warn {
            p.warn
        } else {
            p.danger
        };
        notes = notes.child(
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .text_size(px(12.))
                .text_color(tone)
                .child(
                    Icon::new(IconName::TriangleAlert)
                        .size(px(12.))
                        .text_color(tone),
                )
                .child(warning.text.clone()),
        );
    }
    if has_notes {
        card = card.child(notes.py(px(8.)).border_t_1().border_color(p.border));
    }
    card = card.child(accounts_block(machine, accounts, (*now, zone), p));
    card = card.child(details_fold(store, machine, open, p));
    if open {
        let mut details = div().flex().flex_col().gap(px(8.)).px(px(16.)).pb(px(14.));
        details = bars::resources(p, machine, details);
        if machine
            .history
            .as_ref()
            .is_some_and(|history| history.len() > 1)
        {
            details = details.child(history::trend(p, machine));
        }
        card = card.child(history::sessions(p, machine, details));
    }
    card.test_support()
}

/// The card's head: the kind's icon on a tinted tile, the name, the kind and host name, and
/// whether it is online or asleep.
fn head(machine: &Machine, p: &Pal) -> Div {
    let kind = HutKind::of(machine);
    let presence = Presence::of(&machine.state);
    let awake = matches!(presence, Presence::Online | Presence::Quiet);
    let tint = if awake { p.accent } else { p.faint };
    let (pill_fill, pill_text) = match presence {
        Presence::Online => (p.success.opacity(0.15), p.success),
        Presence::Quiet => (p.warn.opacity(0.15), p.warn),
        Presence::Asleep | Presence::NotYet => (p.faint.opacity(0.15), p.muted),
    };
    let subtitle = match &machine.host {
        Some(host) if *host != machine.name => format!("{} · {host}", kind.word()),
        _ => kind.word().to_owned(),
    };
    div()
        .flex()
        .items_center()
        .gap(px(12.))
        .px(px(16.))
        .pt(px(14.))
        .pb(px(12.))
        .child(
            div()
                .size(px(38.))
                .rounded(px(9.))
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .bg(tint.opacity(0.14))
                .child(
                    Icon::new(kind.icon(machine.battery.is_some()))
                        .size(px(20.))
                        .text_color(tint),
                ),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(2.))
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .truncate()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_size(px(14.5))
                        .child(machine.name.clone()),
                )
                .child(
                    div()
                        .truncate()
                        .text_size(px(12.))
                        .text_color(p.muted)
                        .child(subtitle),
                ),
        )
        .child(pill(presence.word(), pill_fill, pill_text))
}

/// A fact as a grouped list's row: its label on the left, its value on the right, after a
/// hairline; `dot` beside the value in its colour.
fn fact_row(p: &Pal, label: &'static str, value: &str, dot_color: Option<Hsla>) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(10.))
        .mx(px(16.))
        .py(px(8.))
        .border_t_1()
        .border_color(p.border)
        .text_size(px(12.5))
        .child(div().flex_none().text_color(p.muted).child(label))
        .child(div().flex_1())
        .when_some(dot_color, |d, color| d.child(dot(color, 7.)))
        .child(div().truncate().text_right().child(value.to_owned()))
}

/// The hut's accounts, each with the room it has left; or that none is kept here.
fn accounts_block(
    machine: &Machine,
    accounts: &[&ClaudeAccount],
    (now, zone): (Timestamp, &TimeZone),
    p: &Pal,
) -> Div {
    let mut block = div()
        .flex()
        .flex_col()
        .px(px(16.))
        .pt(px(10.))
        .pb(px(8.))
        .border_t_1()
        .border_color(p.border)
        .child(
            div()
                .text_size(px(11.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(p.muted)
                .pb(px(4.))
                .child("ACCOUNTS"),
        );
    if accounts.is_empty() {
        return block.child(
            div()
                .text_size(px(12.5))
                .text_color(p.faint)
                .py(px(4.))
                .child("No assistant accounts on this hut"),
        );
    }
    for account in accounts {
        let room = words::room(account, now, zone);
        block = block.child(account_row(machine, account, &room, p));
    }
    block
}

/// The colour of an account's room.
fn room_color(p: &Pal, tone: RoomTone) -> Hsla {
    match tone {
        RoomTone::Fine => p.success,
        RoomTone::Low => p.warn,
        RoomTone::Out => p.danger,
        RoomTone::Unknown => p.faint,
    }
}

/// One account: its name and address, the room it has left with a thin bar, and its 5-hour
/// and week use under it.
fn account_row(
    machine: &Machine,
    account: &ClaudeAccount,
    room: &Room,
    p: &Pal,
) -> impl IntoElement {
    let color = room_color(p, room.tone);
    let value_color = if room.tone == RoomTone::Unknown {
        p.muted
    } else {
        p.text
    };
    let said = a11y::line(&[&account.name, &room.value, &room.detail]);
    div()
        .id(gpui_kit::ElementId::Name(
            format!("hut-account-{}-{}", machine.id, account.id).into(),
        ))
        .read_as(Role::ListItem, said)
        .flex()
        .flex_col()
        .gap(px(4.))
        .py(px(6.))
        .child(
            div()
                .flex()
                .items_baseline()
                .gap(px(8.))
                .child(
                    div()
                        .text_size(px(13.))
                        .font_weight(FontWeight::MEDIUM)
                        .child(account.name.clone()),
                )
                .when_some(account.email.clone(), |d, email| {
                    d.child(
                        div()
                            .truncate()
                            .min_w_0()
                            .text_size(px(11.5))
                            .text_color(p.faint)
                            .child(email),
                    )
                })
                .child(div().flex_1())
                .child(
                    div()
                        .flex_none()
                        .text_size(px(12.5))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(value_color)
                        .child(room.value.clone()),
                ),
        )
        .when_some(room.share, |d, share| {
            #[expect(
                clippy::cast_possible_truncation,
                reason = "a share from 0 to 1, as a width fraction"
            )]
            let width = share.clamp(0., 1.) as f32;
            d.child(
                div()
                    .h(px(4.))
                    .w_full()
                    .rounded_full()
                    .bg(p.faint.opacity(0.18))
                    .child(div().h_full().w(relative(width)).rounded_full().bg(color)),
            )
        })
        .when(!room.detail.is_empty(), |d| {
            d.child(
                div()
                    .text_size(px(11.5))
                    .text_color(p.muted)
                    .child(room.detail.clone()),
            )
        })
        .test_support()
}

/// The fold over a hut's details: processor, memory, graphics, disks, its last day and the
/// sessions on it.
fn details_fold(store: &Entity<Store>, machine: &Machine, open: bool, p: &Pal) -> impl IntoElement {
    let store = store.clone();
    let id = machine.id.clone();
    let chevron = if open {
        gpui_kit::component::IconName::ChevronDown
    } else {
        gpui_kit::component::IconName::ChevronRight
    };
    let fold = tab_stop(
        div()
            .id(gpui_kit::ElementId::Name(
                format!("hut-details-{}", machine.id).into(),
            ))
            .button_named(format!("Details of {}", machine.name))
            .aria_expanded(open)
            .aria_value(a11y::folded(open))
            .flex()
            .items_center()
            .gap(px(8.))
            .px(px(8.))
            .py(px(6.))
            .border_1()
            .rounded(px(8.))
            .cursor_pointer()
            .hover(|d| d.bg(p.hover))
            .child(Icon::new(chevron).size(px(12.)).text_color(p.faint))
            .child(
                div()
                    .text_size(px(12.5))
                    .font_weight(FontWeight::MEDIUM)
                    .child("Details"),
            )
            .child(
                div()
                    .truncate()
                    .text_size(px(11.5))
                    .text_color(p.muted)
                    .child("CPU, memory, GPUs, disks, last 24 h, sessions"),
            )
            .on_click(move |_, _, cx| store.update(cx, |store, cx| store.toggle_hut(&id, cx))),
        p,
    )
    .test_support();
    div()
        .px(px(8.))
        .py(px(4.))
        .border_t_1()
        .border_color(p.border)
        .child(fold)
}
