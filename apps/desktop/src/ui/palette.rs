//! The colours every view draws with: the theme's tokens for the appearance on screen, a few
//! made from them, and the same applied to gpui-kit's own theme (its buttons, inputs, links).
//! No colour literal appears in a view; the one exception is a room's dot, its goal's
//! categorical colour ([`room_color`]).

use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::{App, Global, Hsla, Window, WindowAppearance, rgb};

use crate::assets::UI_FONT;
use crate::color::{Rgb, mix, parse_hex};
use crate::material::Material;
use crate::material::alpha::Sidebar;
use emberhut_api::theme::{Appearance, ThemePage, Tokens};

/// The palette, kept as a GPUI global.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Pal {
    /// Whether it is the dark appearance.
    pub(crate) dark: bool,
    /// The accent.
    pub(crate) accent: Hsla,
    /// The soft accent.
    pub(crate) accent_soft: Hsla,
    /// The glow around the assistant's picture while it works: the accent, partly see-through.
    pub(crate) glow: Hsla,
    /// Text on the accent.
    pub(crate) on_accent: Hsla,
    /// The window's background.
    pub(crate) background: Hsla,
    /// The page's surface.
    pub(crate) surface: Hsla,
    /// A raised surface.
    pub(crate) surface_raised: Hsla,
    /// Borders.
    pub(crate) border: Hsla,
    /// Body text.
    pub(crate) text: Hsla,
    /// Secondary text.
    pub(crate) muted: Hsla,
    /// The faintest text.
    pub(crate) faint: Hsla,
    /// Success.
    pub(crate) success: Hsla,
    /// Warning.
    pub(crate) warn: Hsla,
    /// Danger.
    pub(crate) danger: Hsla,
    /// Information.
    pub(crate) info: Hsla,
    /// A row under the pointer.
    pub(crate) hover: Hsla,
    /// A control held down: the hover's tint, deeper.
    pub(crate) pressed: Hsla,
    /// The user's chat bubble: the accent in light; in dark the accent sunk into the surface.
    pub(crate) mine: Hsla,
    /// Its text.
    pub(crate) on_mine: Hsla,
    /// A card on the page.
    pub(crate) card: Hsla,
    /// The sidebar's fill: the background, a little translucent over the window's material
    /// (see [`Pal::with_material`]).
    pub(crate) sidebar: Hsla,
    /// The material this palette is drawn over: the one asked for, or opaque when the theme's
    /// colours leave the sidebar (nearly) opaque.
    pub(crate) material: Material,
    /// The sidebar's alpha over the material, for this theme's colours
    /// (see `material::alpha`).
    sidebar_alpha: f32,
}

/// The most of the window's material the sidebar lets through in the light appearance: its
/// fill is never more translucent than this. The theme's colours may hold it more opaque (see
/// `material::alpha`): with the server's presets it ends at 0.98 or 0.99 (1 with paper), where
/// the accent and "running" keep 4.5:1 over a black backdrop, which is above
/// [`SIDEBAR_ALPHA_MOST`]: light mode stays opaque unless a theme allows real glass.
const SIDEBAR_ALPHA_FLOOR_LIGHT: f32 = 0.90;

/// The same in the dark appearance (0.93 with the server's presets, 0.94 with paper, for the
/// faint text).
const SIDEBAR_ALPHA_FLOOR_DARK: f32 = 0.84;

/// The most opaque the sidebar may be and still count as having material. Above it the glass
/// is too faint to see, so the sidebar is opaque and the window isn't blurred for it.
const SIDEBAR_ALPHA_MOST: f32 = 0.96;

/// How much of the accent shows in the working glow, light and dark (a dark page needs more for
/// the glow to read).
const GLOW_LIGHT: f32 = 0.55;
const GLOW_DARK: f32 = 0.7;

/// The share of the text colour that tints a row under the pointer, light and dark.
const HOVER_LIGHT: f32 = 0.05;
const HOVER_DARK: f32 = 0.07;

