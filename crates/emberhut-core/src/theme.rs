//! The theme: a preset (an accent family on its neutrals), an appearance and an optional custom
//! accent, turned into the colour tokens every app draws with. Most presets share the graphite
//! neutrals; paper, the default, has its own: warm kraft grounds and brown-black ink with an
//! ember accent. An accent is held to WCAG AA (4.5:1) as text on the background and the surface
//! and under its own text: one too light for light mode is darkened, one too dark for dark mode
//! lightened, a step at a time, only as far as it needs.

use std::fmt;

/// The presets' version: a client keeps its cache per version.
pub const VERSION: u32 = 1;

/// The preset a choice starts from.
pub const DEFAULT_PRESET: &str = "paper";

/// The token names, in the order the API lists them.
pub const NAMES: [&str; 14] = [
    "accent",
    "accentSoft",
    "onAccent",
    "background",
    "surface",
    "surfaceRaised",
    "border",
    "text",
    "muted",
    "faint",
    "success",
    "warn",
    "danger",
    "info",
];

/// The contrast every accent is held to.
const AA: f64 = 4.5;
/// A preset: its accent in light and dark mode, both readable on its neutrals.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Preset {
    /// Its id.
    pub id: &'static str,
    /// Its name, as shown.
    pub name: &'static str,
    /// The accent in light mode.
    pub light: &'static str,
    /// The accent in dark mode.
    pub dark: &'static str,
    /// "Running" is blue (cyan when the accent is blue itself).
    blue_info: bool,
    /// Its neutrals in light and dark mode.
    neutrals: (&'static Neutrals, &'static Neutrals),
}

const fn preset(
    id: &'static str,
    name: &'static str,
    light: &'static str,
    dark: &'static str,
) -> Preset {
    Preset {
        id,
        name,
        light,
        dark,
        blue_info: true,
        neutrals: (&LIGHT, &DARK),
    }
}

/// Every preset, in the order the API lists them.
pub const PRESETS: [Preset; 8] = [
    Preset {
        neutrals: (&PAPER_LIGHT, &PAPER_DARK),
        ..preset("paper", "Paper", "#B83D0C", "#FF9A5A")
    },
    preset("graphite", "Graphite", "#374151", "#D1D5DB"),
    Preset {
        blue_info: false,
        ..preset("blue", "Blue", "#2563EB", "#60A5FA")
    },
    preset("teal", "Teal", "#0F766E", "#2DD4BF"),
    preset("green", "Green", "#15803D", "#4ADE80"),
    preset("orange", "Orange", "#C2410C", "#FB923C"),
    preset("rose", "Rose", "#BE123C", "#FB7185"),
    preset("purple", "Purple", "#7C3AED", "#A78BFA"),
];

/// Light, dark, or as the system is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Appearance {
    /// Follow the system.
    #[default]
    System,
    /// Always light.
    Light,
    /// Always dark.
    Dark,
}

impl Appearance {
    /// The word for it.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }
}

/// Why a theme change was refused; its text is what the app shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ThemeRefusal {
    /// The change is not an object.
    #[error("theme: {{ preset?, appearance?, accent? }}")]
    NotAChange,
    /// Not one of the presets.
    #[error("preset: one of {}", PresetIds)]
    Preset,
    /// Not an appearance.
    #[error("appearance: system, light or dark")]
    Appearance,
    /// Not a colour.
    #[error("accent: a hex colour like #3366FF (or null for the preset's own)")]
    Accent,
}

/// The presets' ids, comma separated.
struct PresetIds;

impl fmt::Display for PresetIds {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (n, preset) in PRESETS.iter().enumerate() {
            if n > 0 {
                f.write_str(", ")?;
            }
            f.write_str(preset.id)?;
        }
        Ok(())
    }
}

/// The preset `asked` names (trimmed, in any case).
///
/// # Errors
///
/// [`ThemeRefusal::Preset`].
pub fn preset_id(asked: &str) -> Result<&'static str, ThemeRefusal> {
    let asked = asked.trim().to_lowercase();
    PRESETS
        .iter()
        .find(|preset| preset.id == asked)
        .map(|preset| preset.id)
        .ok_or(ThemeRefusal::Preset)
}

