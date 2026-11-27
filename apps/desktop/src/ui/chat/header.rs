//! The chat's bar: the room's name and colour, the assistant's status line, how many things
//! need the user in this room, and (while the sidebar is hidden) in all of them, lit while the
//! window's column shows Needs you. An agent's room is marked as one, with who the agent works
//! for beside the status (Off while it is disabled), and its pending approvals open the agent's
//! own list.

use emberhut_api::agents::AgentRow;
use gpui_kit::assets::IconName as AssetIcon;
use gpui_kit::component::{Icon, IconName};
use gpui_kit::prelude::*;
use gpui_kit::{Context, Div, FontWeight, Stateful, Window, div, px};

use super::{ChatView, reading};
use crate::brand::APP_NAME;
use crate::store::approvals::shown::visible;
use crate::store::approvals::words::{agent_caption, approvals_count};
use crate::store::{Page, lookup};
use crate::ui::a11y::Accessible as _;
use crate::ui::column;
use crate::ui::palette::{Pal, room_color};
use crate::ui::widgets::{dot, icon, pill};

impl ChatView {
    /// The bar over the chat.
    pub(super) fn header(&self, p: &Pal, window: &Window, cx: &mut Context<'_, Self>) -> Div {
        // Lit while this window's column shows Needs you; a click opens it, or shuts it.
        let needs_open = column::shows(Page::Needs, window, cx);
        let store = self.store.read(cx);
        let room = store
            .state()
            .and_then(|state| state.rooms.iter().find(|room| room.id == store.room()));
        let agent = store.agent_of_room(store.room()).cloned();
        let waiting = agent
            .as_ref()
            .map_or(0, |agent| store.waiting_on_you(&agent.id));
        let name = match &agent {
            Some(agent) => visible(&agent.name).into_owned(),
            None => room
                .filter(|room| !room.is_main())
                .map_or_else(|| APP_NAME.to_owned(), |room| room.name.clone()),
        };
        let color = room.filter(|room| !room.is_main()).map(|room| {
            room_color(
                store
                    .state()
                    .and_then(|state| lookup::goal_rank(state, room.goal)),
                p,
            )
        });
        let status = store.status();
        let (status_text, status_color) = match &agent {
            Some(agent) if !agent.enabled => (agent_caption(agent, &status.text), p.faint),
            Some(agent) => (
                agent_caption(agent, &status.text),
                reading::status_color(status.tone, p),
            ),
            None => (status.text, reading::status_color(status.tone, p)),
        };
        let needs = room.map_or(0, |room| room.needs);
        let all_needs = store.needs_count();
        div()
            .flex()
            .items_center()
            .gap(px(8.))
            .h(px(52.))
            .px(px(20.))
            .border_b_1()
            .border_color(p.border)
            .when_some(color, |d, color| d.child(dot(color, 9.)))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .child(title(name, agent.as_ref(), p))
                    .child(
                        div()
                            .text_size(px(11.5))
                            .text_color(status_color)
                            .child(status_text),
                    ),
            )
            .child(div().flex_1())
            .when_some(agent, |d, agent| {
                d.child(approvals_button(&agent, waiting, p).on_click(cx.listener(
                    move |this, _, window, cx| {
                        let key = this
                            .store
                            .update(cx, |store, _| store.agent_key(agent.id.clone()));
                        column::open(Page::Approvals(Some(key)), window, cx);
                    },
                )))
            })
            .child(crate::ui::diary::note_button(p))
            .child(crate::ui::spotlight::search_button(p))
            .when(self.sidebar_hidden, |d| {
                d.child(reading::needs_counter(all_needs, needs_open, p).on_click(
                    cx.listener(|_, _, window, cx| column::toggle(Page::Needs, window, cx)),
                ))
            })
            .when(needs > 0, |d| {
                d.child(
                    div()
                        .id("room-needs")
                        .cursor_pointer()
                        .flex()
                        .items_center()
                        .gap(px(5.))
                        .px(px(9.))
                        .py(px(4.))
                        .rounded(px(12.))
                        .bg(p.accent_soft)
                        .text_color(p.accent)
                        .text_size(px(12.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(icon(IconName::Inbox, p.accent, 13.))
                        .child(format!("{needs} need you"))
                        .on_click(cx.listener(|_, _, window, cx| {
                            column::open(Page::Needs, window, cx);
                        })),
                )
            })
    }
}

/// The room's name; an agent's marked as one, its icon dimmed while the agent is disabled.
fn title(name: String, agent: Option<&AgentRow>, p: &Pal) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(6.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_size(px(14.))
        .when_some(agent, |d, agent| {
            d.child(
                Icon::new(AssetIcon::Bot)
                    .size(px(14.))
                    .text_color(if agent.enabled { p.accent } else { p.faint }),
            )
        })
        .child(name)
        .when(agent.is_some(), |d| {
            d.child(pill("Agent", p.accent_soft, p.accent))
        })
}

/// The button with how many of an agent's requests wait on the user (`Approvals` with none),
/// which opens the agent's whole list (what waits on others too).
fn approvals_button(agent: &AgentRow, waiting: usize, p: &Pal) -> Stateful<Div> {
    let label = approvals_count(waiting);
    div()
        .id("agent-approvals")
        .button_named(format!("{}'s approvals: {label}", visible(&agent.name)))
        .hint("Opens this agent's pending approval requests")
        .cursor_pointer()
        .flex()
        .items_center()
        .gap(px(5.))
        .px(px(9.))
        .py(px(4.))
        .mr(px(6.))
        .rounded(px(12.))
        .when(waiting > 0, |d| d.bg(p.accent_soft).text_color(p.accent))
        .when(waiting == 0, |d| d.text_color(p.muted))
        .hover(|d| d.bg(p.hover))
        .text_size(px(12.))
        .font_weight(FontWeight::SEMIBOLD)
        .child(
            Icon::new(AssetIcon::BadgeCheck)
                .size(px(13.))
                .text_color(if waiting > 0 { p.accent } else { p.muted }),
        )
        .child(label)
}
