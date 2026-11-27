//! How the window looks: the theme's palette over the window's material. [`apply`] is the one
//! place that puts them on the window: the palette, the window's background (blurred or
//! opaque), the fill of gpui-kit's root, which paints the theme's background under the whole
//! window unless the material needs it clear, and the system's own appearance for the app's
//! windows (their title bars, alerts and blur), held to the theme's Light or Dark. Nothing
//! here runs at rest: the watch of Reduce Transparency is a task asleep on the system's
//! notification.

use emberhut_api::theme::{Appearance, ThemePage};
use gpui_kit::base::Root;
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyWindowHandle, App, Entity, Global, Window, WindowAppearance, WindowBackgroundAppearance,
    transparent_black,
};

use super::palette;
use crate::material::{Look, Material, Platform, Transparency};
use crate::store::Store;

/// The material as chosen, and as on the window.
#[derive(Clone, Copy, Debug)]
struct State {
    /// The platform the material is chosen for.
    platform: Platform,
    /// The look the command line asked for.
    look: Look,
    /// The material the platform, the system's setting and the command line allow.
    chosen: Material,
    /// Whether the sidebar is shown: without it nothing is translucent.
    sidebar: bool,
    /// The material on the window: the chosen one, unless the sidebar is hidden or the theme's
    /// colours leave it (nearly) opaque.
    shown: Material,
}

impl State {
    /// Before [`start`] (the views' tests): every surface opaque.
    const OPAQUE: Self = Self {
        platform: Platform::Other,
        look: Look::Opaque,
        chosen: Material::Opaque,
        sidebar: true,
        shown: Material::Opaque,
    };

    /// The state as it was, with the material chosen again from the system's setting now.
    fn rechosen(self) -> Self {
        Self {
            chosen: Material::choose(self.platform, Transparency::current(), self.look),
            ..self
        }
    }

    /// The material the window should have, before the theme's say.
    fn wanted(self) -> Material {
        if self.sidebar {
            self.chosen
        } else {
            Material::Opaque
        }
    }
}

impl Global for State {}

/// The appearance the app's windows were last held to: the theme's Light or Dark, or none to
/// follow the system.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Held(Option<WindowAppearance>);

impl Global for Held {}

fn state(cx: &App) -> State {
    cx.try_global::<State>().copied().unwrap_or(State::OPAQUE)
}

/// Chooses the material at launch, for `look`, and returns the background to open the window
/// with.
pub(crate) fn start(look: Look, cx: &mut App) -> WindowBackgroundAppearance {
    let state = State {
        platform: Platform::CURRENT,
        look,
        ..State::OPAQUE
    }
    .rechosen();
    cx.set_global(State {
        shown: state.chosen,
        ..state
    });
    background(state.chosen)
}

/// The background to open another window with: the material shown now.
#[must_use]
pub(crate) fn background_now(cx: &App) -> WindowBackgroundAppearance {
    background(state(cx).shown)
}

/// Puts `page`'s palette and the material on `window`. Called while the window opens (before
/// gpui-kit's root exists: its fill is set by [`follow`]), and whenever the theme, the
/// appearance, the sidebar or the material changes.
pub(crate) fn apply(page: &ThemePage, window: &mut Window, cx: &mut App) {
    hold_appearance(&page.choice.appearance, cx);
    let state = state(cx);
    let shown = palette::apply(page, state.wanted(), window, cx).material;
    if shown != state.shown {
        window.set_background_appearance(background(shown));
        cx.set_global(State { shown, ..state });
    }
    fill_root(shown, window, cx);
}

/// Shows or hides the sidebar's material with the sidebar: a window with nothing translucent
/// in it isn't blurred.
pub(crate) fn show_sidebar(shown: bool, page: &ThemePage, window: &mut Window, cx: &mut App) {
    if let Some(state) = cx.try_global::<State>().copied() {
        cx.set_global(State {
            sidebar: shown,
            ..state
        });
    }
    apply(page, window, cx);
}