/// The appearance `asked` names (trimmed, in any case).
///
/// # Errors
///
/// [`ThemeRefusal::Appearance`].
pub fn appearance(asked: &str) -> Result<Appearance, ThemeRefusal> {
    match asked.trim().to_lowercase().as_str() {
        "system" => Ok(Appearance::System),
        "light" => Ok(Appearance::Light),
        "dark" => Ok(Appearance::Dark),
        _ => Err(ThemeRefusal::Appearance),
    }
}

/// The custom accent `asked` for: none (or blank) for the preset's own, else a colour.
///
/// # Errors
///
/// [`ThemeRefusal::Accent`] for anything but a hex colour.
pub fn accent(asked: Option<&str>) -> Result<Option<String>, ThemeRefusal> {
    match asked {
        None => Ok(None),
        Some(text) if text.trim().is_empty() => Ok(None),
        Some(text) => hex(text).map(Some).ok_or(ThemeRefusal::Accent),
    }
}

/// One appearance's colours, each `#RRGGBB`.
#[derive(Clone, Debug, PartialEq, Eq)]
#[expect(
    missing_docs,
    reason = "each field is the token of its name (see NAMES)"
)]
pub struct Palette {
    pub accent: String,
    pub accent_soft: String,
    pub on_accent: String,
    pub background: String,
    pub surface: String,
    pub surface_raised: String,
    pub border: String,
    pub text: String,
    pub muted: String,
    pub faint: String,
    pub success: String,
    pub warn: String,
    pub danger: String,
    pub info: String,
}

/// How readable one appearance's accent is (WCAG 2 ratios, rounded down to hundredths).
#[derive(Clone, Debug, PartialEq)]
pub struct Contrast {
    /// The accent as text on the background.
    pub accent_on_background: f64,
    /// The accent as text on the surface.
    pub accent_on_surface: f64,
    /// The text on the accent.
    pub on_accent: f64,
    /// The asked accent was moved to reach AA.
    pub adjusted: bool,
    /// The accent asked for.
    pub asked: String,
}

impl Contrast {
    /// Every ratio is AA (4.5:1) or more.
    #[must_use]
    pub fn is_aa(&self) -> bool {
        self.accent_on_background >= AA && self.accent_on_surface >= AA && self.on_accent >= AA
    }
}

/// The neutrals of one appearance: the grounds, the text levels, the status colours, and the
/// two colours text on an accent is drawn in.
#[derive(Debug, PartialEq, Eq)]
struct Neutrals {
    background: &'static str,
    surface: &'static str,
    surface_raised: &'static str,
    border: &'static str,
    text: &'static str,
    muted: &'static str,
    faint: &'static str,
    success: &'static str,
    warn: &'static str,
    danger: &'static str,
    blue: &'static str,
    cyan: &'static str,
    /// Text on an accent: whichever of these reads better.
    on_light: &'static str,
    on_dark: &'static str,
}

const LIGHT: Neutrals = Neutrals {
    background: "#F5F6F8",
    surface: "#FFFFFF",
    surface_raised: "#ECEEF1",
    border: "#D9DCE1",
    text: "#14171C",
    muted: "#5B6270",
    faint: "#8A909B",
    success: "#15803D",
    warn: "#C2410C",
    danger: "#DC2626",
    blue: "#2563EB",
    cyan: "#0E7490",
    on_light: "#FFFFFF",
    on_dark: "#0B0D10",
};

const DARK: Neutrals = Neutrals {
    background: "#0E1013",
    surface: "#17191D",
    surface_raised: "#202328",
    border: "#2C3036",
    text: "#E9EBEE",
    muted: "#9BA1AB",
    faint: "#6B717B",
    success: "#4ADE80",
    warn: "#FB923C",
    danger: "#F87171",
    blue: "#60A5FA",
    cyan: "#22D3EE",
    on_light: "#FFFFFF",
    on_dark: "#0B0D10",
};

/// Paper in light mode: kraft paper for the window, cream cards, paper a shade deeper for what
/// is raised, the ink's own line at 12% on the paper, brown-black ink. Every text level and
/// status colour reads 4.5:1 on all three grounds (the green is a step deeper than the brand's
/// for it).
const PAPER_LIGHT: Neutrals = Neutrals {
    background: "#F6F0E6",
    surface: "#FFFDF9",
    surface_raised: "#EFE6D8",
    border: "#DCD6CD",
    text: "#1D1712",
    muted: "#65574C",
    faint: "#6E6055",
    success: "#2B7248",
    warn: "#8A5800",
    danger: "#A51D2D",
    blue: "#2B5F8E",
    cyan: "#1F6F78",
    on_light: "#FFF8EF",
    on_dark: "#1D1206",
};

