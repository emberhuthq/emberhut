//! What your ember knows about you: a search, then "To confirm" (facts kept from something the
//! user didn't start, each to confirm or forget), "Learning from you" folded into one line (the
//! lessons from the user's corrections, each to keep or drop before the weekly review decides),
//! then the facts by category, with a chip per category to narrow the list, and last the memory
//! itself: the nightly review's switch and the export into Downloads. Under each fact a line says
//! why the assistant knows it. A fact's pin menu pins it in what the assistant always knows or
//! hides it from that; its Edit opens "Fix a fact" (correct it, mark whether a person wakes the
//! assistant, forget it); its Forget asks once, saying it forgets everywhere.

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonRounded, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::{ContextMenuExt as _, DropdownMenu as _, PopupMenu, PopupMenuItem};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{Disableable as _, Icon, Selectable as _, Sizable as _};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, Div, ElementId, Entity, FocusHandle, FontWeight, Stateful, WeakEntity, Window, div,
    px,
};
use jiff::tz::TimeZone;

use super::parts::{note, page_body, problem_line, section, selectable, sheet};
use crate::store::Store;
use crate::store::settings::about::{
    Export, ProfileSay, SEARCH, Shelf, chips, decide_by, grouped, lesson, listed_count, pending,
    trials, why,
};
use crate::store::settings::words::day_month;
use crate::ui::controls::tip;
use crate::ui::palette::{Pal, pal};
use emberhut_api::facts::{Fact, FactCategory, FactsPage, Trial};
use emberhut_api::ids::{FactId, TrialId};

/// "Learning from you", for tests to find.
pub(super) const LEARNING: &str = "about-learning";

/// The line that opens and folds "Learning from you", for tests to find.
pub(super) const LEARNING_TOGGLE: &str = "about-learning-toggle";

/// A lesson's row, for tests to find.
pub(super) const TRIAL_ROW: &str = "about-trial";

/// The Keep button of a lesson, for tests to find.
pub(super) const TRIAL_KEEP: &str = "about-trial-keep";

/// The Drop button of a lesson, for tests to find.
pub(super) const TRIAL_DROP: &str = "about-trial-drop";

/// The category chips, for tests to find.
pub(super) const CHIPS: &str = "about-chips";

/// A fact's row, for tests to find.
pub(super) const FACT_ROW: &str = "about-fact";

/// A fact's Edit button, for tests to find.
pub(super) const FACT_EDIT: &str = "about-fact-edit";

/// A fact's Forget button, for tests to find.
pub(super) const FACT_FORGET: &str = "about-fact-forget";

/// "To confirm", for tests to find.
pub(super) const TO_CONFIRM: &str = "about-to-confirm";

/// A fact's line saying why the assistant knows it, for tests to find.
pub(super) const FACT_WHY: &str = "about-fact-why";

/// A fact's pin menu, for tests to find.
pub(super) const FACT_PROFILE: &str = "about-fact-profile";

/// The export button, for tests to find.
pub(super) const EXPORT: &str = "about-export";

/// The nightly review's switch, for tests to find.
pub(super) const NIGHTLY: &str = "about-nightly";

/// What "To confirm" is, under it.
const TO_CONFIRM_NOTE: &str = "Kept from someone else's message. Not used until you confirm it.";

/// What Forget does, as its tooltip says it.
const FORGET_TIP: &str = "Forget everywhere: removes it and what was said about it from my memory";

/// What a person marked important does, as their bell's tooltip says it.
const WAKES: &str = "Tells you right away when they write";

/// The What your ember knows about you page.
pub(crate) struct AboutPane {
    store: Entity<Store>,
    search: Entity<InputState>,
    /// The category chip chosen; none: All.
    shelf: Option<Shelf>,
    /// The fact open in "Fix a fact".
    editing: Option<FactId>,
    /// Its text as being corrected.
    pub(super) text: Entity<InputState>,
    /// "Forget this" asked in the sheet, waiting for its confirmation.
    forgetting: bool,
    /// The fact whose Forget, on its row, waits for its confirmation.
    forgetting_row: Option<FactId>,
    /// The lesson whose Drop waits for its confirmation.
    dropping: Option<TrialId>,
    /// The sheet's focus.
    focus: FocusHandle,
}

