//! One party, live: its cast, the talk as it happens (each persona's turns in its colour, the
//! user's words on the right and their directions as a chip), who is speaking word by word,
//! where it stands against its caps, and the user's controls: Pause, Resume or Continue, Stop,
//! and a line that jumps in or steers. The talk sticks to the bottom as it grows.

use std::time::Duration;

use emberhut_api::party::{LineKind, Party, PartyAction, PartyLine, PartyStatus, Speaking};
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::Textarea;
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenu, PopupMenuItem};
use gpui_kit::component::text::TextView;
use gpui_kit::component::{Icon, Sizable as _};
use gpui_kit::prelude::*;
use gpui_kit::{
    Anchor, Animation, AnimationExt as _, AnyElement, Div, ElementId, Entity, FontWeight, Hsla,
    div, pulsating_between, px,
};

use super::PartyWindow;
use super::look::{glyph, initial, persona_color, status_color};
use crate::motion;
use crate::prose::direction;
use crate::store::Store;
use crate::store::party::words;
use crate::ui::a11y::Accessible as _;
use crate::ui::palette::Pal;
use crate::ui::prose::follow_web_link;
use crate::ui::reads;
use crate::ui::widgets::dot;

/// The party's page.
pub(super) fn page(view: &mut PartyWindow, p: &Pal, cx: &mut Context<'_, PartyWindow>) -> Div {
    let store = view.store.read(cx);
    let party_data = store.party();
    let Some(room) = party_data.room() else {
        return div();
    };
    let party = room.party().cloned();
    let lines = room.lines().to_vec();
    let speaking = room.speaking().cloned();
    let problem = room.problem().map(str::to_owned);
    let model_names: Vec<String> = party.as_ref().map_or_default(|party| {
        party
            .personas
            .iter()
            .map(|persona| party_data.roster().model_name(&persona.model))
            .collect()
    });
    // Each new line and each new word scrolls to the end.
    let seen = (
        room.id(),
        lines.len(),
        speaking
            .as_ref()
            .map_or(0, |speaking| speaking.text.len() + 1),
    );
    if view.talk_seen != Some(seen) {
        view.talk_seen = Some(seen);
        view.talk_scroll.scroll_to_bottom();
    }
    let reduced = motion::reduced(cx);
    let mut talk = div().flex().flex_col().gap(px(12.)).px(px(16.)).py(px(12.));
    if let Some(party) = &party {
        talk = talk.child(cast_card(party, &model_names, p));
        for line in &lines {
            // A line that starts in Arabic reads from the right edge of its bubble.
            talk = talk.child(
                div()
                    .when(reads::starts_right(&line.text), Styled::text_right)
                    .child(bubble(line, party, &view.store, p)),
            );
        }
        if let Some(speaking) = &speaking {
            let color = party
                .persona(&speaking.who)
                .map_or(p.faint, |persona| persona_color(&persona.color, p));
            talk = talk.child(
                div()
                    .when(reads::starts_right(&speaking.text), Styled::text_right)
                    .child(speaking_bubble(speaking, color, reduced)),
            );
        }
    }
    div()
        .flex()
        .flex_col()
        .size_full()
        .child(head(party.as_ref(), p, cx))
        .child(
            div()
                .id("party-talk")
                .flex_1()
                .overflow_y_scroll()
                .track_scroll(&view.talk_scroll)
                .child(talk),
        )
        .child(footer(view, party.as_ref(), problem, p, cx))
}

/// The bar: Back to the list, the title, and the controls for where the party stands.
fn head(party: Option<&Party>, p: &Pal, cx: &mut Context<'_, PartyWindow>) -> Div {
    let mut bar = div()
        .flex()
        .items_center()
        .gap(px(6.))
        .h(px(44.))
        .px(px(10.))
        .border_b_1()
        .border_color(p.border)
        .child(
            Button::new("party-back")
                .ghost()
                .small()
                .icon(Icon::new(IconName::ChevronLeft))
                .accessibility_label("Your parties")
                .tooltip("Your parties")
                .on_click(cx.listener(|this, _, _, cx| {
                    this.store
                        .update(cx, |store, cx| store.open_party(None, cx));
                })),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .font_weight(FontWeight::SEMIBOLD)
                .child(party.map_or_else(
                    || "Party".to_owned(),
                    |party| direction::marked(&party.title),
                )),
        );
    let Some(party) = party else {
        return bar;
    };
    let control = |id: &'static str,
                   label: &'static str,
                   name: IconName,
                   tip: &'static str,
                   action: PartyAction,
                   cx: &mut Context<'_, PartyWindow>| {
        Button::new(id)
            .ghost()
            .small()
            .icon(Icon::new(name))
            .label(label)
            .tooltip(tip)
            .on_click(cx.listener(move |this, _, _, cx| {
                let action = action.clone();
                this.store
                    .update(cx, |store, cx| store.act_on_party(action, cx));
            }))
    };
    let paused = party.status == PartyStatus::Paused;
    bar = bar.child(if party.status == PartyStatus::Running {
        control(
            "party-pause",
            "Pause",
            IconName::Pause,
            "Pause after this turn",
            PartyAction::Pause,
            cx,
        )
    } else if paused {
        control(
            "party-resume",
            "Resume",
            IconName::Play,
            "Carry on",
            PartyAction::Resume,
            cx,
        )
    } else {
        control(
            "party-resume",
            "Continue",
            IconName::Play,
            "Carry on for 10 more turns",
            PartyAction::Resume,
            cx,
        )
    });
    if party.status == PartyStatus::Running || paused {
        bar = bar.child(
            control(
                "party-stop",
                "Stop",
                IconName::Square,
                "Stop now",
                PartyAction::Stop,
                cx,
            )
            .danger(),
        );
    }
    bar
}