/// Paper in dark mode: dark paper, its cards a shade lighter, a warm line at 10%, cream ink.
const PAPER_DARK: Neutrals = Neutrals {
    background: "#15110E",
    surface: "#211B16",
    surface_raised: "#261F19",
    border: "#2C2723",
    text: "#F6EEE5",
    muted: "#BDAE9F",
    faint: "#A39486",
    success: "#7FD19B",
    warn: "#E8B04A",
    danger: "#FF8089",
    blue: "#8DB8E0",
    cyan: "#6FCFD6",
    on_light: "#FFF8EF",
    on_dark: "#1D1206",
};

/// The colours of `preset` (the default preset when it is not one), with the `custom` accent
/// in place of its own, in dark mode or light, and how readable the accent is.
#[must_use]
pub fn resolve(preset: &str, custom: Option<&str>, dark: bool) -> (Palette, Contrast) {
    let preset = PRESETS
        .iter()
        .find(|p| p.id == preset)
        .or_else(|| PRESETS.iter().find(|p| p.id == DEFAULT_PRESET))
        .copied()
        .unwrap_or(PRESETS[0]);
    let n = if dark {
        preset.neutrals.1
    } else {
        preset.neutrals.0
    };
    let wanted = custom
        .and_then(hex)
        .unwrap_or_else(|| (if dark { preset.dark } else { preset.light }).to_owned());
    let mut accent = wanted.clone();
    for _ in 0..100 {
        if readable(&accent, n) {
            break;
        }
        accent = shift(&accent, if dark { 0.01 } else { -0.01 });
    }
    let on = on_colour(&accent, n);
    let blue_info = custom.map_or(preset.blue_info, |custom| !is_blue(custom));
    let contrast = Contrast {
        accent_on_background: round_down(ratio(&accent, n.background)),
        accent_on_surface: round_down(ratio(&accent, n.surface)),
        on_accent: round_down(ratio(on, &accent)),
        adjusted: accent != wanted,
        asked: wanted,
    };
    let palette = Palette {
        accent_soft: mix(&accent, n.surface, if dark { 0.22 } else { 0.12 }),
        on_accent: on.to_owned(),
        accent,
        background: n.background.to_owned(),
        surface: n.surface.to_owned(),
        surface_raised: n.surface_raised.to_owned(),
        border: n.border.to_owned(),
        text: n.text.to_owned(),
        muted: n.muted.to_owned(),
        faint: n.faint.to_owned(),
        success: n.success.to_owned(),
        warn: n.warn.to_owned(),
        danger: n.danger.to_owned(),
        info: (if blue_info { n.blue } else { n.cyan }).to_owned(),
    };
    (palette, contrast)
}

fn readable(accent: &str, n: &Neutrals) -> bool {
    ratio(accent, n.background) >= AA
        && ratio(accent, n.surface) >= AA
        && ratio(on_colour(accent, n), accent) >= AA
}

fn on_colour(accent: &str, n: &Neutrals) -> &'static str {
    if ratio(n.on_light, accent) >= ratio(n.on_dark, accent) {
        n.on_light
    } else {
        n.on_dark
    }
}

/// Rounded down, so a ratio never reads as passing a line it misses.
fn round_down(x: f64) -> f64 {
    (x * 100.0).floor() / 100.0
}

/// `#36f`, `3366FF`, ` #3366ff ` as `#3366FF`; anything else none.
#[must_use]
pub fn hex(text: &str) -> Option<String> {
    let digits = text.trim().trim_start_matches('#');
    let digits: String = if digits.len() == 3 {
        digits.chars().flat_map(|c| [c, c]).collect()
    } else {
        digits.to_owned()
    };
    (digits.len() == 6 && digits.bytes().all(|b| b.is_ascii_hexdigit()))
        .then(|| format!("#{}", digits.to_ascii_uppercase()))
}

