//! The colours every view draws with: the theme's tokens for the appearance on screen, a few
//! made from them, and the same applied to gpui-kit's own theme (its buttons, inputs, links).
//! No colour literal appears in a view: the categorical ones too (a room's goal, a category,
//! a calendar tone) are here.

use gpui_kit::component::{Theme, ThemeColor, ThemeMode};
use gpui_kit::{App, Global, Hsla, Window, WindowAppearance, rgb, transparent_black};

use crate::assets::UI_FONT;
use crate::color::{Rgb, mix, parse_hex};
use crate::material::Material;
use crate::material::alpha::Sidebar;
use crate::store::calendar::words::Tone;
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
    /// A message of the user's still waiting to be sent: their bubble paled into the surface,
    /// opaque, so its text is chosen for what is drawn rather than read through a translucent
    /// fill.
    pub(crate) queued: Hsla,
    /// Its text.
    pub(crate) on_queued: Hsla,
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
/// `material::alpha`): with the server's presets it ends at 1 in light mode, where
/// the accent and "running" keep 4.5:1 over a black backdrop, which is above
/// [`SIDEBAR_ALPHA_MOST`]: light mode stays opaque unless a theme allows real glass.
const SIDEBAR_ALPHA_FLOOR_LIGHT: f32 = 0.90;

/// The same in the dark appearance (0.94 with the server's presets, for their faint text).
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

/// The share of the user's bubble left in a waiting message's fill, the rest the surface.
const QUEUED: f32 = 0.6;

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
        Self::of(appearance_tokens(page, dark), dark)
    }

    /// The same theme in the other appearance: what something drawn inverted (the toast, dark
    /// on a light page and light on a dark one) takes its fill, text and marks from, so each
    /// keeps the contrast the theme holds it to on its own grounds.
    #[must_use]
    pub(crate) fn inverse(self, page: &ThemePage) -> Self {
        Self::of(appearance_tokens(page, !self.dark), !self.dark)
    }

    /// The palette of one appearance's `tokens`.
    fn of(tokens: &Tokens, dark: bool) -> Self {
        let c = |token: &str| color(hex(token));
        let hover = if dark { HOVER_DARK } else { HOVER_LIGHT };
        let mine = if dark {
            mix(hex(&tokens.accent), hex(&tokens.surface), 0.35)
        } else {
            hex(&tokens.accent)
        };
        let queued = mix(mine, hex(&tokens.surface), QUEUED);
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
            mine: color(mine),
            on_mine: if dark {
                c(&tokens.text)
            } else {
                c(&tokens.on_accent)
            },
            queued: color(queued),
            on_queued: color(crate::color::readable_on(queued)),
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
        color(crate::color::readable_on(rgb_of(fill)))
    }

    /// The tokens a palette made from `page` draws with in this palette's appearance.
    #[must_use]
    pub(crate) fn tokens_of(self, page: &ThemePage) -> &Tokens {
        appearance_tokens(page, self.dark)
    }
}

/// `page`'s tokens in the dark appearance or the light one.
fn appearance_tokens(page: &ThemePage, dark: bool) -> &Tokens {
    if dark {
        &page.tokens.dark
    } else {
        &page.tokens.light
    }
}

/// Whether `appearance` is a dark one.
#[must_use]
pub(crate) fn is_dark(appearance: WindowAppearance) -> bool {
    matches!(
        appearance,
        WindowAppearance::Dark | WindowAppearance::VibrantDark
    )
}

/// The palette now.
#[must_use]
pub(crate) fn pal(cx: &App) -> Pal {
    *cx.global::<Pal>()
}