/// The cast: each persona's colour, name and role, and its model.
fn cast_card(party: &Party, model_names: &[String], p: &Pal) -> Div {
    let mut card = div()
        .flex()
        .flex_col()
        .gap(px(8.))
        .p(px(12.))
        .mb(px(4.))
        .rounded(px(12.))
        .bg(p.card)
        .border_1()
        .border_color(p.border);
    for (persona, model) in party.personas.iter().zip(model_names) {
        card = card.child(
            div()
                .flex()
                .items_start()
                .gap(px(8.))
                .text_size(px(12.))
                .child(
                    div()
                        .pt(px(4.))
                        .child(dot(persona_color(&persona.color, p), 9.)),
                )
                .child(
                    div().flex_1().min_w_0().line_clamp(2).child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap(px(4.))
                            .child(
                                div()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(persona.name.clone()),
                            )
                            .child(div().text_color(p.muted).child(persona.role.clone())),
                    ),
                )
                .child(
                    div()
                        .flex_none()
                        .text_size(px(11.))
                        .text_color(p.faint)
                        .child(model.clone()),
                ),
        );
    }
    card
}

/// A line of the talk: a persona's turn, the user's words, or their direction.
fn bubble(line: &PartyLine, party: &Party, store: &Entity<Store>, p: &Pal) -> AnyElement {
    let id = |part: &str| ElementId::Name(format!("party-line-{}-{part}", line.id).into());
    match line.kind {
        LineKind::Steer => div()
            .flex()
            .justify_center()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .px(px(12.))
                    .py(px(6.))
                    .rounded_full()
                    .bg(p.accent.opacity(0.12))
                    .text_color(p.accent)
                    .text_size(px(12.))
                    .font_weight(FontWeight::MEDIUM)
                    .child(glyph(IconName::WandSparkles, p.accent, 13.))
                    .child(line.text.clone()),
            )
            .into_any_element(),
        LineKind::Say => div()
            .flex()
            .justify_end()
            .pl(px(48.))
            .child(
                div()
                    .px(px(13.))
                    .py(px(9.))
                    .rounded(px(17.))
                    .bg(p.mine)
                    .text_color(p.on_mine)
                    .child(selectable(id("say"), &line.text, store)),
            )
            .into_any_element(),
        _ => {
            let color = party
                .persona(&line.who)
                .map_or(p.faint, |persona| persona_color(&persona.color, p));
            let text = if line.cut {
                format!("{} —", line.text)
            } else {
                line.text.clone()
            };
            turn(
                &line.name,
                color,
                selectable(id("turn"), &text, store).into_any_element(),
            )
        }
    }
}

