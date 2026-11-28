//! Huts: each computer the assistant works on (a Mac, a Linux PC, a server) as a card with its
//! kind, whether it is online or asleep and since when, its sessions and resources, and its
//! accounts with the room each has left. Its processor, memory, graphics, disks, battery,
//! session runner, last 24 hours and sessions fold under it. "Add a hut" opens the pairing
//! code in Settings › Devices.

use emberhut_api::machines::{Machine, WarningLevel};
use emberhut_api::runner::accounts::ClaudeAccount;
use gpui_kit::assets::IconName;
use gpui_kit::component::button::Button;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{Icon, Sizable as _};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, Div, ElementId, Entity, FontWeight, Hsla, Role, ScrollHandle,
    TestSupportExt as _, Window, div, px, relative,
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
use super::palette::{self, Pal};
use super::widgets::{dot, page_head, scroller};
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
            body = body.child(add_row(p));
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
    let updated = feed
        .fetched()
        .map(|fetched| words::updated_line(fetched, now.0));
    div()
        .size_full()
        .flex()
        .flex_col()
        .child(page_head(p, "Huts", updated))
        .child(scroller("machines-scroll", scroll).child(body))
}

/// What "Add a hut" does, from the button and the dashed row alike: opens the pairing code
/// (Settings › Devices) over this page; Back returns here.
fn add_hut(_: &gpui_kit::ClickEvent, window: &mut Window, cx: &mut App) {
    column::push(Page::SettingsPane(SettingsPane::Devices), window, cx);
}

/// The line over the cards: how many huts and how many asleep, and "Add a hut".
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
                .on_click(add_hut),
        )
}

/// A quiet dashed row after the last card that adds a hut too.
fn add_row(p: &Pal) -> impl IntoElement {
    let ring = p.accent;
    div()
        .id("huts-add-row")
        .button_named("Add a hut")
        .hint("Pair another computer with a code or QR code")
        .flex()
        .items_center()
        .justify_center()
        .gap(px(6.))
        .py(px(12.))
        .rounded(px(12.))
        .border_1()
        .border_dashed()
        .border_color(p.border)
        .text_size(px(12.5))
        .text_color(p.muted)
        .cursor_pointer()
        .hover(|d| d.bg(p.hover))
        .tab_index(0)
        .focus_visible(move |style| style.border_color(ring))
        .child(
            Icon::new(IconName::CirclePlus)
                .size(px(14.))
                .text_color(p.muted),
        )
        .child("Add a hut")
        .on_click(add_hut)
        .test_support()
}

/// A hut's card: its head, then its sessions and resources, its accounts, and its details
/// folded, each group under a full-width line.
fn card(
    store: &Entity<Store>,
    machine: &Machine,
    accounts: &[&ClaudeAccount],
    open: bool,
    (now, zone): &(Timestamp, TimeZone),
    p: &Pal,
) -> impl IntoElement {
    let presence = Presence::of(&machine.state);
    let asleep = presence == Presence::Asleep;
    let pill = words::pill_line(machine, *now);
    let mut rows: Vec<AnyElement> =
        vec![fact_row(p, "Sessions", &words::sessions_line(machine), false).into_any_element()];
    if let Some(line) = words::resources_line(machine) {
        // An asleep hut's figures are its last ones: dimmed, with their age.
        let line = match words::as_of(machine, *now) {
            Some(age) => format!("{line} · {age}"),
            None => line,
        };
        rows.push(fact_row(p, "Resources", &line, asleep).into_any_element());
    }
    // Red is for a hut that should be there: an asleep one's head says it quietly instead.
    if matches!(presence, Presence::Online | Presence::Quiet) {
        if machine.placeable == Some(false) {
            let why = machine.skip_why.as_deref().unwrap_or("it isn't reachable");
            rows.push(note_row(
                p.danger,
                format!("Not taking new sessions: {why}"),
            ));
        }
        for warning in &machine.warnings {
            let tone = if warning.level == WarningLevel::Warn {
                p.warn
            } else {
                p.danger
            };
            rows.push(note_row(tone, warning.text.clone()));
        }
    }
    let mut card = div()
        .id(ElementId::Name(format!("hut-{}", machine.id).into()))
        .read_as(
            Role::Group,
            a11y::line(&[&machine.name, HutKind::of(machine).word(), &pill]),
        )
        .flex()
        .flex_col()
        .rounded(px(12.))
        .border_1()
        .border_color(p.border)
        .bg(p.card)
        .overflow_hidden()
        .child(head(machine, &pill, p))
        .child(group(p, "Now", rows))
        .child(accounts_group(machine, accounts, (*now, zone, asleep), p))
        .child(details_fold(store, machine, open, p));
    if open {
        let mut details = div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .px(px(16.))
            .pt(px(4.))
            .pb(px(14.));
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

/// The card's head: the kind's icon on a neutral tile, the name, the kind and host name (and
/// on an asleep hut, that it takes no new sessions until it wakes), and the pill.
fn head(machine: &Machine, pill_text: &str, p: &Pal) -> Div {
    let kind = HutKind::of(machine);
    let presence = Presence::of(&machine.state);
    let (tile, _) = palette::track(p.muted, p.card);
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
                .bg(tile)
                .child(
                    Icon::new(kind.icon(machine.battery.is_some()))
                        .size(px(20.))
                        .text_color(p.muted),
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
                )
                .when(presence == Presence::Asleep, |d| {
                    d.child(
                        div()
                            .truncate()
                            .text_size(px(12.))
                            .text_color(p.muted)
                            .child(words::ASLEEP_NOTE),
                    )
                }),
        )
        .child(presence_pill(presence, pill_text, p))
}

/// Whether the hut is there, once: a dot and `Online` or `Asleep · 2 h ago`, its text dark
/// enough to read on its tint.
fn presence_pill(presence: Presence, text: &str, p: &Pal) -> Div {
    let ink = match presence {
        Presence::Online => p.success,
        Presence::Quiet => p.warn,
        Presence::Asleep | Presence::NotYet => p.muted,
    };
    let (fill, ink) = palette::tinted(ink, p.card);
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(5.))
        .px(px(8.))
        .py(px(2.))
        .rounded_full()
        .bg(fill)
        .text_color(ink)
        .text_size(px(11.5))
        .font_weight(FontWeight::SEMIBOLD)
        .child(dot(ink, 6.))
        .child(text.to_owned())
}

