//! Your ember's look: the theme, picked by how it looks. Light, Dark or System; each preset the
//! server lists as a small page in its own colours, in the appearance chosen (both, split on a
//! diagonal, for System), the one in use ticked; and when the server lists the ember's tints,
//! its face in each. It is Appearance's choice, not one of its own: one theme, kept on the
//! server, so every device follows. A view of its own over the store, so Settings and the
//! first-run setup show it as it is; the page around it gives the title.

use gpui_kit::TestSupportExt as _;
use gpui_kit::assets::IconName;
use gpui_kit::component::Icon;
use gpui_kit::prelude::*;
use gpui_kit::{Div, ElementId, Entity, FontWeight, Window, div, img, px, transparent_black};

use super::picture::{self, Shown};
use crate::assets::face;
use crate::store::Store;
use crate::ui::a11y::{self, Accessible as _};
use crate::ui::controls::{icon_segment, segment, segments, tab_stop};
use crate::ui::palette::{Pal, pal};
use crate::ui::settings::parts::note;
use emberhut_api::theme::{Appearance, DEFAULT_PRESET, DEFAULT_TINT, EmberTint, Preset, ThemePage};

/// A preset's card (a swatch in the setup), before the preset's id (`look-preset-paper`), for
/// tests to find.
pub(crate) const PRESET_CARD: &str = "look-preset";
/// A tint's button, before the tint's id (`look-tint-coral`), for tests to find.
pub(crate) const TINT_BUTTON: &str = "look-tint";

/// The System, Light and Dark segments: their ids, appearances, labels and icons (the setup's).
const MODES: [(&str, Appearance, &str, IconName); 3] = [
    (
        "look-mode-system",
        Appearance::System,
        "System",
        IconName::Monitor,
    ),
    ("look-mode-light", Appearance::Light, "Light", IconName::Sun),
    ("look-mode-dark", Appearance::Dark, "Dark", IconName::Moon),
];

/// What the picker says while a custom colour is in use, which no card shows.
const CUSTOM: &str = "A custom colour is in use (Appearance). Picking a look uses its own colour.";

/// The group a face's or swatch's button and its ring share, so the ring tints when the button
/// is hovered.
const RING_HOVER: &str = "look-ring-hover";

/// A setup swatch's circle, across.
const SWATCH: f32 = 28.;

