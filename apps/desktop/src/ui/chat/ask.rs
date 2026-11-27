//! Over the composer: the newest question the assistant waits on, with its answers in reach,
//! and in Main one slim line for the asks waiting in the other rooms.
//!
//! The card names the task (its key and "needs you"), what it is about and the question in two
//! lines. Options that come as cards show side by side, each with its name, a sentence, its
//! number, the facts to weigh, the assistant's reason on its pick, and its answer. The first
//! plain answer is prominent, a second sits beside it and the rest are under More; a thing to
//! do has "I did it". Open (or "Read the draft") and a click on the question open the task's
//! page. "· 3 more" opens Needs you, "2 more in other rooms" the rooms (on a line of its own in a
//! narrow chat), and X puts every card
//! showing away until a new question comes.
//!
//! What the card shows is worked out when the store changes ([`AskArea::of`]), not as it is
//! drawn.

use emberhut_api::ids::{RoomId, TaskId};
use gpui_kit::TestSupportExt as _;
use gpui_kit::assets::IconName as AssetIcon;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{Icon, Sizable as _};
use gpui_kit::prelude::*;
use gpui_kit::{
    Anchor, AnyElement, App, ClipboardItem, Div, ElementId, Entity, FontWeight, Stateful, actions,
    div, px,
};

use super::ChatView;
use crate::store::asks::{self, AskCards, AskKey, AskShown, OptionCard};
use crate::store::{Page, Store};
use crate::ui::a11y::{self, Accessible as _};
use crate::ui::column;
use crate::ui::palette::Pal;
use crate::ui::reads::reads;

actions!(
    chat,
    [
        /// Shows the rooms (the sidebar), from the ask card's "more in other rooms".
        ShowRooms,
    ]
);

/// The ask card's top line, for tests to find.
pub(super) const ASK_TOP: &str = "ask-top";

/// The top line's task and "needs you", for tests to find.
pub(super) const ASK_EYEBROW: &str = "ask-eyebrow";

/// The count of asks in other rooms, for tests to find.
pub(super) const ASK_ELSEWHERE: &str = "ask-elsewhere";

/// The chat's width from which the other rooms' count fits on the card's top line, in pixels.
const INLINE_ELSEWHERE_FROM: f32 = 560.;

/// The top line's Hide button, for tests to find.
pub(super) const ASK_HIDE: &str = "ask-hide";

/// What goes over the composer: the ask card, and Main's line for the other rooms (their
/// count of asks).
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct AskArea {
    card: Option<Card>,
    room_asks: Option<u32>,
}

impl AskArea {
    /// What `store` has to show over the composer of its open room.
    pub(super) fn of(store: &Store) -> Self {
        let (Some(state), Some(needs)) = (store.state(), store.needs()) else {
            return Self::default();
        };
        let room = store.room();
        let cards = AskCards::new(state, &needs, room, store.put_away().ask_cards());
        let others = asks::in_other_rooms(state);
        Self {
            card: cards.first.as_ref().map(|shown| Card::of(&cards, shown)),
            room_asks: (room == RoomId::MAIN && store.put_away().shows_room_asks(others))
                .then_some(others),
        }
    }
}

/// What the ask card shows, owned, so the view doesn't hold the store while it builds.
#[derive(Clone, Debug, PartialEq)]
struct Card {
    /// The task answered.
    task: TaskId,
    eyebrow: String,
    title: String,
    question: String,
    /// The options as cards, each with what answering it copies.
    option_cards: Vec<(OptionCard, Option<String>)>,
    /// Each plain answer: its button's label and what is sent.
    answers: Vec<(String, String)>,
    /// What answering each copies (a draft for an answer that sends).
    copies: Vec<Option<String>>,
    open: asks::OpenButton,
    more: usize,
    elsewhere: u32,
    keys: Vec<AskKey>,
}

impl Card {
    fn of(cards: &AskCards<'_>, shown: &AskShown<'_>) -> Self {
        let mut answers: Vec<(String, String)> = shown
            .options()
            .into_iter()
            .map(|option| (asks::answer_label(&option), option))
            .collect();
        if answers.is_empty() && shown.is_a_thing_to_do() {
            answers.push((asks::answer_label("Done"), "Done".to_owned()));
        }
        let option_cards = shown
            .option_cards()
            .into_iter()
            .map(|card| {
                let copied = shown.copied_for(&card.label);
                (card, copied)
            })
            .collect();
        Self {
            task: shown.ask.id,
            option_cards,
            eyebrow: shown.eyebrow(),
            title: shown.title(),
            question: shown.question(),
            copies: answers
                .iter()
                .map(|(_, choice)| shown.copied_for(choice))
                .collect(),
            answers,
            open: shown.open_button(),
            more: cards.more,
            elsewhere: cards.elsewhere,
            keys: cards.keys.clone(),
        }
    }
}

