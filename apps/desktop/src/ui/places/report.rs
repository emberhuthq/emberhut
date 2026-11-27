//! The weekly report: the ultimate concern (or a way to put it into words), the tasks finished
//! per week as bars, every goal's ring (each opening its goal), each goal with the projects that
//! moved it this week, and the week's finished tasks with who did them. Goals and projects stay
//! out while the server hides them.

use emberhut_api::plan::WeekReport;
use emberhut_api::state::State;
use emberhut_api::tasks::ProjectHealth;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::prelude::*;
use gpui_kit::{Div, ElementId, Entity, FontWeight, div, px, relative};

use super::ring::ring;
use super::{MainWindow, body, gone, section, title, who};
use crate::brand::APP_NAME;
use crate::prose::direction;
use crate::store::goals::{label, progress, week_bars};
use crate::store::{Page, Store};
use crate::ui::a11y::{self, Accessible as _};
use crate::ui::column;
use crate::ui::palette::{Pal, room_color};
use crate::ui::reads::{self, reads};

/// The bars' height at most.
const BAR_HEIGHT: f32 = 64.;

/// The page.
pub(super) fn page(window: &mut MainWindow, p: &Pal, cx: &Context<'_, MainWindow>) -> Div {
    let store_entity = window.store.clone();
    let store = window.store.read(cx);
    let Some(state) = store.state() else {
        return gone(p, "Loading…");
    };
    let report = state.report.clone().unwrap_or_default();
    let ui = state.ui.unwrap_or_default();
    let mut page = body()
        .child(title("Weekly report"))
        .child(concern(p, &store_entity, &report));
    if let Some(bars) = bars(p, &report) {
        page = page.child(bars);
    }
    if !ui.hide_goals && !state.goals.is_empty() {
        page = page.child(rings(p, state));
        for goal in &report.goals {
            page = page.child(goal_section(p, state, goal, ui.hide_projects));
        }
    }
    if !report.done.is_empty() {
        page = page.child(section(p, "Finished this week"));
        for done in &report.done {
            page =
                page.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .py(px(4.))
                        .child(who(p, &done.owner))
                        .child(
                            div()
                                .flex_1()
                                .text_size(px(13.))
                                .line_clamp(2)
                                .child(direction::marked(&done.title)),
                        )
                        .children(done.project.clone().filter(|_| !ui.hide_projects).map(
                            |project| div().text_size(px(11.5)).text_color(p.muted).child(project),
                        )),
                );
        }
    }
    page
}

/// The ultimate concern, or `Not set yet` with a way to put it into words.
fn concern(p: &Pal, store: &Entity<Store>, report: &WeekReport) -> Div {
    let mut block = div().flex().flex_col().gap(px(4.)).pt(px(10.)).child(
        div()
            .text_size(px(11.))
            .font_weight(FontWeight::BOLD)
            .text_color(p.warn)
            .child("ULTIMATE CONCERN"),
    );
    if let Some(concern) = &report.concern {
        block = block
            .child(
                reads(&concern.title)
                    .text_size(px(16.))
                    .font_weight(FontWeight::BOLD),
            )
            .child(
                div()
                    .text_size(px(13.))
                    .text_color(p.muted)
                    .when(reads::mostly_right(&concern.why), Styled::text_right)
                    .child(concern.why.clone()),
            );
    } else {
        let store = store.clone();
        block = block
            .child(
                div()
                    .text_size(px(16.))
                    .font_weight(FontWeight::BOLD)
                    .child("Not set yet"),
            )
            .child(
                div().child(
                    Button::new("concern-talk")
                        .label(format!("Put it into words with {APP_NAME}"))
                        .ghost()
                        .small()
                        .on_click(move |_, _, cx| {
                            store.update(cx, |store, cx| {
                                store.start_message(
                                    "Let's put my ultimate concern into words: ",
                                    cx,
                                );
                            });
                        }),
                ),
            );
    }
    block
}

/// The tasks finished per week: a bar each, scaled to the busiest, this week's highlighted.
fn bars(p: &Pal, report: &WeekReport) -> Option<Div> {
    let bars = week_bars(report, &jiff::tz::TimeZone::system())?;
    let mut row = div().flex().items_end().gap(px(8.)).h(px(104.));
    for bar in &bars.bars {
        let color = if bar.now {
            p.warn
        } else {
            p.warn.opacity(0.35)
        };
        let share = f32::from(u16::try_from(bar.count).unwrap_or(u16::MAX))
            / f32::from(u16::try_from(bars.top).unwrap_or(u16::MAX));
        row = row.child(
            div()
                .flex()
                .flex_col()
                .items_center()
                .gap(px(4.))
                .flex_1()
                .max_w(px(72.))
                .child(
                    div()
                        .text_size(px(10.5))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(if bar.now { p.warn } else { p.muted })
                        .child(bar.count.to_string()),
                )
                .child(
                    div()
                        .w_full()
                        .h(px((BAR_HEIGHT * share).max(3.)))
                        .rounded(px(4.))
                        .bg(color),
                )
                .child(
                    div()
                        .text_size(px(10.5))
                        .text_color(p.muted)
                        .truncate()
                        .child(bar.label.clone()),
                ),
        );
    }
    Some(
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .pt(px(12.))
            .child(row)
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(p.muted)
                    .child(bars.caption),
            ),
    )
}