impl AboutPane {
    /// The page over `store`.
    pub(crate) fn new(
        store: Entity<Store>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Self {
        cx.observe(&store, |_, _, cx| cx.notify()).detach();
        let search = cx.new(|cx| InputState::new(window, cx).placeholder(SEARCH));
        cx.subscribe(&search, |_, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        })
        .detach();
        let text = cx.new(|cx| InputState::new(window, cx).placeholder("What's true"));
        cx.subscribe_in(
            &text,
            window,
            |this, _, event: &InputEvent, _, cx| match event {
                InputEvent::PressEnter { .. } => this.save(cx),
                InputEvent::Change => cx.notify(),
                _ => {}
            },
        )
        .detach();
        Self {
            store,
            search,
            shelf: None,
            editing: None,
            text,
            forgetting: false,
            forgetting_row: None,
            dropping: None,
            focus: cx.focus_handle(),
        }
    }

    /// Opens "Fix a fact" on `fact`.
    pub(super) fn edit(&mut self, fact: &Fact, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.editing = Some(fact.id.clone());
        self.forgetting = false;
        self.forgetting_row = None;
        let text = fact.text.clone();
        self.text.update(cx, |input, cx| {
            input.set_value(text, window, cx);
            input.focus(window, cx);
        });
        cx.notify();
    }

    /// Asks, on its row, whether to forget fact `id`.
    fn ask_to_forget(&mut self, id: FactId, cx: &mut Context<'_, Self>) {
        self.forgetting_row = Some(id);
        cx.notify();
    }

    fn close(&mut self, cx: &mut Context<'_, Self>) {
        self.editing = None;
        self.forgetting = false;
        cx.notify();
    }

    /// The fact being fixed, as the store has it now.
    fn edited(&self, cx: &Context<'_, Self>) -> Option<Fact> {
        let id = self.editing.as_ref()?;
        self.store.read(cx).facts().fact(id).cloned()
    }

    /// Whether the text typed differs from the fact's, and isn't blank.
    fn can_save(&self, fact: &Fact, cx: &Context<'_, Self>) -> bool {
        let typed = self.text.read(cx).value();
        !typed.trim().is_empty() && typed.as_ref() != fact.text
    }

    /// Saves the corrected text, and closes the sheet.
    fn save(&mut self, cx: &mut Context<'_, Self>) {
        let Some(fact) = self.edited(cx) else {
            return;
        };
        if !self.can_save(&fact, cx) {
            return;
        }
        let text = self.text.read(cx).value().to_string();
        self.store
            .update(cx, |store, cx| store.fix_fact(&fact, text, cx));
        self.editing = None;
        cx.notify();
    }

