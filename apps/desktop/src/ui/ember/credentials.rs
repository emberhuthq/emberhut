//! What your ember thinks with: one list of the model providers, the picked one's fields open
//! under it. "Test it" checks them and lists the provider's models to choose from; "Use this"
//! saves them, then the view's owner hears it is done (Settings goes back, the first-run setup
//! goes on). The saved provider's key can be forgotten, once confirmed. "Another service" asks
//! which API the service speaks; Ollama's row says when a local one answered as the picker
//! opened, its models ready to choose from.
//!
//! Its data is the store's ([`crate::store::credentials`]); what is saved is read when a page
//! showing it opens ([`Store::open_credentials`]).

use std::rc::Rc;

use gpui_kit::TestSupportExt as _;
use gpui_kit::assets::IconName;
use gpui_kit::component::Icon;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{Disableable as _, Sizable as _};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, Div, ElementId, Entity, FontWeight, Role, Window, div, img, px};

use crate::assets::CLAUDE_LOGO;
use crate::store::Store;
use crate::store::credentials::{
    Check, Credentials, FAILED, FOOTER, ROWS, Row, Takes, coming_next_for, fields, not_in_use,
    passed_line, speaks,
};
use crate::ui::a11y;
use crate::ui::palette::{Pal, pal};
use crate::ui::settings::parts::{note, page_body, problem_line, tag};
use emberhut_api::agent_provider::{ProviderKind, ProviderModel};

/// What the view's owner does once a provider is saved.
type OnDone = Rc<dyn Fn(&mut Window, &mut App)>;

/// The picker.
pub(crate) struct CredentialsView {
    store: Entity<Store>,
    /// The selected row's API key, masked.
    pub(crate) key: Entity<InputState>,
    /// The selected row's base URL.
    pub(crate) base_url: Entity<InputState>,
    on_done: OnDone,
    /// The saves the view has answered, to tell when a new one went through.
    saves_seen: u64,
    /// The row the fields were last emptied for.
    row_for: Option<ProviderKind>,
    /// The provider the fields' placeholders were last set for.
    fields_for: Option<ProviderKind>,
    /// Shown inside the setup's step: no line of its own over the list (the step has it) and
    /// no scroll or margins of its own (the setup's page has them).
    in_setup: bool,
}

impl CredentialsView {
    /// The picker over `store`; `on_done` runs after each save that goes through.
    pub(crate) fn new(
        store: Entity<Store>,
        on_done: impl Fn(&mut Window, &mut App) + 'static,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Self {
        let key = cx.new(|cx| InputState::new(window, cx).masked(true));
        let base_url = cx.new(|cx| InputState::new(window, cx));
        cx.subscribe_in(&key, window, |this, input, event: &InputEvent, _, cx| {
            this.typed(input, event, Store::type_provider_key, cx);
        })
        .detach();
        cx.subscribe_in(
            &base_url,
            window,
            |this, input, event: &InputEvent, _, cx| {
                this.typed(input, event, Store::type_provider_url, cx);
            },
        )
        .detach();
        cx.observe_in(&store, window, |this, _, window, cx| {
            this.store_changed(window, cx);
        })
        .detach();
        let saves_seen = store.read(cx).credentials().saves();
        let mut view = Self {
            store,
            key,
            base_url,
            on_done: Rc::new(on_done),
            saves_seen,
            row_for: None,
            fields_for: None,
            in_setup: false,
        };
        view.store_changed(window, cx);
        view
    }

    /// The picker as the setup's "Light it" step shows it.
    #[must_use]
    pub(crate) fn in_setup(mut self) -> Self {
        self.in_setup = true;
        self
    }

