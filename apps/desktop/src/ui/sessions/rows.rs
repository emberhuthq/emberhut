//! The Sessions screen's rows: an agent working now (a lead, its helpers under it), a running
//! session with its live line, a finished one in one line (its last answer under it once its
//! chevron opens it), and an external thread still running. A click opens the row's page; with
//! plans, a session's right-click menu has Move to task….

use emberhut_api::runner::agents::AgentNow;
use emberhut_api::threads::ExternalThread;
use emberhut_api::work::RunnerSession;
use gpui_kit::TestSupportExt as _;
use gpui_kit::assets::IconName;
use gpui_kit::component::menu::ContextMenuExt as _;
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, Div, ElementId, Entity, FontWeight, Role, Stateful, div, px};
use jiff::ZonedDateTime;

use super::marks::{live_dot, state_mark};
use super::{Open, push};
use crate::prose::direction;
use crate::store::Store;
use crate::store::room_line::ago;
use crate::store::sessions::places::SessionPlace;
use crate::store::sessions::view::SessionRef;
use crate::store::sessions::words::{self, Mark, short_title};
use crate::store::tasks::clock;
use crate::store::tasks::moving::Moving;
use crate::ui::a11y::{self, Accessible as _};
use crate::ui::controls::tab_stop;
use crate::ui::move_to_task;
use crate::ui::palette::Pal;
use crate::ui::reads;
use crate::ui::tasks::marks::glyph;

/// What opens or folds a row's last answer.
pub(crate) type Toggle = Box<dyn Fn(&mut gpui_kit::Window, &mut gpui_kit::App)>;

/// What the rows are drawn with.
pub(crate) struct Rows<'a> {
    /// The store, for a click.
    pub(crate) store: &'a Entity<Store>,
    /// The palette.
    pub(crate) p: &'a Pal,
    /// Now, for the timers and how long ago.
    pub(crate) now: &'a ZonedDateTime,
    /// A task's key, by its id.
    pub(crate) key: &'a dyn Fn(emberhut_api::ids::TaskId) -> String,
    /// The state's Claude accounts, to name a row's account by its id.
    pub(crate) accounts: &'a [emberhut_api::runner::accounts::ClaudeAccount],
    /// A session's row has Move to task… in its right-click menu (the server keeps plans).
    pub(crate) moves: bool,
}