    /// "Learning from you": one line saying how many lessons wait and the day to decide by,
    /// which opens in place to each lesson.
    fn learning(&self, list: &[&Trial], open: bool, p: &Pal, cx: &mut Context<'_, Self>) -> Div {
        let zone = TimeZone::system();
        let count = match list.len() {
            1 => "1 lesson to review".to_owned(),
            n => format!("{n} lessons to review"),
        };
        let summary = match decide_by(list.iter().copied()) {
            Some(by) => format!(
                "{count} · decide by {}",
                day_month(&by.to_string(), &zone).unwrap_or_default()
            ),
            None => count,
        };
        let chevron = if open {
            IconName::ChevronDown
        } else {
            IconName::ChevronRight
        };
        // Its button opens and folds it, for the pointer, the keyboard and a screen reader.
        let toggle = div()
            .flex()
            .items_center()
            .gap(px(4.))
            .px(px(6.))
            .py(px(5.))
            .child(
                div().debug_selector(|| LEARNING_TOGGLE.to_owned()).child(
                    Button::new(LEARNING_TOGGLE)
                        .ghost()
                        .icon(chevron)
                        .label("Learning from you")
                        .accessibility_label(format!("Learning from you, {summary}"))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.store.update(cx, Store::toggle_learning);
                        })),
                ),
            )
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_size(px(12.))
                    .text_color(p.muted)
                    .child(summary),
            );
        let card = div()
            .id(LEARNING)
            .debug_selector(|| LEARNING.to_owned())
            .flex()
            .flex_col()
            .rounded(px(12.))
            .border_1()
            .border_color(p.border)
            .bg(p.card)
            .child(toggle);
        if !open {
            return div().child(card);
        }
        let rows = list.iter().map(|trial| {
            div()
                .px(px(14.))
                .py(px(10.))
                .border_t_1()
                .border_color(p.border)
                .child(self.lesson_row(trial, &zone, p, cx))
        });
        div().child(card.children(rows))
    }

    /// A lesson: what the user said, as a quote; the lesson in plain words; until when it is
    /// on trial and what happens without a choice; Keep and Drop.
    fn lesson_row(
        &self,
        trial: &Trial,
        zone: &TimeZone,
        p: &Pal,
        cx: &mut Context<'_, Self>,
    ) -> Div {
        let id = trial.id;
        let confirming = self.dropping == Some(id);
        let plain = lesson(trial).to_owned();
        let until = format!(
            "On trial until {}. If you don't choose, the weekly review decides.",
            day_month(trial.ends.as_str(), zone).unwrap_or_default()
        );
        let said = trial.said.trim();
        let words = div()
            .flex()
            .flex_col()
            .gap(px(4.))
            // Without `min_w_0` the column is as wide as its longest line unwrapped, and pushes
            // the row and its buttons out of the card.
            .flex_1()
            .min_w_0()
            .when(!said.is_empty(), |d| {
                d.child(
                    div()
                        .border_l_2()
                        .border_color(p.accent)
                        .pl(px(10.))
                        .text_size(px(13.5))
                        .italic()
                        .child(format!("\u{201c}{said}\u{201d}")),
                )
            })
            .child(
                div()
                    .text_size(px(13.))
                    .when(!said.is_empty(), |d| d.text_color(p.muted))
                    .child(plain.clone()),
            )
            .child(div().text_size(px(11.)).text_color(p.faint).child(until));
        let buttons = div()
            .flex()
            .flex_none()
            .gap(px(6.))
            .child(
                div().debug_selector(|| TRIAL_KEEP.to_owned()).child(
                    Button::new(ElementId::Name(format!("trial-{id}-keep").into()))
                        .small()
                        .outline()
                        .label("Keep")
                        .accessibility_label(format!("Keep the lesson: {plain}"))
                        .tooltip("Keep it from now on, without waiting for the review")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.dropping = None;
                            this.store.update(cx, |store, cx| store.keep_trial(id, cx));
                        })),
                ),
            )
            .child(
                div().debug_selector(|| TRIAL_DROP.to_owned()).child(
                    Button::new(ElementId::Name(format!("trial-{id}-drop").into()))
                        .small()
                        .outline()
                        .label("Drop")
                        .accessibility_label(format!("Drop the lesson: {plain}"))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.dropping = Some(id);
                            cx.notify();
                        })),
                ),
            );
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .debug_selector(|| TRIAL_ROW.to_owned())
            .child(
                div()
                    .flex()
                    .items_start()
                    .gap(px(10.))
                    .child(words)
                    .child(buttons),
            )
            .when(confirming, |d| {
                d.child(confirm(
                    p,
                    "Drop this lesson? Your ember stops following it.".to_owned(),
                    (
                        ElementId::Name(format!("trial-{id}-confirm").into()),
                        "Drop",
                    ),
                    cx.listener(move |this, _, _, cx| {
                        this.dropping = None;
                        this.store.update(cx, |store, cx| store.drop_trial(id, cx));
                    }),
                    cx.listener(|this, _, _, cx| {
                        this.dropping = None;
                        cx.notify();
                    }),
                ))
            })
    }

    /// The category chips: All, then each category with facts and how many; the one chosen is
    /// `active`.
    fn chips(
        list: Vec<(Option<Shelf>, usize)>,
        active: Option<&Shelf>,
        cx: &mut Context<'_, Self>,
    ) -> Div {
        let chips = list.into_iter().enumerate().map(|(i, (shelf, count))| {
            let title = shelf.as_ref().map_or("All", Shelf::title);
            let label = if shelf.is_some() {
                format!("{title} {count}")
            } else {
                title.to_owned()
            };
            let selected = shelf.as_ref() == active;
            let chip = Button::new(ElementId::Name(format!("about-chip-{i}").into()))
                .small()
                .rounded(ButtonRounded::Large)
                .label(label)
                .accessibility_label(format!("{title}: {count} facts"))
                .selected(selected)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.shelf.clone_from(&shelf);
                    cx.notify();
                }));
            if selected {
                chip.primary()
            } else {
                chip.outline()
            }
        });
        div()
            .debug_selector(|| CHIPS.to_owned())
            .flex()
            .flex_wrap()
            .gap(px(6.))
            .children(chips)
    }

    /// A fact's words: its title (with a bell for a person whose messages wake the assistant,
    /// and whether it is pinned or hidden), what is true, the assistant's note on a person, why
    /// the assistant knows it, and where it came from and when. A click opens "Fix a fact" (on
    /// one confirmed).
    fn fact_words(fact: &Fact, p: &Pal, cx: &mut Context<'_, Self>) -> Stateful<Div> {
        let zone = TimeZone::system();
        let origin = [
            fact.source.clone(),
            day_month(fact.updated.as_str(), &zone).unwrap_or_default(),
        ]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" · ");
        let id = &fact.id;
        let open = fact.clone();
        div()
            .id(ElementId::Name(format!("fact-{id}-words").into()))
            .flex()
            .flex_col()
            .gap(px(3.))
            .flex_1()
            .min_w_0()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .child(
                        div()
                            .min_w_0()
                            .text_size(px(12.5))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(fact.title.clone()),
                    )
                    .when(fact.is_important(), |d| {
                        d.child(tip(
                            div()
                                .id(ElementId::Name(format!("fact-{id}-wakes").into()))
                                .flex_none()
                                .child(Icon::new(IconName::Bell).size(px(11.)).text_color(p.warn)),
                            WAKES,
                        ))
                    })
                    .when_some(ProfileSay::of(fact).badge(), |d, badge| {
                        d.child(
                            div()
                                .flex_none()
                                .px(px(5.))
                                .rounded(px(4.))
                                .bg(p.accent.opacity(0.12))
                                .text_size(px(10.5))
                                .text_color(p.muted)
                                .child(badge),
                        )
                    }),
            )
            .when(!fact.text.is_empty(), |d| {
                d.child(div().text_size(px(13.)).child(fact.text.clone()))
            })
            .when_some(fact.now.clone(), |d, now| {
                d.child(
                    div()
                        .text_size(px(12.))
                        .text_color(p.muted)
                        .child(format!("Now: {now}")),
                )
            })
            .when_some(why(fact), |d, why| {
                d.child(
                    div()
                        .debug_selector(|| FACT_WHY.to_owned())
                        .text_size(px(11.))
                        .text_color(p.muted)
                        .child(why),
                )
            })
            .when(!origin.is_empty(), |d| {
                d.child(div().text_size(px(11.)).text_color(p.faint).child(origin))
            })
            // One waiting to be confirmed is confirmed or forgotten, not fixed.
            .when(!fact.pending, |d| {
                d.cursor_pointer()
                    .on_click(cx.listener(move |this, _, window, cx| this.edit(&open, window, cx)))
            })
    }

    /// A fact's buttons, always shown: its pin menu, Edit and Forget; Confirm and Forget for
    /// one that waits to be confirmed.
    fn fact_buttons(&self, fact: &Fact, cx: &mut Context<'_, Self>) -> Div {
        let (id, title) = (&fact.id, &fact.title);
        let (edit, forget) = (fact.clone(), fact.id.clone());
        let forget_button = div().debug_selector(|| FACT_FORGET.to_owned()).child(
            Button::new(ElementId::Name(format!("fact-{id}-forget").into()))
                .small()
                .outline()
                .icon(IconName::Trash)
                .accessibility_label(format!("Forget {title} everywhere"))
                .tooltip(FORGET_TIP)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.ask_to_forget(forget.clone(), cx);
                })),
        );
        let buttons = div().flex().flex_none().gap(px(4.));
        if fact.pending {
            let confirmed = fact.id.clone();
            return buttons
                .child(
                    Button::new(ElementId::Name(format!("fact-{id}-confirm").into()))
                        .small()
                        .primary()
                        .label("Confirm")
                        .accessibility_label(format!("Confirm {title}"))
                        .tooltip("Keep it: I use it from now on")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let id = confirmed.clone();
                            this.store
                                .update(cx, |store, cx| store.confirm_fact(id, cx));
                        })),
                )
                .child(forget_button);
        }
        buttons
            .child(self.profile_menu(fact))
            .child(
                div().debug_selector(|| FACT_EDIT.to_owned()).child(
                    Button::new(ElementId::Name(format!("fact-{id}-edit").into()))
                        .small()
                        .outline()
                        .icon(IconName::Pencil)
                        .accessibility_label(format!("Edit {title}"))
                        .tooltip("Edit")
                        .on_click(
                            cx.listener(move |this, _, window, cx| this.edit(&edit, window, cx)),
                        ),
                ),
            )
            .child(forget_button)
    }

    /// A fact's pin menu: always in what the assistant knows, never, or as it comes, the say
    /// it has now checked.
    fn profile_menu(&self, fact: &Fact) -> Div {
        let (id, title) = (fact.id.clone(), fact.title.clone());
        let now = ProfileSay::of(fact);
        let store = self.store.clone();
        div().debug_selector(|| FACT_PROFILE.to_owned()).child(
            Button::new(ElementId::Name(format!("fact-{id}-profile").into()))
                .small()
                .outline()
                .icon(IconName::Pin)
                .selected(now != ProfileSay::Neither)
                .accessibility_label(format!("{title} in what I always know: {}", now.label()))
                .tooltip("Pin it in what I always know, or keep it out")
                .dropdown_menu(move |menu, _, _| profile_items(menu, &store, &id, now)),
        )
    }

    /// A fact's row: its words, Edit and Forget, Forget's question when asked, and the same
    /// two in its right-click menu.
    fn fact_row(&self, fact: &Fact, p: &Pal, cx: &mut Context<'_, Self>) -> AnyElement {
        let id = fact.id.clone();
        let confirming = self.forgetting_row.as_ref() == Some(&id);
        let title = fact.title.clone();
        let pane = cx.entity().downgrade();
        let words = Self::fact_words(fact, p, cx);
        let buttons = self.fact_buttons(fact, cx);
        let menu_fact = fact.clone();
        let store = self.store.clone();
        div()
            .id(ElementId::Name(format!("fact-{id}").into()))
            .debug_selector(|| FACT_ROW.to_owned())
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(
                div()
                    .flex()
                    .items_start()
                    .gap(px(10.))
                    .child(words)
                    .child(buttons),
            )
            .when(confirming, |d| {
                let gone = id.clone();
                d.child(confirm(
                    p,
                    forget_question(&title),
                    (
                        ElementId::Name(format!("fact-{id}-forget-confirm").into()),
                        "Forget everywhere",
                    ),
                    cx.listener(move |this, _, _, cx| {
                        this.forgetting_row = None;
                        let gone = gone.clone();
                        this.store
                            .update(cx, |store, cx| store.forget_fact(gone, cx));
                    }),
                    cx.listener(|this, _, _, cx| {
                        this.forgetting_row = None;
                        cx.notify();
                    }),
                ))
            })
            .context_menu(move |menu, _, _| fact_menu(menu, &pane, &store, &menu_fact))
            .into_any_element()
    }

    /// The facts' part of the page: the chips, then each category's facts (only the chosen
    /// chip's), or a line saying nothing matches the search when nothing does (`lessons`: some
    /// lesson or fact to confirm matches it).
    fn facts_parts(
        &self,
        page: &FactsPage,
        query: &str,
        lessons: bool,
        p: &Pal,
        cx: &mut Context<'_, Self>,
    ) -> Vec<AnyElement> {
        let mut parts = Vec::new();
        let chip_list = chips(page, query);
        // A chip the search left without facts reads as All.
        let active = self.shelf.clone().filter(|shelf| {
            chip_list
                .iter()
                .any(|(chip, _)| chip.as_ref() == Some(shelf))
        });
        let shelves = grouped(page, query);
        let nothing_found = shelves.is_empty() && !lessons && !query.trim().is_empty();
        if !shelves.is_empty() {
            parts.push(Self::chips(chip_list, active.as_ref(), cx).into_any_element());
        }
        let shown = shelves
            .into_iter()
            .filter(|(shelf, _)| active.as_ref().is_none_or(|active| active == shelf));
        for (shelf, facts) in shown {
            let rows = facts
                .iter()
                .map(|fact| self.fact_row(fact, p, cx))
                .collect();
            let head = div()
                .flex()
                .items_center()
                .gap(px(6.))
                .child(
                    Icon::new(category_icon(&shelf))
                        .size(px(13.))
                        .text_color(p.muted),
                )
                .child(shelf.title());
            parts.push(
                section(p, head.into_any_element(), Some(facts.len()), rows, None)
                    .into_any_element(),
            );
        }
        if nothing_found {
            let line = format!("Nothing matches \u{201c}{}\u{201d}.", query.trim());
            parts.push(note(p, line).into_any_element());
        }
        parts
    }

    /// "To confirm": the facts that wait for the user, each with Confirm and Forget, and what
    /// waiting means.
    fn to_confirm(&self, facts: &[&Fact], p: &Pal, cx: &mut Context<'_, Self>) -> Div {
        let rows = facts
            .iter()
            .map(|fact| self.fact_row(fact, p, cx))
            .collect();
        let head = div()
            .flex()
            .items_center()
            .gap(px(6.))
            .child(
                Icon::new(IconName::CircleQuestionMark)
                    .size(px(13.))
                    .text_color(p.muted),
            )
            .child("To confirm");
        div()
            .debug_selector(|| TO_CONFIRM.to_owned())
            .child(section(
                p,
                head.into_any_element(),
                Some(facts.len()),
                rows,
                Some(TO_CONFIRM_NOTE.to_owned()),
            ))
    }

    /// The memory itself: the nightly review's switch, and the export into Downloads with how
    /// the last one went.
    fn memory_part(&self, nightly: bool, export: Option<&Export>, p: &Pal) -> Div {
        let switch_store = self.store.clone();
        let nightly_row = div()
            .flex()
            .flex_col()
            .gap(px(4.))
            .child(
                div().debug_selector(|| NIGHTLY.to_owned()).child(
                    Switch::new(NIGHTLY)
                        .checked(nightly)
                        .label("Review my memory every night")
                        .on_click(move |on, _, cx| {
                            let on = *on;
                            switch_store.update(cx, |store, cx| store.set_memory_nightly(on, cx));
                        }),
                ),
            )
            .child(note(
                p,
                "Each night I merge repeats, close what is out of date and tidy what I keep.",
            ));
        let saving = export == Some(&Export::Saving);
        let line = match export {
            Some(Export::Saved(name)) => format!("Saved to Downloads as {name}."),
            _ => "A zip of every fact, note and lesson I keep, saved to your Downloads.".to_owned(),
        };
        let export_store = self.store.clone();
        let export_row = div()
            .flex()
            .items_center()
            .gap(px(10.))
            .child(div().flex_1().min_w_0().child(note(p, line)))
            .child(
                div()
                    .flex_none()
                    .debug_selector(|| EXPORT.to_owned())
                    .child(
                        Button::new(EXPORT)
                            .small()
                            .outline()
                            .icon(IconName::Download)
                            .label("Export everything I know")
                            .loading(saving)
                            .disabled(saving)
                            .on_click(move |_, _, cx| {
                                export_store.update(cx, Store::export_memory);
                            }),
                    ),
            );
        section(
            p,
            "Your memory",
            None,
            vec![
                nightly_row.into_any_element(),
                export_row.into_any_element(),
            ],
            None,
        )
    }

    /// "Fix a fact": the text, where it came from, a person's note and their switch, Forget.
    fn editor(&self, fact: &Fact, p: &Pal, cx: &mut Context<'_, Self>) -> Div {
        let from = if fact.source.is_empty() {
            "your ember"
        } else {
            fact.source.as_str()
        };
        let can_save = self.can_save(fact, cx);
        let person = fact.category == FactCategory::People;
        let title = fact.title.clone();
        let body = div()
            .flex()
            .flex_col()
            .gap(px(14.))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(p.muted)
                            .child(fact.title.clone()),
                    )
                    .child(Input::new(&self.text))
                    .child(note(
                        p,
                        format!("From {from}. Your ember will use your version from now on."),
                    ))
                    .child(
                        div().flex().justify_end().child(
                            Button::new("fact-save")
                                .primary()
                                .label("Save")
                                .disabled(!can_save)
                                .on_click(cx.listener(|this, _, _, cx| this.save(cx))),
                        ),
                    ),
            )
            .when(person, |d| d.child(person_part(&self.store, fact, p)))
            .child(if self.forgetting {
                confirm(
                    p,
                    forget_question(&title),
                    ("fact-forget-confirm".into(), "Forget everywhere"),
                    cx.listener(|this, _, _, cx| {
                        if let Some(id) = this.editing.clone() {
                            this.store.update(cx, |store, cx| store.forget_fact(id, cx));
                        }
                        this.close(cx);
                    }),
                    cx.listener(|this, _, _, cx| {
                        this.forgetting = false;
                        cx.notify();
                    }),
                )
            } else {
                div().child(
                    Button::new("fact-forget")
                        .ghost()
                        .danger()
                        .label("Forget everywhere")
                        .tooltip(FORGET_TIP)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.forgetting = true;
                            cx.notify();
                        })),
                )
            });
        let close = cx.entity().downgrade();
        sheet(
            p,
            &self.focus,
            "Fix a fact",
            body,
            ("Cancel", move |_, cx| {
                if let Err(error) = close.update(cx, AboutPane::close) {
                    log::debug!("the page closed with its sheet: {error}");
                }
            }),
        )
    }
}