/// Puts the look on `window` now that it is open, then follows the system's Reduce
/// Transparency: on a change the material is chosen again and applied with `store`'s theme.
/// The watch starts before the setting is read again, so no change since [`start`] is lost.
/// It ends at the first change after the window has gone, or with the app.
pub(crate) fn follow(window: AnyWindowHandle, store: Entity<Store>, cx: &mut App) {
    let changes = Transparency::changes();
    let page = store.read(cx).theme().page().clone();
    let opened = window.update(cx, |_, window, cx| {
        rechoose(cx);
        apply(&page, window, cx);
    });
    if let Err(error) = opened {
        log::warn!("couldn't put the material on the window: {error:#}");
    }
    cx.spawn(async move |cx| {
        while changes.next().await {
            let updated = cx.update_window(window, |_, window, cx| {
                if let Some(chosen) = rechoose(cx) {
                    log::info!("the window's material is now {chosen:?}");
                    apply(&store.read(cx).theme().page().clone(), window, cx);
                    window.refresh();
                }
            });
            if updated.is_err() {
                // The window is gone.
                break;
            }
        }
    })
    .detach();
}

/// The system appearance the app's windows are held to for the theme's `appearance`: its
/// Light or Dark, or none (System) to follow the system's.
fn held(appearance: &Appearance) -> Option<WindowAppearance> {
    match appearance {
        Appearance::Light => Some(WindowAppearance::Light),
        Appearance::Dark => Some(WindowAppearance::Dark),
        // System, and an appearance this app doesn't know yet.
        _ => None,
    }
}

/// Holds the app's windows to the theme's `appearance` when that changed, so what the system
/// draws for them (on macOS their title bars, the alerts of `Window::prompt`, menus and the
/// window's blur, through `NSApplication.appearance`) matches what the app draws even when the
/// theme's choice isn't the system's. gpui does nothing with it on other platforms. A window's
/// own appearance follows; its observers apply the theme again then.
fn hold_appearance(appearance: &Appearance, cx: &mut App) {
    let wanted = Held(held(appearance));
    if cx.try_global::<Held>() != Some(&wanted) {
        cx.set_window_appearance(wanted.0);
        cx.set_global(wanted);
    }
}

/// Holds `window` to the dark appearance whatever the theme's, for a window always drawn dark
/// (the Dark room): what the system draws for it, on macOS its title bar, its fields' context
/// menus and its alerts, is then dark too. A window's own appearance wins over the one the app
/// is held to ([`hold_appearance`]). The other platforms draw none of these in a window's own
/// appearance: nothing to do there.
pub(crate) fn always_dark(window: &Window) {
    #[cfg(target_os = "macos")]
    mac::always_dark(window);
    #[cfg(not(target_os = "macos"))]
    let _ = window;
}

/// The window's own appearance on macOS.
#[cfg(target_os = "macos")]
mod mac {
    use gpui_kit::Window;
    use objc2_app_kit::{NSAppearance, NSAppearanceCustomization as _, NSAppearanceNameDarkAqua};

    /// Gives `window`'s `NSWindow` the dark Aqua appearance.
    #[expect(unsafe_code, reason = "AppKit's appearance names are extern statics")]
    pub(super) fn always_dark(window: &Window) {
        let Some(ns_window) = crate::ui::share::content_view(window).and_then(|view| view.window())
        else {
            log::debug!("no NSWindow to hold dark");
            return;
        };
        // SAFETY: a constant AppKit defines, alive for the whole process.
        let dark = NSAppearance::appearanceNamed(unsafe { NSAppearanceNameDarkAqua });
        ns_window.setAppearance(dark.as_deref());
    }
}

