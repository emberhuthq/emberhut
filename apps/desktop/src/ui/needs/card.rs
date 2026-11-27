//! A card that needs the user, on the list: the task, the thing to do in bold, `Read the draft
//! first`, what it costs, its project, when it was asked (or that it may be outdated), its
//! decide-by chip and its picture; under it its quick answers (the first prominent, one more
//! beside it or the rest in a menu, `I did it` for a thing to do) or an X post's replies. A
//! click anywhere else opens its task; a right-click offers Not now and Archive.

use emberhut_api::ids::TaskId;
use emberhut_api::work::WorkRow;
use gpui_kit::ClipboardItem;
use gpui_kit::assets::IconName;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{ContextMenuExt as _, DropdownMenu as _, PopupMenuItem};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, Div, Entity, FontWeight, ImageSource, MouseButton, Role, Stateful, div, img,
    px,
};

use super::{MainWindow, card_id, open_task};
use crate::store::Store;
use crate::store::asks;
use crate::store::lookup;
use crate::store::need_card::{Asked, CardAnswers, ChipTone, DecideChip, NeedCard};
use crate::ui::a11y::{self, Accessible as _};
use crate::ui::controls::tab_stop;
use crate::ui::palette::Pal;
use crate::ui::reads::reads;
use crate::ui::tasks::marks::glyph;
use crate::ui::x_reply;

/// A card on the list.
pub(super) fn card(
    p: &Pal,
    store: &Entity<Store>,
    shown: &NeedCard<'_>,
    row: &WorkRow,
    cx: &Context<'_, MainWindow>,
) -> AnyElement {
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
                    reads(shown.title)
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_size(px(12.5))
                        .text_color(p.muted),
                )
                .children(shown.chip.as_ref().map(|chip| decide_chip(p, chip))),
        )
        .child(
            reads(&shown.action)
                .text_size(px(14.))
                .font_weight(FontWeight::BOLD)
                .line_clamp(3),
        );
    if shown.draft {
        words = words.child(
            div()
                .flex()
                .items_center()
                .gap(px(4.))
                .text_size(px(11.5))
                .font_weight(FontWeight::MEDIUM)
                .text_color(p.accent)
                .child(glyph(IconName::MessageSquareText, p.accent, 12.))
                .child("Read the draft first"),
        );
    }
    words = words
        .child(foot(p, shown))
        .child(crate::ui::browser::thumb::LiveThumb::new(store, row, 52.));
    let mut top = div()
        .flex()
        .items_start()
        .gap(px(10.))
        .child(glyph(IconName::Hand, p.accent, 18.))
        .child(words);
    if let Some(image) = shown.thumb.and_then(|thumb| store.read(cx).picture(thumb)) {
        top = top.child(
            img(ImageSource::Render(image))
                .size(px(52.))
                .rounded(px(8.))
                .flex_none(),
        );
    }
    let mut card = div()
        .id(card_id("need", row))
        // A card holds its answers: it is read as its title and what it asks, then its buttons.
        .read_as(Role::Group, a11y::line(&[shown.title, &shown.action]))
        .flex()
        .flex_col()
        .gap(px(8.))
        .p(px(12.))
        .rounded(px(12.))
        .bg(p.accent.opacity(0.09))
        .border_1()
        .border_color(p.border)
        .cursor_pointer()
        .child(top);
    if let (Some(reply), Some(task)) = (shown.reply, shown.task) {
        card = card.child(
            own_clicks(div().pl(px(28.))).child(x_reply::view(p, store, reply, task, true, cx)),
        );
    } else if let Some(answers) = &shown.answers {
        card = card.child(own_clicks(div().pl(px(28.))).child(answer_row(p, store, answers, row)));
    }
    let Some(task) = shown.task else {
        // A thread's ask, with no task: its page opens from here.
        if row.thread.is_some() {
            card = card.child(
                own_clicks(div().pl(px(28.))).child(crate::ui::sessions::work_link(
                    store,
                    row.id.clone(),
                    p,
                )),
            );
        }
        return super::thread_card(card, store, row, p);
    };
    card = tab_stop(card, p).on_click(move |_, window, cx| open_task(task, window, cx));
    with_menu(card, store, task).into_any_element()
}

/// `area` keeps its clicks (its answers, their menu, the post's replies) from opening the card's
/// task.
fn own_clicks(area: Div) -> Div {
    area.on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
}

