//! Your ember's Name: a field to rename it, Save (or Return) to keep the name, and a way back
//! to "Your ember". The new name shows at once wherever the ember is named; a name the server
//! refuses goes back, the field too, with the reason under it. A name changed on another
//! device fills the field, unless the user is typing in it.

use gpui_kit::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{Disableable as _, Sizable as _};
use gpui_kit::prelude::*;
use gpui_kit::{Entity, Window, div, px};

use super::parts::{note, problem_line, section};
use crate::store::Store;
use crate::ui::palette::Pal;
use emberhut_api::ember_name::{DEFAULT_NAME, MAX_CHARS, clean_name};

/// The field, for tests to find.
pub(crate) const NAME_FIELD: &str = "ember-name-field";
/// The Save button, for tests to find.
pub(crate) const NAME_SAVE: &str = "ember-name-save";
/// The button back to the default name, for tests to find.
pub(crate) const NAME_RESET: &str = "ember-name-reset";

/// The Name section of Your ember's page.
pub(crate) struct NameField {
    store: Entity<Store>,
    pub(super) input: Entity<InputState>,
    /// The name last put in the field (empty: the default), so a change from elsewhere fills
    /// it only while the user hasn't typed over it.
    shown: String,
    /// The field's text at its last change, so only a change of the text clears a refusal's
    /// reason.
    last: String,
}

impl NameField {
    /// The section over `store`, its field holding the ember's name.
    pub(crate) fn new(
        store: Entity<Store>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Self {
        let shown = store
            .read(cx)
            .ember_custom_name()
            .unwrap_or_default()
            .to_owned();
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(DEFAULT_NAME)
                .default_value(shown.clone())
        });
        cx.subscribe_in(
            &input,
            window,
            |this, _, event: &InputEvent, _, cx| match event {
                InputEvent::PressEnter { .. } => this.save(cx),
                InputEvent::Change => this.typed(cx),
                _ => {}
            },
        )
        .detach();
        cx.observe_in(&store, window, |this, store, window, cx| {
            this.follow(&store, window, cx);
        })
        .detach();
        Self {
            store,
            input,
            last: shown.clone(),
            shown,
        }
    }

    /// The name changed in the store (a rename here or on another device, or one refused):
    /// the field follows, unless the user typed over what it showed.
    fn follow(&mut self, store: &Entity<Store>, window: &mut Window, cx: &mut Context<'_, Self>) {
        let name = store
            .read(cx)
            .ember_custom_name()
            .unwrap_or_default()
            .to_owned();
        let typed = self.input.read(cx).value().to_string();
        if name != self.shown && typed == self.shown {
            self.input
                .update(cx, |input, cx| input.set_value(name.clone(), window, cx));
        }
        self.shown = name;
        cx.notify();
    }

    /// The field changed: once the user types over what it showed, the last refusal's reason
    /// goes.
    fn typed(&mut self, cx: &mut Context<'_, Self>) {
        let typed = self.input.read(cx).value().to_string();
        if typed != self.last
            && typed != self.shown
            && self.store.read(cx).ember_name_problem().is_some()
        {
            self.store.update(cx, Store::dismiss_ember_name_problem);
        }
        self.last = typed;
        cx.notify();
    }

    /// Keeps the name typed (blank: back to the default).
    fn save(&mut self, cx: &mut Context<'_, Self>) {
        let typed = self.input.read(cx).value().to_string();
        if let Ok(name) = clean_name(&typed) {
            // What the field shows now is the name asked for: a refusal brings the old one back.
            self.shown = name.unwrap_or_default();
        }
        self.store
            .update(cx, |store, cx| store.rename_ember(&typed, cx));
    }

    /// Back to "Your ember".
    fn reset(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.input
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.save(cx);
    }
}

impl Render for NameField {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let p: Pal = crate::ui::palette::pal(cx);
        let store = self.store.read(cx);
        let custom = store.ember_custom_name().map(str::to_owned);
        let problem = store.ember_name_problem().map(str::to_owned);
        let typed = self.input.read(cx).value().to_string();
        let changed = clean_name(&typed).map_or(true, |name| name != custom);
        let row = div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .p(px(12.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .child(
                        div()
                            .id(NAME_FIELD)
                            .flex_1()
                            .max_w(px(260.))
                            .child(Input::new(&self.input))
                            .test_support(),
                    )
                    .child(
                        Button::new(NAME_SAVE)
                            .primary()
                            .label("Save")
                            .disabled(!changed)
                            .on_click(cx.listener(|this, _, _, cx| this.save(cx))),
                    ),
            )
            .child(note(
                &p,
                format!("What you call your ember, on every device. Up to {MAX_CHARS} characters."),
            ))
            .when_some(custom, |d, _| {
                d.child(
                    div().child(
                        Button::new(NAME_RESET)
                            .ghost()
                            .small()
                            .label(format!("Go back to \u{201c}{DEFAULT_NAME}\u{201d}"))
                            .on_click(cx.listener(|this, _, window, cx| this.reset(window, cx))),
                    ),
                )
            })
            .children(problem.map(|problem| problem_line(&p, problem)));
        section(&p, "Name", None, vec![row.into_any_element()], None)
    }
}
