//! A running turn as it happens: its to-do list and the step it is on, pinned over the box (the
//! word for what it does, the step, how long, how many tokens), its thinking as it is written,
//! its newest calls each with a glance at what it did (older ones folded), helper agents side
//! by side (a click opens one's own steps), and the green Live button back to the newest step.

use emberhut_api::runner::transcript::LiveAgent;
use emberhut_api::time::Timestamp;
use gpui_kit::assets::IconName;
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, Div, ElementId, Entity, FontWeight, div, px};

use super::page::SessionPage;
use super::reply;
use super::spark::{shimmer, spark};
use super::todos::todo_list;
use super::transcript::{Draw, diff_line, tool_line};
use super::{Open, push};
use crate::store::sessions::places::SessionPlace;
use crate::store::sessions::summary::Summary;
use crate::store::sessions::turns::{self, LIVE_CALLS, Peek, ToolCall};
use crate::store::sessions::view::SessionRef;
use crate::store::sessions::watch::SessionWatch;
use crate::store::sessions::words;
use crate::store::tasks::clock;
use crate::ui::a11y::{self, Accessible as _};
use crate::ui::palette::Pal;
use crate::ui::tasks::marks::glyph;

/// The foot of a session's page: the running turn's to-do list and its step, then the box (none
/// on a helper's page).
pub(crate) fn foot(
    page: &SessionPage,
    watch: &Entity<SessionWatch>,
    head: &Summary,
    p: &Pal,
    cx: &Context<'_, SessionPage>,
) -> Div {
    let view = watch.read(cx).view();
    let helper = view.reference().agent.is_some();
    let mut foot = div().flex().flex_col();
    let running_turn = view.head().and_then(|h| h.turn.as_ref());
    if head.running
        && !helper
        && let Some(todos) = turns::live_todos(view.steps(), running_turn)
    {
        let draw = Draw { page, p, cx };
        foot = foot.child(
            div()
                .px(px(16.))
                .py(px(8.))
                .border_t_1()
                .border_color(p.border)
                .child(todo_list(&draw, "live", todos, true)),
        );
    }
    if head.running {
        let helper_tokens =
            crate::store::sessions::summary::helper(view, page.store.read(cx).state())
                .and_then(|agent| agent.tokens);
        foot = foot.child(now_bar(watch, head, helper_tokens, p));
    }
    if !helper {
        foot = foot.child(reply::composer(page, head.direct, head.running, "", p, cx));
    }
    foot
}

/// The step a running turn is on: the word, the step, how long, the tokens.
fn now_bar(
    watch: &Entity<SessionWatch>,
    head: &Summary,
    helper_tokens: Option<f64>,
    p: &Pal,
) -> Div {
    let watch = watch.clone();
    div().border_t_1().border_color(p.border).child(NowBar {
        watch,
        doing: head.doing.clone(),
        since: head.since.clone(),
        helper_tokens,
    })
}

/// The pinned bar, drawn each second for its timer and its word.
#[derive(IntoElement)]
struct NowBar {
    watch: Entity<SessionWatch>,
    doing: Option<String>,
    since: Option<Timestamp>,
    helper_tokens: Option<f64>,
}

impl RenderOnce for NowBar {
    fn render(self, _: &mut gpui_kit::Window, cx: &mut gpui_kit::App) -> impl IntoElement {
        let p = crate::ui::palette::pal(cx);
        let view = self.watch.read(cx).view();
        let helper = view.reference().agent.is_some();
        let steps = view.steps();
        let now = turns::now(steps);
        let thinking = view.thinking().is_some();
        let typing = !view.typing().is_empty();
        let running_turn = view.head().and_then(|h| h.turn.as_ref());
        let todo = turns::live_todos(steps, running_turn).and_then(turns::todo_now);
        let instant = jiff::Timestamp::now();
        let still = crate::motion::reduced(cx);
        let tick = if still { 0 } else { words::word_tick(instant) };
        let word = words::live_word(thinking, typing, now.tool, todo, tick);
        let detail = if helper {
            self.doing.clone()
        } else if thinking || typing || now.tool.is_none() {
            None
        } else {
            self.doing.as_deref().map(words::step_detail)
        };
        let timer = self
            .since
            .as_ref()
            .and_then(clock::instant)
            .map(|since| words::timer(since, instant));
        let tokens = if helper {
            self.helper_tokens
        } else {
            view.tokens()
        }
        .filter(|t| *t > 0.);
        div()
            .flex()
            .items_center()
            .gap(px(10.))
            .px(px(16.))
            .py(px(8.))
            .bg(p.surface)
            .child(spark(p.accent, 18., cx))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .min_w_0()
                            .text_size(px(13.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(shimmer("now-word", format!("{word}…"), cx)),
                    )
                    .when_some(detail.filter(|d| !d.trim().is_empty()), |d, detail| {
                        d.child(
                            div()
                                .truncate()
                                .text_size(px(12.))
                                .text_color(p.muted)
                                .child(detail),
                        )
                    }),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_end()
                    .when_some(timer, |d, timer| {
                        d.child(div().text_size(px(12.)).text_color(p.muted).child(timer))
                    })
                    .when_some(tokens, |d, tokens| {
                        d.child(
                            div()
                                .text_size(px(11.))
                                .text_color(p.faint)
                                .child(format!("↓ {} tokens", words::tokens(tokens))),
                        )
                    }),
            )
    }
}