/// Where the picker shows, which sets how much it says around the choices.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LookVariant {
    /// On Your ember's settings page: Mode, Colours (each preset a card with its page and name)
    /// and Your ember (each face named), and a line when a custom colour from Appearance is in
    /// use.
    Settings,
    /// In the first-run setup's step, which has its own words: no headings or notes; the modes
    /// as three icons, the presets as one row of split circles (the chosen one named), the faces
    /// smaller and unnamed.
    Setup,
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

    /// Whether it is the setup's.
    fn setup(&self) -> bool {
        self.variant == LookVariant::Setup
    }

    /// System, Light or Dark, the chosen one raised: words on Settings, icons in the setup.
    fn modes(&self, chosen: &Appearance, p: &Pal) -> Div {
        let mut modes = segments(p);
        for (id, appearance, label, icon) in MODES {
            let store = self.store.clone();
            let on = *chosen == appearance;
            let segment = if self.setup() {
                icon_segment(id, Icon::new(icon), label, on, p)
            } else {
                segment(id, label, on, p)
            };
            let segment = segment.on_click(move |_, _, cx| {
                store.update(cx, |store, cx| {
                    store.pick_theme(None, Some(appearance.clone()), cx);
                });
            });
            // Seen by the headless tests (a no-op outside them).
            modes = modes.child(segment.test_support());
        }
        modes
    }

    /// A preset's button: picking it picks the preset, in its own colour.
    fn preset_button(&self, preset: &Preset, on: bool) -> gpui_kit::Stateful<Div> {
        let (store, id) = (self.store.clone(), preset.id.clone());
        div()
            .id(ElementId::Name(
                format!("{PRESET_CARD}-{}", preset.id).into(),
            ))
            // The chosen one is read as selected, and Paper as the default.
            .button_named(a11y::preset(&preset.name, preset.id == DEFAULT_PRESET))
            .aria_selected(on)
            .cursor_pointer()
            .on_click(move |_, _, cx| {
                store.update(cx, |store, cx| store.pick_theme(Some(id.clone()), None, cx));
            })
    }

    /// A preset's card: its page in the appearance `shown`, then its name. The chosen one is
    /// the loud one: accent edge, soft fill and a tick; hovering another tints its edge.
    fn card(&self, preset: &Preset, on: bool, shown: Shown, p: &Pal) -> impl IntoElement + use<> {
        let page = div()
            .w_full()
            .rounded(px(7.))
            .border_1()
            .border_color(p.outline)
            .overflow_hidden()
            .child(picture::page(preset, shown));
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
        let hover = p.accent.opacity(0.5);
        let card = self
            .preset_button(preset, on)
            .w(px(118.))
            .flex()
            .flex_col()
            .gap(px(6.))
            .p(px(6.))
            .rounded(px(10.))
            .border_1()
            .child(page)
            .child(name);
        tab_stop(card, p)
            .border_color(if on { p.accent } else { p.outline })
            .when(on, |d| d.bg(p.accent_soft))
            .when(!on, |d| d.hover(move |s| s.border_color(hover)))
            .test_support()
    }

    /// A preset's swatch in the setup: a circle of its paper and accent (in the appearance in
    /// use), ringed and named when `on`.
    fn swatch(&self, preset: &Preset, on: bool, p: &Pal) -> impl IntoElement + use<> {
        let hover = p.accent.opacity(0.5);
        let ring = div()
            .rounded_full()
            .p(px(2.))
            .border_2()
            .border_color(if on { p.accent } else { transparent_black() })
            .when(!on, |d| {
                d.group_hover(RING_HOVER, move |s| s.border_color(hover))
            })
            .child(picture::swatch(preset, p.dark, p.outline, SWATCH));
        let swatch = self
            .preset_button(preset, on)
            .group(RING_HOVER)
            .flex()
            .flex_col()
            .items_center()
            .gap(px(3.))
            .rounded(px(10.))
            .border_1()
            .child(ring)
            .when(on, |d| {
                d.child(
                    div()
                        .text_size(px(11.5))
                        .font_weight(FontWeight::SEMIBOLD)
                        .whitespace_nowrap()
                        .child(preset.name.clone()),
                )
            });
        tab_stop(swatch, p).test_support()
    }

    /// A tint: the ember's face in it, ringed in the accent when `on`, named on Settings (in
    /// bold when `on`). No fill: the cards above are the loud choice.
    fn tint(&self, tint: &EmberTint, on: bool, p: &Pal) -> impl IntoElement + use<> {
        let (store, id) = (self.store.clone(), tint.id.clone());
        let size = if self.setup() { 32. } else { 36. };
        let hover = p.accent.opacity(0.5);
        let ring = div()
            .rounded_full()
            .p(px(2.))
            .border_2()
            .border_color(if on { p.accent } else { transparent_black() })
            .when(!on, |d| {
                d.group_hover(RING_HOVER, move |s| s.border_color(hover))
            })
            .child(img(face(&tint.id)).size(px(size)));
        let button = div()
            .id(ElementId::Name(format!("{TINT_BUTTON}-{}", tint.id).into()))
            .group(RING_HOVER)
            .button_named(a11y::preset(
                &format!("{} ember", tint.name),
                tint.id == DEFAULT_TINT,
            ))
            .aria_selected(on)
            .flex()
            .flex_col()
            .items_center()
            .gap(px(4.))
            .p(px(2.))
            .rounded(px(10.))
            .border_1()
            .cursor_pointer()
            .child(ring)
            .when(!self.setup(), |d| {
                d.child(
                    div()
                        .text_size(px(11.5))
                        .when(on, |d| d.font_weight(FontWeight::BOLD))
                        .when(!on, |d| d.text_color(p.muted))
                        .child(tint.name.clone()),
                )
            })
            .on_click(move |_, _, cx| {
                store.update(cx, |store, cx| store.pick_ember_tint(id.clone(), cx));
            });
        tab_stop(button, p).test_support()
    }

    /// Settings' picker: Mode, Colours, a hairline, Your ember.
    fn settings(&self, page: &ThemePage, p: &Pal) -> Div {
        let choice = &page.choice;
        let custom = choice.accent.is_some();
        let shown = Shown::of(&choice.appearance);
        let cards = page
            .presets
            .iter()
            .map(|preset| self.card(preset, !custom && choice.preset == preset.id, shown, p));
        let tints: Vec<_> = page
            .tints
            .iter()
            .map(|tint| self.tint(tint, worn(page) == tint.id, p))
            .collect();
        div()
            .flex()
            .flex_col()
            .gap(px(14.))
            .py(px(4.))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .justify_between()
                    .gap(px(8.))
                    .child(label(p, "Mode"))
                    .child(self.modes(&choice.appearance, p)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .child(label(p, "Colours"))
                    .child(div().flex().flex_wrap().gap(px(10.)).children(cards))
                    .when(custom, |d| d.child(note(p, CUSTOM))),
            )
            .when(!tints.is_empty(), |d| {
                d.child(div().h(px(1.)).bg(p.outline)).child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(8.))
                        .child(label(p, "Your ember"))
                        .child(div().flex().flex_wrap().gap(px(10.)).children(tints)),
                )
            })
    }

    /// The setup's picker: the mode icons, a row of swatches, the faces.
    fn compact(&self, page: &ThemePage, p: &Pal) -> Div {
        let choice = &page.choice;
        let custom = choice.accent.is_some();
        let swatches = page
            .presets
            .iter()
            .map(|preset| self.swatch(preset, !custom && choice.preset == preset.id, p));
        let tints: Vec<_> = page
            .tints
            .iter()
            .map(|tint| self.tint(tint, worn(page) == tint.id, p))
            .collect();
        div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(div().flex().child(self.modes(&choice.appearance, p)))
            .child(div().flex().items_start().gap(px(4.)).children(swatches))
            .when(!tints.is_empty(), |d| {
                d.child(div().flex().gap(px(6.)).children(tints))
            })
    }
}