/// The red, green and blue of a `#RRGGBB` colour, 0 to 1 (black for anything else).
fn rgb(hex: &str) -> (f64, f64, f64) {
    let digits = hex.trim_start_matches('#');
    let channel = |at: usize| {
        digits
            .get(at..at + 2)
            .and_then(|pair| u8::from_str_radix(pair, 16).ok())
            .map_or(0.0, |value| f64::from(value) / 255.0)
    };
    (channel(0), channel(2), channel(4))
}

fn to_hex(r: f64, g: f64, b: f64) -> String {
    let byte = |c: f64| {
        // Halves to even.
        let scaled = (c.clamp(0.0, 1.0) * 255.0).round_ties_even();
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "clamped to 0..=255 first"
        )]
        let byte = scaled as u8;
        byte
    };
    format!("#{:02X}{:02X}{:02X}", byte(r), byte(g), byte(b))
}

/// A colour's relative luminance (WCAG 2), 0 to 1.
#[must_use]
pub fn luminance(hex: &str) -> f64 {
    let linear = |c: f64| {
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    let (r, g, b) = rgb(hex);
    0.2126 * linear(r) + 0.7152 * linear(g) + 0.0722 * linear(b)
}

/// The contrast ratio of two colours (WCAG 2), 1 to 21.
#[must_use]
pub fn ratio(a: &str, b: &str) -> f64 {
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

fn mix(a: &str, b: &str, share: f64) -> String {
    let (ar, ag, ab) = rgb(a);
    let (br, bg, bb) = rgb(b);
    to_hex(
        ar * share + br * (1.0 - share),
        ag * share + bg * (1.0 - share),
        ab * share + bb * (1.0 - share),
    )
}

#[expect(clippy::float_cmp, reason = "the channel that is the maximum, exactly")]
#[expect(
    clippy::many_single_char_names,
    reason = "the formula's own names: red, green, blue, hue, saturation, lightness"
)]
fn hsl(hex: &str) -> (f64, f64, f64) {
    let (r, g, b) = rgb(hex);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = f64::midpoint(max, min);
    let d = max - min;
    if d == 0.0 {
        return (0.0, 0.0, l);
    }
    let s = if l > 0.5 {
        d / (2.0 - max - min)
    } else {
        d / (max + min)
    };
    let h = if max == r {
        (g - b) / d + if g < b { 6.0 } else { 0.0 }
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    (h * 60.0, s, l)
}

#[expect(
    clippy::many_single_char_names,
    reason = "the formula's own names: hue, saturation, lightness and its two helpers"
)]
fn from_hsl(h: f64, s: f64, l: f64) -> String {
    fn channel(p: f64, q: f64, mut t: f64) -> f64 {
        if t < 0.0 {
            t += 1.0;
        }
        if t > 1.0 {
            t -= 1.0;
        }
        if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 0.5 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        }
    }
    if s == 0.0 {
        return to_hex(l, l, l);
    }
    let q = if l < 0.5 {
        l * (1.0 + s)
    } else {
        l + s - l * s
    };
    let p = 2.0 * l - q;
    let h = h / 360.0;
    to_hex(
        channel(p, q, h + 1.0 / 3.0),
        channel(p, q, h),
        channel(p, q, h - 1.0 / 3.0),
    )
}

fn shift(hex: &str, by: f64) -> String {
    let (h, s, l) = hsl(hex);
    from_hsl(h, s, (l + by).clamp(0.0, 1.0))
}

