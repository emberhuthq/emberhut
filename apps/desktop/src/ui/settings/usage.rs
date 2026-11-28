//! Settings' Claude usage: every account once, so the user sees at a glance which one has
//! room. The one with the most room on top, the account the assistant runs on (Auto, or one
//! the user pins), then a row per account: its name and tags, its 5-hour and weekly bars with
//! when each resets, or why it has none. A click opens its details. Under them, when the
//! numbers were checked, and Check now.

use emberhut_api::usage::{AssistantAccount, UsageRow};
use gpui_kit::assets::IconName;
use gpui_kit::component::button::Button;
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::{Disableable as _, Icon, Sizable as _};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, Div, ElementId, Entity, FontWeight, Hsla, div, px, relative};
use jiff::Timestamp;
use jiff::tz::TimeZone;

use super::parts::{note, section, tag};
use crate::store::Store;
use crate::store::working::usage::{
    Fill, RowLine, checked, choices, details, fill, most_room, percent, picked, resets, row_line,
    runs_on,
};
use crate::ui::a11y::{self, Accessible as _};
use crate::ui::controls::tab_stop;
use crate::ui::palette::Pal;
use crate::ui::widgets::share;

/// The section.
pub(super) fn card(store: &Entity<Store>, p: &Pal, cx: &App) -> Div {
    let read = store.read(cx);
    let rows = read.state().map_or(&[][..], |state| state.usage.as_slice());
    if rows.is_empty() {
        let empty = note(p, "Usage shows after the server's next update.").into_any_element();
        return section(p, "Claude usage", None, vec![empty], None);
    }
    let (now, zone) = (Timestamp::now(), TimeZone::system());
    let account = read
        .state()
        .and_then(|state| state.assistant_account.as_ref());
    let mut parts: Vec<AnyElement> = Vec::new();
    if let Some(best) = rows.iter().find(|row| row.best) {
        parts.push(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(
                    Icon::new(IconName::CircleCheck)
                        .size(px(15.))
                        .text_color(p.success),
                )
                .child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_size(px(13.5))
                        .child(most_room(account, best)),
                )
                .into_any_element(),
        );
    }
    if let Some(account) = account {
        parts.push(runs_on_row(store, account, rows, p, cx).into_any_element());
    }
    let on = account.and_then(|account| account.on.as_deref());
    for row in rows {
        let open = read.usage_card().is_open(&row.id);
        parts.push(account_row(store, row, on, open, (now, &zone), p).into_any_element());
    }
    let store = store.clone();
    parts.push(
        div()
            .flex()
            .items_center()
            .gap(px(8.))
            .child(note(p, checked(rows, now).unwrap_or_default()).flex_1())
            .child(
                Button::new("usage-check-now")
                    .small()
                    .label("Check now")
                    .on_click(move |_, _, cx| store.read(cx).check_usage_now()),
            )
            .into_any_element(),
    );
    section(p, "Claude usage", None, parts, None)
}

/// Which account the assistant runs on, and the user's choice: Auto, or one pinned.
fn runs_on_row(
    store: &Entity<Store>,
    account: &AssistantAccount,
    rows: &[UsageRow],
    p: &Pal,
    cx: &App,
) -> Div {
    let (line, out) = runs_on(account, rows);
    let options = choices(rows);
    let picking = store.read(cx).usage_card().is_picking();
    let store = store.clone();
    div()
        .flex()
        .flex_col()
        .gap(px(4.))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(
                    div()
                        .flex_1()
                        .text_size(px(13.))
                        .child(format!("{} runs on", store.read(cx).ember_name())),
                )
                .child(
                    Button::new("usage-runs-on")
                        .small()
                        .label(picked(account, rows))
                        .dropdown_caret(true)
                        .disabled(picking)
                        .dropdown_menu(move |mut menu, _, _| {
                            for (name, request) in &options {
                                let (store, request) = (store.clone(), request.clone());
                                menu = menu.item(PopupMenuItem::new(name.clone()).on_click(
                                    move |_, _, cx| {
                                        let request = request.clone();
                                        store.update(cx, |store, cx| {
                                            store.use_account(request, cx);
                                        });
                                    },
                                ));
                            }
                            menu
                        }),
                ),
        )
        .child(
            div()
                .text_size(px(11.5))
                .text_color(if out { p.warn } else { p.muted })
                .child(line),
        )
}

