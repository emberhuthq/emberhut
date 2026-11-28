//! Your ember's look: the theme, picked by how it looks. Each preset the server lists is a small
//! card drawn in its own colours (its paper, ink and accent in light, and a swatch of it in
//! dark), the one in use ticked; Light, Dark or System sit beside them; and when the server
//! lists the ember's tints, its face in each. It is Appearance's choice, not one of its own:
//! one theme, kept on the server, so every device follows. A view of its own over the store, so
//! Settings and the first-run setup show it as it is; the page around it gives the title.

use gpui_kit::TestSupportExt as _;
use gpui_kit::assets::IconName;
use gpui_kit::component::Icon;
use gpui_kit::prelude::*;
use gpui_kit::{Div, ElementId, Entity, FontWeight, Window, div, img, px, relative};

use crate::assets::face;
use crate::store::Store;
use crate::ui::a11y::{self, Accessible as _};
use crate::ui::controls::{segment, tab_stop};
use crate::ui::palette::{Pal, pal};
use crate::ui::settings::parts::note;
use crate::ui::widgets::dot;
use emberhut_api::theme::{Appearance, DEFAULT_PRESET, DEFAULT_TINT, EmberTint, Preset, ThemePage};

/// A preset's card, before the preset's id (`look-preset-paper`), for tests to find.
pub(crate) const PRESET_CARD: &str = "look-preset";
/// A tint's button, before the tint's id (`look-tint-rose`), for tests to find.
pub(crate) const TINT_BUTTON: &str = "look-tint";

/// The System, Light and Dark segments: their ids, appearances and labels.
const MODES: [(&str, Appearance, &str); 3] = [
    ("look-mode-system", Appearance::System, "System"),
    ("look-mode-light", Appearance::Light, "Light"),
    ("look-mode-dark", Appearance::Dark, "Dark"),
];

/// What the picker says while a custom colour is in use, which no card shows.
const CUSTOM: &str = "A custom colour is in use (Appearance). Picking a look uses its own colour.";

/// Where the picker shows, which sets how much it says around the cards.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LookVariant {
    /// On Your ember's settings page: its parts headed, the cards roomy, and a line when a
    /// custom colour from Appearance is in use.
    Settings,
    /// In the first-run setup's step: no headings or notes (the step has its own words), the
    /// cards smaller in a tight grid.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "the first-run setup's look step shows it next")
    )]
    Setup,
}

impl LookVariant {
    /// A card's width and the height of the page drawn on it.
    fn card_size(self) -> (f32, f32) {
        match self {
            Self::Settings => (118., 60.),
            Self::Setup => (96., 48.),
        }
    }

    /// The gap between cards, and between the picker's parts.
    fn gaps(self) -> (f32, f32) {
        match self {
            Self::Settings => (10., 12.),
            Self::Setup => (6., 10.),
        }
    }
}

/// The picker.
pub(crate) struct LookPicker {
    store: Entity<Store>,
    variant: LookVariant,
}

impl LookPicker {
    /// The picker over `store`, drawn for `variant`: it draws the theme the store holds and
    /// picks through it.
    pub(crate) fn new(
        store: Entity<Store>,
        variant: LookVariant,
        cx: &mut Context<'_, Self>,
    ) -> Self {
        cx.observe(&store, |_, _, cx| cx.notify()).detach();
        Self { store, variant }
    }

    /// System, Light or Dark, the chosen one raised.
    fn modes(&self, chosen: &Appearance, p: &Pal) -> Div {
        let mut modes = div()
            .flex()
            .flex_none()
            .gap(px(2.))
            .p(px(2.))
            .rounded(px(9.))
            .bg(p.surface_raised);
        for (id, appearance, label) in MODES {
            let store = self.store.clone();
            let on = *chosen == appearance;
            let segment = segment(id, label, on, p).on_click(move |_, _, cx| {
                store.update(cx, |store, cx| {
                    store.pick_theme(None, Some(appearance.clone()), cx);
                });
            });
            // Seen by the headless tests (a no-op outside them).
            modes = modes.child(segment.test_support());
        }
        modes
    }