/// Keeps `window`'s look in step while it is open: applies `store`'s theme again when the
/// window's appearance changes (the system's light or dark, which the palette follows while the
/// theme does) and when the theme does (from the server or Settings). Every window the app
/// opens does this, each for its own background and root fill.
pub(crate) fn keep<V: 'static>(
    store: &Entity<Store>,
    window: &mut Window,
    cx: &mut Context<'_, V>,
) {
    // Each redraws the whole window, not only this view: every view in it draws with the
    // palette.
    let observed = store.clone();
    cx.observe_window_appearance(window, move |_, window, cx| {
        let page = observed.read(cx).theme().page().clone();
        apply(&page, window, cx);
        window.refresh();
    })
    .detach();
    let mut revision = store.read(cx).theme().revision();
    cx.observe_in(store, window, move |_, store, window, cx| {
        let theme = store.read(cx).theme();
        if theme.revision() != revision {
            revision = theme.revision();
            let page = theme.page().clone();
            apply(&page, window, cx);
            window.refresh();
        }
    })
    .detach();
}

/// Chooses the material again from the system's setting; the new one if it changed.
fn rechoose(cx: &mut App) -> Option<Material> {
    let before = cx.try_global::<State>().copied()?;
    let state = before.rechosen();
    cx.set_global(state);
    (state.chosen != before.chosen).then_some(state.chosen)
}

/// The window background `material` draws over.
fn background(material: Material) -> WindowBackgroundAppearance {
    if material.blurs_window() {
        WindowBackgroundAppearance::Blurred
    } else {
        WindowBackgroundAppearance::Opaque
    }
}