/// A persona's turn: its initial and name in its colour over a bubble tinted with it.
fn turn(name: &str, color: Hsla, words: AnyElement) -> AnyElement {
    div()
        .flex()
        .items_start()
        .gap(px(9.))
        .pr(px(24.))
        .child(initial(name, color, 28.))
        .child(
            div()
                .flex()
                .flex_col()
                .min_w_0()
                .gap(px(3.))
                .child(
                    div()
                        .text_size(px(12.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(color)
                        .child(name.to_owned()),
                )
                .child(
                    div()
                        .px(px(12.))
                        .py(px(8.))
                        .rounded(px(15.))
                        .bg(color.opacity(0.10))
                        .child(words),
                ),
        )
        .into_any_element()
}

/// Who is speaking: their words so far, or a pulsing ellipsis before the first one (still when
/// the system asks for less motion).
fn speaking_bubble(speaking: &Speaking, color: Hsla, reduced: bool) -> AnyElement {
    let words = if speaking.text.is_empty() {
        let ellipsis = glyph(IconName::Ellipsis, color, 16.);
        if reduced {
            ellipsis.into_any_element()
        } else {
            div()
                .child(ellipsis)
                .with_animation(
                    "party-speaking",
                    Animation::new(Duration::from_millis(1200))
                        .repeat()
                        .with_easing(pulsating_between(0.3, 1.)),
                    Styled::opacity,
                )
                .into_any_element()
        }
    } else {
        div().child(speaking.text.clone()).into_any_element()
    };
    turn(&speaking.name, color, words)
}

/// Text the user can select and copy, shown as it was written (not as markdown), as the
/// reference client shows a party's lines. A link in it opens only as a web or mail link
/// ([`follow_web_link`]).
fn selectable(id: ElementId, text: &str, store: &Entity<Store>) -> TextView {
    let store = store.clone();
    TextView::markdown(id, crate::prose::literal::literal(text))
        .selectable(true)
        .on_link_click(move |url, _, _, cx| follow_web_link(&store, url, cx))
}

/// The foot: where the party stands, why it stopped, a problem, and the user's line.
fn footer(
    view: &PartyWindow,
    party: Option<&Party>,
    problem: Option<String>,
    p: &Pal,
    cx: &mut Context<'_, PartyWindow>,
) -> Div {
    let mut foot = div()
        .flex()
        .flex_col()
        .gap(px(8.))
        .px(px(14.))
        .pt(px(8.))
        .pb(px(10.))
        .border_t_1()
        .border_color(p.border)
        .bg(p.surface);
    if let Some(party) = party {
        foot = foot.child(
            div()
                // Said as the party starts, pauses, stops or ends.
                .id("party-status")
                .announced(words::status_word(&party.status))
                .flex()
                .items_center()
                .gap(px(6.))
                .text_size(px(12.))
                .text_color(p.muted)
                .child(dot(status_color(&party.status, p), 7.))
                .child(
                    div()
                        .font_weight(FontWeight::MEDIUM)
                        .child(words::status_word(&party.status)),
                )
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .child(format!("· {}", words::meter(party))),
                ),
        );
        if let Some(why) = party
            .why
            .clone()
            .filter(|_| party.status != PartyStatus::Running)
        {
            let color = if party.status == PartyStatus::Paused {
                p.warn
            } else {
                p.muted
            };
            foot = foot.child(div().text_size(px(12.)).text_color(color).child(why));
        }
    }
    if let Some(problem) = problem {
        foot = foot.child(
            div()
                .id("party-problem")
                .announced(problem.clone())
                .text_size(px(12.))
                .text_color(p.warn)
                .child(problem),
        );
    }
    let ready = !view.line.read(cx).value().trim().is_empty();
    foot.child(
        div()
            .flex()
            .items_end()
            .gap(px(8.))
            .child(mode_menu(view.steer, p, cx))
            .child(div().flex_1().min_w_0().child(Textarea::new(&view.line)))
            .child(
                div()
                    .id("party-send")
                    .button_named("Send")
                    .accessibility_id("party send")
                    .p(px(2.))
                    .cursor_pointer()
                    .child(glyph(
                        IconName::CircleArrowUp,
                        if ready { p.accent } else { p.faint },
                        28.,
                    ))
                    .on_click(cx.listener(|this, _, window, cx| this.send_line(window, cx))),
            ),
    )
}

/// What the user's line does: jump in (talk with them) or steer (tell them where to go).
fn mode_menu(steer: bool, p: &Pal, cx: &mut Context<'_, PartyWindow>) -> impl IntoElement {
    let this = cx.entity().downgrade();
    let (name, tip) = if steer {
        (
            IconName::WandSparkles,
            "Steer: a direction everyone follows",
        )
    } else {
        (
            IconName::MessageSquareText,
            "Jump in: your words join the talk",
        )
    };
    div().text_color(p.muted).child(
        Button::new("party-mode")
            .ghost()
            .small()
            .icon(Icon::new(name))
            .accessibility_label(tip)
            .tooltip(tip)
            .dropdown_menu_with_anchor(Anchor::BottomLeft, move |menu: PopupMenu, _, _| {
                let mut menu = menu;
                for (label, glyph_name, choice) in [
                    (
                        "Jump in: talk with them",
                        IconName::MessageSquareText,
                        false,
                    ),
                    ("Steer: tell them where to go", IconName::WandSparkles, true),
                ] {
                    let this = this.clone();
                    menu = menu.item(
                        PopupMenuItem::new(label)
                            .icon(Icon::new(glyph_name))
                            .checked(steer == choice)
                            .on_click(move |_, window, cx| {
                                if let Err(error) = this.update(cx, |this, cx| {
                                    this.set_steer(choice, window, cx);
                                }) {
                                    log::debug!("the Party window closed: {error}");
                                }
                            }),
                    );
                }
                menu
            }),
    )
}