/// Answers `task` with `choice`; once the answer is taken, copies `copied` (a draft to paste
/// where it goes).
fn answer(store: &Entity<Store>, task: TaskId, choice: &str, copied: Option<&str>, cx: &mut App) {
    let choice = choice.to_owned();
    let taken = store.update(cx, |store, cx| store.decide(task, choice, cx));
    if let Some(copied) = copied.filter(|_| taken) {
        cx.write_to_clipboard(ClipboardItem::new_string(copied.to_owned()));
    }
}

impl ChatView {
    /// What goes over the composer: the ask card, and Main's line for the other rooms.
    pub(super) fn ask_area(&self, p: &Pal, cx: &Context<'_, Self>) -> Vec<AnyElement> {
        let card = self
            .ask
            .card
            .as_ref()
            .map(|card| self.ask_card(card, p, cx).into_any_element());
        let line = self
            .ask
            .room_asks
            .map(|others| Self::room_asks_line(others, p, cx).into_any_element());
        card.into_iter().chain(line).collect()
    }

    /// The card's top line: which task, how many more, the other rooms' count when `inline`,
    /// and Hide.
    fn ask_top(card: &Card, inline: bool, p: &Pal, cx: &Context<'_, Self>) -> Div {
        let hide = if card.more > 0 {
            format!("Hide all {}", card.more + 1)
        } else {
            "Hide".to_owned()
        };
        let keys = card.keys.clone();
        div()
            .flex()
            .items_center()
            .gap(px(6.))
            .debug_selector(|| ASK_TOP.to_owned())
            .child(
                // The one part that gives way in a narrow chat: it shrinks and ends in "…"
                // before anything after it is pushed out of the card.
                div()
                    .debug_selector(|| ASK_EYEBROW.to_owned())
                    .min_w_0()
                    .truncate()
                    .text_size(px(11.5))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(p.warn)
                    .child(card.eyebrow.clone()),
            )
            .when(card.more > 0, |d| {
                d.child(
                    div()
                        .id("ask-more")
                        .button_named(format!("{} more asks: show them all", card.more))
                        .cursor_pointer()
                        .flex_none()
                        .flex()
                        .items_center()
                        .gap(px(2.))
                        .text_size(px(11.5))
                        .text_color(p.muted)
                        .hover(|d| d.text_color(p.text))
                        .child(format!("· {} more", card.more))
                        .child(Icon::new(AssetIcon::ChevronRight).size(px(11.)))
                        .on_click(cx.listener(|_, _, window, cx| {
                            column::open(Page::Needs, window, cx);
                        })),
                )
            })
            .child(div().flex_1())
            .when(inline && card.elsewhere > 0, |d| {
                d.child(Self::elsewhere(card.elsewhere, p))
            })
            .child(
                div()
                    .id("ask-hide")
                    .debug_selector(|| ASK_HIDE.to_owned())
                    .button_named(hide.clone())
                    .flex_none()
                    .size(px(22.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(6.))
                    .cursor_pointer()
                    .hover(|d| d.bg(p.hover))
                    .child(Icon::new(AssetIcon::X).size(px(13.)).text_color(p.muted))
                    .tooltip(move |window, cx| Tooltip::new(hide.clone()).build(window, cx))
                    // Seen by the headless tests (a no-op outside them).
                    .test_support()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let keys = keys.clone();
                        this.store
                            .update(cx, |store, cx| store.put_away_asks(keys, cx));
                    })),
            )
    }

    /// The count of asks in other rooms, as a pill that opens the rooms. It keeps its width.
    fn elsewhere(count: u32, p: &Pal) -> Stateful<Div> {
        let words = format!("{count} more in other rooms");
        div()
            .id("ask-elsewhere")
            .debug_selector(|| ASK_ELSEWHERE.to_owned())
            .button_named(words.clone())
            .cursor_pointer()
            .flex_none()
            .px(px(8.))
            .py(px(3.))
            .rounded(px(10.))
            .bg(p.accent_soft)
            .text_color(p.accent)
            .text_size(px(11.5))
            .font_weight(FontWeight::SEMIBOLD)
            .child(words)
            .on_click(|_, window, cx| window.dispatch_action(Box::new(ShowRooms), cx))
    }

    fn ask_card(&self, card: &Card, p: &Pal, cx: &Context<'_, Self>) -> Div {
        let task = card.task;
        // In a chat too narrow for it, the other rooms' count leaves the top line for a line of
        // its own, so the task's line keeps its words. Before the list is laid out its width
        // is unknown (0): the count takes its own line then too.
        let width = f32::from(self.list.viewport_bounds().size.width);
        let inline = width >= INLINE_ELSEWHERE_FROM;
        let question = div()
            .id("ask-question")
            // Read as the ask itself; a press opens its task.
            .button_named(a11y::line(&[&card.title, &card.question]))
            .hint("Shows its details")
            .flex()
            .flex_col()
            .gap(px(2.))
            .cursor_pointer()
            .child(
                reads(&card.title)
                    .truncate()
                    .text_size(px(11.5))
                    .text_color(p.muted),
            )
            .child(
                reads(&card.question)
                    .line_clamp(2)
                    .text_size(px(13.5))
                    .font_weight(FontWeight::SEMIBOLD),
            )
            .on_click(cx.listener(move |_, _, window, cx| {
                column::push(Page::Task(task), window, cx);
            }));
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .p(px(12.))
            .rounded(px(18.))
            .border_1()
            .border_color(p.border)
            .bg(p.card)
            .child(Self::ask_top(card, inline, p, cx))
            .when(!inline && card.elsewhere > 0, |d| {
                d.child(div().flex().child(Self::elsewhere(card.elsewhere, p)))
            })
            .child(question)
            .when(!card.option_cards.is_empty(), |d| {
                d.child(self.option_cards(card, p))
            })
            .child(self.ask_buttons(card, cx))
    }

    /// The options that come as cards, side by side, each with its answer.
    fn option_cards(&self, card: &Card, p: &Pal) -> Div {
        let task = card.task;
        let cards = card
            .option_cards
            .iter()
            .enumerate()
            .map(|(i, (option, copied))| {
                let (store, choice, copied) =
                    (self.store.clone(), option.label.clone(), copied.clone());
                let pick = Button::new(ElementId::Name(format!("ask-choice-{task}-{i}").into()))
                    .label(asks::answer_label(&option.label))
                    .small()
                    .on_click(move |_, _, cx| answer(&store, task, &choice, copied.as_deref(), cx));
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .flex_1()
                    .min_w(px(160.))
                    .p(px(10.))
                    .rounded(px(12.))
                    .border_1()
                    .border_color(if option.picked { p.accent } else { p.border })
                    .bg(p.surface)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(px(13.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(option.title.clone()),
                            )
                            .children(option.badge.clone().map(|badge| {
                                div()
                                    .px(px(6.))
                                    .rounded(px(6.))
                                    .bg(p.accent_soft)
                                    .text_color(p.accent)
                                    .text_size(px(11.5))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(badge)
                            })),
                    )
                    .children(option.summary.clone().map(|summary| {
                        div()
                            .line_clamp(2)
                            .text_size(px(12.))
                            .text_color(p.muted)
                            .child(summary)
                    }))
                    .children(option.fields.iter().map(|(key, value)| {
                        div()
                            .flex()
                            .gap(px(4.))
                            .text_size(px(12.))
                            .child(
                                div()
                                    .flex_none()
                                    .text_color(p.muted)
                                    .child(format!("{key}:")),
                            )
                            .child(div().min_w_0().truncate().child(value.clone()))
                    }))
                    .children(option.why.clone().map(|why| {
                        div()
                            .line_clamp(2)
                            .text_size(px(12.))
                            .text_color(p.accent)
                            .child(why)
                    }))
                    .child(div().pt(px(2.)).child(if option.picked {
                        pick.primary()
                    } else {
                        pick
                    }))
            });
        div().flex().flex_wrap().gap(px(8.)).children(cards)
    }

    /// The first answer prominent, a second beside it, the rest under More, then Open.
    fn ask_buttons(&self, card: &Card, cx: &Context<'_, Self>) -> Div {
        let task = card.task;
        let button = |i: usize, label: &str| {
            let store = self.store.clone();
            let choice = card.answers[i].1.clone();
            let copied = card.copies[i].clone();
            Button::new(ElementId::Name(format!("ask-answer-{task}-{i}").into()))
                .label(label.to_owned())
                .small()
                .on_click(move |_, _, cx| answer(&store, task, &choice, copied.as_deref(), cx))
        };
        let mut row = div().flex().flex_wrap().items_center().gap(px(8.));
        // Beside cards, the plain answers ("More ideas") stay quiet: a card's answer leads.
        if let Some((label, _)) = card.answers.first() {
            let first = button(0, label);
            row = row.child(if card.option_cards.is_empty() {
                first.primary()
            } else {
                first
            });
        }
        match card.answers.len() {
            0 | 1 => {}
            2 => row = row.child(button(1, &card.answers[1].0)),
            _ => {
                let store = self.store.clone();
                let rest: Vec<(String, String, Option<String>)> = card.answers[1..]
                    .iter()
                    .zip(&card.copies[1..])
                    .map(|((label, choice), copied)| {
                        (label.clone(), choice.clone(), copied.clone())
                    })
                    .collect();
                row = row.child(
                    Button::new("ask-other-answers")
                        .label("More")
                        .accessibility_label("Other answers")
                        .small()
                        .dropdown_menu_with_anchor(Anchor::BottomLeft, move |menu, _, _| {
                            rest.iter().fold(menu, |menu, (label, choice, copied)| {
                                let (store, choice, copied) =
                                    (store.clone(), choice.clone(), copied.clone());
                                menu.item(PopupMenuItem::new(label.clone()).on_click(
                                    move |_, _, cx| {
                                        answer(&store, task, &choice, copied.as_deref(), cx);
                                    },
                                ))
                            })
                        }),
                );
            }
        }
        row.child(
            Button::new("ask-open")
                .label(card.open.label.clone())
                .small()
                .on_click(cx.listener(move |_, _, window, cx| {
                    column::push(Page::Task(task), window, cx);
                })),
        )
    }

    /// Main's line: "3 asks in your rooms ›" opens Needs you; its X hides it until more come.
    fn room_asks_line(count: u32, p: &Pal, cx: &Context<'_, Self>) -> Div {
        let words = if count == 1 {
            "1 ask in your rooms".to_owned()
        } else {
            format!("{count} asks in your rooms")
        };
        div()
            .flex()
            .items_center()
            .gap(px(8.))
            .px(px(14.))
            .py(px(7.))
            .rounded(px(18.))
            .border_1()
            .border_color(p.border)
            .bg(p.card)
            .text_size(px(13.))
            .child(
                div()
                    .id("room-asks")
                    .flex_1()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .cursor_pointer()
                    .child(
                        Icon::new(AssetIcon::Hand)
                            .size(px(14.))
                            .text_color(p.accent),
                    )
                    .child(words)
                    .child(
                        Icon::new(AssetIcon::ChevronRight)
                            .size(px(12.))
                            .text_color(p.faint),
                    )
                    .on_click(cx.listener(|_, _, window, cx| {
                        column::open(Page::Needs, window, cx);
                    })),
            )
            .child(
                div()
                    .id("room-asks-hide")
                    .size(px(22.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(6.))
                    .cursor_pointer()
                    .hover(|d| d.bg(p.hover))
                    .child(Icon::new(AssetIcon::X).size(px(13.)).text_color(p.muted))
                    .tooltip(|window, cx| Tooltip::new("Hide").build(window, cx))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.store.update(cx, Store::put_away_room_asks);
                    })),
            )
    }
}

