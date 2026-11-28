//! "Make it yours": the ember's name (a field with suggestions) and its look, with a small
//! preview of how it shows in the sidebar and the chat. The look is Your ember's own picker
//! (`LookPicker`, its setup variant), the same choice as Settings; the whole window takes a
//! look as it is picked.

use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::Input;
use gpui_kit::prelude::*;
use gpui_kit::{
    Context, Div, FontWeight, Role, SharedString, TestSupportExt as _, Window, div, img, px,
};

use super::SetupView;
use crate::assets::FACE;
use crate::store::Store;
use crate::store::setup::SUGGESTIONS;
use crate::ui::palette::Pal;
use crate::ui::settings::parts::note;

impl SetupView {
    /// Step 3: the name, the look, the preview, and Continue.
    pub(super) fn yours(
        &self,
        p: &Pal,
        _window: &Window,
        cx: &Context<'_, Self>,
    ) -> impl IntoElement {
        let store = self.store.read(cx);
        let name = store.setup_name();
        let problem = store.setup_name_problem();
        let view = cx.entity().downgrade();
        let chips = SUGGESTIONS.iter().map(|suggestion| {
            let (view, store) = (view.clone(), self.store.clone());
            let on = *suggestion == name;
            Button::new(SharedString::from(format!("setup-name-{suggestion}")))
                .small()
                .label(*suggestion)
                .when(on, ButtonVariants::primary)
                .when(!on, ButtonVariants::ghost)
                .on_click(move |_, window, cx| {
                    view.update(cx, |view, cx| {
                        view.name
                            .update(cx, |field, cx| field.set_value(*suggestion, window, cx));
                    })
                    .ok();
                    store.update(cx, |store, cx| store.name_ember(suggestion, cx));
                })
        });
        let done = self.store.clone();
        div()
            .id("setup-yours")
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(section_title("Its name"))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(px(8.))
                    .child(div().w(px(240.)).child(Input::new(&self.name)))
                    .children(chips),
            )
            .when_some(problem, |d, why| {
                d.child(
                    div()
                        .id("setup-name-problem")
                        .role(Role::Status)
                        .text_size(px(12.5))
                        .text_color(p.warn)
                        .child(why),
                )
            })
            .child(preview(&name, p))
            .child(section_title("Its look"))
            .child(self.look.clone())
            .child(note(p, "Change either any time in Settings."))
            .child(
                div().flex().pt(px(4.)).child(
                    Button::new("setup-yours-continue")
                        .primary()
                        .label("Continue")
                        .on_click(move |_, _, cx| done.update(cx, Store::yours_done)),
                ),
            )
            .test_support()
    }
}

/// A small heading inside the step.
fn section_title(words: &'static str) -> Div {
    div()
        .text_size(px(13.))
        .font_weight(FontWeight::SEMIBOLD)
        .child(words)
}

/// How the ember shows with `name`: its row at the top of the sidebar, and its first words in
/// the chat, in the look picked.
fn preview(name: &str, p: &Pal) -> impl IntoElement {
    let hello: SharedString = format!("Hi, I'm {name}. Tell me what to look after.").into();
    div()
        .id("setup-yours-preview")
        .role(Role::Note)
        .aria_label(SharedString::from(format!("Preview: {name}")))
        .flex()
        .items_center()
        .gap(px(16.))
        .p(px(12.))
        .rounded(px(12.))
        .border_1()
        .border_color(p.border)
        .bg(p.background)
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .px(px(10.))
                .py(px(7.))
                .rounded(px(8.))
                .bg(p.sidebar)
                .min_w(px(150.))
                .child(img(FACE).size(px(20.)).rounded_full())
                .child(
                    div()
                        .text_size(px(13.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(SharedString::from(name.to_owned())),
                ),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .px(px(12.))
                .py(px(8.))
                .rounded(px(12.))
                .bg(p.surface_raised)
                .text_size(px(13.))
                .child(hello),
        )
        .test_support()
}