    /// A preset's card: a page in its light colours (paper, a card of ink lines and an accent
    /// button) with its dark ground and accent in the corner, then its name, ticked when `on`.
    fn card(&self, preset: &Preset, on: bool, p: &Pal) -> impl IntoElement + use<> {
        let (light, dark) = (&preset.light, &preset.dark);
        let (width, height) = self.variant.card_size();
        let page = div()
            .relative()
            .h(px(height))
            .w_full()
            .p(px(7.))
            .rounded(px(7.))
            .overflow_hidden()
            .bg(Pal::token(&light.background))
            .border_1()
            .border_color(Pal::token(&light.border))
            .child(
                div()
                    .w(relative(0.7))
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .p(px(6.))
                    .rounded(px(5.))
                    .bg(Pal::token(&light.surface))
                    .child(line(&light.text, 0.85))
                    .child(line(&light.muted, 0.55))
                    .child(
                        div()
                            .mt(px(2.))
                            .h(px(9.))
                            .w(px(28.))
                            .rounded_full()
                            .bg(Pal::token(&light.accent)),
                    ),
            )
            .child(
                div()
                    .absolute()
                    .top(px(6.))
                    .right(px(6.))
                    .size(px(20.))
                    .rounded(px(5.))
                    .bg(Pal::token(&dark.background))
                    .border_1()
                    .border_color(Pal::token(&dark.border))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(dot(Pal::token(&dark.accent), 8.)),
            );
        let name = div()
            .flex()
            .items_center()
            .gap(px(4.))
            .text_size(px(12.5))
            .when(on, |d| {
                d.font_weight(FontWeight::SEMIBOLD).child(
                    Icon::new(IconName::Check)
                        .size(px(13.))
                        .text_color(p.accent),
                )
            })
            .child(preset.name.clone());
        let (store, id) = (self.store.clone(), preset.id.clone());
        let card = div()
            .id(ElementId::Name(
                format!("{PRESET_CARD}-{}", preset.id).into(),
            ))
            // The chosen one is read as selected, and Paper as the default.
            .button_named(a11y::preset(&preset.name, preset.id == DEFAULT_PRESET))
            .aria_selected(on)
            .w(px(width))
            .flex()
            .flex_col()
            .gap(px(6.))
            .p(px(6.))
            .rounded(px(10.))
            .border_1()
            .cursor_pointer()
            .hover(|d| d.bg(p.hover))
            .child(page)
            .child(name)
            .on_click(move |_, _, cx| {
                store.update(cx, |store, cx| store.pick_theme(Some(id.clone()), None, cx));
            });
        tab_stop(card, p)
            .border_color(if on { p.accent } else { p.border })
            .when(on, |d| d.bg(p.accent_soft))
            .test_support()
    }

    /// A tint: the ember's face in it and its name, ringed when `on`.
    fn tint(&self, tint: &EmberTint, on: bool, p: &Pal) -> impl IntoElement + use<> {
        let (store, id) = (self.store.clone(), tint.id.clone());
        let button = div()
            .id(ElementId::Name(format!("{TINT_BUTTON}-{}", tint.id).into()))
            .button_named(a11y::preset(
                &format!("{} ember", tint.name),
                tint.id == DEFAULT_TINT,
            ))
            .aria_selected(on)
            .w(px(64.))
            .flex()
            .flex_col()
            .items_center()
            .gap(px(4.))
            .py(px(6.))
            .rounded(px(10.))
            .border_1()
            .cursor_pointer()
            .hover(|d| d.bg(p.hover))
            .child(img(face(&tint.id)).size(px(34.)))
            .child(
                div()
                    .text_size(px(11.5))
                    .when(on, |d| d.font_weight(FontWeight::SEMIBOLD))
                    .when(!on, |d| d.text_color(p.muted))
                    .child(tint.name.clone()),
            )
            .on_click(move |_, _, cx| {
                store.update(cx, |store, cx| store.pick_ember_tint(id.clone(), cx));
            });
        tab_stop(button, p)
            .border_color(if on { p.accent } else { p.border })
            .when(on, |d| d.bg(p.accent_soft))
            .test_support()
    }
}

impl Render for LookPicker {
    fn render(&mut self, _: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let p = pal(cx);
        let page: ThemePage = self.store.read(cx).theme().page().clone();
        let choice = &page.choice;
        let custom = choice.accent.is_some();
        let cards = page
            .presets
            .iter()
            .map(|preset| self.card(preset, !custom && choice.preset == preset.id, &p));
        let tints: Vec<_> = page
            .tints
            .iter()
            .map(|tint| self.tint(tint, choice.ember_tint == tint.id, &p))
            .collect();
        let settings = self.variant == LookVariant::Settings;
        let (between_cards, between_parts) = self.variant.gaps();
        div()
            .id("ember-look")
            .flex()
            .flex_col()
            .gap(px(between_parts))
            .when(settings, |d| d.py(px(4.)))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .justify_between()
                    .gap(px(8.))
                    .when(settings, |d| d.child(label(&p, "Theme")))
                    .child(self.modes(&choice.appearance, &p)),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap(px(between_cards))
                    .children(cards),
            )
            .when(settings && custom, |d| d.child(note(&p, CUSTOM)))
            .when(!tints.is_empty(), |d| {
                d.when(settings, |d| d.child(label(&p, "Your ember's colour")))
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap(px(between_cards))
                            .children(tints),
                    )
            })
            .test_support()
    }
}