/// The green Live button, back to the newest step.
pub(crate) fn back_to_live(p: &Pal, cx: &Context<'_, SessionPage>) -> Div {
    div()
        .absolute()
        .bottom(px(8.))
        .left_0()
        .right_0()
        .flex()
        .justify_center()
        .child(
            div()
                .id("back-to-live")
                .button_named("Live: back to the newest step")
                .flex()
                .items_center()
                .gap(px(4.))
                .px(px(10.))
                .py(px(6.))
                .rounded_full()
                .bg(p.success)
                .text_color(p.on_accent)
                .text_size(px(12.))
                .font_weight(FontWeight::SEMIBOLD)
                .cursor_pointer()
                .child(glyph(IconName::ArrowDown, p.on_accent, 12.))
                .child("Live")
                .on_click(cx.listener(|page, _, _, cx| page.back_to_live(cx))),
        )
}

/// Its thinking as it is written: the newest lines, under a quiet Thinking with its timer.
pub(crate) fn thinking(
    p: &Pal,
    text: &str,
    since: Option<&Timestamp>,
    cx: &gpui_kit::App,
) -> AnyElement {
    let timer = since
        .and_then(clock::instant)
        .map(|since| words::timer(since, jiff::Timestamp::now()));
    let tail: String = {
        let chars: Vec<char> = text.chars().collect();
        chars[chars.len().saturating_sub(420)..].iter().collect()
    };
    div()
        .flex()
        .flex_col()
        .gap(px(4.))
        .pl(px(2.))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .child(spark(p.muted, 12., cx))
                .child(
                    div()
                        .text_size(px(12.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(p.muted)
                        .child("Thinking"),
                )
                .when_some(timer, |d, timer| {
                    d.child(div().text_size(px(11.)).text_color(p.faint).child(timer))
                }),
        )
        .when(!tail.trim().is_empty(), |d| {
            d.child(
                div()
                    .text_size(px(12.))
                    .italic()
                    .text_color(p.muted)
                    .line_clamp(5)
                    .child(tail),
            )
        })
        .into_any_element()
}

/// A running turn's calls: older ones folded into a count, the newest few each on a line (the
/// last two with a glance at what they did), helper agents started together side by side.
pub(crate) fn tools(draw: &Draw<'_>, calls: &[ToolCall<'_>]) -> AnyElement {
    let cut = calls.len().saturating_sub(LIVE_CALLS);
    let mut run = div().flex().flex_col().gap(px(5.));
    if cut > 0 {
        run = run.child(
            div()
                .text_size(px(12.))
                .text_color(draw.p.muted)
                .child(turns::folded(&calls[..cut])),
        );
    }
    let mut agents: Vec<&ToolCall<'_>> = Vec::new();
    for (i, call) in calls.iter().enumerate().skip(cut) {
        if call.is_agent() {
            agents.push(call);
            continue;
        }
        if !agents.is_empty() {
            run = run.child(agents_run(draw, &std::mem::take(&mut agents)));
        }
        run = run.child(tool_line(draw, call, true, i + 2 >= calls.len()));
    }
    if !agents.is_empty() {
        run = run.child(agents_run(draw, &agents));
    }
    run.into_any_element()
}

/// What a finished call did, in a glance.
pub(crate) fn peek(p: &Pal, call: &ToolCall<'_>) -> AnyElement {
    let lines: Vec<AnyElement> = match turns::peek(call) {
        Peek::Nothing => return div().into_any_element(),
        Peek::Diff(lines, more) => {
            let mut out: Vec<AnyElement> = lines
                .iter()
                .map(|line| diff_line(p, line).into_any_element())
                .collect();
            if more > 0 {
                out.push(faint(p, format!("+{more} more lines")));
            }
            out
        }
        Peek::Read(n) => vec![faint(
            p,
            format!("Read {n} line{}", if n == 1 { "" } else { "s" }),
        )],
        Peek::Found(0) => vec![faint(p, "Nothing found".to_owned())],
        Peek::Found(n) => vec![faint(
            p,
            format!("{n} result{}", if n == 1 { "" } else { "s" }),
        )],
        Peek::Lines(lines, before) => {
            let mut out = Vec::new();
            if before > 0 {
                out.push(faint(p, format!("… {before} more lines")));
            }
            let color = if call.failed() { p.danger } else { p.muted };
            out.extend(lines.iter().map(|line| {
                div()
                    .truncate()
                    .text_size(px(11.))
                    .font_family("monospace")
                    .text_color(color)
                    .child((*line).to_owned())
                    .into_any_element()
            }));
            out
        }
    };
    div()
        .ml(px(7.))
        .pl(px(9.))
        .border_l_1()
        .border_color(p.border)
        .flex()
        .flex_col()
        .text_size(px(11.))
        .font_family("monospace")
        .children(lines)
        .into_any_element()
}