/// A fact's right-click menu: Edit, where it stands in what the assistant always knows, and
/// Forget everywhere (which asks on the row, as its button does); Confirm and Forget for one
/// that waits to be confirmed.
fn fact_menu(
    mut menu: PopupMenu,
    pane: &WeakEntity<AboutPane>,
    store: &Entity<Store>,
    fact: &Fact,
) -> PopupMenu {
    let (edit_pane, forget_pane) = (pane.clone(), pane.clone());
    let (edit, forget) = (fact.clone(), fact.id.clone());
    if fact.pending {
        let (store, id) = (store.clone(), fact.id.clone());
        menu = menu.item(PopupMenuItem::new("Confirm").on_click(move |_, _, cx| {
            let id = id.clone();
            store.update(cx, |store, cx| store.confirm_fact(id, cx));
        }));
    } else {
        menu = menu.item(PopupMenuItem::new("Edit…").on_click(move |_, window, cx| {
            if let Err(error) = edit_pane.update(cx, |pane, cx| pane.edit(&edit, window, cx)) {
                log::debug!("the page closed with its menu: {error}");
            }
        }));
        menu = profile_items(menu.separator(), store, &fact.id, ProfileSay::of(fact)).separator();
    }
    menu.item(
        PopupMenuItem::new("Forget everywhere…").on_click(move |_, _, cx| {
            let forget = forget.clone();
            if let Err(error) = forget_pane.update(cx, |pane, cx| pane.ask_to_forget(forget, cx)) {
                log::debug!("the page closed with its menu: {error}");
            }
        }),
    )
}