/// Every goal's ring this week, three to a row; a click opens the goal.
fn rings(p: &Pal, state: &State) -> Div {
    let mut grid = div().flex().flex_wrap().gap_y(px(14.)).pt(px(14.));
    for goal in &state.goals {
        let ring_of = progress(goal.id, &state.todos);
        let id = goal.id;
        grid = grid.child(
            div()
                .id(ElementId::Name(format!("report-ring-{id}").into()))
                // The ring says nothing: its goal and how far it is, as one line.
                .button_named(a11y::line(&[&label(goal), &ring_of.words()]))
                .w(relative(1. / 3.))
                .flex()
                .flex_col()
                .items_center()
                .gap(px(5.))
                .cursor_pointer()
                .child(ring(
                    ElementId::Name(format!("report-ring-mark-{id}").into()),
                    46.,
                    6.,
                    ring_of.share(),
                    room_color(Some(goal.rank), p),
                    p.faint.opacity(0.25),
                ))
                .child(
                    div()
                        .text_size(px(11.5))
                        .font_weight(FontWeight::MEDIUM)
                        .line_clamp(2)
                        .child(direction::marked(&label(goal))),
                )
                .child(
                    div()
                        .text_size(px(10.5))
                        .text_color(p.muted)
                        .child(ring_of.words()),
                )
                .on_click(move |_, window, cx| column::push(Page::Goal(id), window, cx)),
        );
    }
    div().flex().flex_col().child(grid).child(
        div()
            .pt(px(6.))
            .text_size(px(11.5))
            .text_color(p.muted)
            .child("Each ring: this week's tasks for that goal, done of all."),
    )
}

/// A goal with its projects this week; its title opens the goal, a project its page.
fn goal_section(
    p: &Pal,
    state: &State,
    goal: &emberhut_api::plan::ReportGoal,
    hide_projects: bool,
) -> Div {
    let id = goal.id;
    let ring_of = progress(id, &state.todos);
    let count = if ring_of.total == 0 {
        "nothing yet".to_owned()
    } else {
        format!("{} done", ring_of.words())
    };
    let mut block = div().flex().flex_col().pt(px(14.)).child(
        div()
            .id(ElementId::Name(format!("report-goal-{id}").into()))
            .button_named(a11y::line(&[&goal.title, &count]))
            .flex()
            .items_center()
            .gap(px(8.))
            .pb(px(4.))
            .cursor_pointer()
            .child(
                div()
                    .flex_1()
                    .text_size(px(13.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(direction::marked(&goal.title)),
            )
            .child(div().text_size(px(12.)).text_color(p.muted).child(count))
            .on_click(move |_, window, cx| column::push(Page::Goal(id), window, cx)),
    );
    if hide_projects {
        return block;
    }
    for project in &goal.projects {
        let pid = project.id;
        let health = match project.health {
            Some(ProjectHealth::Late) => Some(p.danger),
            Some(ProjectHealth::AtRisk) => Some(p.warn),
            Some(ProjectHealth::OnTrack) => Some(p.success),
            _ => None,
        };
        block = block.child(
            div()
                .id(ElementId::Name(format!("report-project-{pid}").into()))
                .button_named(a11y::line(&[
                    &project.name,
                    &format!("{} done · {} open", project.done_week, project.open),
                ]))
                .flex()
                .items_center()
                .gap(px(8.))
                .py(px(5.))
                .cursor_pointer()
                .hover(|d| d.bg(p.hover))
                .child(
                    div()
                        .flex_1()
                        .text_size(px(13.))
                        .child(direction::marked(&project.name)),
                )
                .children(health.map(|color| crate::ui::widgets::dot(color, 7.)))
                .child(div().text_size(px(12.)).text_color(p.muted).child(format!(
                    "{} done · {} open",
                    project.done_week, project.open
                )))
                .on_click(move |_, window, cx| column::push(Page::Project(pid), window, cx)),
        );
    }
    block
}
