//! Agents on a task's run: ultra mode's summary (parts done, its state, a bar, what it spent and
//! planned), the run's lead with what it does and what it used, and every sub-agent in a tree
//! with its state, how long it ran, its latest step and what it used. The page is asked for
//! again every few seconds while they run, and their timers tick with it.

use emberhut_api::ultra::{AgentKind, AgentState, UltraAgent, UltraLead, UltraRun};
use gpui_kit::assets::IconName;
use gpui_kit::prelude::*;
use gpui_kit::{Div, FontWeight, Hsla, div, px};

use super::{Part, section as heading};
use crate::store::sessions::places::SessionPlace;
use crate::store::sessions::view::SessionRef;
use crate::store::tasks::agents;
use crate::ui::palette::Pal;
use crate::ui::sessions;
use crate::ui::tasks::marks::{glyph, progress, tone_color};
use emberhut_api::ids::{SessionId, ToolCallId};
use gpui_kit::ElementId;

/// The section, when the page has a run of agents to show.
pub(super) fn section(mut body: Div, part: &Part<'_>) -> Div {
    let Some(run) = part
        .page
        .and_then(|page| page.ultra_run.as_ref())
        .filter(|run| run.shows())
    else {
        return body;
    };
    let p = part.p;
    let now = part.now.timestamp();
    let mut head = div()
        .flex()
        .items_center()
        .child(div().flex_1().child(heading(p, "Agents")));
    if let Some(working) = agents::working_line(run) {
        head = head.child(
            div()
                .pt(px(10.))
                .text_size(px(11.5))
                .text_color(p.success)
                .child(working),
        );
    }
    body = body.child(head);
    if run.on {
        body = body.child(summary(p, run));
    }
    if let Some(lead) = &run.lead {
        let row = lead_row(p, lead, run.agents.len(), now);
        let place = SessionPlace::Session(SessionRef::whole(SessionId::from(lead.viewer.clone())));
        body = body.child(sessions::link(part.store, place, "agent-lead", row, p));
    }
    for agent in &run.agents {
        let row = agent_row(p, agent, agents::level(agent, run), now);
        body = body.child(match &agent.viewer {
            Some(viewer) => {
                let name = agent.name.clone().unwrap_or_else(|| "Agent".to_owned());
                let reference = SessionRef::of_agent(
                    SessionId::from(viewer.clone()),
                    agent.parent.clone().map(ToolCallId::from),
                    name,
                );
                let id = ElementId::Name(format!("agent-{}", agent.id).into());
                sessions::link(part.store, SessionPlace::Session(reference), id, row, p)
                    .into_any_element()
            }
            None => row.into_any_element(),
        });
    }
    body
}

/// Ultra mode's summary.
fn summary(p: &Pal, run: &UltraRun) -> Div {
    let (state, tone) = agents::run_state(run);
    let fill = if matches!(tone, crate::store::tasks::words::Tone::Accent) {
        p.accent
    } else {
        p.info
    };
    div()
        .flex()
        .flex_col()
        .gap(px(4.))
        .pb(px(8.))
        .child(
            div()
                .flex()
                .items_baseline()
                .child(
                    div()
                        .flex_1()
                        .text_size(px(13.))
                        .font_weight(FontWeight::MEDIUM)
                        .child(agents::parts_line(run)),
                )
                .child(
                    div()
                        .text_size(px(11.5))
                        .text_color(tone_color(p, tone))
                        .child(state),
                ),
        )
        .child(progress(p, agents::parts_done(run), fill))
        .child(
            div()
                .text_size(px(11.5))
                .text_color(p.muted)
                .child(agents::usage_line(run)),
        )
}

