//! A session's transcript as its page draws it, turn by turn: a message to it in a bubble that
//! says who sent it (folded past six lines, its files under it), the model's words as markdown
//! (a long answer folded until Show more), the running turn's newest calls each on its own
//! line and a finished turn's folded into one (`12 steps · Ran the tests · 1 failed`), a call
//! opening to its input and output (an edit's diff, a helper's own steps), a finished thought,
//! a question card whose options answer it while it waits, the to-do list, the turn's end and
//! the runner's notes. The words, messages and blocks can be selected and copied.

use emberhut_api::runner::steps::{Sender, SessionStep, StepKind};
use gpui_kit::assets::IconName;
use gpui_kit::base::SelectableText;
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, Div, ElementId, Entity, FontWeight, SharedString, Stateful, div, px};

use super::live;
use super::page::SessionPage;
use crate::store::sessions::summary::Summary;
use crate::store::sessions::turns::{
    self, BLOCK_LINES, DIFF_LINES, Item, ToolCall, ToolIcon, is_long_block,
};
use crate::store::sessions::watch::SessionWatch;
use crate::store::sessions::words;
use crate::ui::a11y::{self, Accessible as _};
use crate::ui::controls::tab_stop;
use crate::ui::palette::Pal;
use crate::ui::reads;
use crate::ui::tasks::marks::glyph;

/// How many lines a message to the session shows until Show more.
const MESSAGE_LINES: usize = 6;

/// A long answer: past this many characters or lines it folds.
const LONG_CHARS: usize = 1200;
const LONG_LINES: usize = 18;

/// What a part of the transcript is drawn with.
pub(crate) struct Draw<'a> {
    pub(crate) page: &'a SessionPage,
    pub(crate) p: &'a Pal,
    pub(crate) cx: &'a Context<'a, SessionPage>,
}

