//! A decision on its task's page, laid out to decide in a glance: why it is asked now and by
//! when, what the user needs to know with when it was asked and checked, the message drafted to
//! send with Copy, what the question says after it, then the options: cards side by side when
//! there is room (each with its name, a number to compare, the assistant's pick and why, a
//! sentence or two, its facts, its pictures, what picking it starts, More and its button), or a
//! row each with what it does. A click answers and the page goes back.

use emberhut_api::tasks::{Decision, TaskStatus};
use gpui_kit::assets::IconName;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::prelude::*;
use gpui_kit::{ClipboardItem, Div, ElementId, FontWeight, ImageSource, div, img, px};

use super::text::{Long, collapsible, link_button, rich};
use super::{Part, TaskView, section};
use crate::brand::APP_NAME;
use crate::prose::copy;
use crate::store::asks;
use crate::store::decision::{self, DecisionPage, OptionCard, OptionRow, Options};
use crate::store::files::RemoteFile;
use crate::ui::palette::Pal;
use crate::ui::tasks::marks::glyph;
use crate::ui::viewer::OpenInViewer;

/// The narrowest a card is before the cards stack.
const CARD_MIN: f32 = 300.;

/// The widest a card is.
const CARD_MAX: f32 = 560.;

/// The decision's sections, after the title and the live card.
pub(super) fn section_of(
    mut body: Div,
    part: &Part<'_>,
    view: &TaskView,
    cx: &Context<'_, TaskView>,
) -> Div {
    let task = part.task;
    let Some(decision) = task
        .decision
        .as_ref()
        .filter(|_| task.status == TaskStatus::Waiting)
    else {
        return body;
    };
    let p = part.p;
    let artifacts = if task.artifacts.is_empty() {
        part.page.map_or(&[][..], |page| &page.task.artifacts)
    } else {
        &task.artifacts
    };
    let shown = decision::page(task, decision, artifacts, part.now);
    body = context(body, p, &shown);
    body = know(body, part, &shown, decision, view, cx);
    body = draft(body, part, &shown, view, cx);
    if let Some(after) = &shown.after {
        body = body.child(div().pt(px(8.)).child(collapsible(
            p,
            part.store,
            Long {
                key: "after",
                text: after,
                lines: 3,
                title: None,
            },
            view,
            cx,
        )));
    }
    match &shown.options {
        Options::None => body,
        Options::Rows(rows) => {
            body = body.child(section(p, "Your options"));
            rows_of(body, p, rows, cx)
        }
        Options::Cards { cards, rows } => {
            body = body.child(section(p, "Your options"));
            let mut grid = div().flex().flex_wrap().gap(px(12.));
            for card in cards {
                grid = grid.child(option_card(part, card, view, cx));
            }
            body = body.child(grid);
            rows_of(body, p, rows, cx)
        }
    }
}

/// An X post to reply to, with its drafted replies in full.
pub(super) fn x_posts(mut body: Div, part: &Part<'_>, cx: &Context<'_, TaskView>) -> Div {
    let task = part.task;
    let artifacts = if task.artifacts.is_empty() {
        part.page.map_or(&[][..], |page| &page.task.artifacts)
    } else {
        &task.artifacts
    };
    for artifact in artifacts.iter().filter(|artifact| artifact.x.is_some()) {
        body = body.child(div().pt(px(12.)).child(crate::ui::x_reply::view(
            part.p, part.store, artifact, task.id, false, cx,
        )));
    }
    body
}

/// Why it is asked now, and `Decide by Fri 28 Nov · 3 options to compare`.
fn context(mut body: Div, p: &Pal, shown: &DecisionPage<'_>) -> Div {
    if shown.context.is_none() && shown.by_line.is_none() {
        return body;
    }
    let mut block = div().flex().flex_col().gap(px(4.)).pt(px(10.));
    if let Some(context) = shown.context {
        block = block.child(div().text_size(px(13.5)).child(context.to_owned()));
    }
    if let Some(line) = &shown.by_line {
        block = block.child(
            div()
                .text_size(px(12.))
                .text_color(p.muted)
                .child(line.clone()),
        );
    }
    body = body.child(block);
    body
}