/// One account: its name and tags, its bars or why it has none, its details when open.
fn account_row(
    store: &Entity<Store>,
    row: &UsageRow,
    on: Option<&str>,
    open: bool,
    (now, zone): (Timestamp, &TimeZone),
    p: &Pal,
) -> impl IntoElement {
    let chevron = if open {
        IconName::ChevronDown
    } else {
        IconName::ChevronRight
    };
    let head = div()
        .flex()
        .items_center()
        .gap(px(6.))
        .child(
            div()
                .font_weight(FontWeight::SEMIBOLD)
                .text_size(px(13.))
                .child(row.name.clone()),
        )
        .when(row.best, |d| d.child(tag("most room", p.success)))
        .when(on == Some(row.id.as_str()), |d| {
            d.child(tag("In use", p.muted))
        })
        .child(div().flex_1())
        .child(Icon::new(chevron).size(px(12.)).text_color(p.faint));
    let body: Vec<AnyElement> = match row_line(row, now, zone) {
        RowLine::NotLoggedIn => vec![small("not logged in", p.danger).into_any_element()],
        RowLine::NoData => vec![small("no data", p.muted).into_any_element()],
        RowLine::Bars { out } => out
            .map(|out| {
                small(out, p.danger)
                    .font_weight(FontWeight::SEMIBOLD)
                    .into_any_element()
            })
            .into_iter()
            .chain([
                bar(
                    "5h",
                    row.used_5h,
                    resets(row.resets_5h.as_ref(), now, zone),
                    p,
                )
                .into_any_element(),
                bar(
                    "Week",
                    row.used_7d,
                    resets(row.resets_7d.as_ref(), now, zone),
                    p,
                )
                .into_any_element(),
            ])
            .collect(),
    };
    let spoken = account_words(row, on, (now, zone));
    let lines = open.then(|| {
        div().flex().flex_col().gap(px(3.)).pt(px(2.)).children(
            details(row, now, zone)
                .into_iter()
                .map(|line| small(line, p.muted)),
        )
    });
    let (store, id) = (store.clone(), row.id.clone());
    tab_stop(
        div()
            .id(ElementId::Name(format!("usage-{}", row.id).into()))
            // One line for the account, its details open or folded.
            .button_named(spoken)
            .aria_expanded(open)
            .aria_value(a11y::folded(open))
            .border_1()
            .rounded(px(6.))
            .flex()
            .flex_col()
            .gap(px(6.))
            .cursor_pointer()
            .child(head)
            .children(body)
            .children(lines)
            .on_click(move |_, _, cx| {
                store.update(cx, |store, cx| store.toggle_usage_row(&id, cx));
            }),
        p,
    )
}

/// An account's row as one line: its name and tags, then its windows or why it has none
/// ("Work, most room, 5 hours: 42% used, resets 4 pm, week: 10% used, resets Tue 9 am").
fn account_words(row: &UsageRow, on: Option<&str>, (now, zone): (Timestamp, &TimeZone)) -> String {
    let state = match row_line(row, now, zone) {
        RowLine::NotLoggedIn => "not logged in".to_owned(),
        RowLine::NoData => "no data".to_owned(),
        RowLine::Bars { out } => a11y::line(&[
            out.as_deref().unwrap_or_default(),
            &a11y::usage_window(
                "5 hours",
                row.used_5h,
                &resets(row.resets_5h.as_ref(), now, zone),
            ),
            &a11y::usage_window(
                "week",
                row.used_7d,
                &resets(row.resets_7d.as_ref(), now, zone),
            ),
        ]),
    };
    let theirs = if on == Some(row.id.as_str()) {
        "in use".to_owned()
    } else {
        String::new()
    };
    a11y::line(&[
        &row.name,
        if row.best { "most room" } else { "" },
        &theirs,
        &state,
    ])
}

/// A small line of text in `color`.
fn small(text: impl Into<gpui_kit::SharedString>, color: Hsla) -> Div {
    div()
        .text_size(px(12.))
        .text_color(color)
        .child(text.into())
}

/// A window's bar: its name, a capsule filled to its use in its colour, the percentage, and
/// when it resets.
fn bar(label: &'static str, used: Option<f64>, resets: String, p: &Pal) -> Div {
    let color = match fill(used) {
        Fill::Green => p.success,
        Fill::Amber => p.warn,
        Fill::Red => p.danger,
    };
    let width = share(used.unwrap_or(0.) / 100.);
    let full = used.unwrap_or(0.) > 70.;
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .text_size(px(12.))
        .child(div().w(px(36.)).text_color(p.muted).child(label))
        .child(
            div().flex_1().h(px(7.)).rounded_full().bg(p.border).child(
                div()
                    .h_full()
                    .rounded_full()
                    .bg(color)
                    .w(relative(width))
                    .when(width > 0., |d| d.min_w(px(7.))),
            ),
        )
        .child(
            div()
                .w(px(36.))
                .flex()
                .justify_end()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(if full { color } else { p.text })
                .child(percent(used)),
        )
        .child(
            div()
                .w(px(112.))
                .truncate()
                .text_color(p.muted)
                .child(resets),
        )
}