impl Draw<'_> {
    /// A fold's toggle: a click opens or folds part `key`.
    pub(super) fn toggle(
        &self,
        key: String,
    ) -> impl Fn(&gpui_kit::ClickEvent, &mut gpui_kit::Window, &mut gpui_kit::App) + 'static {
        self.cx
            .listener(move |page, _, _, cx| page.toggle(&key, cx))
    }

    /// Whether part `key` was opened (or, for a part open by itself, folded).
    pub(super) fn open(&self, key: &str) -> bool {
        self.page.is_open(key)
    }

    /// A link that opens or folds part `key`: `more` while folded, `less` once open.
    fn more(
        &self,
        key: String,
        open: bool,
        more: impl Into<SharedString>,
        less: &'static str,
    ) -> Stateful<Div> {
        let label: SharedString = if open { less.into() } else { more.into() };
        tab_stop(
            div()
                .id(ElementId::Name(format!("more-{key}").into()))
                .role(gpui_kit::Role::Button)
                .aria_expanded(open)
                .border_1()
                .rounded(px(4.))
                .text_size(px(12.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(self.p.accent)
                .cursor_pointer()
                .child(label)
                .on_click(self.toggle(key)),
            self.p,
        )
    }
}

/// The scroller's content: the loading spinner, the note about older steps, the turns, the
/// thinking and words being written.
pub(crate) fn body(
    page: &SessionPage,
    watch: &Entity<SessionWatch>,
    head: &Summary,
    p: &Pal,
    cx: &Context<'_, SessionPage>,
) -> Div {
    let draw = Draw { page, p, cx };
    let view = watch.read(cx).view();
    let mut body = div().flex().flex_col().gap(px(12.)).px(px(16.)).py(px(10.));
    if !view.is_loaded() {
        body = body.child(
            div()
                .flex()
                .justify_center()
                .pt(px(30.))
                .child(super::marks::spinner(p, "session-loading", 18., cx)),
        );
    }
    if view.has_earlier() {
        body = body.child(
            div()
                .text_size(px(12.))
                .text_color(p.muted)
                .child("Older steps aren't shown here."),
        );
    }
    let steps = view.steps();
    let live_turn = view
        .head()
        .and_then(|h| h.turn.as_ref())
        .or_else(|| steps.last().and_then(|s| s.turn.as_ref()));
    let asking = (head.status == "waiting")
        .then(|| turns::asking(steps))
        .flatten();
    let all = turns::turns(steps);
    for turn in &all {
        let live = head.running && turn.id.is_some() && turn.id == live_turn;
        let queued = turns::is_queued(
            turn.id,
            view.head().and_then(|h| h.turn.as_ref()),
            head.running,
        );
        let items = turns::items(&turn.steps, None);
        body = body.child(turn_view(&draw, &items, live, queued, asking));
    }
    let helper = view.reference().agent.is_some();
    if head.running
        && !helper
        && let Some((text, since)) = view.thinking()
    {
        body = body.child(live::thinking(p, text, since, cx));
    }
    if !view.typing().is_empty() {
        body = body.child(div().px(px(2.)).child(crate::ui::prose::markdown_view(
            "session-typing".into(),
            view.typing(),
            &page.store,
        )));
    }
    body
}

/// One turn's items, with a divider under it.
fn turn_view(
    draw: &Draw<'_>,
    items: &[Item<'_>],
    live: bool,
    queued: bool,
    asking: Option<u64>,
) -> Div {
    let last_todos = items
        .iter()
        .rev()
        .find(|item| matches!(item, Item::Todos(_)))
        .map(Item::seq);
    let last_tools = items
        .iter()
        .rev()
        .find(|item| matches!(item, Item::Tools(_)))
        .map(Item::seq);
    let mut turn = div()
        .flex()
        .flex_col()
        .gap(px(10.))
        .pb(px(10.))
        .border_b_1()
        .border_color(draw.p.border);
    for item in items {
        turn = turn.child(match item {
            Item::Said(step) => said(draw, step, queued),
            Item::Tools(calls) if live && Some(item.seq()) == last_tools => {
                live::tools(draw, calls)
            }
            Item::Tools(calls) => folded(draw, calls),
            Item::Note(step) => note(draw.p, step),
            Item::Question(step) => question(draw, step, Some(step.seq) == asking),
            Item::Todos(step) if !live && Some(step.seq) == last_todos => {
                super::todos::todo_list(draw, &step.seq.to_string(), &step.todos, false)
            }
            Item::Todos(_) => div().into_any_element(),
        });
    }
    turn
}

/// A message to the session, or the model's words.
fn said(draw: &Draw<'_>, step: &SessionStep, queued: bool) -> AnyElement {
    let p = draw.p;
    if step.kind != StepKind::User {
        return long_text(draw, step.seq, step.text.as_deref().unwrap_or_default());
    }
    let mine = step.by == Some(Sender::Person);
    let who_color = match step.by {
        Some(Sender::Person) => p.accent,
        Some(Sender::Assistant) => p.info,
        _ => p.muted,
    };
    let text = words::brief(step.text.as_deref().unwrap_or_default());
    let files_only =
        !step.files.is_empty() && text == emberhut_api::runner::actions::FILES_ONLY_TEXT;
    let mut bubble = div()
        .flex()
        .flex_col()
        .gap(px(4.))
        .p(px(10.))
        .rounded(px(12.))
        .bg(if mine { p.accent_soft } else { p.hover })
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .child(
                    div()
                        .text_size(px(12.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(who_color)
                        .child(words::sender(step.by.as_ref())),
                )
                .when(queued, |d| {
                    d.child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(3.))
                            .text_size(px(11.))
                            .text_color(p.muted)
                            .child(glyph(IconName::Clock, p.muted, 11.))
                            .child("Queued"),
                    )
                }),
        );
    if !text.trim().is_empty() && !files_only {
        let key = format!("message-{}", step.seq);
        let lines: Vec<&str> = text.lines().collect();
        let long = lines.len() > MESSAGE_LINES;
        let open = draw.open(&key);
        let shown = if long && !open {
            lines[..MESSAGE_LINES].join("\n")
        } else {
            text.to_owned()
        };
        let from_right = reads::starts_right(&shown);
        bubble = bubble.child(
            div()
                .text_size(px(13.))
                .when(from_right, Styled::text_right)
                .child(SelectableText::new(
                    ElementId::Name(format!("{key}-{open}").into()),
                    shown,
                )),
        );
        if long {
            bubble = bubble.child(draw.more(key, open, "Show more", "Show less"));
        }
    }
    if !step.files.is_empty() {
        let mut files = div().flex().flex_wrap().gap(px(6.));
        for file in &step.files {
            files = files.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .px(px(6.))
                    .py(px(2.))
                    .rounded(px(6.))
                    .border_1()
                    .border_color(p.border)
                    .text_size(px(11.5))
                    .child(glyph(IconName::File, p.muted, 12.))
                    .child(file.name.clone()),
            );
        }
        bubble = bubble.child(files);
    }
    bubble.into_any_element()
}