/// What the user needs to know: the bullets, else the question short (the question folded
/// under them when there are bullets), then when it was asked and checked.
fn know(
    mut body: Div,
    part: &Part<'_>,
    shown: &DecisionPage<'_>,
    decision: &Decision,
    view: &TaskView,
    cx: &Context<'_, TaskView>,
) -> Div {
    let p = part.p;
    body = body.child(section(p, "What you need to know"));
    for (i, bullet) in shown.brief.iter().enumerate() {
        body = body.child(
            div()
                .flex()
                .gap(px(8.))
                .text_size(px(13.))
                .child(div().text_color(p.muted).child("•"))
                .child(rich(
                    ElementId::Name(format!("brief-{i}").into()),
                    bullet,
                    None,
                    part.store,
                )),
        );
    }
    let cards = decision.card_count() > 0;
    let (key, lines, title) = if shown.brief.is_empty() {
        ("question", if cards { 3 } else { 6 }, "question")
    } else {
        ("details", 3, "details")
    };
    let title = format!("{} {title}", part.task.key);
    if !shown.summary.trim().is_empty() {
        body = body.child(div().pt(px(4.)).child(collapsible(
            p,
            part.store,
            Long {
                key,
                text: &shown.summary,
                lines,
                title: Some(&title),
            },
            view,
            cx,
        )));
    }
    let asked = &shown.asked;
    let mut when = div()
        .flex()
        .flex_col()
        .gap(px(2.))
        .pt(px(6.))
        .text_size(px(12.))
        .text_color(p.muted);
    for line in [&asked.asked, &asked.checked].into_iter().flatten() {
        when = when.child(line.clone());
    }
    if asked.stale {
        when = when.child(div().text_color(p.warn).child(decision::stale_warning()));
    }
    body.child(when)
}

/// The message drafted to send: its own block, apart from the notes, with Copy.
fn draft(
    mut body: Div,
    part: &Part<'_>,
    shown: &DecisionPage<'_>,
    view: &TaskView,
    cx: &Context<'_, TaskView>,
) -> Div {
    let Some(draft) = &shown.draft else {
        return body;
    };
    let p = part.p;
    let copied = if draft.plain {
        draft.text.clone()
    } else {
        copy::for_pasting(&draft.text)
    };
    body = body.child(
        div()
            .flex()
            .items_center()
            .child(div().flex_1().child(section(p, draft.label.clone())))
            .child(
                Button::new("copy-draft")
                    .label("Copy")
                    .icon(glyph(IconName::Copy, p.muted, 13.))
                    .small()
                    .ghost()
                    .tooltip("Copy the message, ready to paste where it goes")
                    .on_click(move |_, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(copied.clone()));
                    }),
            ),
    );
    body.child(
        div()
            .p(px(12.))
            .rounded(px(10.))
            .bg(p.card)
            .border_1()
            .border_color(p.border)
            .child(collapsible(
                p,
                part.store,
                Long {
                    key: "draft",
                    text: &draft.text,
                    lines: 5,
                    title: None,
                },
                view,
                cx,
            )),
    )
}

