//! The sidebar's Agents: every agent but the default one, each with its own room. A row has
//! the agent's name (bold while its room has something unread), a pulsing dot while the
//! assistant works there, how many of its requests wait for an answer, and under it who the
//! agent works for, Off while it is disabled. A click opens the agent's room.

use gpui_kit::assets::IconName as AssetIcon;
use gpui_kit::component::Icon;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, ElementId, FontWeight, div, px};

use super::palette::Pal;
use super::widgets::{dot, section_title};
use super::window::MainWindow;
use crate::store::Page;
use crate::store::approvals::lists::AgentRoom;
use crate::store::approvals::shown::visible;
use crate::store::approvals::words::{agent_line, approvals_count};
use crate::ui::a11y::{self, Accessible as _};
use crate::ui::controls::tab_stop;

/// The Agents section: its title and a row per agent with a room of its own; nothing without
/// one.
pub(crate) fn section(
    window: &MainWindow,
    p: &Pal,
    cx: &Context<'_, MainWindow>,
) -> Vec<AnyElement> {
    let store = window.store.read(cx);
    let own = store.agent_rooms();
    if own.is_empty() {
        return Vec::new();
    }
    // Where the window is: the page its column shows, else the chat's room.
    let page = window
        .column
        .read(cx)
        .current()
        .unwrap_or(Page::Room(store.room()));
    let mut rows = vec![section_title(p, "Agents").into_any_element()];
    for agent in own {
        let selected = page == Page::Room(agent.agent.room);
        rows.push(row(agent, selected, p, cx));
    }
    rows
}

/// What a screen reader says of an agent's row, in one line.
#[must_use]
fn spoken(agent: AgentRoom<'_>) -> String {
    let unread = agent.room.map_or(0, |room| room.unread);
    let working = agent.room.is_some_and(|room| room.busy);
    let waiting = agent.waiting;
    a11y::line(&[
        &visible(&agent.agent.name),
        "agent",
        &agent_line(agent.agent),
        &if waiting > 0 {
            approvals_count(waiting)
        } else {
            String::new()
        },
        &if unread > 0 {
            format!("{unread} new")
        } else {
            String::new()
        },
        if working { "working" } else { "" },
    ])
}

/// An agent's row.
fn row(agent: AgentRoom<'_>, selected: bool, p: &Pal, cx: &Context<'_, MainWindow>) -> AnyElement {
    let AgentRoom {
        agent: row,
        room,
        waiting,
    } = agent;
    let id = row.room;
    let unread = room.is_some_and(|room| room.unread > 0);
    let working = room.is_some_and(|room| room.busy);
    let enabled = row.enabled;
    let name = div()
        .flex()
        .items_center()
        .gap(px(5.))
        .text_size(px(13.))
        .child(
            div()
                .truncate()
                .when(!enabled, |d| d.text_color(p.muted))
                .when(unread, |d| d.font_weight(FontWeight::SEMIBOLD))
                .child(visible(&row.name).into_owned()),
        )
        .when(working, |d| d.child(dot(p.info, 6.)))
        .when(unread, |d| d.child(dot(p.accent, 6.)))
        .child(div().flex_1())
        .when(waiting > 0, |d| {
            d.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(3.))
                    .text_size(px(11.))
                    .text_color(p.accent)
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(
                        Icon::new(AssetIcon::BadgeCheck)
                            .size(px(11.))
                            .text_color(p.accent),
                    )
                    .child(waiting.to_string()),
            )
        });
    let line = div()
        .truncate()
        .text_size(px(11.5))
        .text_color(if enabled { p.muted } else { p.faint })
        .child(agent_line(row));
    let off = if enabled { "" } else { " (off)" };
    let help = format!(
        "{}: an agent working for {}{off}",
        visible(&row.name),
        visible(&row.principal)
    );
    tab_stop(
        div()
            .id(ElementId::Name(format!("agent-room-{id}").into()))
            .button_named(spoken(agent))
            .aria_selected(selected)
            .border_1()
            .flex()
            .gap(px(8.))
            .px(px(10.))
            .py(px(5.))
            .mx(px(8.))
            .rounded(px(6.))
            .when(selected, |d| d.bg(p.accent_soft))
            .when(!selected, |d| d.hover(|d| d.bg(p.hover)))
            .cursor_pointer()
            .child(
                div().pt(px(3.)).child(
                    Icon::new(AssetIcon::Bot)
                        .size(px(13.))
                        .text_color(if enabled { p.accent } else { p.faint }),
                ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .gap(px(1.))
                    .child(name)
                    .child(line),
            )
            .tooltip(move |window, cx| Tooltip::new(help.clone()).build(window, cx))
            .on_click(
                cx.listener(move |this, _, window, cx| this.open(Page::Room(id), window, cx)),
            ),
        p,
    )
    .into_any_element()
}