    /// A field's text changed, or Return was pressed in it: the store hears the text, and
    /// Return is "Use this" once the check passed, else "Test it".
    fn typed(
        &self,
        input: &Entity<InputState>,
        event: &InputEvent,
        tell: fn(&mut Store, &str, &mut Context<'_, Store>),
        cx: &mut Context<'_, Self>,
    ) {
        match event {
            InputEvent::Change => {
                let text = input.read(cx).value();
                self.store.update(cx, |store, cx| tell(store, &text, cx));
            }
            InputEvent::PressEnter { .. } => self.store.update(cx, |store, cx| {
                if store.credentials().can_save() {
                    store.save_provider(cx);
                } else {
                    store.test_provider(cx);
                }
            }),
            _ => {}
        }
    }

    /// The store changed: a save that went through empties the fields and tells the owner,
    /// another row picked gets blank fields of its own, and the provider chosen inside a row
    /// gets its placeholders (what was typed stays).
    fn store_changed(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let picker = self.store.read(cx).credentials();
        let saves = picker.saves();
        let kind = picker.picked().cloned();
        let row_kind = picker.picked_row().map(|row| row.kind.clone());
        let key_hint = kind
            .as_ref()
            .and_then(|kind| picker.saved_key_hint(kind))
            .map(|hint| format!("Saved key ending {hint}: paste a key to test it"));
        let went_through = saves > self.saves_seen;
        if went_through || row_kind != self.row_for {
            for input in [&self.key, &self.base_url] {
                input.update(cx, |input, cx| input.set_value("", window, cx));
            }
            self.row_for = row_kind;
        }
        if went_through || kind != self.fields_for {
            let takes = kind.as_ref().map(fields);
            let key_words = key_hint.unwrap_or_else(|| match takes.map(|takes| takes.key) {
                Some(Takes::Optional) => "API key, if it needs one".to_owned(),
                _ => "API key".to_owned(),
            });
            let url_words = takes.map_or("", |takes| takes.url_hint);
            self.key.update(cx, |input, cx| {
                input.set_placeholder(key_words, window, cx);
            });
            self.base_url.update(cx, |input, cx| {
                input.set_placeholder(url_words, window, cx);
            });
            self.fields_for = kind;
        }
        if went_through {
            self.saves_seen = saves;
            (self.on_done)(window, cx);
        }
        cx.notify();
    }

    /// A row: its radio mark, its logo, its name with a tag and its line; the fields under it
    /// once it is picked. A row the agent can't run on yet is drawn faint and isn't picked.
    fn row(&self, row: &Row, picker: &Credentials, p: &Pal, cx: &Context<'_, Self>) -> Div {
        let on = picker.picked_row() == Some(row);
        let open = picker.can_pick(row);
        let tag_words = picker.row_tag(row);
        let label = a11y::line(&[row.name, tag_words.as_deref().unwrap_or_default(), row.line]);
        let head = div()
            .id(ElementId::Name(
                format!("provider-{}", row.kind.as_str()).into(),
            ))
            .role(Role::RadioButton)
            .aria_label(label)
            .aria_selected(on)
            .flex()
            .items_start()
            .gap(px(10.))
            .py(px(4.))
            .child(
                Icon::new(if on {
                    IconName::CircleDot
                } else {
                    IconName::Circle
                })
                .size(px(14.))
                .mt(px(3.))
                .text_color(if on { p.accent } else { p.faint }),
            )
            .child(logo(&row.kind, p))
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
                            .flex_wrap()
                            .items_center()
                            .gap(px(6.))
                            .child(
                                div()
                                    .text_size(px(13.5))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(if open { p.text } else { p.muted })
                                    .child(row.name),
                            )
                            .children(
                                tag_words
                                    .map(|words| tag(words, if open { p.accent } else { p.muted })),
                            ),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(if open { p.muted } else { p.faint })
                            .child(row.line),
                    ),
            )
            .when(open, |head| {
                let (store, kind) = (self.store.clone(), row.kind.clone());
                head.cursor_pointer().on_click(move |_, _, cx| {
                    store.update(cx, |store, cx| store.pick_provider(&kind, cx));
                })
            })
            .test_support();
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(head)
            .when(on, |d| d.child(self.fields(row, picker, p, cx)))
    }

    /// The selected row's fields, its check and what it found, and its buttons.
    fn fields(&self, row: &Row, picker: &Credentials, p: &Pal, cx: &Context<'_, Self>) -> Div {
        let kind = picker.picked().unwrap_or(&row.kind);
        let takes = fields(kind);
        let testing = picker.check() == &Check::Testing;
        let passed = matches!(picker.check(), Check::Passed { .. });
        let test = Button::new("provider-test")
            .label("Test it")
            .loading(testing)
            .disabled(!picker.can_test())
            .on_click(cx.listener(|this, _, _, cx| {
                this.store.update(cx, Store::test_provider);
            }));
        let test = if passed {
            test.outline()
        } else {
            test.primary()
        };
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            // Under the row's name, past its radio mark and logo.
            .pl(px(54.))
            .pb(px(6.))
            .children(row.hint.map(|hint| note(p, hint)))
            .children(
                row.also
                    .is_some()
                    .then(|| self.speaks_choice(row, kind, picker, p)),
            )
            .when(takes.base_url != Takes::No, |d| {
                d.child(field(p, "Address", Input::new(&self.base_url)))
            })
            .when(takes.key != Takes::No, |d| {
                d.child(field(p, "API key", Input::new(&self.key).mask_toggle()))
            })
            .child(self.outcome(picker, p))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(px(8.))
                    .child(test)
                    .when(passed, |d| {
                        d.child(
                            Button::new("provider-use")
                                .primary()
                                .label("Use this")
                                .loading(picker.is_saving())
                                .disabled(!picker.can_save())
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.store.update(cx, Store::save_provider);
                                })),
                        )
                    }),
            )
            .children(
                picker
                    .save_problem()
                    .map(|why| problem_line(p, why.to_owned())),
            )
            .children(Self::forget(kind, picker, p, cx))
    }

    /// A two-way row's choice of the API the service speaks, with Coming next under it while
    /// the agent can't run on the one chosen.
    fn speaks_choice(&self, row: &Row, kind: &ProviderKind, picker: &Credentials, p: &Pal) -> Div {
        let choices = row.kinds().map(|choice| {
            let chosen = choice == kind;
            let (store, choice) = (self.store.clone(), choice.clone());
            div()
                .id(ElementId::Name(
                    format!("provider-speaks-{}", choice.as_str()).into(),
                ))
                .role(Role::RadioButton)
                .aria_label(speaks(&choice))
                .aria_selected(chosen)
                .px(px(10.))
                .py(px(4.))
                .rounded(px(7.))
                .text_size(px(12.))
                .cursor_pointer()
                .when(chosen, |d| d.bg(p.surface).text_color(p.text))
                .when(!chosen, |d| d.text_color(p.muted))
                .child(speaks(&choice))
                .on_click(move |_, _, cx| {
                    store.update(cx, |store, cx| store.speak_provider(&choice, cx));
                })
                .test_support()
        });
        div()
            .flex()
            .flex_col()
            .gap(px(4.))
            .child(note(p, "It speaks"))
            .child(
                div()
                    .id("provider-speaks")
                    .role(Role::RadioGroup)
                    .aria_label("It speaks")
                    .flex()
                    .flex_wrap()
                    .gap(px(2.))
                    .p(px(2.))
                    .rounded(px(9.))
                    .bg(p.surface_raised)
                    .max_w(px(420.))
                    .children(choices),
            )
            .when(!picker.runs(kind), |d| {
                d.child(note(p, coming_next_for(kind)))
            })
    }

    /// What the check found: that it works, the ember's answer when one ran, and the models
    /// to choose from; or that it didn't, in the server's sentence.
    fn outcome(&self, picker: &Credentials, p: &Pal) -> AnyElement {
        match picker.check() {
            Check::Idle | Check::Testing => div().into_any_element(),
            Check::Failed(why) => div()
                .id("provider-failed")
                .role(Role::Status)
                .aria_label(format!("{FAILED}. {why}"))
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(
                    div()
                        .text_size(px(12.5))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(p.danger)
                        .child(FAILED),
                )
                .child(problem_line(p, why.clone()))
                .test_support()
                .into_any_element(),
            Check::Passed { models, answer } => div()
                .flex()
                .flex_col()
                .gap(px(8.))
                .children(answer.as_ref().map(|answer| bubble(p, answer.clone())))
                .child(
                    div()
                        .id("provider-works")
                        .role(Role::Status)
                        .aria_label(passed_line(models.len()))
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .text_size(px(12.5))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(p.success)
                        .child(Icon::new(IconName::CircleCheck).size(px(14.)))
                        .child(passed_line(models.len()))
                        .test_support(),
                )
                .when(!models.is_empty(), |d| {
                    d.child(self.models(models, picker.model(), p))
                })
                .into_any_element(),
        }
    }

    /// The models the check listed, the chosen one marked; a long list scrolls.
    fn models(&self, models: &[ProviderModel], chosen: Option<&str>, p: &Pal) -> AnyElement {
        let chips = models.iter().enumerate().map(|(i, model)| {
            let on = chosen == Some(model.id.as_str());
            let shown = model.name.clone().unwrap_or_else(|| model.id.clone());
            let (store, id) = (self.store.clone(), model.id.clone());
            div()
                .id(("provider-model", i))
                .role(Role::RadioButton)
                .aria_label(shown.clone())
                .aria_selected(on)
                .px(px(9.))
                .py(px(4.))
                .rounded(px(7.))
                .border_1()
                .border_color(if on { p.accent } else { p.border })
                .when(on, |d| d.bg(p.accent_soft))
                .text_size(px(12.))
                .cursor_pointer()
                .child(shown)
                .on_click(move |_, _, cx| {
                    store.update(cx, |store, cx| store.choose_provider_model(&id, cx));
                })
                .test_support()
        });
        div()
            .flex()
            .flex_col()
            .gap(px(4.))
            .child(note(p, "Model"))
            .child(
                div()
                    .id("provider-models")
                    .max_h(px(168.))
                    .overflow_y_scroll()
                    .flex()
                    .flex_wrap()
                    .gap(px(6.))
                    .children(chips),
            )
            .into_any_element()
    }

    /// "Forget this key" on the saved provider's row, and its confirmation once asked.
    fn forget(
        kind: &ProviderKind,
        picker: &Credentials,
        p: &Pal,
        cx: &Context<'_, Self>,
    ) -> Option<Div> {
        let hint = picker.saved_key_hint(kind)?;
        if !picker.forget_asked() {
            return Some(
                div().child(
                    Button::new("provider-forget")
                        .ghost()
                        .small()
                        .label("Forget this key")
                        .loading(picker.is_forgetting())
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.store
                                .update(cx, |store, cx| store.ask_forget_provider(true, cx));
                        })),
                ),
            );
        }
        Some(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap(px(8.))
                .child(
                    div()
                        .min_w_0()
                        .text_size(px(12.))
                        .text_color(p.text)
                        .child(format!(
                            "Forget the key ending {hint}? Your ember goes back to its own \
                             sign-in."
                        )),
                )
                .child(
                    Button::new("provider-forget-confirm")
                        .danger()
                        .small()
                        .label("Forget")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.store.update(cx, Store::forget_provider);
                        })),
                )
                .child(
                    Button::new("provider-forget-cancel")
                        .ghost()
                        .small()
                        .label("Keep it")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.store
                                .update(cx, |store, cx| store.ask_forget_provider(false, cx));
                        })),
                ),
        )
    }
}