/// Puts `page`'s palette over `material` in place for `window`, and into gpui-kit's theme;
/// returns it. A theme following the system takes the system's appearance for the app (which,
/// while the theme holds the app to Light or Dark, is that one). `ui::look::apply` is the one caller: it also puts the material on the window.
pub(crate) fn apply(
    page: &ThemePage,
    material: Material,
    window: &mut Window,
    cx: &mut App,
) -> Pal {
    // The app's appearance, not `window`'s own: a window held to an appearance of its own (the
    // Dark room's) would otherwise turn the whole app's palette with it, from its own appearance
    // observer or from whichever window applied a theme change last.
    let p = Pal::new(page, is_dark(cx.window_appearance())).with_material(material);
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
        kit_colors(&p, &mut t.colors);
    });
    p
}

/// The share a control's hover and press keep of its fill, the page showing through the rest.
const KIT_HOVER: f32 = 0.9;
const KIT_ACTIVE: f32 = 0.8;

/// Every one of gpui-kit's colours from the palette `p`. `Theme::change` loads gpui-kit's own
/// default theme first, a cool grey one, so a colour left out here would draw in it: the code
/// blocks, the default and ghost buttons' text, the danger buttons' text, tables, lists and
/// scrollbars all did.
pub(super) fn kit_colors(p: &Pal, t: &mut ThemeColor) {
    kit_grounds(p, t);
    kit_states(p, t);
    kit_buttons(p, t);
    kit_rows(p, t);
    kit_controls(p, t);
    kit_hues(p, t);
}

/// A control's fill under the pointer: the page showing through a little of it.
fn kit_hover(fill: Hsla) -> Hsla {
    fill.opacity(KIT_HOVER)
}

/// A control's fill held down.
fn kit_active(fill: Hsla) -> Hsla {
    fill.opacity(KIT_ACTIVE)
}

/// gpui-kit's grounds, text, lines, focus and links.
fn kit_grounds(p: &Pal, t: &mut ThemeColor) {
    t.background = p.surface;
    t.foreground = p.text;
    t.border = p.border;
    t.input = p.border;
    t.muted = p.surface_raised;
    t.muted_foreground = p.muted;
    t.accent = p.accent_soft;
    t.accent_foreground = p.text;
    // Menus and popovers stand out from the page: in light the surface, the lightest ground
    // (the raised one is a shade darker than the page, so a menu in it looked sunk); in dark
    // the raised one, a shade lighter.
    t.popover = if p.dark { p.surface_raised } else { p.surface };
    t.popover_foreground = p.text;
    // What a dialog dims the window with: the ink in light, the background in dark.
    t.overlay = if p.dark {
        p.background.opacity(0.6)
    } else {
        p.text.opacity(0.2)
    };
    t.window_border = p.border;
    t.ring = p.accent;
    t.caret = p.accent;
    t.selection = p.accent.opacity(0.3);
    t.drag_border = p.accent;
    t.drop_target = p.accent.opacity(0.12);
    t.link = p.accent;
    t.link_hover = kit_hover(p.accent);
    t.link_active = kit_active(p.accent);
}

/// gpui-kit's states as fills, each with the text that reads on it.
fn kit_states(p: &Pal, t: &mut ThemeColor) {
    let on = Pal::readable_on;
    t.primary = p.accent;
    t.primary_hover = kit_hover(p.accent);
    t.primary_active = kit_active(p.accent);
    t.primary_foreground = p.on_accent;
    t.secondary = p.surface_raised;
    t.secondary_hover = p.surface_raised.blend(p.hover);
    t.secondary_active = p.surface_raised.blend(p.pressed);
    t.secondary_foreground = p.text;
    t.danger = p.danger;
    t.danger_hover = kit_hover(p.danger);
    t.danger_active = kit_active(p.danger);
    t.danger_foreground = on(p.danger);
    t.warning = p.warn;
    t.warning_hover = kit_hover(p.warn);
    t.warning_active = kit_active(p.warn);
    t.warning_foreground = on(p.warn);
    t.success = p.success;
    t.success_hover = kit_hover(p.success);
    t.success_active = kit_active(p.success);
    t.success_foreground = on(p.success);
    t.info = p.info;
    t.info_hover = kit_hover(p.info);
    t.info_active = kit_active(p.info);
    t.info_foreground = on(p.info);
}

