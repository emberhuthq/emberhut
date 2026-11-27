//! About you: what the assistant knows about the user, by category, with the rules it is trying
//! out on top; a search over both. A fact opens "Fix a fact" (correct it, mark whether a person
//! wakes the assistant, forget it); a rule on trial can be dropped.

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::{ContextMenuExt as _, PopupMenuItem};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{Disableable as _, Icon, Sizable as _};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, Div, ElementId, Entity, FocusHandle, FontWeight, Window, div, px};
use jiff::tz::TimeZone;

use super::parts::{note, page_body, problem_line, section, selectable, sheet};
use crate::brand::APP_NAME;
use crate::store::Store;
use crate::store::settings::about::{LISTED, category_title, grouped, trials};
use crate::store::settings::words::day_month;
use crate::ui::palette::{Pal, category_color, pal};
use emberhut_api::facts::{Fact, FactCategory, Trial};
use emberhut_api::ids::{FactId, TrialId};

/// A rule on trial's row, for tests to find.
pub(super) const TRIAL_ROW: &str = "about-trial";

/// The Drop button of a rule on trial, for tests to find.
pub(super) const TRIAL_DROP: &str = "about-trial-drop";

/// A fact's row, for tests to find.
pub(super) const FACT_ROW: &str = "about-fact";