/// The answers as rows: what each is and does, the pick marked; a click answers.
fn rows_of(mut body: Div, p: &Pal, rows: &[OptionRow], cx: &Context<'_, TaskView>) -> Div {
    for (i, row) in rows.iter().enumerate() {
        let choice = row.choice.clone();
        let mut words = div()
            .flex()
            .flex_col()
            .gap(px(3.))
            .flex_1()
            .min_w_0()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(
                        div()
                            .text_size(px(14.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(row.text.clone()),
                    )
                    .when(row.pick, |d| d.child(pick_badge(p))),
            );
        if let Some(note) = &row.note {
            words = words.child(
                div()
                    .text_size(px(12.5))
                    .text_color(p.muted)
                    .child(note.clone()),
            );
        }
        body = body.child(
            div()
                .id(ElementId::Name(format!("option-{i}").into()))
                .flex()
                .items_center()
                .gap(px(10.))
                .py(px(8.))
                .px(px(6.))
                .rounded(px(8.))
                .cursor_pointer()
                .hover(|d| d.bg(p.hover))
                .child(words)
                .child(glyph(
                    IconName::CircleArrowUp,
                    if row.pick { p.accent } else { p.muted },
                    20.,
                ))
                .on_click(cx.listener(move |this, _, _, cx| this.choose(&choice, cx))),
        );
    }
    body
}

/// `Emberhut's pick`, on the accent.
fn pick_badge(p: &Pal) -> Div {
    div()
        .px(px(6.))
        .py(px(1.))
        .rounded_full()
        .bg(p.accent)
        .text_color(p.on_accent)
        .text_size(px(10.5))
        .font_weight(FontWeight::SEMIBOLD)
        .child(format!("{APP_NAME}'s pick"))
}

/// One option as a card: everything needed to weigh it, and its button.
fn option_card(
    part: &Part<'_>,
    card: &OptionCard<'_>,
    view: &TaskView,
    cx: &Context<'_, TaskView>,
) -> Div {
    let p = part.p;
    let choice = card.choice;
    let more_key = format!("choice-more-{}", card.index);
    let more = view.is_open(&more_key);
    let mut column = div()
        .flex()
        .flex_col()
        .gap(px(10.))
        .flex_1()
        .min_w(px(CARD_MIN))
        .max_w(px(CARD_MAX))
        .p(px(14.))
        .rounded(px(16.))
        .bg(p.card)
        .border_1()
        .border_color(if card.pick {
            p.accent.opacity(0.6)
        } else {
            p.border
        })
        .child(card_head(p, card));
    column = card_words(column, p, card, more);
    if !card.media.is_empty() {
        column = column.child(card_pictures(part, card));
    }
    if card.long {
        let key = more_key.clone();
        column = column.child(
            link_button(
                p,
                ElementId::Name(format!("{more_key}-toggle").into()),
                if more { "Less" } else { "More" },
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                this.toggle_open(&key);
                cx.notify();
            })),
        );
    }
    if let Some(then) = choice.then.as_deref().filter(|t| !t.trim().is_empty()) {
        column = column.child(
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .text_size(px(12.))
                .text_color(p.muted)
                .child(glyph(IconName::CornerDownRight, p.muted, 13.))
                .child(then.to_owned()),
        );
    }
    let answer = choice.label.clone();
    let button = Button::new(ElementId::Name(format!("choice-{}", card.index).into()))
        .label(card.button.clone())
        .w_full();
    let button = if card.pick {
        button.primary()
    } else {
        button.outline()
    };
    column.child(button.on_click(cx.listener(move |this, _, _, cx| this.choose(&answer, cx))))
}

/// A card's name and its number to compare (in the accent on the pick).
fn card_head(p: &Pal, card: &OptionCard<'_>) -> Div {
    let mut head = div().flex().items_center().gap(px(8.)).child(
        div()
            .flex_1()
            .text_size(px(15.))
            .font_weight(FontWeight::SEMIBOLD)
            .child(card.title.to_owned()),
    );
    if let Some(badge) = card
        .choice
        .badge
        .as_deref()
        .filter(|b| !b.trim().is_empty())
    {
        let tint = if card.pick { p.accent } else { p.muted };
        head = head.child(
            div()
                .px(px(8.))
                .py(px(2.))
                .rounded_full()
                .bg(tint.opacity(0.12))
                .text_color(tint)
                .text_size(px(12.5))
                .font_weight(FontWeight::SEMIBOLD)
                .child(badge.to_owned()),
        );
    }
    head
}