/// A line of ink on a card's page: `share` of its width, in `colour`.
fn line(colour: &str, share: f32) -> Div {
    div()
        .h(px(3.))
        .w(relative(share))
        .rounded_full()
        .bg(Pal::token(colour))
}

/// A part's small heading.
fn label(p: &Pal, text: &'static str) -> Div {
    div()
        .text_size(px(12.5))
        .font_weight(FontWeight::MEDIUM)
        .text_color(p.muted)
        .child(text)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{TestAppContext, VisualTestContext};

    use super::*;
    use crate::backend::fake::{Call, FakeBackend};
    use emberhut_api::theme::ThemeChange;

    /// The picker alone in a headless window, over a store holding `page`.
    fn open(
        page: ThemePage,
        cx: &mut TestAppContext,
    ) -> (Arc<FakeBackend>, Entity<Store>, VisualTestContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            cx.set_global(Pal::new(&crate::theme::built_in(), false));
        });
        let backend = Arc::new(FakeBackend::new([]));
        let shared = Arc::clone(&backend) as Arc<dyn crate::backend::Backend>;
        let store = cx.new(|_| Store::for_tests(shared));
        store.update(cx, |store, _| store.put_theme_for_tests(page));
        let window =
            cx.add_window(|_, cx| LookPicker::new(store.clone(), LookVariant::Settings, cx));
        let cx = VisualTestContext::from_window(window.into(), cx);
        (backend, store, cx)
    }

    /// The built-in theme as a server listing every preset would send it.
    fn every_preset() -> ThemePage {
        let mut page = crate::theme::built_in();
        page.presets = emberhut_core::theme::PRESETS
            .iter()
            .map(|preset| {
                let tokens = crate::theme::preset_tokens(preset.id);
                Preset {
                    id: preset.id.into(),
                    name: preset.name.into(),
                    light: tokens.light,
                    dark: tokens.dark,
                }
            })
            .collect();
        page
    }

    /// The ids of the cards and tint buttons drawn, and whether each is selected.
    fn drawn(page: &ThemePage, cx: &mut VisualTestContext) -> (Vec<(String, bool)>, Vec<bool>) {
        cx.update(|window, cx| {
            window.render_frame(cx);
            let cards = page
                .presets
                .iter()
                .filter_map(|preset| {
                    let id = format!("{PRESET_CARD}-{}", preset.id);
                    window
                        .try_find(ElementId::Name(id.clone().into()))
                        .map(|card| (id, card.selected() == Some(true)))
                })
                .collect();
            let tints = page
                .tints
                .iter()
                .filter_map(|tint| {
                    let id = format!("{TINT_BUTTON}-{}", tint.id);
                    window
                        .try_find(ElementId::Name(id.into()))
                        .map(|button| button.selected() == Some(true))
                })
                .collect();
            (cards, tints)
        })
    }

    /// The theme changes sent, in order.
    fn sent(backend: &FakeBackend) -> Vec<ThemeChange> {
        backend
            .calls()
            .into_iter()
            .filter_map(|call| match call {
                Call::ChangeTheme(change) => Some(change),
                _ => None,
            })
            .collect()
    }

    #[gpui_kit::test]
    fn it_lists_every_preset_the_server_lists_the_current_one_ticked(cx: &mut TestAppContext) {
        let mut page = every_preset();
        // One the picker has never heard of shows as the others do.
        let mut new = page.presets[1].clone();
        new.id = "calm".into();
        new.name = "Calm light".into();
        page.presets.push(new);
        page.choice.preset = "teal".into();
        let (_, _, mut cx) = open(page.clone(), cx);
        let (cards, tints) = drawn(&page, &mut cx);
        let ids: Vec<&str> = cards.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "look-preset-paper",
                "look-preset-graphite",
                "look-preset-blue",
                "look-preset-teal",
                "look-preset-green",
                "look-preset-orange",
                "look-preset-rose",
                "look-preset-purple",
                "look-preset-calm",
            ]
        );
        let ticked: Vec<&str> = cards
            .iter()
            .filter(|(_, on)| *on)
            .map(|(id, _)| id.as_str())
            .collect();
        assert_eq!(ticked, ["look-preset-teal"]);
        assert_eq!(
            tints,
            [true, false, false, false],
            "the ember's own colours"
        );
    }

    #[gpui_kit::test]
    fn a_custom_colour_ticks_no_card(cx: &mut TestAppContext) {
        let mut page = every_preset();
        page.choice.accent = Some("#3366FF".into());
        let (_, _, mut cx) = open(page.clone(), cx);
        let (cards, _) = drawn(&page, &mut cx);
        assert!(cards.iter().all(|(_, on)| !on), "{cards:?}");
    }

    #[gpui_kit::test]
    fn choosing_a_card_sends_the_theme_change_and_ticks_it_at_once(cx: &mut TestAppContext) {
        let page = every_preset();
        let (backend, store, mut cx) = open(page.clone(), cx);
        cx.update(|window, cx| {
            window.render_frame(cx);
            window.click("look-preset-purple", cx);
        });
        assert_eq!(
            store.read_with(&cx, |store, _| store.theme().page().choice.preset.clone()),
            "purple",
            "shown before the server answers"
        );
        cx.run_until_parked();
        let changes = sent(&backend);
        assert_eq!(changes.len(), 1, "{changes:?}");
        assert_eq!(changes[0].preset.as_deref(), Some("purple"));
        assert!(!changes[0].preview);
        cx.update(|window, cx| {
            window.render_frame(cx);
            window.click("look-mode-dark", cx);
        });
        cx.run_until_parked();
        assert_eq!(
            sent(&backend).last().and_then(|c| c.appearance.clone()),
            Some(Appearance::Dark)
        );
    }

    #[gpui_kit::test]
    fn choosing_a_tint_sends_it_with_the_theme(cx: &mut TestAppContext) {
        let page = every_preset();
        let (backend, store, mut cx) = open(page.clone(), cx);
        cx.update(|window, cx| {
            window.render_frame(cx);
            window.click("look-tint-sage", cx);
        });
        cx.run_until_parked();
        let changes = sent(&backend);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].ember_tint.as_deref(), Some("sage"));
        assert_eq!(
            changes[0].preset.as_deref(),
            Some("paper"),
            "the preset stays"
        );
        let tint = store.read_with(&cx, |store, _| {
            store.theme().page().choice.ember_tint.clone()
        });
        assert_eq!(tint, "sage");
        let (_, tints) = drawn(&page, &mut cx);
        assert_eq!(tints, [false, false, false, true]);
    }

    #[gpui_kit::test]
    fn in_setup_it_is_the_same_choice_with_less_around_it(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            cx.set_global(Pal::new(&crate::theme::built_in(), false));
        });
        let backend = Arc::new(FakeBackend::new([]));
        let shared = Arc::clone(&backend) as Arc<dyn crate::backend::Backend>;
        let store = cx.new(|_| Store::for_tests(shared));
        let mut page = every_preset();
        page.choice.accent = Some("#3366FF".into());
        store.update(cx, |store, _| store.put_theme_for_tests(page));
        let window = cx.add_window(|_, cx| LookPicker::new(store.clone(), LookVariant::Setup, cx));
        let mut cx = VisualTestContext::from_window(window.into(), cx);
        cx.update(|window, cx| {
            window.render_frame(cx);
            let card = window.find("look-preset-paper").bounds();
            assert_eq!(card.size.width, px(96.), "the tight grid's card");
            assert!(window.try_find("look-tint-rose").is_some());
            window.click("look-preset-green", cx);
        });
        cx.run_until_parked();
        assert_eq!(
            sent(&backend)
                .first()
                .and_then(|c| c.preset.clone())
                .as_deref(),
            Some("green"),
            "the same theme change as on Settings"
        );
    }

    #[gpui_kit::test]
    fn without_tints_listed_there_is_no_tint_row(cx: &mut TestAppContext) {
        let mut page = every_preset();
        page.tints.clear();
        let (_, _, mut cx) = open(page, cx);
        cx.update(|window, cx| {
            window.render_frame(cx);
            assert!(window.try_find("look-tint-ember").is_none());
            assert!(window.try_find("look-preset-paper").is_some());
        });
    }
}