/// The About you page.
pub(crate) struct AboutPane {
    store: Entity<Store>,
    search: Entity<InputState>,
    /// The fact open in "Fix a fact".
    editing: Option<FactId>,
    /// Its text as being corrected.
    pub(super) text: Entity<InputState>,
    /// "Forget this" asked, waiting for its confirmation.
    forgetting: bool,
    /// The rule whose Drop waits for its confirmation.
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
        let search = cx.new(|cx| {
            InputState::new(window, cx).placeholder(format!("Search what {APP_NAME} knows"))
        });
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
            editing: None,
            text,
            forgetting: false,
            dropping: None,
            focus: cx.focus_handle(),
        }
    }

    /// Opens "Fix a fact" on `fact`.
    pub(super) fn edit(&mut self, fact: &Fact, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.editing = Some(fact.id.clone());
        self.forgetting = false;
        let text = fact.text.clone();
        self.text.update(cx, |input, cx| {
            input.set_value(text, window, cx);
            input.focus(window, cx);
        });
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

    fn trials_section(&self, list: &[&Trial], p: &Pal, cx: &mut Context<'_, Self>) -> Div {
        let zone = TimeZone::system();
        let rows = list
            .iter()
            .map(|trial| {
                let id = trial.id;
                let confirming = self.dropping == Some(id);
                let dates = format!(
                    "From your correction on {} · on trial until {}",
                    day_month(trial.at.as_str(), &zone).unwrap_or_default(),
                    day_month(trial.ends.as_str(), &zone).unwrap_or_default()
                );
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .debug_selector(|| TRIAL_ROW.to_owned())
                    .child(
                        div()
                            .flex()
                            .items_start()
                            .gap(px(10.))
                            .child(
                                // Without `min_w_0` the column is as wide as its longest line
                                // unwrapped, and pushes the row and its Drop out of the card.
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap(px(3.))
                                    .flex_1()
                                    .min_w_0()
                                    .child(div().text_size(px(13.)).child(trial.text.clone()))
                                    .when(!trial.said.is_empty(), |d| {
                                        d.child(
                                            div()
                                                .text_size(px(12.))
                                                .text_color(p.muted)
                                                .line_clamp(2)
                                                .child(format!("You said: {}", trial.said)),
                                        )
                                    })
                                    .child(
                                        div().text_size(px(11.)).text_color(p.faint).child(dates),
                                    ),
                            )
                            .child(
                                div()
                                    .flex_none()
                                    .debug_selector(|| TRIAL_DROP.to_owned())
                                    .child(
                                        Button::new(ElementId::Name(
                                            format!("trial-{id}-drop").into(),
                                        ))
                                        .small()
                                        .danger()
                                        .label("Drop")
                                        .on_click(
                                            cx.listener(move |this, _, _, cx| {
                                                this.dropping = Some(id);
                                                cx.notify();
                                            }),
                                        ),
                                    ),
                            ),
                    )
                    .when(confirming, |d| {
                        d.child(confirm(
                            p,
                            format!(
                                "Drop this rule? {APP_NAME} stops following \"{}\".",
                                trial.text
                            ),
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
                    .into_any_element()
            })
            .collect();
        section(
            p,
            format!("{APP_NAME} is trying out"),
            Some(list.len()),
            rows,
            Some(format!(
                "Lessons from your corrections. {APP_NAME} follows each one for 14 days; \
                 Saturday's review keeps or drops it."
            )),
        )
    }

    fn fact_row(&self, fact: &Fact, p: &Pal, cx: &mut Context<'_, Self>) -> AnyElement {
        let zone = TimeZone::system();
        let origin = [
            fact.source.clone(),
            day_month(fact.updated.as_str(), &zone).unwrap_or_default(),
        ]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" · ");
        let (open, forget) = (fact.clone(), fact.id.clone());
        let store = self.store.clone();
        div()
            .id(ElementId::Name(format!("fact-{}", fact.id).into()))
            .debug_selector(|| FACT_ROW.to_owned())
            .flex()
            .flex_col()
            .gap(px(3.))
            .cursor_pointer()
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
                        d.child(Icon::new(IconName::Bell).size(px(11.)).text_color(p.warn))
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
            .when(!origin.is_empty(), |d| {
                d.child(div().text_size(px(11.)).text_color(p.faint).child(origin))
            })
            .on_click(cx.listener(move |this, _, window, cx| this.edit(&open, window, cx)))
            .context_menu(move |menu, _, _| {
                let (store, forget) = (store.clone(), forget.clone());
                menu.item(PopupMenuItem::new("Forget").on_click(move |_, _, cx| {
                    let forget = forget.clone();
                    store.update(cx, |store, cx| store.forget_fact(forget, cx));
                }))
            })
            .into_any_element()
    }

    /// "Fix a fact": the text, where it came from, a person's note and their switch, Forget.
    fn editor(&self, fact: &Fact, p: &Pal, cx: &mut Context<'_, Self>) -> Div {
        let from = if fact.source.is_empty() {
            APP_NAME.to_owned()
        } else {
            fact.source.clone()
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
                        format!("From {from}. {APP_NAME} will use your version from now on."),
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
                    format!("Forget \"{title}\"? {APP_NAME} stops using it."),
                    ("fact-forget-confirm".into(), "Forget"),
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
                        .label("Forget this")
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
                    log::debug!("About you closed with its sheet: {error}");
                }
            }),
        )
    }
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
                .label(format!("Wake {APP_NAME} when they write"))
                .on_click(move |on, _, cx| {
                    let (id, on) = (id.clone(), *on);
                    store.update(cx, |store, cx| store.mark_important(id, on, cx));
                }),
        )
        .child(note(
            p,
            format!(
                "{APP_NAME} keeps this note from your chats. Important people's messages reach \
                 {APP_NAME} at once instead of every 15 minutes."
            ),
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
        .child(div().flex_1().text_size(px(12.5)).child(question))
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
fn category_icon(category: &FactCategory) -> IconName {
    match category {
        FactCategory::People => IconName::Users,
        FactCategory::Work => IconName::Briefcase,
        FactCategory::Money => IconName::Banknote,
        FactCategory::Preferences => IconName::SlidersHorizontal,
        FactCategory::Health => IconName::Heart,
        FactCategory::Home => IconName::House,
        _ => IconName::Folder,
    }
}

impl Render for AboutPane {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let p = pal(cx);
        let query = self.search.read(cx).value().to_string();
        let store = self.store.read(cx);
        let feed = store.facts();
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
        let editor = self.edited(cx).map(|fact| self.editor(&fact, &p, cx));
        let tried = trials(&page, &query);
        let mut parts: Vec<AnyElement> =
            vec![div().child(Input::new(&self.search)).into_any_element()];
        if page.facts.is_empty() {
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
                        format!(
                            "As {APP_NAME} learns about you, short facts show up here, and you \
                             can fix or forget any of them."
                        ),
                    ))
                    .into_any_element(),
            );
        }
        if !tried.is_empty() {
            parts.push(self.trials_section(&tried, &p, cx).into_any_element());
        }
        for (category, facts) in grouped(&page, &query) {
            let index = LISTED
                .iter()
                .position(|c| *c == category)
                .unwrap_or_default();
            let tint = category_color(index);
            let rows = facts
                .iter()
                .map(|fact| self.fact_row(fact, &p, cx))
                .collect();
            let head = div()
                .flex()
                .items_center()
                .gap(px(6.))
                .child(
                    Icon::new(category_icon(&category))
                        .size(px(13.))
                        .text_color(tint),
                )
                .child(category_title(&category));
            parts.push(
                section(&p, head.into_any_element(), Some(facts.len()), rows, None)
                    .into_any_element(),
            );
        }
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
