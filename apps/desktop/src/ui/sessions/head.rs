//! A session page's head: its title (a helper's says whose it is), its status with its dot and
//! timer, Live or Reconnecting while it runs, where it runs (machine, account, folder, branch),
//! its model and effort on the user's own, its tasks (each opens its page), and the last thing
//! that failed. Its actions: Stop (asked first on the assistant's session), and on the user's
//! own Stop this turn and the settings menu (model, effort, Stop and drop what's queued); with
//! plans, Move to task… on a whole session. Also the confirm card and a new session's draft card.

use emberhut_api::runner::actions::{Effort, SessionSettings};
use gpui_kit::assets::IconName;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::prelude::*;
use gpui_kit::{Div, ElementId, Entity, FontWeight, Role, Stateful, div, px};

use super::marks::{live_dot, spinner};
use super::page::SessionPage;
use super::{Open, push};
use crate::prose::direction;
use crate::store::sessions::summary::Summary;
use crate::store::sessions::view::SessionRef;
use crate::store::sessions::watch::SessionWatch;
use crate::store::sessions::words::{self, MODELS};
use crate::store::tasks::clock;
use crate::store::tasks::moving::{Moving, can_move};
use crate::ui::a11y::{self, Accessible as _};
use crate::ui::controls::tab_stop;
use crate::ui::move_to_task;
use crate::ui::palette::Pal;
use crate::ui::reads::reads;
use crate::ui::tasks::marks::glyph;

/// The head: actions, then the card.
pub(crate) fn bar(
    page: &SessionPage,
    watch: &Entity<SessionWatch>,
    head: &Summary,
    p: &Pal,
    cx: &Context<'_, SessionPage>,
) -> Div {
    let read = watch.read(cx);
    let view = read.view();
    let mut card = div()
        .flex()
        .flex_col()
        .gap(px(6.))
        .p(px(12.))
        .rounded(px(14.))
        .bg(p.card)
        .border_1()
        .border_color(p.border)
        .child(
            reads(&head.title)
                .font_weight(FontWeight::SEMIBOLD)
                .text_size(px(15.)),
        );
    if let Some(of) = &head.helper_of {
        card = card.child(muted(p, of.clone()));
    }
    card = card.child(status_row(
        head,
        view.is_loaded(),
        view.is_connected(),
        p,
        cx,
    ));
    if !head.place.is_empty() {
        card = card.child(muted(p, head.place.clone()));
    }
    if head.direct
        && let Some(line) = head.model_line()
    {
        card = card.child(muted(p, line));
    }
    card = task_links(card, page, head, p, cx);
    if let Some(problem) = read.problem() {
        card = card.child(
            div()
                .id("session-problem")
                .announced(problem.to_owned())
                .text_size(px(12.))
                .text_color(p.danger)
                .child(problem.to_owned()),
        );
    }
    div()
        .flex()
        .flex_col()
        .gap(px(8.))
        .px(px(14.))
        .pt(px(10.))
        .pb(px(6.))
        .child(actions(page, head, view.reference(), cx))
        .child(card)
}

fn muted(p: &Pal, text: String) -> Div {
    div().text_size(px(12.)).text_color(p.muted).child(text)
}

/// Move to task… (a whole session, on a server that keeps plans), then Stop, and on the user's
/// own session Stop this turn and its settings.
fn actions(
    page: &SessionPage,
    head: &Summary,
    shown: &SessionRef,
    cx: &Context<'_, SessionPage>,
) -> Div {
    let helper = shown.agent.is_some();
    let mut actions = div().flex().items_center().gap(px(6.)).justify_end();
    if !helper && can_move(page.store.read(cx).state()) {
        let moving = Moving::new(shown.id.clone(), &head.title, head.tasks.clone());
        actions = actions.child(move_to_task::button("session-move-to-task", moving));
    }
    if head.direct {
        if head.running {
            actions = actions.child(
                Button::new("session-stop-turn-bar")
                    .label("Stop")
                    .small()
                    .danger()
                    .tooltip("Stop this turn (⌘.); what's queued runs next")
                    .on_click(cx.listener(|page, _, _, cx| page.stop_turn(cx))),
            );
        }
        actions = actions.child(settings_menu(page, head));
    } else if head.running && !helper {
        actions = actions.child(
            Button::new("session-stop")
                .label("Stop")
                .small()
                .danger()
                .on_click(cx.listener(|page, _, _, cx| page.stop_pressed(cx))),
        );
    }
    actions
}