impl Rows<'_> {
    fn clickable(&self, id: ElementId, open: Open) -> Stateful<Div> {
        let store = self.store.clone();
        tab_stop(
            div()
                .id(id)
                // Read as one line, named by its caller.
                .role(Role::Button)
                .hint("Opens its page")
                .border_1()
                .rounded(px(8.))
                .px(px(10.))
                .py(px(6.))
                .cursor_pointer()
                .hover(|d| d.bg(self.p.hover))
                .on_click(move |_, window, cx| push(&store, open.clone(), window, cx)),
            self.p,
        )
    }

    /// `row`, with Move to task… for `session` in its right-click menu while sessions move.
    fn with_menu<E>(&self, row: E, session: &RunnerSession) -> AnyElement
    where
        E: InteractiveElement + ParentElement + Styled + IntoElement + 'static,
    {
        if !self.moves {
            return row.into_any_element();
        }
        let moving = Moving::of(session);
        row.context_menu(move |menu, _, _| menu.item(move_to_task::menu_item(moving.clone())))
            .into_any_element()
    }

    fn timer(&self, since: Option<&emberhut_api::time::Timestamp>) -> Option<String> {
        let since = since.and_then(clock::instant)?;
        Some(words::timer(since, self.now.timestamp()))
    }

    /// An agent working now: a lead's row, or a helper's (under its lead when `lone` is false).
    pub(crate) fn agent(&self, agent: &AgentNow, lone: bool, cx: &gpui_kit::App) -> AnyElement {
        let p = self.p;
        let (title, doing) = words::agent_lines(agent, lone);
        let open = agent.viewer.clone().map(|viewer| {
            let name = if agent.lead {
                title.clone()
            } else {
                agent.name.clone().unwrap_or_else(|| "Agent".to_owned())
            };
            Open::Place(SessionPlace::Session(SessionRef::of_agent(
                viewer,
                agent.parent.clone(),
                name,
            )))
        });
        let id = ElementId::Name(format!("working-{}", agent.id).into());
        let timer = self.timer(agent.started.as_ref());
        let spoken = a11y::line(&[
            &title,
            "running",
            timer.as_deref().unwrap_or_default(),
            &doing,
        ]);
        let body = div()
            .flex()
            .items_start()
            .gap(px(10.))
            .child(
                div()
                    .pt(px(5.))
                    .child(live_dot(p, "running", id.clone(), cx)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .gap(px(2.))
                    .child(
                        div()
                            .flex()
                            .gap(px(6.))
                            .child(
                                div()
                                    .flex_1()
                                    .truncate()
                                    .text_size(px(if agent.lead { 13.5 } else { 12.5 }))
                                    .when(agent.lead, |d| d.font_weight(FontWeight::MEDIUM))
                                    .child(direction::marked(&title)),
                            )
                            .when_some(timer, |d, timer| {
                                d.child(
                                    div().text_size(px(11.5)).text_color(p.success).child(timer),
                                )
                            }),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_size(px(12.))
                            .text_color(p.muted)
                            .child(doing),
                    ),
            );
        let row = match open {
            Some(open) => self
                .clickable(id, open)
                .aria_label(spoken)
                .child(body)
                .into_any_element(),
            None => div().px(px(10.)).py(px(6.)).child(body).into_any_element(),
        };
        if agent.lead || lone {
            row
        } else {
            div()
                .ml(px(22.))
                .pl(px(10.))
                .border_l_1()
                .border_color(p.border)
                .child(row)
                .into_any_element()
        }
    }

    /// A running session: its dot, title, timer, current step and muted line.
    pub(crate) fn running(&self, session: &RunnerSession, cx: &gpui_kit::App) -> AnyElement {
        let p = self.p;
        let id = ElementId::Name(format!("session-{}", session.id).into());
        let open = Open::Place(SessionPlace::Session(SessionRef::whole(session.id.clone())));
        let timer = self.timer(session.since.as_ref());
        let doing = session
            .doing
            .clone()
            .unwrap_or_else(|| words::status_text(&session.status));
        let spoken = a11y::line(&[
            short_title(&session.title),
            "running",
            timer.as_deref().unwrap_or_default(),
            &doing,
        ]);
        let row = self
            .clickable(id.clone(), open)
            .aria_label(spoken)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(7.))
                    .child(live_dot(p, &session.status, id, cx))
                    .child(
                        div()
                            .flex_1()
                            .truncate()
                            .text_size(px(13.5))
                            .font_weight(FontWeight::MEDIUM)
                            .child(direction::marked(short_title(&session.title))),
                    )
                    .when_some(timer, |d, timer| {
                        d.child(div().text_size(px(11.5)).text_color(p.success).child(timer))
                    }),
            )
            .child(
                div()
                    .text_size(px(12.5))
                    .text_color(p.muted)
                    .line_clamp(2)
                    .child(doing),
            )
            .child(
                div()
                    .truncate()
                    .text_size(px(11.5))
                    .text_color(p.faint)
                    .child(words::meta(session, self.accounts, self.key)),
            )
            .test_support();
        self.with_menu(row, session)
    }

    /// A finished, stopped, failed or waiting session in one line; `open` shows its last answer
    /// under it, `toggle` opens or folds that.
    pub(crate) fn line(
        &self,
        session: &RunnerSession,
        open: bool,
        toggle: Option<Toggle>,
        cx: &gpui_kit::App,
    ) -> AnyElement {
        let p = self.p;
        let id = ElementId::Name(format!("session-{}", session.id).into());
        let place = Open::Place(SessionPlace::Session(SessionRef::whole(session.id.clone())));
        let waiting = Mark::of(&session.status) == Mark::Waiting;
        let when = ago(Some(session.updated_at.as_str()), self.now);
        let spoken = a11y::line(&[
            short_title(&session.title),
            &words::status_text(&session.status),
            &words::meta(session, self.accounts, self.key),
            when.as_deref().unwrap_or_default(),
        ]);
        let line = self.clickable(id.clone(), place).flex_1().min_w_0().child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(state_mark(p, &session.status, id, cx))
                .child(
                    div()
                        .flex_shrink(1.)
                        .min_w_0()
                        .truncate()
                        .text_size(px(13.))
                        .when(waiting, |d| d.font_weight(FontWeight::SEMIBOLD))
                        .child(direction::marked(short_title(&session.title))),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_size(px(12.))
                        .text_color(p.faint)
                        .child(words::meta(session, self.accounts, self.key)),
                )
                .when_some(when, |d, when| {
                    d.child(
                        div()
                            .flex_none()
                            .text_size(px(12.))
                            .text_color(p.faint)
                            .child(when),
                    )
                }),
        );
        let line = self.with_menu(line.aria_label(spoken), session);
        let summary = words::summary(session);
        let chevron = toggle.filter(|_| summary.is_some()).map(|toggle| {
            tab_stop(
                div()
                    .id(ElementId::Name(
                        format!("session-fold-{}", session.id).into(),
                    ))
                    .button_named(if open {
                        "Hide its last answer"
                    } else {
                        "Show its last answer"
                    })
                    .aria_expanded(open)
                    .border_1()
                    .rounded(px(4.))
                    .p(px(3.))
                    .cursor_pointer()
                    .child(glyph(
                        if open {
                            IconName::ChevronDown
                        } else {
                            IconName::ChevronRight
                        },
                        p.faint,
                        11.,
                    ))
                    .on_click(move |_, window, cx| toggle(window, cx)),
                p,
            )
            .test_support()
        });
        div()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .items_center()
                    .child(div().w(px(22.)).flex_none().children(chevron))
                    .child(line),
            )
            .when_some(summary.filter(|_| open), |d, text| {
                d.child(
                    div()
                        .pl(px(44.))
                        .pr(px(10.))
                        .pb(px(6.))
                        .text_size(px(12.))
                        .text_color(p.muted)
                        .when(reads::starts_right(&text), Styled::text_right)
                        .child(text),
                )
            })
            .into_any_element()
    }

    /// An external thread still running: its dot, title, how long ago, its last line and muted line.
    pub(crate) fn thread(&self, thread: &ExternalThread, cx: &gpui_kit::App) -> AnyElement {
        let p = self.p;
        let id = ElementId::Name(format!("thread-{}", thread.id).into());
        let open = Open::Place(SessionPlace::Thread {
            machine: thread.machine.clone(),
            thread: thread.id.clone(),
        });
        let status = if thread.is_waiting() {
            "interrupted"
        } else {
            thread.status.as_str()
        };
        let meta = thread
            .tasks
            .iter()
            .map(|t| (self.key)(*t))
            .chain([thread.project.clone(), thread.place().to_owned()])
            .chain(thread.is_waiting().then(|| "waiting for you".to_owned()))
            .filter(|part| !part.trim().is_empty())
            .collect::<Vec<_>>()
            .join(" · ");
        let last = thread
            .last
            .as_deref()
            .and_then(words::plain)
            .map(|text| text.chars().take(200).collect::<String>());
        let when = ago(Some(thread.updated_at.as_str()), self.now);
        let spoken = a11y::line(&[
            &thread.title,
            status,
            when.as_deref().unwrap_or_default(),
            &meta,
        ]);
        self.clickable(id.clone(), open)
            .aria_label(spoken)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(7.))
                    .child(live_dot(p, status, id, cx))
                    .child(
                        div()
                            .flex_1()
                            .truncate()
                            .text_size(px(13.5))
                            .font_weight(FontWeight::MEDIUM)
                            .child(direction::marked(&thread.title)),
                    )
                    .when_some(when, |d, when| {
                        d.child(div().text_size(px(11.5)).text_color(p.faint).child(when))
                    }),
            )
            .when_some(last, |d, last| {
                d.child(
                    div()
                        .text_size(px(12.5))
                        .text_color(p.muted)
                        .line_clamp(2)
                        .child(last),
                )
            })
            .child(
                div()
                    .truncate()
                    .text_size(px(11.5))
                    .text_color(p.faint)
                    .child(meta),
            )
            .into_any_element()
    }
}

/// A section's header with a count in a capsule of its colour.
pub(crate) fn counted(p: &Pal, title: &'static str, count: usize, green: bool) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(6.))
        .px(px(10.))
        .pt(px(14.))
        .pb(px(4.))
        .child(
            div()
                .text_size(px(11.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(p.faint)
                .child(title),
        )
        .child(crate::ui::widgets::pill(
            count.to_string(),
            if green { p.success } else { p.border },
            if green { p.on_accent } else { p.muted },
        ))
}

/// The time now in this device's zone.
#[must_use]
pub(crate) fn now() -> ZonedDateTime {
    ZonedDateTime::now()
}