/// The says on a fact in what the assistant always knows, as menu items, `now` checked.
fn profile_items(
    mut menu: PopupMenu,
    store: &Entity<Store>,
    id: &FactId,
    now: ProfileSay,
) -> PopupMenu {
    for say in ProfileSay::ALL {
        let (store, id) = (store.clone(), id.clone());
        menu = menu.item(
            PopupMenuItem::new(say.label())
                .checked(say == now)
                .on_click(move |_, _, cx| {
                    let id = id.clone();
                    store.update(cx, |store, cx| store.set_fact_profile(id, say, cx));
                }),
        );
    }
    menu
}

/// Forget's question for the fact called `title`.
fn forget_question(title: &str) -> String {
    format!(
        "Forget \u{201c}{title}\u{201d} everywhere? This removes it and what was said about it \
         from my memory."
    )
}

/// A person's part of "Fix a fact": the assistant's note on what is going on with them, and
/// whether their messages wake it at once.
fn person_part(store: &Entity<Store>, fact: &Fact, p: &Pal) -> Div {
    let (store, id) = (store.clone(), fact.id.clone());
    div()
        .flex()
        .flex_col()
        .gap(px(6.))
        .child(
            div()
                .text_size(px(12.))
                .text_color(p.muted)
                .child("What's going on now"),
        )
        .when_some(fact.now.clone(), |d, now| {
            d.child(selectable(
                ElementId::Name(format!("fact-{}-now", fact.id).into()),
                &now,
            ))
        })
        .child(
            Switch::new("fact-wakes")
                .checked(fact.is_important())
                .label("Wake your ember when they write")
                .on_click(move |on, _, cx| {
                    let (id, on) = (id.clone(), *on);
                    store.update(cx, |store, cx| store.mark_important(id, on, cx));
                }),
        )
        .child(note(
            p,
            "Your ember keeps this note from your chats. Important people's messages reach it at \
             once instead of every 15 minutes.",
        ))
}