/// The status with its dot and timer, and Live or Reconnecting while it runs. A screen reader
/// hears the status and Live or Reconnecting as they change (not the timer's every second).
fn status_row(
    head: &Summary,
    loaded: bool,
    connected: bool,
    p: &Pal,
    cx: &gpui_kit::App,
) -> Stateful<Div> {
    let timer = head
        .since
        .as_ref()
        .and_then(clock::instant)
        .filter(|_| head.running)
        .map(|since| words::timer(since, jiff::Timestamp::now()));
    let mut status = div()
        .flex()
        .items_center()
        .gap(px(8.))
        .child(live_dot(p, &head.status, "session-head-dot", cx))
        .child(
            div()
                .text_size(px(13.))
                .font_weight(FontWeight::SEMIBOLD)
                .child(words::status_text(&head.status)),
        );
    if let Some(timer) = timer {
        status = status.child(div().text_size(px(13.)).text_color(p.success).child(timer));
    }
    status = status.child(div().flex_1());
    let link = match (loaded && head.running, connected) {
        (true, true) => "Live",
        (true, false) => "Reconnecting",
        (false, _) => "",
    };
    let spoken = a11y::line(&[&words::status_text(&head.status), link]);
    if loaded && head.running {
        let (word, icon, color) = if connected {
            ("Live", IconName::Radio, p.success)
        } else {
            ("Reconnecting", IconName::WifiOff, p.warn)
        };
        status = status.child(
            div()
                .flex()
                .items_center()
                .gap(px(4.))
                .text_size(px(11.5))
                .text_color(color)
                .child(glyph(icon, color, 12.))
                .child(word),
        );
    }
    status.id("session-status").announced(spoken)
}

/// Its tasks, each opening its page.
fn task_links(
    mut card: Div,
    page: &SessionPage,
    head: &Summary,
    p: &Pal,
    cx: &gpui_kit::App,
) -> Div {
    let store = page.store.read(cx);
    for task in &head.tasks {
        let title = store.task_of(*task).map_or_else(
            || store.task_key(*task),
            |issue| format!("{} {}", issue.key, issue.title),
        );
        let (owner, task) = (page.store.clone(), *task);
        card = card.child(tab_stop(
            div()
                .id(ElementId::Name(format!("session-task-{task}").into()))
                .button_named(title.clone())
                .hint("Opens its page")
                .border_1()
                .rounded(px(6.))
                .flex()
                .items_center()
                .gap(px(6.))
                .py(px(2.))
                .cursor_pointer()
                .text_color(p.accent)
                .child(
                    div()
                        .flex_1()
                        .truncate()
                        .text_size(px(13.))
                        .child(direction::marked(&title)),
                )
                .child(glyph(IconName::ChevronRight, p.accent, 12.))
                .on_click(move |_, window, cx| push(&owner, Open::Task(task), window, cx)),
            p,
        ));
    }
    card
}