/// The model's words: markdown, a long answer folded to its first paragraphs until Show more.
fn long_text(draw: &Draw<'_>, seq: u64, text: &str) -> AnyElement {
    let key = format!("text-{seq}");
    let long = text.chars().count() > LONG_CHARS || text.lines().count() > LONG_LINES;
    let open = draw.open(&key);
    let shown = if long && !open {
        turns::folded_text(text)
    } else {
        text
    };
    div()
        .flex()
        // An answer that starts in Arabic reads from the right edge, as the chat's do.
        .when(reads::starts_right(shown), Styled::text_right)
        .flex_col()
        .gap(px(6.))
        .px(px(2.))
        .child(
            crate::ui::prose::markdown_view(
                ElementId::Name(format!("session-text-{seq}-{open}").into()),
                shown,
                &draw.page.store,
            )
            .selectable(true),
        )
        .when(long, |d| {
            d.child(draw.more(key, open, "Show more", "Show less"))
        })
        .into_any_element()
}

/// A turn's end (`Turn 3 · done`) or the runner's own note (moved account, restarted).
fn note(p: &Pal, step: &SessionStep) -> AnyElement {
    if step.kind == StepKind::End {
        let outcome = match step.outcome.as_deref() {
            Some("completed") => "done",
            Some(other) => other,
            None => "",
        };
        return div()
            .text_size(px(11.5))
            .text_color(p.faint)
            .child(format!("Turn {} · {outcome}", step.n.unwrap_or(0)))
            .into_any_element();
    }
    let text = step.text.clone().unwrap_or_default();
    let icon = if text.starts_with("Moved") {
        IconName::ArrowLeftRight
    } else {
        IconName::Info
    };
    div()
        .flex()
        .items_center()
        .gap(px(6.))
        .text_size(px(12.))
        .text_color(p.warn)
        .child(glyph(icon, p.warn, 12.))
        .child(text)
        .into_any_element()
}

/// A finished turn's calls, folded: `12 steps · Ran the tests · 1 failed`; open, each call.
fn folded(draw: &Draw<'_>, calls: &[ToolCall<'_>]) -> AnyElement {
    let p = draw.p;
    let first = calls.first().map_or(0, |c| c.call.seq);
    let key = format!("tools-{first}");
    let open = draw.open(&key);
    let failed = calls.iter().any(ToolCall::failed);
    let line = turns::folded(calls);
    let mut fold = div().flex().flex_col().gap(px(4.)).child(tab_stop(
        div()
            .id(ElementId::Name(key.clone().into()))
            // Read as its line, open or folded.
            .button_named(line.clone())
            .aria_expanded(open)
            .aria_value(a11y::folded(open))
            .border_1()
            .rounded(px(6.))
            .flex()
            .items_center()
            .gap(px(6.))
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
            .child(glyph(IconName::Wrench, p.muted, 12.))
            .child(
                div()
                    .truncate()
                    .text_size(px(12.5))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(if failed { p.danger } else { p.text })
                    .child(line),
            )
            .on_click(draw.toggle(key)),
        p,
    ));
    if open {
        for call in calls {
            fold = fold.child(tool_line(draw, call, false, false));
        }
    }
    fold.into_any_element()
}