/// Clears gpui-kit's root fill under a blurred window (its own style goes over the theme's
/// background its plugin gives it), and gives it back otherwise. Nothing to do before the root
/// exists.
fn fill_root(material: Material, window: &mut Window, cx: &mut App) {
    let Some(Some(root)) = window.root::<Root>() else {
        return;
    };
    let fill = material.blurs_window().then(|| transparent_black().into());
    root.update(cx, |root, cx| {
        if root.style().background != fill {
            root.style().background = fill;
            cx.notify();
        }
    });
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use gpui_kit::component::ActiveTheme as _;
    use gpui_kit::{Hsla, StyleRefinement, TestAppContext, WindowOptions};

    use super::*;
    use crate::backend::Backend;
    use crate::backend::fake::FakeBackend;
    use crate::theme::built_in;
    use crate::ui::palette::{Pal, pal};

    struct Blank;

    impl Render for Blank {
        fn render(&mut self, _: &mut Window, _: &mut Context<'_, Self>) -> impl IntoElement {
            gpui_kit::div()
        }
    }

    /// What gpui-kit's root paints under the window: its plugin's fill with the root's own
    /// style over it. The order mirrors gpui-kit 0.11, which exposes no resolved style:
    /// gpui-component's `WindowState` plugin (`RootPlugin::style` in its `root.rs`) refines the
    /// surface with `tokens.background`, then gpui-base's `Root::render` applies the root's own
    /// style with `refine_style`.
    fn root_fill(window: &mut Window, cx: &mut App) -> Option<Hsla> {
        let root = window.root::<Root>().flatten().expect("a gpui-kit window");
        let own = root.update(cx, |root, _| root.style().clone());
        let mut resolved = StyleRefinement::default().bg(cx.theme().tokens.background);
        resolved.refine(&own);
        resolved
            .background
            .and_then(|fill| fill.color())
            .and_then(|fill| fill.as_solid())
    }

    /// A store whose theme is the built-in one in `appearance`.
    fn store(appearance: Appearance, cx: &mut TestAppContext) -> Entity<Store> {
        let backend = Arc::new(FakeBackend::new([])) as Arc<dyn Backend>;
        let store = cx.new(|_| Store::for_tests(backend));
        store.update(cx, |store, _| store.fix_theme(None, Some(appearance)));
        store
    }

    /// Whether this machine lets windows be translucent (a Mac set to reduce transparency
    /// doesn't, and the material then stays opaque).
    fn transparency_allowed() -> bool {
        Transparency::current() == Transparency::Allowed
    }

    /// Opens a window as the app does, on macOS (the material allowed), and follows.
    fn open_on_macos(store: &Entity<Store>, cx: &mut TestAppContext) -> AnyWindowHandle {
        cx.update(|cx| {
            gpui_kit::init(cx);
            cx.set_global(Pal::new(&built_in(), false));
            cx.set_global(State {
                platform: Platform::MacOs,
                look: Look::Material,
                ..State::OPAQUE
            });
            let page = store.read(cx).theme().page().clone();
            let (window, _) =
                crate::shell::unseen::open_window(WindowOptions::default(), cx, |window, cx| {
                    apply(&page, window, cx);
                    cx.new(|_| Blank)
                })
                .unwrap();
            follow(window, store.clone(), cx);
            window
        })
    }

    /// Whether the root paints nothing, after checking what stays opaque whatever the material.
    fn root_is_clear(window: AnyWindowHandle, cx: &mut TestAppContext) -> bool {
        cx.update(|cx| {
            window
                .update(cx, |_, window, cx| {
                    let p = pal(cx);
                    let clear = root_fill(window, cx).unwrap().a == 0.0;
                    assert_eq!(p.material.blurs_window(), clear);
                    assert_eq!(p.sidebar.a < 1.0, clear);
                    assert_eq!(state(cx).shown, p.material);
                    // The page, the report and the composer keep their opaque fill.
                    assert!((p.surface.a - 1.0).abs() < f32::EPSILON);
                    // gpui-kit's components keep an opaque background token.
                    assert!((cx.theme().tokens.background.a - 1.0).abs() < f32::EPSILON);
                    clear
                })
                .unwrap()
        })
    }

    #[gpui_kit::test]
    fn start_chooses_for_this_platform_and_its_setting(cx: &mut TestAppContext) {
        cx.update(|cx| {
            let opened = start(Look::Material, cx);
            let expected =
                Material::choose(Platform::CURRENT, Transparency::current(), Look::Material);
            assert_eq!(opened, background(expected));
            assert_eq!(cx.global::<State>().chosen, expected);
            assert_eq!(start(Look::Opaque, cx), WindowBackgroundAppearance::Opaque);
            assert_eq!(cx.global::<State>().chosen, Material::Opaque);
        });
    }

    #[gpui_kit::test]
    fn a_window_that_keeps_its_look_takes_a_new_theme(cx: &mut TestAppContext) {
        let store = store(Appearance::Light, cx);
        cx.update(|cx| {
            gpui_kit::init(cx);
            let page = store.read(cx).theme().page().clone();
            crate::shell::unseen::open_window(WindowOptions::default(), cx, |window, cx| {
                apply(&page, window, cx);
                cx.new(|cx| {
                    keep(&store, window, cx);
                    Blank
                })
            })
            .unwrap();
            assert!(!pal(cx).dark);
        });
        store.update(cx, |store, cx| {
            store.pick_theme(None, Some(Appearance::Dark), cx);
        });
        cx.run_until_parked();
        cx.update(|cx| {
            assert!(pal(cx).dark, "the palette");
            assert!(cx.theme().is_dark(), "gpui-kit's theme");
            assert_eq!(cx.global::<Held>().0, Some(WindowAppearance::Dark));
        });
    }

    #[test]
    fn only_a_dark_appearance_is_dark() {
        use WindowAppearance::{Dark, Light, VibrantDark, VibrantLight};
        assert!(palette::is_dark(Dark) && palette::is_dark(VibrantDark));
        assert!(!palette::is_dark(Light) && !palette::is_dark(VibrantLight));
    }

    #[gpui_kit::test]
    fn on_system_the_palette_takes_the_apps_appearance(cx: &mut TestAppContext) {
        // gpui's test platform gives no window an appearance of its own (its windows and the
        // app are both light), so this checks the source: the app's appearance decides.
        let store = store(Appearance::System, cx);
        open_on_macos(&store, cx);
        cx.update(|cx| {
            assert_eq!(pal(cx).dark, palette::is_dark(cx.window_appearance()));
            assert!(!pal(cx).dark, "the test platform's app is light");
        });
    }

    #[test]
    fn the_windows_are_held_to_the_themes_light_or_dark_and_let_go_for_system() {
        assert_eq!(held(&Appearance::Light), Some(WindowAppearance::Light));
        assert_eq!(held(&Appearance::Dark), Some(WindowAppearance::Dark));
        assert_eq!(held(&Appearance::System), None);
    }

    #[gpui_kit::test]
    fn applying_the_theme_holds_the_windows_to_its_choice(cx: &mut TestAppContext) {
        let store = store(Appearance::Dark, cx);
        let window = open_on_macos(&store, cx);
        let held_now = |cx: &mut TestAppContext| cx.update(|cx| cx.global::<Held>().0);
        assert_eq!(held_now(cx), Some(WindowAppearance::Dark));
        for (appearance, expected) in [
            (Appearance::System, None),
            (Appearance::Light, Some(WindowAppearance::Light)),
        ] {
            store.update(cx, |store, _| store.fix_theme(None, Some(appearance)));
            cx.update(|cx| {
                let page = store.read(cx).theme().page().clone();
                window
                    .update(cx, |_, window, cx| apply(&page, window, cx))
                    .unwrap();
            });
            assert_eq!(held_now(cx), expected);
        }
    }

    #[test]
    fn only_the_sidebar_material_blurs_the_window() {
        assert_eq!(
            background(Material::Sidebar),
            WindowBackgroundAppearance::Blurred
        );
        assert_eq!(
            background(Material::Opaque),
            WindowBackgroundAppearance::Opaque
        );
    }

    #[gpui_kit::test]
    fn once_followed_the_dark_window_paints_nothing_under_the_sidebar(cx: &mut TestAppContext) {
        let store = store(Appearance::Dark, cx);
        let window = open_on_macos(&store, cx);
        // A Mac set to reduce transparency keeps every window opaque.
        assert_eq!(root_is_clear(window, cx), transparency_allowed());
    }

    #[gpui_kit::test]
    fn light_mode_is_opaque_with_the_server_presets(cx: &mut TestAppContext) {
        // Paper's sidebar is opaque outright, the other presets' 0.98 opaque: too faint to count.
        let store = store(Appearance::Light, cx);
        let window = open_on_macos(&store, cx);
        assert!(!root_is_clear(window, cx));
    }

    #[gpui_kit::test]
    fn a_hidden_sidebar_turns_the_blur_off(cx: &mut TestAppContext) {
        let store = store(Appearance::Dark, cx);
        let window = open_on_macos(&store, cx);
        for shown in [false, true, false] {
            cx.update(|cx| {
                let page = store.read(cx).theme().page().clone();
                window
                    .update(cx, |_, window, cx| show_sidebar(shown, &page, window, cx))
                    .unwrap();
            });
            assert_eq!(
                root_is_clear(window, cx),
                shown && transparency_allowed(),
                "shown={shown}"
            );
        }
    }

    /// Checks that every one of gpui-kit's colours is the palette's after a window opened in
    /// `appearance`.
    fn assert_kit_colours_are_the_palettes(appearance: Appearance, cx: &mut TestAppContext) {
        let store = store(appearance, cx);
        open_on_macos(&store, cx);
        cx.update(|cx| {
            let mut expected = gpui_kit::component::ThemeColor::default();
            palette::kit_colors(&pal(cx), &mut expected);
            assert_eq!(cx.theme().colors, expected);
        });
    }

    #[gpui_kit::test]
    fn every_gpui_kit_colour_is_the_palettes_in_light(cx: &mut TestAppContext) {
        assert_kit_colours_are_the_palettes(Appearance::Light, cx);
    }

    #[gpui_kit::test]
    fn every_gpui_kit_colour_is_the_palettes_in_dark(cx: &mut TestAppContext) {
        assert_kit_colours_are_the_palettes(Appearance::Dark, cx);
    }

    #[gpui_kit::test]
    fn the_opaque_look_keeps_the_root_filled(cx: &mut TestAppContext) {
        let store = store(Appearance::Dark, cx);
        let window = open_on_macos(&store, cx);
        cx.update(|cx| {
            cx.set_global(State {
                look: Look::Opaque,
                ..state(cx)
            });
            let page = store.read(cx).theme().page().clone();
            window
                .update(cx, |_, window, cx| {
                    rechoose(cx);
                    apply(&page, window, cx);
                })
                .unwrap();
        });
        assert!(!root_is_clear(window, cx));
    }
}