/// A card's words: the pick and why, its sentence or two, its facts (four, all with More),
/// and with More its long version.
fn card_words(mut column: Div, p: &Pal, card: &OptionCard<'_>, more: bool) -> Div {
    let choice = card.choice;
    if card.pick {
        let mut line = div().flex().items_center().gap(px(6.)).child(pick_badge(p));
        if let Some(why) = choice.why.as_deref().filter(|w| !w.trim().is_empty()) {
            line = line.child(
                div()
                    .text_size(px(12.))
                    .text_color(p.accent)
                    .child(why.to_owned()),
            );
        }
        column = column.child(line);
    }
    if let Some(summary) = choice.summary.as_deref().filter(|s| !s.trim().is_empty()) {
        let text = div().text_size(px(13.)).child(summary.to_owned());
        column = column.child(if more { text } else { text.line_clamp(4) });
    }
    for field in card.fields(more) {
        let value = div().text_size(px(13.)).child(field.value.clone());
        column = column.child(
            div()
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(
                    div()
                        .text_size(px(10.5))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(p.muted)
                        .child(field.key.to_uppercase()),
                )
                .child(if more { value } else { value.line_clamp(3) }),
        );
    }
    if more && let Some(long) = choice.more.as_deref().filter(|m| !m.trim().is_empty()) {
        column = column.child(
            div()
                .text_size(px(13.))
                .text_color(p.muted)
                .child(long.to_owned()),
        );
    }
    column
}

/// A card's pictures and videos, small; a click opens one in the system's viewer or player.
fn card_pictures(part: &Part<'_>, card: &OptionCard<'_>) -> Div {
    let p = part.p;
    let mut strip = div().flex().flex_wrap().gap(px(8.));
    for artifact in &card.media {
        let thumb = artifact
            .thumb
            .as_deref()
            .and_then(|thumb| part.read.picture(thumb));
        let picture = match thumb {
            Some(image) => img(ImageSource::Render(image))
                .w(px(120.))
                .h(px(80.))
                .rounded(px(8.))
                .into_any_element(),
            None => div()
                .w(px(120.))
                .h(px(80.))
                .rounded(px(8.))
                .bg(p.hover)
                .into_any_element(),
        };
        let (url, name) = (artifact.url.clone(), artifact.name.clone());
        let video = artifact.kind == "video";
        strip = strip.child(
            div()
                .id(ElementId::Name(
                    format!("choice-media-{}", artifact.id).into(),
                ))
                .relative()
                .cursor_pointer()
                .child(picture)
                .when(video, |d| {
                    d.child(
                        div()
                            .absolute()
                            .inset_0()
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(glyph(IconName::CirclePlay, p.on_accent, 26.)),
                    )
                })
                .on_click(move |_, window, cx| {
                    let file = RemoteFile::new(url.clone(), name.clone());
                    window.dispatch_action(Box::new(OpenInViewer { file }), cx);
                }),
        );
    }
    strip
}

impl TaskView {
    /// Answers the page's ask with `choice`: an answer that sends puts the draft on the
    /// clipboard first; the page goes back after a beat, while the toast offers Undo.
    fn choose(&mut self, choice: &str, cx: &mut Context<'_, Self>) {
        let Some(task) = self.shown else {
            return;
        };
        if self.tapped.is_some() {
            return;
        }
        let copied = self
            .store
            .read(cx)
            .task_of(task)
            .and_then(|task| task.decision.as_ref())
            .and_then(|decision| asks::copied_draft(choice, decision));
        let owned = choice.to_owned();
        let taken = self
            .store
            .update(cx, |store, cx| store.decide(task, owned, cx));
        if let Some(copied) = copied.filter(|_| taken) {
            cx.write_to_clipboard(ClipboardItem::new_string(copied));
        }
        self.leave(&asks::answer_label(choice), cx);
    }
}