/// A confirmation in place: its question, the action (`id`, its label) and Cancel.
fn confirm(
    p: &Pal,
    question: String,
    (id, label): (ElementId, &'static str),
    yes: impl Fn(&gpui_kit::ClickEvent, &mut Window, &mut gpui_kit::App) + 'static,
    no: impl Fn(&gpui_kit::ClickEvent, &mut Window, &mut gpui_kit::App) + 'static,
) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .px(px(10.))
        .py(px(8.))
        .rounded(px(9.))
        .bg(p.danger.opacity(0.08))
        .child(div().flex_1().min_w_0().text_size(px(12.5)).child(question))
        .child(Button::new(id).small().danger().label(label).on_click(yes))
        .child(
            Button::new("confirm-cancel")
                .small()
                .ghost()
                .label("Cancel")
                .on_click(no),
        )
}

/// A category's icon.
fn category_icon(shelf: &Shelf) -> IconName {
    match shelf {
        Shelf::Category(FactCategory::People) => IconName::Users,
        Shelf::Category(FactCategory::Work) => IconName::Briefcase,
        Shelf::Category(FactCategory::Money) => IconName::Banknote,
        Shelf::Category(FactCategory::Preferences) => IconName::SlidersHorizontal,
        Shelf::Category(FactCategory::Health) => IconName::Heart,
        Shelf::Category(FactCategory::Home) => IconName::House,
        _ => IconName::Folder,
    }
}