/// gpui-kit's buttons: the default one on the surface, the others their state's colours.
fn kit_buttons(p: &Pal, t: &mut ThemeColor) {
    t.button = p.surface;
    t.button_hover = p.surface.blend(p.hover);
    t.button_active = p.surface.blend(p.pressed);
    t.button_foreground = p.text;
    t.button_primary = t.primary;
    t.button_primary_hover = t.primary_hover;
    t.button_primary_active = t.primary_active;
    t.button_primary_foreground = t.primary_foreground;
    t.button_secondary = t.secondary;
    t.button_secondary_hover = t.secondary_hover;
    t.button_secondary_active = t.secondary_active;
    t.button_secondary_foreground = t.secondary_foreground;
    t.button_danger = t.danger;
    t.button_danger_hover = t.danger_hover;
    t.button_danger_active = t.danger_active;
    t.button_danger_foreground = t.danger_foreground;
    t.button_warning = t.warning;
    t.button_warning_hover = t.warning_hover;
    t.button_warning_active = t.warning_active;
    t.button_warning_foreground = t.warning_foreground;
    t.button_success = t.success;
    t.button_success_hover = t.success_hover;
    t.button_success_active = t.success_active;
    t.button_success_foreground = t.success_foreground;
    t.button_info = t.info;
    t.button_info_hover = t.info_hover;
    t.button_info_active = t.info_active;
    t.button_info_foreground = t.info_foreground;
}

/// gpui-kit's lists, tables and the like: the surface, their heads raised, a row tinted under
/// the pointer and the chosen one in the soft accent.
fn kit_rows(p: &Pal, t: &mut ThemeColor) {
    let hovered = p.surface.blend(p.hover);
    t.list = p.surface;
    t.list_even = p.surface;
    t.list_head = p.surface_raised;
    t.list_hover = hovered;
    t.list_active = p.accent_soft;
    t.list_active_border = p.accent;
    t.list_highlight_foreground = p.accent;
    t.table = p.surface;
    t.table_even = p.surface;
    t.table_head = p.surface_raised;
    t.table_head_foreground = p.text;
    t.table_foot = p.surface_raised;
    t.table_foot_foreground = p.muted;
    t.table_hover = hovered;
    t.table_active = p.accent_soft;
    t.table_active_border = p.accent;
    t.table_row_border = p.border;
    t.description_list_label = p.surface_raised;
    t.description_list_label_foreground = p.muted;
    t.group_box = p.surface_raised;
    t.group_box_foreground = p.text;
    t.accordion = p.surface;
}

/// gpui-kit's controls, and the window's own bars, sidebar and tabs.
fn kit_controls(p: &Pal, t: &mut ThemeColor) {
    t.scrollbar = transparent_black();
    t.scrollbar_thumb = p.faint.opacity(0.45);
    t.scrollbar_thumb_hover = p.faint.opacity(0.7);
    t.switch = p.faint.opacity(0.5);
    t.switch_thumb = if p.dark { p.text } else { p.surface };
    t.slider_bar = p.accent;
    t.slider_thumb = p.surface;
    t.progress_bar = p.accent;
    t.skeleton = p.surface_raised;
    t.title_bar = p.background;
    t.title_bar_border = p.border;
    t.status_bar = p.background;
    t.status_bar_border = p.border;
    t.sidebar = p.background;
    t.sidebar_foreground = p.text;
    t.sidebar_border = p.border;
    t.sidebar_accent = p.accent_soft;
    t.sidebar_accent_foreground = p.text;
    t.sidebar_primary = p.accent;
    t.sidebar_primary_foreground = p.on_accent;
    t.tab = p.surface;
    t.tab_foreground = p.muted;
    t.tab_active = p.surface_raised;
    t.tab_active_foreground = p.text;
    t.tab_bar = p.background;
    t.tab_bar_segmented = p.surface_raised;
}