/// An agent's state mark: spinning while it runs, a clock queued, a tick (a seal for a check)
/// done, a cross failed, a stop sign stopped.
fn state_mark(p: &Pal, state: Option<&AgentState>, verify: bool, size: f32) -> impl IntoElement {
    let (icon, color): (IconName, Hsla) = match state {
        Some(AgentState::Running) => (IconName::LoaderCircle, p.success),
        Some(AgentState::Queued) => (IconName::Clock, p.muted),
        Some(AgentState::Done) if verify => (IconName::BadgeCheck, p.success),
        Some(AgentState::Done) => (IconName::CircleCheck, p.success),
        Some(AgentState::Failed) => (IconName::CircleX, p.danger),
        _ => (IconName::CircleStop, p.muted),
    };
    glyph(icon, color, size)
}

/// How long it ran, green while it runs.
fn elapsed(p: &Pal, text: Option<String>, running: bool) -> Option<Div> {
    text.map(|text| {
        div()
            .text_size(px(11.5))
            .text_color(if running { p.success } else { p.faint })
            .child(text)
    })
}

/// The lead's row.
fn lead_row(p: &Pal, lead: &UltraLead, helpers: usize, now: jiff::Timestamp) -> Div {
    let time = agents::elapsed(
        lead.started.as_ref(),
        lead.ended.as_ref(),
        lead.running,
        now,
    );
    div()
        .flex()
        .items_start()
        .gap(px(10.))
        .py(px(4.))
        .child(
            div()
                .pt(px(1.))
                .child(state_mark(p, Some(&lead.state), false, 18.)),
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
                        .items_baseline()
                        .gap(px(6.))
                        .child(
                            div()
                                .flex_1()
                                .truncate()
                                .text_size(px(13.5))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(lead.name.clone()),
                        )
                        .children(elapsed(p, time, lead.running)),
                )
                .child(
                    div()
                        .text_size(px(12.5))
                        .text_color(p.muted)
                        .line_clamp(2)
                        .child(agents::lead_doing(lead)),
                )
                .child(
                    div()
                        .truncate()
                        .text_size(px(11.))
                        .text_color(p.faint)
                        .child(agents::lead_line(lead, helpers)),
                ),
        )
}

/// A sub-agent's row, indented by its level with the tree's line down its side.
fn agent_row(p: &Pal, agent: &UltraAgent, level: usize, now: jiff::Timestamp) -> Div {
    let running = agent.state == Some(AgentState::Running);
    let time = agents::elapsed(agent.started.as_ref(), agent.ended.as_ref(), running, now);
    // Level 2 sits one step further in than level 1.
    let indent = if level > 1 { 18.0 } else { 0.0 };
    let small = agents::agent_line(agent);
    let row = div()
        .flex()
        .items_start()
        .gap(px(10.))
        .py(px(4.))
        .child(div().pt(px(2.)).child(state_mark(
            p,
            agent.state.as_ref(),
            agent.kind == Some(AgentKind::Verify),
            14.,
        )))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .gap(px(1.))
                .child(
                    div()
                        .flex()
                        .items_baseline()
                        .gap(px(6.))
                        .child(
                            div()
                                .flex_1()
                                .truncate()
                                .text_size(px(12.5))
                                .when(running, |d| d.font_weight(FontWeight::MEDIUM))
                                .child(
                                    agent
                                        .name
                                        .clone()
                                        .unwrap_or_else(|| format!("Agent {}", agent.id)),
                                ),
                        )
                        .children(elapsed(p, time, running)),
                )
                .children(
                    agent
                        .last
                        .clone()
                        .filter(|last| !last.is_empty())
                        .map(|last| {
                            div()
                                .text_size(px(11.5))
                                .text_color(p.muted)
                                .line_clamp(2)
                                .child(last)
                        }),
                )
                .when(!small.is_empty(), |d| {
                    d.child(
                        div()
                            .truncate()
                            .text_size(px(11.))
                            .text_color(p.faint)
                            .child(small),
                    )
                }),
        );
    if level == 0 {
        return row;
    }
    div()
        .flex()
        .pl(px(indent + 2.))
        .child(div().w(px(1.5)).flex_none().bg(p.border).mr(px(10.)))
        .child(row.flex_1())
}