/// One call as a compact line (or a finished thought); open, its input and output, and a
/// helper's own steps.
pub(crate) fn tool_line(
    draw: &Draw<'_>,
    call: &ToolCall<'_>,
    live: bool,
    peek: bool,
) -> AnyElement {
    if call.is_thought() {
        return thought(draw, call.call);
    }
    let p = draw.p;
    let step = call.call;
    let key = format!("call-{}", step.seq);
    let open = draw.open(&key);
    let icon = match ToolIcon::of(step.name.as_deref()) {
        ToolIcon::Terminal => IconName::Terminal,
        ToolIcon::Document => IconName::FileText,
        ToolIcon::Pencil => IconName::Pencil,
        ToolIcon::Search => IconName::Search,
        ToolIcon::People => IconName::Users,
        ToolIcon::Globe => IconName::Globe,
        ToolIcon::Checklist => IconName::ListChecks,
        ToolIcon::Wrench => IconName::Wrench,
    };
    let mut head = div()
        .id(ElementId::Name(key.clone().into()))
        // Read as the call's line, failed or not, open or folded.
        .button_named(a11y::line(&[
            &turns::tool_line(step),
            if call.failed() { "failed" } else { "" },
        ]))
        .aria_expanded(open)
        .aria_value(a11y::folded(open))
        .border_1()
        .rounded(px(6.))
        .flex()
        .items_center()
        .gap(px(7.))
        .cursor_pointer()
        .child(div().w(px(16.)).child(glyph(icon, p.muted, 12.)))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .when(!open, Styled::truncate)
                .text_size(px(12.))
                .font_family("monospace")
                .child(turns::tool_line(step)),
        );
    if call.result.is_none() && live {
        head = head.child(super::marks::spinner(
            p,
            ElementId::Name(format!("spin-{key}").into()),
            12.,
            draw.cx,
        ));
    } else if call.failed() {
        head = head.child(glyph(IconName::CircleX, p.danger, 12.));
    }
    if !call.inside.is_empty() {
        head = head.child(
            div()
                .text_size(px(11.))
                .text_color(p.muted)
                .child(call.helper_calls().to_string()),
        );
    }
    head = head.child(glyph(
        if open {
            IconName::ChevronUp
        } else {
            IconName::ChevronDown
        },
        p.faint,
        10.,
    ));
    let mut column = div()
        .flex()
        .flex_col()
        .gap(px(6.))
        .py(px(1.))
        .child(tab_stop(head.on_click(draw.toggle(key.clone())), p));
    if peek && !open && !call.is_agent() {
        column = column.child(live::peek(p, call));
    }
    if open {
        if let Some(diff) = step.diff.as_deref().filter(|d| !d.trim().is_empty()) {
            column = column.child(diff_block(draw, &key, diff));
        } else if let Some(input) = step.input.as_deref().filter(|i| !i.trim().is_empty()) {
            column = column.child(block(draw, &format!("{key}-input"), "Input", input, false));
        }
        if let Some(result) = call.result
            && let Some(out) = result.text.as_deref().filter(|o| !o.trim().is_empty())
        {
            let failed = result.failed();
            column = column.child(block(
                draw,
                &format!("{key}-output"),
                if failed { "Error" } else { "Output" },
                out,
                failed,
            ));
        }
        if !call.inside.is_empty() {
            column = column.child(helper_steps(draw, call, live));
        }
    }
    column.into_any_element()
}