/// gpui-kit's chart colours and the named hues a component may ask for: the theme's accent
/// and status colours, each with a light wash of it.
fn kit_hues(p: &Pal, t: &mut ThemeColor) {
    let wash = |colour: Hsla| colour.opacity(0.15);
    t.chart_1 = p.accent;
    t.chart_2 = p.info;
    t.chart_3 = p.success;
    t.chart_4 = p.warn;
    t.chart_5 = p.danger;
    t.chart_bullish = p.success;
    t.chart_bearish = p.danger;
    t.chart_grid = p.border;
    t.red = p.danger;
    t.red_light = wash(p.danger);
    t.green = p.success;
    t.green_light = wash(p.success);
    t.blue = p.info;
    t.blue_light = wash(p.info);
    t.yellow = p.warn;
    t.yellow_light = wash(p.warn);
    t.magenta = p.accent;
    t.magenta_light = wash(p.accent);
    t.cyan = p.info;
    t.cyan_light = wash(p.info);
}

/// A room's dot: its goal's colour by rank, as on the other apps (categorical, not the
/// theme's), in the appearance on screen; a room without a goal takes the faint grey.
#[must_use]
pub(crate) fn room_color(goal_rank: Option<i32>, p: &Pal) -> Hsla {
    match goal_rank
        .and_then(|rank| usize::try_from(rank).ok())
        .filter(|&rank| rank >= 1)
    {
        Some(rank) => {
            let goals = if p.dark { &GOALS_DARK } else { &GOALS_LIGHT };
            color(goals[(rank - 1) % goals.len()])
        }
        None => p.faint,
    }
}

/// The goals' colours in light, by rank: orange, green, purple, cyan, pink, yellow and mint,
/// each deep enough to read 3:1 on any light ground (as a dot, an icon, a progress ring).
/// The iOS colours they stand for are too light for it: the yellow reads 1.4:1 on paper.
const GOALS_LIGHT: [Rgb; 7] = [
    0xB4_5A_00, 0x21_80_3A, 0x8E_3F_BF, 0x1F_72_A8, 0xC8_20_3F, 0x8A_6A_00, 0x00_83_7C,
];

/// The same in dark: iOS's own dark-mode colours, light enough on any dark ground.
const GOALS_DARK: [Rgb; 7] = [
    0xFF_9F_0A, 0x30_D1_58, 0xBF_5A_F2, 0x64_D2_FF, 0xFF_37_5F, 0xFF_D6_0A, 0x63_E6_E2,
];

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

/// One appearance's ground and text for a page drawn outside GPUI, each `#RRGGBB`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct WebColours {
    /// The page's surface.
    pub(crate) surface: String,
    /// The body text.
    pub(crate) text: String,
}

/// The page's surface and text in light, then in dark (a token that isn't a colour is grey):
/// what a report's web view shows where the report has no background or text colour of its
/// own.
#[must_use]
pub(crate) fn web_colours(page: &ThemePage) -> [WebColours; 2] {
    [&page.tokens.light, &page.tokens.dark].map(|tokens| WebColours {
        surface: format!("#{:06X}", hex(&tokens.surface)),
        text: format!("#{:06X}", hex(&tokens.text)),
    })
}

/// A calendar tone's colours, for the appearance on screen.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct CalendarColors {
    /// Its colour: a chip's fill, a block's edge, a dot.
    pub(crate) fill: Hsla,
    /// Text and icons on the fill.
    pub(crate) on_fill: Hsla,
    /// A light wash of it over the page: a timed block, today's plan.
    pub(crate) tint: Hsla,
    /// Text and icons on the wash.
    pub(crate) on_tint: Hsla,
}