impl Render for LookPicker {
    fn render(&mut self, _: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let p = pal(cx);
        let page: ThemePage = self.store.read(cx).theme().page().clone();
        let body = if self.setup() {
            self.compact(&page, &p)
        } else {
            self.settings(&page, &p)
        };
        div().id("ember-look").child(body).test_support()
    }
}

/// The tint the face is drawn in: the one chosen, or the default when the page doesn't list it
/// (an early tint, held from before it was dropped), as the face itself draws.
fn worn(page: &ThemePage) -> &str {
    let chosen = page.choice.ember_tint.as_str();
    if page.tints.iter().any(|tint| tint.id == chosen) {
        chosen
    } else {
        DEFAULT_TINT
    }
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
            window.click("look-tint-blue", cx);
        });
        cx.run_until_parked();
        let changes = sent(&backend);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].ember_tint.as_deref(), Some("blue"));
        assert_eq!(
            changes[0].preset.as_deref(),
            Some("paper"),
            "the preset stays"
        );
        let tint = store.read_with(&cx, |store, _| {
            store.theme().page().choice.ember_tint.clone()
        });
        assert_eq!(tint, "blue");
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
            let swatch = window.find("look-preset-paper").bounds();
            assert!(
                swatch.size.width < px(SWATCH + 16.),
                "a swatch, not a card: {swatch:?}"
            );
            assert!(window.try_find("look-tint-coral").is_some());
            assert!(window.try_find("look-mode-dark").is_some());
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
    fn a_tint_no_longer_listed_rings_the_default(cx: &mut TestAppContext) {
        let mut page = every_preset();
        page.choice.ember_tint = "rose".into();
        let (_, _, mut cx) = open(page.clone(), cx);
        let (_, tints) = drawn(&page, &mut cx);
        assert_eq!(tints, [true, false, false, false]);
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