/// A group of rows under a full-width line, with its label in the sidebar's style; the rows
/// part with inset lines.
fn group(p: &Pal, label: &'static str, rows: Vec<AnyElement>) -> Div {
    let mut group = div()
        .flex()
        .flex_col()
        .pt(px(10.))
        .pb(px(4.))
        .border_t_1()
        .border_color(p.border)
        .child(
            div()
                .px(px(16.))
                .pb(px(2.))
                .text_size(px(12.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(p.muted)
                .child(label),
        );
    for (i, row) in rows.into_iter().enumerate() {
        group = group.child(
            div()
                .mx(px(16.))
                .when(i > 0, |d| d.border_t_1().border_color(p.border))
                .child(row),
        );
    }
    group
}

/// A fact: its label on the left, its value on the right; `dim` for figures that are old.
fn fact_row(p: &Pal, label: &'static str, value: &str, dim: bool) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(10.))
        .py(px(8.))
        .text_size(px(12.5))
        .child(div().flex_none().text_color(p.muted).child(label))
        .child(div().flex_1())
        .child(
            div()
                .truncate()
                .text_right()
                .text_color(if dim { p.muted } else { p.text })
                .child(value.to_owned()),
        )
}

/// A warning or a refusal, in its colour.
fn note_row(tone: Hsla, text: String) -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap(px(6.))
        .py(px(8.))
        .text_size(px(12.))
        .text_color(tone)
        .child(
            Icon::new(IconName::TriangleAlert)
                .size(px(12.))
                .text_color(tone),
        )
        .child(text)
        .into_any_element()
}

/// The hut's accounts, each with the room it has left; or that none is kept here.
fn accounts_group(
    machine: &Machine,
    accounts: &[&ClaudeAccount],
    (now, zone, asleep): (Timestamp, &TimeZone, bool),
    p: &Pal,
) -> Div {
    let rows = if accounts.is_empty() {
        vec![
            div()
                .py(px(8.))
                .text_size(px(12.5))
                .text_color(p.muted)
                .child("No assistant accounts on this hut")
                .into_any_element(),
        ]
    } else {
        accounts
            .iter()
            .map(|account| {
                let room = words::room(account, now, zone, asleep);
                account_row(machine, account, &room, p).into_any_element()
            })
            .collect()
    };
    group(p, "Accounts", rows)
}

/// The colour of an account's room: its bar's, and its words' (taken dark enough to read).
fn room_color(p: &Pal, tone: RoomTone) -> Hsla {
    match tone {
        RoomTone::Fine => p.success,
        RoomTone::Low => p.warn,
        RoomTone::Out => p.danger,
        RoomTone::Unknown => p.muted,
    }
}

/// One account: its name and address, the room it has left in its colour with a bar, and its
/// 5-hour and week room under it.
fn account_row(
    machine: &Machine,
    account: &ClaudeAccount,
    room: &Room,
    p: &Pal,
) -> impl IntoElement {
    let color = room_color(p, room.tone);
    let (_, words_color) = palette::tinted(color, p.card);
    let (track, outline) = palette::track(p.muted, p.card);
    let said = a11y::line(&[&account.name, &room.value, &room.detail]);
    div()
        .id(ElementId::Name(
            format!("hut-account-{}-{}", machine.id, account.id).into(),
        ))
        .read_as(Role::ListItem, said)
        .flex()
        .flex_col()
        .gap(px(5.))
        .py(px(8.))
        .child(
            div()
                .flex()
                .items_baseline()
                .gap(px(8.))
                .child(
                    div()
                        .flex_none()
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
                            .text_color(p.muted)
                            .child(email),
                    )
                })
                .child(div().flex_1())
                .child(
                    div()
                        .flex_none()
                        .text_size(px(12.5))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(words_color)
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
                    .h(px(6.))
                    .w_full()
                    .rounded_full()
                    .border_1()
                    .border_color(outline)
                    .bg(track)
                    .overflow_hidden()
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
/// sessions on it; the whole row is the button.
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
            .id(ElementId::Name(
                format!("hut-details-{}", machine.id).into(),
            ))
            .button_named(format!("Details of {}", machine.name))
            .aria_expanded(open)
            .aria_value(a11y::folded(open))
            .flex()
            .items_center()
            .gap(px(8.))
            .px(px(16.))
            .py(px(10.))
            .border_1()
            .cursor_pointer()
            .hover(|d| d.bg(p.hover))
            .child(Icon::new(chevron).size(px(12.)).text_color(p.muted))
            .child(
                div()
                    .text_size(px(12.5))
                    .font_weight(FontWeight::MEDIUM)
                    .child("Details"),
            )
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(p.muted)
                    .child("· charts, last 24 h"),
            )
            .on_click(move |_, _, cx| store.update(cx, |store, cx| store.toggle_hut(&id, cx))),
        p,
    )
    .test_support();
    div().border_t_1().border_color(p.border).child(fold)
}