/// How much of a tone's colour its wash takes, the page showing through the rest.
const CALENDAR_TINT: f32 = 0.22;

/// The share of the body text mixed into a tone's colour for text on its wash, in light: the
/// colours themselves are made for white text on them, a shade too light to read on their
/// own pale wash.
const CALENDAR_INK_LIGHT: f32 = 0.45;

/// A calendar tone's colours: blue for meetings, teal for events, green for focus, orange and
/// red for deadlines, the accent for an ask's day and indigo for today's plan. In light they
/// are deep enough for white text, in dark light enough for near-black; each reads 4.5:1 as
/// text on its own wash too.
#[must_use]
pub(crate) fn calendar(tone: Tone, p: &Pal) -> CalendarColors {
    // Each tone's colour in light, then in dark.
    let (light, dark): (Rgb, Rgb) = match tone {
        Tone::Meeting => (0x1F_62_C4, 0x6C_AE_FF),
        Tone::Event => (0x0B_6E_7A, 0x4F_C6_D3),
        Tone::Focus => (0x1C_7A_3E, 0x5F_D3_8E),
        Tone::Deadline => (0xA9_50_08, 0xFF_AB_5C),
        Tone::Hard => (0xC4_26_2E, 0xFF_7F_78),
        Tone::Today => (0x4F_46_E5, 0xA9_A4_FF),
        Tone::Decide => {
            return CalendarColors {
                fill: p.accent,
                on_fill: p.on_accent,
                tint: p.accent.opacity(CALENDAR_TINT),
                on_tint: p.text,
            };
        }
    };
    let fill = color(if p.dark { dark } else { light });
    CalendarColors {
        fill,
        on_fill: Pal::readable_on(fill),
        tint: fill.opacity(CALENDAR_TINT),
        on_tint: if p.dark {
            fill
        } else {
            fill.blend(p.text.opacity(CALENDAR_INK_LIGHT))
        },
    }
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

/// The WCAG 2 contrast ratio of two colours, each drawn opaque: for the tests of what views
/// draw.
#[cfg(test)]
#[must_use]
pub(crate) fn contrast(a: Hsla, b: Hsla) -> f64 {
    crate::color::contrast_ratio(rgb_of(a), rgb_of(b))
}

/// A colour as `0xRRGGBB`, its alpha left out.
fn rgb_of(colour: Hsla) -> Rgb {
    let colour = colour.to_rgb();
    let channel = |value: f32| u32::from(crate::color::to_byte(value * 255.0));
    (channel(colour.r) << 16) | (channel(colour.g) << 8) | channel(colour.b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::{contrast_ratio, delta_e_2000, lab, luminance};
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

    /// Every preset's theme as the server makes it, with its palette in each appearance.
    fn every_palette() -> Vec<(&'static str, ThemePage, Pal)> {
        emberhut_core::theme::PRESETS
            .iter()
            .flat_map(|preset| {
                let page = preset_page(preset.id);
                both(&page).map(|p| (preset.id, page.clone(), p))
            })
            .collect()
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
        // The values the README and the floors' docs quote for the graphite presets: light's
        // faint text is a hair above AA, so its sidebar stays opaque.
        assert!(
            (light.sidebar_alpha - 1.0).abs() < f32::EPSILON,
            "{}",
            light.sidebar_alpha
        );
        assert_eq!(light.material, Material::Opaque);
        assert_eq!(light.sidebar, light.background);
        assert!(
            (dark.sidebar.a - 0.94).abs() < f32::EPSILON,
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
                // The values the README quotes: near-AA inks (paper's ember and green, every
                // preset's faint) leave a light sidebar no room at all and a dark one 0.94.
                let (low, high) = if p.dark { (0.94, 0.94) } else { (1.0, 1.0) };
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
        page.tokens.light.faint = "#4B5563".into();
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

    #[test]
    fn the_inverse_is_the_other_appearance_of_the_same_theme() {
        for (preset, page, p) in every_palette() {
            let inverse = p.inverse(&page);
            assert_eq!(inverse.dark, !p.dark, "{preset}");
            assert_eq!(
                inverse.surface,
                Pal::new(&page, !p.dark).surface,
                "{preset}"
            );
            assert_eq!(inverse.inverse(&page).text, p.text, "{preset}");
        }
    }

    #[test]
    fn every_presets_toast_inks_read_on_its_inverted_fill() {
        for (preset, page, p) in every_palette() {
            let toast = p.inverse(&page);
            let inks = [
                ("text", toast.text),
                ("muted", toast.muted),
                ("warn", toast.warn),
                ("success", toast.success),
                ("accent", toast.accent),
            ];
            for (name, ink) in inks {
                let r = contrast(ink, toast.surface);
                assert!(r >= AA, "{preset} dark={} {name}: {r:.2}", p.dark);
            }
        }
    }

    #[test]
    fn a_waiting_message_reads_on_its_paler_bubble() {
        for (preset, _, p) in every_palette() {
            assert!((p.queued.a - 1.0).abs() < f32::EPSILON, "{preset}: opaque");
            assert_ne!(p.queued, p.mine, "{preset}: told apart from a sent one");
            let r = contrast(p.on_queued, p.queued);
            assert!(r >= AA, "{preset} dark={}: {r:.2}", p.dark);
        }
    }

    #[test]
    fn every_calendar_chip_reads_in_every_preset() {
        for (preset, _, p) in every_palette() {
            for tone in Tone::LEGEND {
                let c = calendar(tone, &p);
                let on_fill = contrast(c.on_fill, c.fill);
                assert!(
                    on_fill >= AA,
                    "{preset} dark={} {tone:?}: {on_fill:.2}",
                    p.dark
                );
                let wash = p.surface.blend(c.tint);
                let on_tint = contrast(c.on_tint, wash);
                assert!(
                    on_tint >= AA,
                    "{preset} dark={} {tone:?} wash: {on_tint:.2}",
                    p.dark
                );
            }
            // Now and today: the danger colour, its own text on it.
            let today = contrast(Pal::readable_on(p.danger), p.danger);
            assert!(today >= AA, "{preset} dark={}: {today:.2}", p.dark);
        }
    }

    /// gpui-kit's colours with every field set to a colour no palette makes.
    fn blank_kit_colors() -> (ThemeColor, serde_json::Value) {
        let unset = serde_json::Value::from("#12345678");
        let mut fields = serde_json::to_value(ThemeColor::default()).unwrap();
        for value in fields.as_object_mut().unwrap().values_mut() {
            *value = unset.clone();
        }
        (serde_json::from_value(fields).unwrap(), unset)
    }

    #[test]
    fn every_gpui_kit_colour_comes_from_the_palette() {
        for (preset, _, p) in every_palette() {
            let (mut colors, unset) = blank_kit_colors();
            kit_colors(&p, &mut colors);
            let fields = serde_json::to_value(colors).unwrap();
            for (name, value) in fields.as_object().unwrap() {
                assert_ne!(
                    value, &unset,
                    "{preset} dark={}: {name} is left as it was",
                    p.dark
                );
            }
        }
    }

    #[test]
    fn gpui_kit_text_reads_on_its_fills() {
        for (preset, _, p) in every_palette() {
            let (mut t, _) = blank_kit_colors();
            kit_colors(&p, &mut t);
            let pairs = [
                ("foreground", t.foreground, t.background),
                ("muted", t.muted_foreground, t.background),
                ("code", t.foreground, t.muted),
                ("popover", t.popover_foreground, t.popover),
                ("button", t.button_foreground, t.button),
                ("ghost", t.secondary_foreground, t.background),
                ("primary", t.primary_foreground, t.primary),
                ("danger", t.danger_foreground, t.danger),
                ("warning", t.warning_foreground, t.warning),
                ("success", t.success_foreground, t.success),
                ("info", t.info_foreground, t.info),
                ("table head", t.table_head_foreground, t.table_head),
                ("link", t.link, t.background),
            ];
            for (name, ink, fill) in pairs {
                let r = contrast(ink, fill);
                assert!(r >= AA, "{preset} dark={} {name}: {r:.2}", p.dark);
            }
        }
    }

    #[test]
    fn menus_stand_out_from_the_page_in_either_appearance() {
        for (preset, _, p) in every_palette() {
            let (mut t, _) = blank_kit_colors();
            kit_colors(&p, &mut t);
            let popover = luminance(rgb_of(t.popover));
            let dark = p.dark;
            assert!(
                popover > luminance(rgb_of(p.background)),
                "{preset} dark={dark}"
            );
            // In light, not sunk below what is raised either.
            assert!(
                dark || popover >= luminance(rgb_of(p.surface_raised)),
                "{preset} dark={dark}"
            );
        }
    }

    #[test]
    fn every_goals_colour_reads_on_every_presets_grounds() {
        for (preset, _, p) in every_palette() {
            for rank in 1..=7 {
                let goal = room_color(Some(rank), &p);
                for (ground_name, ground) in [
                    ("background", p.background),
                    ("surface", p.surface),
                    ("raised", p.surface_raised),
                ] {
                    let r = contrast(goal, ground);
                    assert!(
                        r >= 3.0,
                        "{preset} dark={} goal {rank} on {ground_name}: {r:.2}",
                        p.dark
                    );
                }
            }
            assert_eq!(
                room_color(Some(8), &p),
                room_color(Some(1), &p),
                "round the seven"
            );
            assert_eq!(room_color(None, &p), p.faint);
        }
    }

    /// How far apart (CIEDE2000) the accent, warn and danger must look. About 1 is the least
    /// an eye sees between two large patches side by side; these are small marks (an icon, a
    /// word) met one at a time, and "warn" must not read as "danger" without the other beside
    /// it to compare. 10 is the top of the band usually called perceptible at a glance, past
    /// which two colours read as different ones.
    const DISTINCT: f64 = 10.0;

    /// What falls short in `preset`'s palettes, in either appearance: a text level (body,
    /// muted, faint) under 4.5:1 on a ground, or two of the accent, warn and danger closer
    /// than [`DISTINCT`].
    fn shortfalls(preset: &str) -> Vec<String> {
        let mut short = Vec::new();
        for p in both(&preset_page(preset)) {
            let dark = p.dark;
            let grounds = [
                ("background", p.background),
                ("surface", p.surface),
                ("raised", p.surface_raised),
            ];
            for (ink_name, ink) in [("text", p.text), ("muted", p.muted), ("faint", p.faint)] {
                for (ground_name, ground) in grounds {
                    let r = contrast(ink, ground);
                    if r < AA {
                        short.push(format!(
                            "{preset} dark={dark}: {ink_name} on {ground_name} {r:.2}"
                        ));
                    }
                }
            }
            let marks = [("accent", p.accent), ("warn", p.warn), ("danger", p.danger)];
            for (i, (a_name, a)) in marks.iter().enumerate() {
                for (b_name, b) in &marks[i + 1..] {
                    let apart = delta_e_2000(lab(rgb_of(*a)), lab(rgb_of(*b)));
                    if apart < DISTINCT {
                        short.push(format!(
                            "{preset} dark={dark}: {a_name} and {b_name} {apart:.1} apart"
                        ));
                    }
                }
            }
        }
        short
    }

    #[test]
    fn every_presets_text_reads_and_its_status_colours_differ() {
        let short: Vec<String> = emberhut_core::theme::PRESETS
            .iter()
            .flat_map(|preset| shortfalls(preset.id))
            .collect();
        assert!(short.is_empty(), "{short:#?}");
    }
}