/// What it costs, its project, and when it was asked or that it may be outdated.
fn foot(p: &Pal, shown: &NeedCard<'_>) -> Div {
    let mut foot = div()
        .flex()
        .items_center()
        .gap(px(6.))
        .text_size(px(11.5))
        .text_color(p.faint)
        .child(
            div()
                .flex_none()
                .font_weight(FontWeight::MEDIUM)
                .text_color(p.accent.opacity(0.8))
                .child(shown.kind),
        );
    if let Some(project) = shown.project {
        foot = foot.child(div().truncate().child(project.to_owned()));
    }
    match &shown.asked {
        Asked::Outdated => foot.child(
            div()
                .flex_none()
                .text_color(p.warn)
                .child("may be outdated"),
        ),
        Asked::Ago(ago) => foot.child(div().truncate().child(ago.clone())),
        Asked::Unknown => foot,
    }
}

/// The decide-by chip: late in red, someone else's deadline in orange with a flag, today in
/// the accent, the rest muted.
pub(crate) fn decide_chip(p: &Pal, chip: &DecideChip) -> Div {
    let color = match chip.tone {
        ChipTone::Late => p.danger,
        ChipTone::Hard => p.warn,
        ChipTone::Today => p.accent,
        ChipTone::Plain => p.muted,
    };
    let strong = chip.tone != ChipTone::Plain;
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(3.))
        .px(px(6.))
        .py(px(1.))
        .rounded_full()
        .bg(color.opacity(if strong { 0.16 } else { 0.1 }))
        .text_color(color)
        .text_size(px(11.))
        .font_weight(FontWeight::SEMIBOLD)
        .when(chip.hard, |d| d.child(glyph(IconName::Flag, color, 10.)))
        .child(chip.text.clone())
}

/// The quick answers: the first prominent; one more bordered, or the rest in a `…` menu.
fn answer_row(p: &Pal, store: &Entity<Store>, answers: &CardAnswers, row: &WorkRow) -> Div {
    let to = answers.to;
    let mut line = div().flex().items_center().gap(px(8.));
    if let Some(first) = answers.first() {
        line = line.child(
            Button::new(card_id("answer-first", row))
                .label(asks::answer_label(first))
                .primary()
                .small()
                .on_click(answer(store, to, first)),
        );
    }
    if let Some(second) = answers.second() {
        line = line.child(
            Button::new(card_id("answer-second", row))
                .label(asks::answer_label(second))
                .outline()
                .small()
                .on_click(answer(store, to, second)),
        );
    }
    let menu = answers.menu().to_vec();
    if !menu.is_empty() {
        let store = store.clone();
        line = line.child(
            Button::new(card_id("answer-more", row))
                .icon(glyph(IconName::Ellipsis, p.muted, 14.))
                .outline()
                .small()
                .tooltip("Other answers")
                .dropdown_menu(move |mut menu_list, _, _| {
                    for choice in &menu {
                        menu_list = menu_list.item(
                            PopupMenuItem::new(asks::answer_label(choice))
                                .on_click(answer(&store, to, choice)),
                        );
                    }
                    menu_list
                }),
        );
    }
    line
}

/// Answers `to` with `choice`: an answer that sends puts the ask's draft on the clipboard,
/// ready to paste where it goes.
fn answer(
    store: &Entity<Store>,
    to: TaskId,
    choice: &str,
) -> impl Fn(&gpui_kit::ClickEvent, &mut gpui_kit::Window, &mut App) + 'static {
    let (store, choice) = (store.clone(), choice.to_owned());
    move |_, _, cx| {
        cx.stop_propagation();
        let copied = store
            .read(cx)
            .state()
            .and_then(|state| lookup::issue(state, to))
            .and_then(|issue| issue.decision.as_ref())
            .and_then(|decision| asks::copied_draft(&choice, decision));
        let choice = choice.clone();
        // Once the answer is taken: one already answered copies nothing.
        let taken = store.update(cx, |store, cx| store.decide(to, choice, cx));
        if let Some(copied) = copied.filter(|_| taken) {
            cx.write_to_clipboard(ClipboardItem::new_string(copied));
        }
    }
}

/// `card` with its right-click menu: Not now (put off a week) and Archive.
pub(super) fn with_menu(
    card: Stateful<Div>,
    store: &Entity<Store>,
    task: TaskId,
) -> impl IntoElement {
    let store = store.clone();
    card.context_menu(move |menu, _, _| {
        let (later, away) = (store.clone(), store.clone());
        menu.item(
            PopupMenuItem::new("Not now")
                .on_click(move |_, _, cx| later.update(cx, |store, cx| store.not_now(task, cx))),
        )
        .item(
            PopupMenuItem::new("Archive")
                .on_click(move |_, _, cx| away.update(cx, |store, cx| store.archive(task, cx))),
        )
    })
}