/// The user's own session's settings: the model and effort its next turn opens with, and Stop
/// and drop what's queued (asked first).
fn settings_menu(page: &SessionPage, head: &Summary) -> impl IntoElement {
    let page = page.this.clone();
    let (model, effort) = (head.model.clone(), head.effort.clone());
    let can_drop = head.running || head.queued > 0;
    Button::new("session-settings")
        .label("Settings")
        .small()
        .ghost()
        .dropdown_caret(true)
        .dropdown_menu(move |mut menu, _, _| {
            for (id, name) in MODELS {
                let page = page.clone();
                menu = menu.item(
                    PopupMenuItem::new(format!("Model: {name}"))
                        .checked(model.as_deref() == Some(id))
                        .on_click(move |_, _, cx| {
                            let settings = SessionSettings {
                                model: Some(id.to_owned()),
                                effort: None,
                            };
                            page.update(cx, |page, cx| page.change(settings, cx))
                                .inspect_err(|error| log::debug!("the page closed: {error}"))
                                .ok();
                        }),
                );
            }
            menu = menu.separator();
            for level in Effort::ALL {
                let page = page.clone();
                let on = effort.as_deref().unwrap_or("high") == level.as_str();
                menu = menu.item(
                    PopupMenuItem::new(format!("Effort: {}", level.name()))
                        .checked(on)
                        .on_click(move |_, _, cx| {
                            let settings = SessionSettings {
                                model: None,
                                effort: Some(level.clone()),
                            };
                            page.update(cx, |page, cx| page.change(settings, cx))
                                .inspect_err(|error| log::debug!("the page closed: {error}"))
                                .ok();
                        }),
                );
            }
            if can_drop {
                let page = page.clone();
                menu = menu.separator().item(
                    PopupMenuItem::new("Stop and drop what's queued").on_click(move |_, _, cx| {
                        page.update(cx, |page, cx| {
                            page.confirm_stop = true;
                            cx.notify();
                        })
                        .inspect_err(|error| log::debug!("the page closed: {error}"))
                        .ok();
                    }),
                );
            }
            menu
        })
}

/// "Stop this session?": what it does, Stop it and Cancel.
pub(crate) fn confirm(p: &Pal, cx: &Context<'_, SessionPage>) -> Div {
    div()
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .bg(p.text.opacity(0.18))
        .child(
            div()
                .id("stop-session-ask")
                .read_as(Role::AlertDialog, "Stop this session?")
                .w(px(360.))
                .max_w_full()
                .flex()
                .flex_col()
                .gap(px(10.))
                .p(px(16.))
                .rounded(px(14.))
                .border_1()
                .border_color(p.border)
                .bg(p.surface)
                .shadow_lg()
                .child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_size(px(15.))
                        .child("Stop this session?"),
                )
                .child(
                    div()
                        .text_size(px(12.5))
                        .text_color(p.muted)
                        .child("The step it's on ends now and nothing queued after it runs. A reply later carries it on."),
                )
                .child(
                    div()
                        .flex()
                        .justify_end()
                        .gap(px(8.))
                        .child(Button::new("stop-cancel").label("Cancel").on_click(cx.listener(
                            |page, _, _, cx| {
                                page.confirm_stop = false;
                                cx.notify();
                            },
                        )))
                        .child(
                            Button::new("stop-confirm")
                                .label("Stop it")
                                .danger()
                                .on_click(cx.listener(|page, _, _, cx| page.stop_all(cx))),
                        ),
                ),
        )
}

/// A new session about to open: where and on what, Opening… while its first message goes, and
/// why it didn't open.
pub(crate) fn draft_card(
    p: &Pal,
    where_line: &str,
    what_line: &str,
    opening: bool,
    problem: Option<&str>,
    cx: &gpui_kit::App,
) -> Div {
    let mut card = div()
        .flex()
        .flex_col()
        .gap(px(6.))
        .p(px(12.))
        .rounded(px(14.))
        .bg(p.card)
        .border_1()
        .border_color(p.border)
        .child(
            div()
                .font_weight(FontWeight::SEMIBOLD)
                .text_size(px(15.))
                .child("New session"),
        )
        .child(
            div()
                .text_size(px(12.))
                .text_color(p.muted)
                .child(where_line.to_owned()),
        )
        .child(
            div()
                .text_size(px(12.))
                .text_color(p.muted)
                .child(what_line.to_owned()),
        );
    if opening {
        card = card.child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .pt(px(6.))
                .id("draft-opening-line")
                .announced("Opening…")
                .child(spinner(p, "draft-opening", 14., cx))
                .child(div().text_size(px(13.)).child("Opening…")),
        );
    }
    if let Some(problem) = problem {
        card = card.child(
            div()
                .id("draft-problem")
                .announced(problem.to_owned())
                .pt(px(6.))
                .text_size(px(12.))
                .text_color(p.danger)
                .child(problem.to_owned()),
        );
    }
    card
}