impl Render for CredentialsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let p = pal(cx);
        let picker = self.store.read(cx).credentials();
        let read_problem = picker.read_problem().map(str::to_owned);
        let limit = picker.saved().and_then(not_in_use);
        let loading = picker.is_loading() && picker.saved().is_none();
        let rows: Vec<AnyElement> = ROWS
            .iter()
            .enumerate()
            .map(|(i, row)| {
                div()
                    .px(px(14.))
                    .py(px(8.))
                    .when(i > 0, |d| d.border_t_1().border_color(p.border))
                    .child(self.row(row, picker, &p, cx))
                    .into_any_element()
            })
            .collect();
        let content = div()
            .flex()
            .flex_col()
            .gap(px(14.))
            .when(!self.in_setup, |d| {
                d.child(
                    div()
                        .text_size(px(13.))
                        .text_color(p.muted)
                        .child("Choose the model your ember thinks with."),
                )
            })
            .children(read_problem.map(|why| problem_line(&p, why)))
            .when(loading, |d| {
                d.child(note(&p, "Reading what it thinks with now…"))
            })
            .children(limit.map(|limit| {
                div()
                    .id("provider-not-in-use")
                    .role(Role::Status)
                    .aria_label(limit)
                    .text_size(px(12.))
                    .text_color(p.warn)
                    .child(limit)
                    .test_support()
            }))
            .child(
                div()
                    .id("provider-list")
                    .role(Role::RadioGroup)
                    .aria_label("Providers")
                    .flex()
                    .flex_col()
                    .rounded(px(12.))
                    .border_1()
                    .border_color(p.border)
                    .bg(p.card)
                    .children(rows),
            )
            .child(note(&p, FOOTER).px(px(4.)));
        if self.in_setup {
            return content.into_any_element();
        }
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(page_body("credentials-scroll", content))
            .into_any_element()
    }
}