/// The share of the text colour that tints a control while it is held down, light and dark.
const PRESSED_LIGHT: f32 = 0.10;
const PRESSED_DARK: f32 = 0.13;

impl Global for Pal {}

impl Pal {
    /// The palette for `page` in its appearance (`system_dark` when it follows the system).
    #[must_use]
    pub(crate) fn new(page: &ThemePage, system_dark: bool) -> Self {
        let dark = match page.choice.appearance {
            Appearance::Light => false,
            Appearance::Dark => true,
            // System, and an appearance this app doesn't know yet.
            _ => system_dark,
        };
        let tokens = if dark {
            &page.tokens.dark
        } else {
            &page.tokens.light
        };
        let c = |token: &str| color(hex(token));
        let hover = if dark { HOVER_DARK } else { HOVER_LIGHT };
        // Every colour the sidebar draws text and icons in.
        let inks = [
            &tokens.text,
            &tokens.muted,
            &tokens.faint,
            &tokens.accent,
            &tokens.info,
        ]
        .map(|token| hex(token));
        let sidebar_alpha = Sidebar {
            fill: hex(&tokens.background),
            inks: &inks,
            hover: (hex(&tokens.text), hover),
        }
        .alpha(if dark {
            SIDEBAR_ALPHA_FLOOR_DARK
        } else {
            SIDEBAR_ALPHA_FLOOR_LIGHT
        });
        Self {
            dark,
            accent: c(&tokens.accent),
            accent_soft: c(&tokens.accent_soft),
            glow: c(&tokens.accent).opacity(if dark { GLOW_DARK } else { GLOW_LIGHT }),
            on_accent: c(&tokens.on_accent),
            background: c(&tokens.background),
            surface: c(&tokens.surface),
            surface_raised: c(&tokens.surface_raised),
            border: c(&tokens.border),
            text: c(&tokens.text),
            muted: c(&tokens.muted),
            faint: c(&tokens.faint),
            success: c(&tokens.success),
            warn: c(&tokens.warn),
            danger: c(&tokens.danger),
            info: c(&tokens.info),
            hover: c(&tokens.text).opacity(hover),
            pressed: c(&tokens.text).opacity(if dark { PRESSED_DARK } else { PRESSED_LIGHT }),
            mine: if dark {
                color(mix(hex(&tokens.accent), hex(&tokens.surface), 0.35))
            } else {
                c(&tokens.accent)
            },
            on_mine: if dark {
                c(&tokens.text)
            } else {
                c(&tokens.on_accent)
            },
            card: if dark {
                c(&tokens.surface_raised)
            } else {
                c(&tokens.background)
            },
            sidebar: c(&tokens.background),
            material: Material::Opaque,
            sidebar_alpha,
        }
    }

    /// This palette over `material`: the sidebar's fill is the background token, at the
    /// theme's sidebar alpha when the sidebar has material, opaque otherwise. A theme whose
    /// colours hold the sidebar above [`SIDEBAR_ALPHA_MOST`] gets no material at all, so the
    /// window isn't blurred for glass no one would see. Every other colour is the theme's as
    /// it is, so the page's contrast never changes.
    #[must_use]
    pub(crate) fn with_material(self, material: Material) -> Self {
        let (material, alpha) = match material {
            Material::Sidebar if self.sidebar_alpha <= SIDEBAR_ALPHA_MOST => {
                (material, self.sidebar_alpha)
            }
            Material::Sidebar | Material::Opaque => (Material::Opaque, 1.0),
        };
        Self {
            sidebar: self.background.opacity(alpha),
            material,
            ..self
        }
    }

    /// A token's colour (`accent` of a preview, a preset's swatch).
    #[must_use]
    pub(crate) fn token(hex_text: &str) -> Hsla {
        color(hex(hex_text))
    }

