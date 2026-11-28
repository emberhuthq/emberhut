//! A session with no task as an item of the Tasks list (on a server that keeps plans): a
//! terminal mark, its title in quotes, where it runs and how it stands (waiting on the user in
//! the accent, running in amber), what it asks or last said, and how long since it moved. It
//! never has a bar. A click opens the session; Back returns to the list; its right-click menu
//! has Move to task…. The foot's heading has a way to every session.

use emberhut_api::work::RunnerSession;
use gpui_kit::assets::IconName;
use gpui_kit::component::menu::ContextMenuExt as _;
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, Div, ElementId, Entity, FontWeight, div, px};

use super::marks::glyph;
use crate::store::sessions::places::SessionPlace;
use crate::store::sessions::view::SessionRef;
use crate::store::sessions::words::{self as session_words, Mark};
use crate::store::tasks::clock;
use crate::store::tasks::moving::Moving;
use crate::store::{Page, Store};
use crate::ui::a11y::{self, Accessible as _};
use crate::ui::column;
use crate::ui::controls::tab_stop;
use crate::ui::move_to_task;
use crate::ui::palette::Pal;
use crate::ui::sessions::{self, Open};

/// The session's item.
pub(super) fn row(store: &Entity<Store>, p: &Pal, session: &RunnerSession) -> AnyElement {
    let mark = Mark::of(&session.status);
    let title = format!("“{}”", session_words::short_title(&session.title));
    let (state, color) = match mark {
        Mark::Waiting => ("waiting on you", p.accent),
        Mark::Running => ("running", p.warn),
        _ => ("finished", p.muted),
    };
    let place = if session.place.is_empty() {
        "No task".to_owned()
    } else {
        format!("No task · on {}", session.place)
    };
    let said = first_line(session);
    let since =
        clock::instant(&session.updated_at).map(|at| clock::relative(at, jiff::Timestamp::now()));
    let spoken = a11y::line(&[
        &title,
        "session with no task",
        state,
        said.as_deref().unwrap_or_default(),
    ]);
    let page = SessionPlace::Session(SessionRef::whole(session.id.clone()));
    let store = store.clone();
    let id = session.id.as_str().to_owned();
    let moving = Moving::of(session);
    tab_stop(
        div()
            .id(ElementId::Name(format!("loose-session-{id}").into()))
            .button_named(spoken)
            .hint("Opens the session")
            .border_1()
            .flex()
            .items_start()
            .gap(px(10.))
            .px(px(8.))
            .py(px(7.))
            .rounded(px(7.))
            .cursor_pointer()
            .hover(|d| d.bg(p.hover))
            .child(
                div()
                    .w(px(16.))
                    .h(px(18.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(glyph(IconName::SquareTerminal, p.faint, 14.)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(1.))
                    .flex_1()
                    .min_w_0()
                    .child(div().truncate().text_size(px(13.)).child(title))
                    .child(
                        div()
                            .flex()
                            .gap(px(4.))
                            .text_size(px(11.5))
                            .text_color(p.muted)
                            .child(place)
                            .child("·")
                            .child(
                                div()
                                    .text_color(color)
                                    .when(mark == Mark::Waiting, |d| {
                                        d.font_weight(FontWeight::SEMIBOLD)
                                    })
                                    .child(state),
                            ),
                    )
                    .children(said.map(|said| {
                        div()
                            .truncate()
                            .text_size(px(11.5))
                            .text_color(p.faint)
                            .child(said)
                    })),
            )
            .children(since.map(|since| {
                div()
                    .flex_none()
                    .pt(px(1.))
                    .text_size(px(11.5))
                    .text_color(p.faint)
                    .child(since)
            }))
            .on_click(move |_, window, cx| {
                sessions::push(&store, Open::Place(page.clone()), window, cx);
            }),
        p,
    )
    .context_menu(move |menu, _, _| menu.item(move_to_task::menu_item(moving.clone())))
    .into_any_element()
}

/// The first line of what `session` asks or last said.
fn first_line(session: &RunnerSession) -> Option<String> {
    session_words::summary(session).and_then(|said| {
        said.lines()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .map(str::to_owned)
    })
}

/// The foot's heading over the sessions with no task, with `All sessions`, which opens the
/// Sessions screen over the list.
pub(super) fn section_head(p: &Pal, count: usize) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .px(px(8.))
        .pt(px(16.))
        .pb(px(6.))
        .child(
            div()
                .text_size(px(12.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(p.muted)
                .child("Sessions with no task"),
        )
        .child(
            div()
                .text_size(px(11.5))
                .text_color(p.faint)
                .child(count.to_string()),
        )
        .child(div().flex_1())
        .child(tab_stop(
            div()
                .id("all-sessions")
                .button_named("All sessions")
                .hint("Opens every session")
                .border_1()
                .rounded(px(6.))
                .px(px(6.))
                .py(px(2.))
                .text_size(px(11.5))
                .text_color(p.accent)
                .cursor_pointer()
                .hover(|d| d.bg(p.hover))
                .child("All sessions ›")
                .on_click(|_, window, cx| column::push(Page::Sessions, window, cx)),
            p,
        ))
}