/// A provider's logo: Claude's own mark, a plain icon on a tinted square for the others.
fn logo(kind: &ProviderKind, p: &Pal) -> AnyElement {
    let glyph = match kind {
        ProviderKind::Anthropic => return img(CLAUDE_LOGO).size(px(20.)).into_any_element(),
        ProviderKind::OpenAi => IconName::Bot,
        ProviderKind::DeepSeek => IconName::Brain,
        ProviderKind::OpenRouter => IconName::Network,
        ProviderKind::Ollama => IconName::Cpu,
        _ => IconName::Globe,
    };
    div()
        .size(px(20.))
        .flex_none()
        .rounded(px(5.))
        .bg(p.surface_raised)
        .flex()
        .items_center()
        .justify_center()
        .child(Icon::new(glyph).size(px(12.)).text_color(p.muted))
        .into_any_element()
}

/// A field with its label over it.
fn field(p: &Pal, label: &'static str, input: Input) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(4.))
        .max_w(px(420.))
        .child(note(p, label))
        .child(input)
}

/// What the ember said, as a chat bubble.
fn bubble(p: &Pal, text: String) -> Div {
    div()
        .max_w(px(420.))
        .px(px(12.))
        .py(px(8.))
        .rounded(px(12.))
        .bg(p.surface_raised)
        .border_1()
        .border_color(p.border)
        .text_size(px(13.))
        .text_color(p.text)
        .child(text)
}
