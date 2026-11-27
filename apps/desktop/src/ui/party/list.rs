//! The user's parties, the one changed last first: each with its cast as coloured initials,
//! its title and cost, its state and cast, and its last line; a click opens it, a right-click
//! deletes it. With none yet, what Party is and a New party button; while the first read is
//! on its way, "Looking…".

use emberhut_api::party::Party;
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{ContextMenuExt as _, PopupMenu, PopupMenuItem};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{Icon, Sizable as _};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, Div, ElementId, Entity, FontWeight, div, px};

use super::look::{cast_dots, status_color};
use super::{NewParty, PartyWindow};
use crate::prose::direction;
use crate::store::Store;
use crate::store::party::words;
use crate::ui::a11y::{self, Accessible as _};
use crate::ui::controls::tab_stop;
use crate::ui::palette::Pal;
use crate::ui::widgets::{dot, scroller};

/// The list's page.
pub(super) fn page(view: &PartyWindow, p: &Pal, cx: &mut Context<'_, PartyWindow>) -> Div {
    let mut body = div().flex().flex_col().gap(px(4.)).px(px(16.)).py(px(12.));
    let none = {
        let roster = view.store.read(cx).party().roster();
        roster.is_loaded() && roster.parties().is_empty()
    };
    if none {
        body = body.child(empty(p, cx));
    }
    let head = head(p, cx);
    let roster = view.store.read(cx).party().roster();
    for party in roster.parties() {
        body = body.child(row(party, &view.store, p));
    }
    if let Some(problem) = roster.problem() {
        body = body.child(
            div()
                .id("party-list-problem")
                .announced(problem.to_owned())
                .pt(px(8.))
                .text_size(px(12.))
                .text_color(p.danger)
                .child(problem.to_owned()),
        );
    }
    if !roster.is_loaded() && roster.problem().is_none() {
        body = body.child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .py(px(8.))
                .text_color(p.muted)
                .child(Spinner::new().small().color(p.muted))
                .child("Looking…"),
        );
    }
    div()
        .flex()
        .flex_col()
        .size_full()
        .child(head)
        .child(scroller("party-list", &view.list_scroll).child(body))
}

/// The bar: the title, and + for a new party.
fn head(p: &Pal, cx: &mut Context<'_, PartyWindow>) -> Div {
    div()
        .flex()
        .items_center()
        .h(px(44.))
        .px(px(16.))
        .border_b_1()
        .border_color(p.border)
        .child(
            div()
                .flex_1()
                .font_weight(FontWeight::SEMIBOLD)
                .text_size(px(15.))
                .child("Party"),
        )
        .child(
            Button::new("party-new")
                .ghost()
                .small()
                .icon(Icon::new(IconName::Plus))
                .accessibility_label("New party")
                .tooltip_with_action("New party", &NewParty, Some(super::CONTEXT))
                .on_click(cx.listener(|this, _, window, cx| this.new_party(window, cx))),
        )
}

/// What Party is, with a New party button.
fn empty(p: &Pal, cx: &mut Context<'_, PartyWindow>) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(10.))
        .py(px(8.))
        .child(
            div()
                .font_weight(FontWeight::SEMIBOLD)
                .child("AI personas talking to each other"),
        )
        .child(div().text_size(px(13.)).text_color(p.muted).child(
            "Pick 2 to 5 personas, give them a topic, and watch them talk it out. Jump in, \
             steer them or stop them any time.",
        ))
        .child(
            div().pt(px(4.)).child(
                Button::new("party-new-empty")
                    .primary()
                    .icon(Icon::new(IconName::Plus))
                    .label("New party")
                    .on_click(cx.listener(|this, _, window, cx| this.new_party(window, cx))),
            ),
        )
}

/// A party's row: a click opens it, a right-click offers Delete.
fn row(party: &Party, store: &Entity<Store>, p: &Pal) -> AnyElement {
    let id = party.id;
    let line = div()
        .flex()
        .items_center()
        .gap(px(5.))
        .text_size(px(12.))
        .text_color(p.muted)
        .child(dot(status_color(&party.status, p), 7.))
        .child(div().min_w_0().truncate().child(format!(
            "{} · {}",
            words::status_word(&party.status),
            words::cast(party)
        )));
    let text = div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .gap(px(3.))
        .child(
            div()
                .flex()
                .items_baseline()
                .gap(px(6.))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .font_weight(FontWeight::MEDIUM)
                        .child(direction::marked(&party.title)),
                )
                .child(
                    div()
                        .flex_none()
                        .text_size(px(12.))
                        .text_color(p.muted)
                        .child(words::cost(party.cost)),
                ),
        )
        .child(line)
        .children(party.last.clone().map(|last| {
            div()
                .text_size(px(13.))
                .text_color(p.muted)
                .line_clamp(2)
                .when(crate::ui::reads::starts_right(&last), Styled::text_right)
                .child(last)
        }));
    let (open, delete) = (store.clone(), store.clone());
    // One line for the row: its title, its state and cast, its cost.
    let spoken = a11y::line(&[
        &party.title,
        &words::status_word(&party.status),
        &words::cast(party),
        &words::cost(party.cost),
    ]);
    let row = div()
        .id(ElementId::Name(format!("party-{id}").into()))
        .button_named(spoken)
        .flex()
        .items_start()
        .gap(px(10.))
        .px(px(8.))
        .py(px(7.))
        .rounded(px(8.))
        .border_1()
        .cursor_pointer()
        .hover(|d| d.bg(p.hover))
        .child(div().pt(px(3.)).child(cast_dots(&party.personas, p)))
        .child(text)
        .on_click(move |_, _, cx| open.update(cx, |store, cx| store.open_party(Some(id), cx)));
    tab_stop(row, p)
        .context_menu(move |menu: PopupMenu, _, _| {
            let delete = delete.clone();
            menu.item(
                PopupMenuItem::new("Delete")
                    .icon(Icon::new(IconName::Trash))
                    .on_click(move |_, _, cx| {
                        delete.update(cx, |store, cx| store.delete_party(id, cx));
                    }),
            )
        })
        .into_any_element()
}