#[cfg(test)]
mod tests {
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{TestAppContext, VisualTestContext};
    use serde_json::json;

    use super::super::tests::open_chat;
    use super::*;
    use crate::backend::fake::Call;
    use crate::fixtures::{issue, room, state};
    use crate::store::answers::UNDO_WINDOW;

    fn asking() -> emberhut_api::state::State {
        state(json!({
            "rooms": [room(1, json!({}))],
            "issues": [issue(5, "waiting", json!({"decision": {"question": "Ship it?",
                "options": ["Yes", "No", "Later", "Never"], "asked": "2026-11-25T08:00:00Z"}}))],
        }))
    }

    #[gpui_kit::test]
    fn an_answer_from_the_card_waits_for_undo_and_the_shortcut_takes_it_back(
        cx: &mut TestAppContext,
    ) {
        let (backend, store, window) = open_chat(cx);
        cx.update(|cx| crate::ui::toast::undo_on_shortcut(&store, cx).detach());
        store.update(cx, |store, cx| store.take_state_for_tests(asking(), cx));
        cx.update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
            window.click("ask-answer-5-0", cx);
        })
        .unwrap();
        store.read_with(cx, |store, _| {
            let toast = store.answers().toast().unwrap();
            assert_eq!(toast.text, "Answered: Yes");
            assert!(toast.undoable());
        });
        let mut window = VisualTestContext::from_window(window.into(), cx);
        window.simulate_keystrokes(undo_keys());
        store.read_with(&window, |store, _| {
            assert_eq!(store.answers().toast(), None);
            assert!(store.answers().gone().is_empty(), "the ask is back");
        });
        window.executor().advance_clock(UNDO_WINDOW * 2);
        window.run_until_parked();
        assert!(
            !backend
                .calls()
                .iter()
                .any(|call| matches!(call, Call::Decide(..))),
            "an undone answer is never sent"
        );
    }

    fn undo_keys() -> &'static str {
        if cfg!(target_os = "macos") {
            "cmd-z"
        } else {
            "ctrl-z"
        }
    }

    #[gpui_kit::test]
    fn an_option_card_is_answered_from_the_ask_card(cx: &mut TestAppContext) {
        let (_backend, store, window) = open_chat(cx);
        let cards = state(json!({
            "rooms": [room(1, json!({}))],
            "issues": [issue(6, "waiting", json!({"decision": {"question": "Which plan?",
                "options": ["Plan A", "Plan B", "More ideas"], "recommend": 1,
                "choices": [
                    {"label": "Plan A", "title": "The small plan", "summary": "Done in a week.",
                        "fields": [{"k": "Cost", "v": "100"}]},
                    {"label": "Plan B", "title": "The big plan", "badge": "8/10",
                        "fields": [{"k": "Cost", "v": "400"}], "why": "It lasts"},
                    {"label": "More ideas"}],
                "asked": "2026-11-25T08:00:00Z"}}))],
        }));
        store.update(cx, |store, cx| store.take_state_for_tests(cards, cx));
        cx.update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
            assert!(
                window.try_find("ask-answer-6-0").is_some(),
                "More ideas, beside the cards"
            );
            window.click("ask-choice-6-1", cx);
        })
        .unwrap();
        store.read_with(cx, |store, _| {
            let toast = store.answers().toast().unwrap();
            assert_eq!(
                (toast.task, toast.text.as_str()),
                (TaskId::new(6), "Answered: Plan B")
            );
        });
    }

    #[gpui_kit::test]
    fn a_refusal_of_an_earlier_answer_leaves_the_held_ones_undo(cx: &mut TestAppContext) {
        let backend = crate::backend::fake::FakeBackend::new([]);
        backend.push_decision(Err(crate::backend::Error::Refused {
            status: Some(409),
            message: Some("Task 7 is done already".into()),
        }));
        let (backend, store, window) = super::super::tests::open_chat_over(backend, cx);
        cx.update(|cx| crate::ui::toast::undo_on_shortcut(&store, cx).detach());
        store.update(cx, |store, cx| {
            store.decide(TaskId::new(7), "Yes".into(), cx);
            store.decide(TaskId::new(8), "No".into(), cx);
        });
        cx.run_until_parked();
        assert_eq!(
            backend.calls(),
            [Call::Decide(TaskId::new(7), "Yes".into())]
        );
        store.read_with(cx, |store, _| {
            let toast = store.answers().toast().unwrap();
            assert_eq!(toast.task, TaskId::new(8), "the held answer's toast stays");
            assert!(toast.undoable());
        });
        let mut window = VisualTestContext::from_window(window.into(), cx);
        window.simulate_keystrokes(undo_keys());
        store.read_with(&window, |store, _| {
            assert!(
                !store.answers().gone().contains(&TaskId::new(8)),
                "the shortcut took it back"
            );
            let toast = store.answers().toast().unwrap();
            assert_eq!(
                (toast.task, toast.text.as_str()),
                (TaskId::new(7), "Answer not sent: Task 7 is done already"),
                "then the refusal shows"
            );
        });
        window.executor().advance_clock(UNDO_WINDOW * 2);
        window.run_until_parked();
        assert_eq!(backend.calls().len(), 1, "the undone answer is never sent");
    }

    #[gpui_kit::test]
    fn hide_puts_the_card_away_and_open_opens_the_task(cx: &mut TestAppContext) {
        let (_backend, store, window) = open_chat(cx);
        store.update(cx, |store, cx| store.take_state_for_tests(asking(), cx));
        cx.update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
            assert!(
                window.try_find("ask-other-answers").is_some(),
                "the rest under More"
            );
            window.click("ask-open", cx);
        })
        .unwrap();
        store.read_with(cx, |store, _| {
            assert!(
                store.shows_page(Page::Task(TaskId::new(5))),
                "its page in the window's column"
            );
        });
        store.update(cx, |store, cx| store.switch_room(RoomId::MAIN, cx));
        cx.update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
            window.click("ask-hide", cx);
        })
        .unwrap();
        // The card is worked out again once the store's change reaches the chat.
        cx.update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
            assert!(window.try_find("ask-open").is_none(), "put away");
        })
        .unwrap();
        store.read_with(cx, |store, _| {
            assert!(
                store
                    .put_away()
                    .ask_cards()
                    .iter()
                    .any(|key| key.to_string() == "5@2026-11-25T08:00:00Z")
            );
        });
    }
}