/// A helper agent's own steps inside its call, indented with a line beside them.
fn helper_steps(draw: &Draw<'_>, call: &ToolCall<'_>, live: bool) -> Div {
    let p = draw.p;
    let mut steps = div()
        .flex()
        .flex_col()
        .gap(px(4.))
        .pl(px(12.))
        .border_l_2()
        .border_color(p.border)
        .child(
            div()
                .text_size(px(11.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(p.muted)
                .child("Helper agent's steps"),
        );
    for item in turns::items(&call.inside, call.call.call.as_ref()) {
        steps = steps.child(match &item {
            Item::Said(step) => div()
                .text_size(px(12.))
                .line_clamp(8)
                .child(step.text.clone().unwrap_or_default())
                .into_any_element(),
            Item::Tools(calls) => {
                let mut run = div().flex().flex_col().gap(px(4.));
                for call in calls {
                    run = run.child(tool_line(draw, call, live, false));
                }
                run.into_any_element()
            }
            Item::Note(step) => note(p, step),
            Item::Question(step) => question(draw, step, false),
            Item::Todos(_) => div().into_any_element(),
        });
    }
    steps
}

/// A finished thinking block: `Thought for 6s`; open, what it thought, when it shared it.
fn thought(draw: &Draw<'_>, step: &SessionStep) -> AnyElement {
    let p = draw.p;
    let text = step.text.clone().unwrap_or_default();
    let has = !text.trim().is_empty();
    let key = format!("thought-{}", step.seq);
    let open = draw.open(&key);
    let label = step.ms.map_or_else(
        || "Thought".to_owned(),
        |ms| format!("Thought for {}s", (ms / 1000).max(1)),
    );
    let label_said = label.clone();
    let mut row = div()
        .id(ElementId::Name(key.clone().into()))
        .border_1()
        .rounded(px(6.))
        .flex()
        .items_center()
        .gap(px(7.))
        .child(div().w(px(16.)).child(glyph(IconName::Brain, p.muted, 12.)))
        .child(
            div()
                .text_size(px(12.))
                .italic()
                .text_color(p.muted)
                .child(label),
        );
    if has {
        row = row
            .button_named(label_said)
            .aria_expanded(open)
            .aria_value(a11y::folded(open))
            .cursor_pointer()
            .child(div().flex_1())
            .child(glyph(
                if open {
                    IconName::ChevronUp
                } else {
                    IconName::ChevronDown
                },
                p.faint,
                10.,
            ))
            .on_click(draw.toggle(key));
    }
    div()
        .flex()
        .flex_col()
        .gap(px(4.))
        .child(tab_stop(row, p))
        .when(open && has, |d| {
            d.child(
                div()
                    .pl(px(23.))
                    .text_size(px(12.))
                    .italic()
                    .text_color(p.muted)
                    .child(text),
            )
        })
        .into_any_element()
}

/// A tool's input or output, monospaced, twelve lines until More; red for an error.
fn block(draw: &Draw<'_>, key: &str, title: &'static str, text: &str, red: bool) -> AnyElement {
    let p = draw.p;
    let open = draw.open(key);
    let long = is_long_block(text);
    let shown: SharedString = if long && !open {
        text.lines()
            .take(BLOCK_LINES)
            .collect::<Vec<_>>()
            .join("\n")
            .into()
    } else {
        text.to_owned().into()
    };
    div()
        .flex()
        .flex_col()
        .gap(px(3.))
        .p(px(8.))
        .rounded(px(8.))
        .bg(p.hover)
        .child(
            div()
                .text_size(px(11.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(p.muted)
                .child(title),
        )
        .child(
            div()
                .text_size(px(11.5))
                .font_family("monospace")
                .text_color(if red { p.danger } else { p.text })
                .child(SelectableText::new(
                    ElementId::Name(format!("block-{key}-{open}").into()),
                    shown,
                )),
        )
        .when(long, |d| {
            d.child(draw.more(key.to_owned(), open, "More", "Less"))
        })
        .into_any_element()
}

/// An edit's changed lines: removed in red, added in green, sixteen until the rest.
fn diff_block(draw: &Draw<'_>, key: &str, diff: &str) -> AnyElement {
    let p = draw.p;
    let key = format!("{key}-diff");
    let open = draw.open(&key);
    let lines: Vec<&str> = diff.lines().collect();
    let more = lines.len().saturating_sub(DIFF_LINES);
    let mut block = div()
        .flex()
        .flex_col()
        .p(px(8.))
        .rounded(px(8.))
        .bg(p.hover)
        .text_size(px(11.5))
        .font_family("monospace");
    for line in lines
        .iter()
        .take(if open { lines.len() } else { DIFF_LINES })
    {
        block = block.child(diff_line(p, line));
    }
    if more > 0 {
        block = block.child(
            draw.more(key, open, format!("{more} more lines"), "Less")
                .mt(px(4.)),
        );
    }
    block.into_any_element()
}

/// One line of a diff, coloured by what happened to it.
pub(crate) fn diff_line(p: &Pal, line: &str) -> Div {
    let (color, fill) = if line.starts_with("- ") {
        (p.danger, Some(p.danger.opacity(0.08)))
    } else if line.starts_with("+ ") {
        (p.success, Some(p.success.opacity(0.08)))
    } else {
        (p.muted, None)
    };
    div()
        .w_full()
        .text_color(color)
        .when_some(fill, Styled::bg)
        .child(if line.is_empty() {
            " ".to_owned()
        } else {
            line.to_owned()
        })
}

/// What a session asks the user: each question with its options; while it waits, a click on
/// one answers it.
fn question(draw: &Draw<'_>, step: &SessionStep, asking: bool) -> AnyElement {
    let p = draw.p;
    let mut card = div()
        .flex()
        .flex_col()
        .gap(px(10.))
        .p(px(10.))
        .rounded(px(12.))
        .bg(p.accent_soft)
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .text_size(px(12.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(p.accent)
                .child(glyph(IconName::MessageCircle, p.accent, 12.))
                .child(if asking {
                    "It asks you"
                } else {
                    "It asked you"
                }),
        );
    for (qi, q) in step.questions.iter().enumerate() {
        let mut part = div().flex().flex_col().gap(px(6.));
        if let Some(header) = q.header.as_deref().filter(|h| !h.trim().is_empty()) {
            part = part.child(
                div()
                    .text_size(px(11.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(p.muted)
                    .child(header.to_uppercase()),
            );
        }
        part = part.child(
            div()
                .text_size(px(13.))
                .font_weight(FontWeight::MEDIUM)
                .when(reads::starts_right(&q.question), Styled::text_right)
                .child(q.question.clone()),
        );
        for (oi, option) in q.options.iter().enumerate() {
            let answer = turns::answer(step, qi, oi);
            let mut button = div()
                .id(ElementId::Name(
                    format!("answer-{}-{qi}-{oi}", step.seq).into(),
                ))
                // Read as one answer: its label and what it means.
                .button_named(a11y::line(&[
                    &option.label,
                    option.description.as_deref().unwrap_or_default(),
                ]))
                .border_1()
                .flex()
                .flex_col()
                .gap(px(2.))
                .px(px(10.))
                .py(px(7.))
                .rounded(px(10.))
                .bg(p.surface)
                .child(
                    div()
                        .text_size(px(13.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(option.label.clone()),
                )
                .when_some(
                    option.description.clone().filter(|d| !d.trim().is_empty()),
                    |d, description| {
                        d.child(
                            div()
                                .text_size(px(12.))
                                .text_color(p.muted)
                                .child(description),
                        )
                    },
                );
            if asking && let Some(answer) = answer {
                button =
                    button
                        .cursor_pointer()
                        .hover(|d| d.bg(p.hover))
                        .on_click(draw.cx.listener(move |page, _, _, cx| {
                            page.answer(&answer, cx);
                        }));
            } else {
                button = button.opacity(0.6);
            }
            part = part.child(tab_stop(button, p));
        }
        card = card.child(part);
    }
    if asking {
        card = card.child(
            div()
                .text_size(px(12.))
                .text_color(p.muted)
                .child("Or write your own answer below."),
        );
    }
    card.into_any_element()
}