impl Render for AboutPane {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let p = pal(cx);
        let query = self.search.read(cx).value().to_string();
        let store = self.store.read(cx);
        let feed = store.facts();
        let learning_open = feed.learning_open();
        // Why the page couldn't be read, and why the last fix didn't take (the app's line for a
        // refused action: "Not forgotten: <reason>").
        let problem: Vec<String> = [feed.problem(), store.action_problem()]
            .into_iter()
            .flatten()
            .map(str::to_owned)
            .collect();
        let Some(page) = feed.page().cloned() else {
            let waiting = if feed.is_loading() {
                div()
                    .flex()
                    .justify_center()
                    .py(px(40.))
                    .child(Spinner::new().color(p.muted))
            } else {
                div()
            };
            return div().size_full().flex().flex_col().child(page_body(
                "about-you-scroll",
                div()
                    .flex()
                    .flex_col()
                    .gap(px(12.))
                    .child(waiting)
                    .children(problem.into_iter().map(|problem| problem_line(&p, problem))),
            ));
        };
        let (nightly, export) = (feed.nightly(), feed.export().cloned());
        let editor = self.edited(cx).map(|fact| self.editor(&fact, &p, cx));
        let lessons = trials(&page, &query);
        let waiting = pending(&page, &query);
        let mut parts: Vec<AnyElement> =
            vec![div().child(Input::new(&self.search)).into_any_element()];
        if listed_count(&page) == 0 && page.trials.is_empty() {
            parts.push(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(6.))
                    .py(px(30.))
                    .child(div().font_weight(FontWeight::SEMIBOLD).child("Nothing yet"))
                    .child(note(
                        &p,
                        "As your ember learns about you, short facts show up here, and you can \
                         fix or forget any of them.",
                    ))
                    .into_any_element(),
            );
        }
        if !waiting.is_empty() {
            parts.push(self.to_confirm(&waiting, &p, cx).into_any_element());
        }
        if !lessons.is_empty() {
            parts.push(
                self.learning(&lessons, learning_open, &p, cx)
                    .into_any_element(),
            );
        }
        let found_elsewhere = !lessons.is_empty() || !waiting.is_empty();
        parts.extend(self.facts_parts(&page, &query, found_elsewhere, &p, cx));
        parts.push(
            self.memory_part(nightly, export.as_ref(), &p)
                .into_any_element(),
        );
        let content = div()
            .flex()
            .flex_col()
            .gap(px(18.))
            .children(parts)
            .children(problem.into_iter().map(|problem| problem_line(&p, problem)));
        div()
            .size_full()
            .relative()
            .flex()
            .flex_col()
            .child(page_body("about-you-scroll", content))
            .children(editor)
    }
}