fn is_blue(hex: &str) -> bool {
    let (h, s, _) = hsl(hex);
    s > 0.2 && (190.0..=250.0).contains(&h)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_colours_are_written_in_full_and_upper_case() {
        assert_eq!(hex("#36f").as_deref(), Some("#3366FF"));
        assert_eq!(hex(" 3366ff ").as_deref(), Some("#3366FF"));
        assert_eq!(hex("#12345"), None);
        assert_eq!(hex("blue"), None);
    }

    #[test]
    fn the_blue_preset_resolves_to_its_colours() {
        let (light, contrast) = resolve("blue", None, false);
        assert_eq!(light.accent, "#2563EB");
        assert_eq!(light.accent_soft, "#E5ECFD");
        assert_eq!(light.on_accent, "#FFFFFF");
        assert_eq!(light.info, "#0E7490", "cyan: the accent is blue itself");
        assert!((contrast.accent_on_background - 4.77).abs() < 1e-9);
        assert!(contrast.is_aa());
        let (dark, contrast) = resolve("blue", None, true);
        assert_eq!(dark.accent_soft, "#27384E");
        assert_eq!(dark.on_accent, "#0B0D10");
        assert!((contrast.on_accent - 7.65).abs() < 1e-9);
        assert!(!contrast.adjusted);
    }

    #[test]
    fn the_paper_preset_is_the_default_and_resolves_to_its_colours() {
        assert_eq!(DEFAULT_PRESET, "paper");
        assert_eq!(PRESETS[0].id, DEFAULT_PRESET);
        let (light, contrast) = resolve("paper", None, false);
        assert_eq!(light.accent, "#B83D0C");
        assert_eq!(light.on_accent, "#FFF8EF");
        assert_eq!(light.background, "#F6F0E6");
        assert_eq!(light.surface, "#FFFDF9");
        assert_eq!(light.text, "#1D1712");
        assert_eq!(light.info, "#2B5F8E");
        assert!(contrast.is_aa());
        assert!(!contrast.adjusted);
        let (dark, contrast) = resolve("paper", None, true);
        assert_eq!(dark.accent, "#FF9A5A");
        assert_eq!(dark.on_accent, "#1D1206");
        assert_eq!(dark.background, "#15110E");
        assert_eq!(dark.text, "#F6EEE5");
        assert!(contrast.is_aa());
        assert!(!contrast.adjusted);
    }

    #[test]
    fn every_paper_text_level_reads_aa_on_every_ground() {
        for dark in [false, true] {
            for custom in [None, Some("#2563EB")] {
                let (p, _) = resolve("paper", custom, dark);
                let grounds = [
                    ("background", &p.background),
                    ("surface", &p.surface),
                    ("surfaceRaised", &p.surface_raised),
                ];
                let inks = [
                    ("text", &p.text),
                    ("muted", &p.muted),
                    ("faint", &p.faint),
                    ("success", &p.success),
                    ("warn", &p.warn),
                    ("danger", &p.danger),
                    ("info", &p.info),
                ];
                for (ground_name, ground) in grounds {
                    for (ink_name, ink) in inks {
                        let r = ratio(ink, ground);
                        assert!(
                            r >= AA,
                            "dark={dark} {custom:?} {ink_name} {ink} on {ground_name} {ground}: \
                             {r:.2}"
                        );
                    }
                }
                assert!(ratio(&p.on_accent, &p.accent) >= AA, "dark={dark}");
                assert!(ratio(&p.text, &p.accent_soft) >= AA, "dark={dark}");
            }
        }
    }

    #[test]
    fn a_blue_accent_on_paper_runs_in_cyan() {
        let (light, _) = resolve("paper", Some("#2563EB"), false);
        assert_eq!(light.info, PAPER_LIGHT.cyan);
        let (light, _) = resolve("paper", Some("#BE123C"), false);
        assert_eq!(light.info, PAPER_LIGHT.blue);
    }

    #[test]
    fn an_unreadable_accent_is_moved_until_it_reads() {
        let (light, contrast) = resolve("blue", Some("#FFFF00"), false);
        assert!(contrast.adjusted);
        assert_eq!(contrast.asked, "#FFFF00");
        assert_ne!(light.accent, "#FFFF00");
        assert!(ratio(&light.accent, LIGHT.background) >= AA);
        assert_eq!(light.info, "#2563EB", "yellow is not blue: running is blue");
    }

    #[test]
    fn an_unknown_preset_is_the_default() {
        assert_eq!(
            resolve("nowhere", None, false),
            resolve("paper", None, false)
        );
    }

    #[test]
    fn changes_are_read_leniently() {
        assert_eq!(preset_id(" Teal "), Ok("teal"));
        assert_eq!(
            preset_id("pink").unwrap_err().to_string(),
            "preset: one of paper, graphite, blue, teal, green, orange, rose, purple"
        );
        assert_eq!(appearance("DARK"), Ok(Appearance::Dark));
        assert_eq!(appearance("dim"), Err(ThemeRefusal::Appearance));
        assert_eq!(accent(Some(" ")), Ok(None));
        assert_eq!(accent(Some("#abc")), Ok(Some("#AABBCC".to_owned())));
        assert_eq!(accent(Some("red")), Err(ThemeRefusal::Accent));
        assert_eq!(
            ThemeRefusal::NotAChange.to_string(),
            "theme: { preset?, appearance?, accent? }"
        );
    }
}