    /// Text and icons that read on `fill` (a coloured fill other than the accent): white or
    /// near-black by the fill's own luminance.
    #[must_use]
    pub(crate) fn readable_on(fill: Hsla) -> Hsla {
        let fill = fill.to_rgb();
        let channel = |value: f32| u32::from(crate::color::to_byte(value * 255.0));
        let rgb = (channel(fill.r) << 16) | (channel(fill.g) << 8) | channel(fill.b);
        color(crate::color::readable_on(rgb))
    }

    /// The tokens a palette made from `page` draws with in this palette's appearance.
    #[must_use]
    pub(crate) fn tokens_of(self, page: &ThemePage) -> &Tokens {
        if self.dark {
            &page.tokens.dark
        } else {
            &page.tokens.light
        }
    }
}

/// The palette now.
#[must_use]
pub(crate) fn pal(cx: &App) -> Pal {
    *cx.global::<Pal>()
}

/// Puts `page`'s palette over `material` in place for `window`, and into gpui-kit's theme;
/// returns it. `ui::look::apply` is the one caller: it also puts the material on the window.
pub(crate) fn apply(
    page: &ThemePage,
    material: Material,
    window: &mut Window,
    cx: &mut App,
) -> Pal {
    let system_dark = matches!(
        window.appearance(),
        WindowAppearance::Dark | WindowAppearance::VibrantDark
    );
    let p = Pal::new(page, system_dark).with_material(material);
    cx.set_global(p);
    Theme::change(
        if p.dark {
            ThemeMode::Dark
        } else {
            ThemeMode::Light
        },
        Some(window),
        cx,
    );
    Theme::update(cx, |t| {
        t.font_family = UI_FONT.into();
        t.primary = p.accent;
        t.primary_hover = p.accent.opacity(0.9);
        t.primary_active = p.accent.opacity(0.8);
        t.ring = p.accent;
        t.caret = p.accent;
        t.primary_foreground = p.on_accent;
        t.button_primary_foreground = p.on_accent;
        t.accent = p.accent_soft;
        t.accent_foreground = p.text;
        t.selection = p.accent.opacity(0.3);
        t.background = p.surface;
        t.foreground = p.text;
        t.muted_foreground = p.muted;
        t.popover = p.surface_raised;
        t.popover_foreground = p.text;
        t.input = p.border;
        t.border = p.border;
        t.success = p.success;
        t.warning = p.warn;
        t.danger = p.danger;
        t.info = p.info;
        t.button_primary = p.accent;
        t.button_primary_hover = p.accent.opacity(0.9);
        t.button_primary_active = p.accent.opacity(0.8);
        t.link = p.accent;
    });
    p
}

/// A room's dot: its goal's colour by rank, as on the other apps (categorical, not the
/// theme's); a room without a goal takes the faint grey.
#[must_use]
pub(crate) fn room_color(goal_rank: Option<i32>, p: &Pal) -> Hsla {
    // Orange, green, purple, cyan, pink, yellow, mint.
    const GOALS: [Rgb; 7] = [
        0xFF_95_00, 0x34_C7_59, 0xAF_52_DE, 0x32_AD_E6, 0xFF_2D_55, 0xFF_CC_00, 0x00_C7_BE,
    ];
    match goal_rank
        .and_then(|rank| usize::try_from(rank).ok())
        .filter(|&rank| rank >= 1)
    {
        Some(rank) => color(GOALS[(rank - 1) % GOALS.len()]),
        None => p.faint,
    }
}

/// A categorical colour, steady for `index` (an About you category, a Chrome profile's
/// badge): blue, indigo, green, orange, pink, teal, purple, red, as on the other apps. Like a
/// room's dot, it tells things apart and isn't the theme's.
#[must_use]
pub(crate) fn category_color(index: usize) -> Hsla {
    const CATEGORIES: [Rgb; 8] = [
        0x00_7A_FF, 0x58_56_D6, 0x34_C7_59, 0xFF_95_00, 0xFF_2D_55, 0x30_B0_C7, 0xAF_52_DE,
        0xFF_3B_30,
    ];
    color(CATEGORIES[index % CATEGORIES.len()])
}