fn faint(p: &Pal, text: String) -> AnyElement {
    div()
        .text_size(px(11.))
        .text_color(p.faint)
        .child(text)
        .into_any_element()
}

/// Helper agents started together: each with its state, what it is on, its tools and tokens; a
/// click opens its own steps.
fn agents_run(draw: &Draw<'_>, calls: &[&ToolCall<'_>]) -> AnyElement {
    let p = draw.p;
    let watch = draw.page.session().cloned();
    let (session, agents) = watch.as_ref().map_or((None, Vec::new()), |watch| {
        let view = watch.read(draw.cx).view();
        (
            Some(view.reference().id.clone()),
            view.head().map_or_default(|h| h.agents.clone()),
        )
    });
    let busy = calls.iter().filter(|c| is_running(&agents, c)).count();
    let mut card = div()
        .flex()
        .flex_col()
        .gap(px(6.))
        .p(px(10.))
        .rounded(px(12.))
        .bg(p.hover);
    if calls.len() > 1 {
        let line = match busy {
            0 => format!("{} agents done", calls.len()),
            1 => format!("1 of {} agents working", calls.len()),
            n => format!("{n} agents working in parallel"),
        };
        card = card.child(
            div()
                .text_size(px(11.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(p.muted)
                .child(line),
        );
    }
    for call in calls {
        card = card.child(agent_row(draw, &agents, session.as_ref(), call));
    }
    card.into_any_element()
}

/// Whether a helper's call still runs: as the runner counts it, else while it hasn't returned.
fn is_running(agents: &[LiveAgent], call: &ToolCall<'_>) -> bool {
    agents
        .iter()
        .find(|a| Some(&a.id) == call.call.call.as_ref())
        .and_then(|a| a.running)
        .unwrap_or(call.result.is_none())
}

/// One helper of a run: its mark, name, what it is on, its counts; a click opens its steps.
fn agent_row(
    draw: &Draw<'_>,
    agents: &[LiveAgent],
    session: Option<&emberhut_api::ids::SessionId>,
    call: &ToolCall<'_>,
) -> AnyElement {
    let p = draw.p;
    let info = agents
        .iter()
        .find(|a| Some(&a.id) == call.call.call.as_ref());
    let name = info
        .and_then(|a| a.description.clone())
        .or_else(|| call.call.summary.clone())
        .unwrap_or_else(|| "Agent".to_owned());
    let on = is_running(agents, call);
    let tools = info
        .and_then(|a| a.tools)
        .map_or_else(|| call.helper_calls(), |n| n as usize);
    let counts = [
        (tools > 0).then(|| format!("{tools} tool{}", if tools == 1 { "" } else { "s" })),
        info.and_then(|a| a.tokens)
            .filter(|t| *t > 0.)
            .map(|t| format!("{} tok", words::tokens(t))),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" · ");
    let spoken = a11y::line(&[&name, &counts]);
    let now = call
        .inside
        .iter()
        .rev()
        .find(|s| s.kind == emberhut_api::runner::steps::StepKind::Tool)
        .and_then(|s| s.doing.as_deref().map(words::step_detail))
        .or_else(|| info.and_then(|a| a.last.clone()));
    let timer = on
        .then(|| {
            info.and_then(|a| a.started.as_ref())
                .or(call.call.at.as_ref())
                .and_then(clock::instant)
        })
        .flatten()
        .map(|since| words::timer(since, jiff::Timestamp::now()));
    let mark: AnyElement = if on {
        spark(p.accent, 13., draw.cx).into_any_element()
    } else if call.failed() {
        glyph(IconName::CircleX, p.danger, 13.).into_any_element()
    } else {
        glyph(IconName::CircleCheck, p.success, 13.).into_any_element()
    };
    let mut row = div()
        .id(ElementId::Name(
            format!("agent-row-{}", call.call.seq).into(),
        ))
        .flex()
        .items_start()
        .gap(px(8.))
        .child(div().pt(px(2.)).child(mark))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .truncate()
                        .text_size(px(12.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(name.clone()),
                )
                .when_some(now.filter(|_| on), |d, now| {
                    d.child(
                        div()
                            .truncate()
                            .text_size(px(11.))
                            .font_family("monospace")
                            .text_color(p.muted)
                            .child(now),
                    )
                }),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .items_end()
                .when_some(timer, |d, timer| {
                    d.child(div().text_size(px(11.)).text_color(p.muted).child(timer))
                })
                .child(div().text_size(px(11.)).text_color(p.faint).child(counts)),
        );
    if let (Some(session), Some(call_id)) = (session.cloned(), call.call.call.clone()) {
        let store = draw.page.store.clone();
        // A helper that opens reads as one button: its name and what it did.
        row = row.button_named(spoken).hint("Opens its own steps");
        row = row.cursor_pointer().on_click(move |_, window, cx| {
            let place = SessionPlace::Session(SessionRef::helper(
                session.clone(),
                call_id.clone(),
                name.clone(),
            ));
            push(&store, Open::Place(place), window, cx);
        });
    }
    row.into_any_element()
}
