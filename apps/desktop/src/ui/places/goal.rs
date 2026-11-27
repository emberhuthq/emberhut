//! A goal's page: its big ring and `n of m done this week`, `x by you · y by Emberhut`, its status
//! and why; To do (the user's with a tick, the assistant's marked; a right-click: Done, Next
//! week, Archive), Done this week with the day; Talk about this goal at the foot.

use emberhut_api::ids::GoalId;
use emberhut_api::plan::Todo;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{ContextMenuExt as _, PopupMenuItem};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, Div, ElementId, Entity, FontWeight, div, px};

use super::ring::ring;
use super::{MainWindow, body, gone, section, title, who};
use crate::brand::APP_NAME;
use crate::prose::direction;
use crate::store::goals::{label, progress};
use crate::store::tasks::clock;
use crate::store::{Store, lookup};
use crate::ui::palette::{Pal, room_color};
use crate::ui::reads;
use crate::ui::tasks::swipe::{RowSwipes, Swiped};
use crate::ui::tasks::tick::tick;

/// The page.
pub(super) fn page(
    window: &mut MainWindow,
    p: &Pal,
    goal: GoalId,
    cx: &Context<'_, MainWindow>,
) -> Div {
    let store_entity = window.store.clone();
    let store = window.store.read(cx);
    let Some(state) = store.state() else {
        return gone(p, "Loading…");
    };
    let Some(shown) = state.goals.iter().find(|g| g.id == goal) else {
        return gone(p, "Goal removed");
    };
    let (color, track) = (room_color(Some(shown.rank), p), p.faint.opacity(0.25));
    let ring_of = progress(goal, &state.todos);
    let todos: Vec<&Todo> = state
        .todos
        .iter()
        .filter(|todo| todo.goal == Some(goal))
        .collect();
    let (done, open): (Vec<&Todo>, Vec<&Todo>) = todos.iter().partition(|todo| todo.is_done());
    let open: Vec<&Todo> = open
        .into_iter()
        .filter(|todo| !store.is_gone(todo.id))
        .collect();
    let headline = if ring_of.total == 0 {
        "No tasks yet".to_owned()
    } else {
        format!("{} of {} done this week", ring_of.done, ring_of.total)
    };
    let mut page = body().child(title(label(shown))).child(
        div()
            .flex()
            .items_center()
            .gap(px(14.))
            .pt(px(8.))
            .child(ring("goal-ring", 52., 7., ring_of.share(), color, track))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(3.))
                    .child(
                        div()
                            .text_size(px(15.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(headline),
                    )
                    .child(div().text_size(px(12.5)).text_color(p.muted).child(format!(
                        "{} by you · {} by {APP_NAME}",
                        shown.week.by(lookup::PERSON_OWNER),
                        shown.week.by(crate::wire::ASSISTANT_OWNER)
                    ))),
            ),
    );
    if let Some(status) = shown.status.as_deref().filter(|s| !s.is_empty()) {
        page = page.child(
            div()
                .pt(px(8.))
                .text_size(px(13.5))
                .when(reads::mostly_right(status), Styled::text_right)
                .child(status.to_owned()),
        );
    }
    page = page
        .child(
            div()
                .text_size(px(12.5))
                .text_color(p.muted)
                .when(reads::mostly_right(&shown.why), Styled::text_right)
                .child(shown.why.clone()),
        )
        .child(section(p, "To do"));
    if open.is_empty() {
        page = page.child(
            div()
                .text_size(px(13.))
                .text_color(p.muted)
                .child(format!("Nothing listed yet. Ask {APP_NAME} what's next.")),
        );
    }
    for todo in open {
        let ticked = store.is_ticked(todo.id);
        page = page.child(todo_row(p, &store_entity, todo, ticked, &window.row_swipes));
    }
    if !done.is_empty() {
        page = page
            .child(section(p, "Done this week"))
            .children(done_rows(p, &done));
    }
    let (talk, about) = (store_entity, format!("About my goal \"{}\": ", shown.title));
    page.child(
        div().pt(px(18.)).child(
            Button::new("goal-talk")
                .label(format!("Talk to {APP_NAME} about this goal"))
                .primary()
                .w_full()
                .on_click(move |_, _, cx| {
                    let about = about.clone();
                    talk.update(cx, |store, cx| store.start_message(about, cx));
                }),
        ),
    )
}

/// A task on the goal's list: the user's with a tick to mark it done (struck through once
/// ticked), the assistant's marked as theirs; a two-finger swipe right is Done (the user's),
/// left Next week; a right-click offers Done (the user's), Next week and Archive.
fn todo_row(
    p: &Pal,
    store: &Entity<Store>,
    todo: &Todo,
    ticked: bool,
    swipes: &RowSwipes,
) -> AnyElement {
    let id = todo.id;
    let mine = todo.who != crate::wire::ASSISTANT_OWNER;
    let mark = if mine {
        tick(store, p, id, &todo.text, 16., ticked).into_any_element()
    } else {
        who(p, &todo.who).into_any_element()
    };
    let (on_swipe, swipes) = (store.clone(), swipes.clone());
    let row = div()
        .id(ElementId::Name(format!("todo-{id}").into()))
        .flex()
        .items_center()
        .gap(px(10.))
        .py(px(5.))
        .child(mark)
        .child(
            div()
                .flex_1()
                .text_size(px(13.))
                .when(ticked, |d| d.line_through().text_color(p.muted))
                .child(direction::marked(&todo.text)),
        )
        .on_scroll(move |event, _, cx| match swipes.feed(id, event) {
            Some(Swiped::Right) if mine => on_swipe.update(cx, |store, cx| store.tick_done(id, cx)),
            Some(Swiped::Left) => on_swipe.update(cx, |store, cx| store.not_now(id, cx)),
            _ => {}
        });
    let store = store.clone();
    row.context_menu(move |mut menu, _, _| {
        if mine {
            let done = store.clone();
            menu = menu.item(PopupMenuItem::new("Done").on_click(move |_, _, cx| {
                done.update(cx, |store, cx| store.tick_done(id, cx));
            }));
        }
        let (later, away) = (store.clone(), store.clone());
        menu.item(
            PopupMenuItem::new("Next week")
                .on_click(move |_, _, cx| later.update(cx, |store, cx| store.not_now(id, cx))),
        )
        .item(
            PopupMenuItem::new("Archive")
                .on_click(move |_, _, cx| away.update(cx, |store, cx| store.archive(id, cx))),
        )
    })
    .into_any_element()
}

/// This week's done tasks: who did each, and the day.
fn done_rows(p: &Pal, done: &[&Todo]) -> Vec<Div> {
    let zone = jiff::tz::TimeZone::system();
    done.iter()
        .map(|todo| {
            let day = todo.done.as_ref().and_then(clock::instant).map(|at| {
                at.to_zoned_datetime(zone.clone())
                    .strftime("%a")
                    .to_string()
            });
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .py(px(4.))
                .child(who(p, &todo.who))
                .child(
                    div()
                        .flex_1()
                        .text_size(px(13.))
                        .child(direction::marked(&todo.text)),
                )
                .children(day.map(|day| div().text_size(px(11.5)).text_color(p.muted).child(day)))
        })
        .collect()
}