/// A token's colour; a token that isn't one draws grey (and is logged).
fn hex(token: &str) -> Rgb {
    parse_hex(token).unwrap_or_else(|| {
        log::warn!("the theme token {token:?} isn't a colour; drawing it grey");
        0x80_80_80
    })
}

fn color(value: Rgb) -> Hsla {
    rgb(value).into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::{contrast_ratio, luminance};
    use crate::material::alpha::{AA, BACKDROPS, KEEP_BELOW_AA};
    use crate::theme::{built_in, preset_tokens};

    /// The theme with `preset`'s colours, as the server makes it.
    fn preset_page(preset: &str) -> ThemePage {
        let mut page = built_in();
        page.tokens = preset_tokens(preset);
        page
    }

    fn both(page: &ThemePage) -> [Pal; 2] {
        [Pal::new(page, false), Pal::new(page, true)]
    }

    /// Checks, without `material::alpha`, that every colour the sidebar draws text in reads
    /// at the palette's sidebar alpha over a black and a white backdrop, plain and hovered.
    fn assert_sidebar_reads(page: &ThemePage, p: &Pal, name: &str) {
        let tokens = p.tokens_of(page);
        let fill = hex(&tokens.background);
        let hover = (
            hex(&tokens.text),
            if p.dark { HOVER_DARK } else { HOVER_LIGHT },
        );
        let inks = [
            ("text", &tokens.text),
            ("muted", &tokens.muted),
            ("faint", &tokens.faint),
            ("accent", &tokens.accent),
            ("info", &tokens.info),
        ];
        for (ink_name, token) in inks {
            let ink = hex(token);
            for hovered in [false, true] {
                let on = |under: Rgb| {
                    if hovered {
                        mix(hover.0, under, hover.1)
                    } else {
                        under
                    }
                };
                let opaque = contrast_ratio(ink, on(fill));
                let floor = if opaque >= AA {
                    AA
                } else {
                    opaque * KEEP_BELOW_AA
                };
                let [darkest, lightest] =
                    BACKDROPS.map(|backdrop| luminance(on(mix(fill, backdrop, p.sidebar_alpha))));
                assert!(
                    !(darkest.min(lightest)..=darkest.max(lightest)).contains(&luminance(ink)),
                    "{name} dark={} {ink_name}: some backdrop matches it",
                    p.dark
                );
                for backdrop in BACKDROPS {
                    let ratio = contrast_ratio(ink, on(mix(fill, backdrop, p.sidebar_alpha)));
                    assert!(
                        ratio >= floor,
                        "{name} dark={} {ink_name} hovered={hovered} over {backdrop:06X}: \
                         {ratio:.2} < {floor:.2} at alpha {}",
                        p.dark,
                        p.sidebar_alpha
                    );
                }
            }
        }
    }

    #[test]
    fn without_material_the_sidebar_is_the_opaque_background() {
        for p in both(&built_in()) {
            assert_eq!(p.sidebar, p.background, "dark={}", p.dark);
            assert_eq!(p.material, Material::Opaque);
            let opaque = p.with_material(Material::Opaque);
            assert_eq!(opaque.sidebar, p.background);
            assert_eq!(opaque.material, Material::Opaque);
        }
    }

    #[test]
    fn with_material_the_sidebar_takes_the_themes_alpha() {
        let [light, dark] = both(&preset_page("blue")).map(|p| p.with_material(Material::Sidebar));
        // The values the README and the floors' docs quote for the graphite presets. Light's
        // 0.98 is too faint to count: no material there.
        assert!(
            (light.sidebar_alpha - 0.98).abs() < f32::EPSILON,
            "{}",
            light.sidebar_alpha
        );
        assert_eq!(light.material, Material::Opaque);
        assert_eq!(light.sidebar, light.background);
        assert!(
            (dark.sidebar.a - 0.93).abs() < f32::EPSILON,
            "{}",
            dark.sidebar.a
        );
        assert_eq!(dark.material, Material::Sidebar);
        for p in [light, dark] {
            // The background's colour, only translucent.
            assert_eq!(p.sidebar.alpha(1.0), p.background);
            // The page, its text and the selected rows' fill stay as the theme has them.
            for opaque in [
                p.surface,
                p.background,
                p.text,
                p.muted,
                p.card,
                p.accent_soft,
            ] {
                assert!((opaque.a - 1.0).abs() < f32::EPSILON);
            }
        }
    }

    #[test]
    fn the_material_comes_off_again() {
        for p in both(&built_in()) {
            let back = p
                .with_material(Material::Sidebar)
                .with_material(Material::Opaque);
            assert_eq!(back.sidebar, p.background);
            assert_eq!(back.material, Material::Opaque);
        }
    }

    #[test]
    fn every_presets_sidebar_text_keeps_aa_over_any_backdrop() {
        for preset in emberhut_core::theme::PRESETS.map(|preset| preset.id) {
            let page = preset_page(preset);
            for p in both(&page) {
                let p = p.with_material(Material::Sidebar);
                // The range the README quotes: paper's near-AA ember and green leave its
                // light sidebar no room at all.
                let (low, high) = match (preset, p.dark) {
                    ("paper", false) => (1.0, 1.0),
                    ("paper", true) => (0.94, 0.94),
                    (_, false) => (0.98, 0.99),
                    (_, true) => (0.93, 0.93),
                };
                assert!(
                    (low - 1e-6..=high + 1e-6).contains(&p.sidebar_alpha),
                    "{} dark={}: {}",
                    preset,
                    p.dark,
                    p.sidebar_alpha
                );
                // Light's glass is too faint to count (above `SIDEBAR_ALPHA_MOST`).
                let expected = if p.dark {
                    Material::Sidebar
                } else {
                    Material::Opaque
                };
                assert_eq!(p.material, expected, "{} dark={}", preset, p.dark);
                assert_sidebar_reads(&page, &p, preset);
            }
        }
    }

    #[test]
    fn a_light_theme_with_strong_colours_gets_real_glass() {
        let mut page = preset_page("blue");
        page.tokens.light.accent = "#1D4ED8".into();
        page.tokens.light.info = "#155E75".into();
        let p = Pal::new(&page, false).with_material(Material::Sidebar);
        assert!(p.sidebar_alpha <= SIDEBAR_ALPHA_MOST, "{}", p.sidebar_alpha);
        assert_eq!(p.material, Material::Sidebar);
        assert!(p.sidebar.a < 1.0);
        assert_sidebar_reads(&page, &p, "strong");
    }

    #[test]
    fn an_accent_right_at_aa_keeps_the_sidebar_and_the_window_opaque() {
        // A custom accent the server let through at 4.5:1 or a hair above on the background.
        let background = hex(&built_in().tokens.light.background);
        let accent = (0x40_u32..0xA0)
            .map(|v| v << 16 | v << 8 | v)
            .rfind(|&accent| contrast_ratio(accent, background) >= AA)
            .unwrap();
        let ratio = contrast_ratio(accent, background);
        assert!((AA..AA + 0.1).contains(&ratio), "{ratio}");
        let mut page = built_in();
        page.tokens.light.accent = format!("#{accent:06X}");
        let p = Pal::new(&page, false).with_material(Material::Sidebar);
        assert!((p.sidebar_alpha - 1.0).abs() < f32::EPSILON);
        assert_eq!(p.material, Material::Opaque);
        assert_eq!(p.sidebar, p.background);
        assert_sidebar_reads(&page, &p, "at AA");
    }
}
