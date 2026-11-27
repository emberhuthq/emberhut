//! The theme: one choice per person, the same on every device. `GET /api/theme` gives the choice
//! with its colour tokens in both appearances and every preset's; `POST /api/theme` changes it.
//! The state carries the choice alone ([`ThemeChoice`]): a newer `at` than a device's own means
//! another device changed it, so fetch the page.

use serde::{Deserialize, Serialize};

use crate::open_enum::open_enum;
use crate::time::Timestamp;

open_enum! {
    /// Light, dark, or as the system is.
    #[derive(Default)]
    pub enum Appearance {
        /// Follow the system: the server's default.
        #[default]
        System = "system",
        /// Always light.
        Light = "light",
        /// Always dark.
        Dark = "dark",
    }
}

/// The preset a choice starts from when none was made.
pub const DEFAULT_PRESET: &str = "blue";

/// A theme choice: a preset (its accent family), an appearance, and an optional custom accent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ThemeChoice {
    /// The preset's id (`blue`, `graphite`, `teal`, ...).
    pub preset: String,
    /// Light, dark or system.
    pub appearance: Appearance,
    /// A custom accent (`#RRGGBB`) in place of the preset's; none: the preset's own.
    pub accent: Option<String>,
    /// When it last changed; none: never set (the default).
    pub at: Option<Timestamp>,
}

impl Default for ThemeChoice {
    /// The server's default: the blue preset, following the system, never set.
    fn default() -> Self {
        Self {
            preset: DEFAULT_PRESET.to_owned(),
            appearance: Appearance::System,
            accent: None,
            at: None,
        }
    }
}

/// One appearance's colours, each `#RRGGBB`. Every colour a client draws comes from these.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Tokens {
    /// The accent: what waits on the person, selection, the primary button.
    pub accent: String,
    /// The accent mixed into the surface: a selected row, a badge's ground.
    pub accent_soft: String,
    /// Text on the accent.
    pub on_accent: String,
    /// The window's ground.
    pub background: String,
    /// Cards and panes.
    pub surface: String,
    /// Raised surfaces: popovers, a card in dark mode.
    pub surface_raised: String,
    /// Lines.
    pub border: String,
    /// Body text.
    pub text: String,
    /// Secondary text.
    pub muted: String,
    /// Faint text and icons.
    pub faint: String,
    /// Done, ok.
    pub success: String,
    /// Look at it.
    pub warn: String,
    /// Failed, destructive.
    pub danger: String,
    /// Running, informational.
    pub info: String,
}

/// A colour set in both appearances.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TokenPair {
    /// In light mode.
    pub light: Tokens,
    /// In dark mode.
    pub dark: Tokens,
}

/// How readable one appearance's accent is (WCAG 2 contrast ratios, rounded down).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ContrastFacts {
    /// The accent as text on the background.
    pub accent_on_background: f64,
    /// The accent as text on the surface.
    pub accent_on_surface: f64,
    /// `onAccent` text on the accent.
    pub on_accent: f64,
    /// The asked accent was darkened or lightened to reach AA (4.5:1).
    pub adjusted: bool,
    /// The accent that was asked for.
    pub asked: String,
}

/// The accent's contrast in both appearances.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Contrast {
    /// In light mode.
    pub light: ContrastFacts,
    /// In dark mode.
    pub dark: ContrastFacts,
    /// Every ratio is 4.5:1 or more.
    pub aa: bool,
}

/// A preset, with its colours in both appearances.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Preset {
    /// Its id.
    pub id: String,
    /// Its name, as shown.
    pub name: String,
    /// Its colours in light mode.
    pub light: Tokens,
    /// Its colours in dark mode.
    pub dark: Tokens,
}

/// `GET /api/theme`'s answer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ThemePage {
    /// The presets' version: a client keeps its cache per version.
    pub version: u32,
    /// The choice.
    pub choice: ThemeChoice,
    /// The choice's colours.
    pub tokens: TokenPair,
    /// How readable its accent is.
    pub contrast: Contrast,
    /// Every preset.
    pub presets: Vec<Preset>,
    /// The token names, in order.
    pub names: Vec<String>,
}

impl ThemePage {
    /// A preset's own colours, from the presets this page lists; none when it isn't one of them.
    ///
    /// These are the colours of a choice of that preset without a custom accent. A custom
    /// accent, and the contrast facts of any other choice, the server works out: ask it with
    /// `POST /api/theme` and `preview`.
    #[must_use]
    pub fn preset_tokens(&self, preset: &str) -> Option<TokenPair> {
        self.presets
            .iter()
            .find(|p| p.id == preset)
            .map(|p| TokenPair {
                light: p.light.clone(),
                dark: p.dark.clone(),
            })
    }
}

/// `POST /api/theme`: the fields given change, the rest stay.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "validate", derive(garde::Validate))]
pub struct ThemeChange {
    /// A preset's id. Lower-case ASCII, 1 to 32 characters (the server trims and lower-cases).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "validate", garde(ascii, length(min = 1, max = 32)))]
    pub preset: Option<String>,
    /// Light, dark or system; a word the contract doesn't know is refused.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "validate", garde(custom(crate::rules::known_appearance)))]
    pub appearance: Option<Appearance>,
    /// The custom accent: left out to keep it, `null` for the preset's own, or a colour.
    #[serde(default, skip_serializing_if = "AccentChange::is_keep")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    #[cfg_attr(feature = "validate", garde(custom(crate::rules::accent_change)))]
    pub accent: AccentChange,
    /// Work the page out without saving it.
    #[serde(default, skip_serializing_if = "crate::serde_util::is_false")]
    #[cfg_attr(feature = "validate", garde(skip))]
    pub preview: bool,
}

/// What a [`ThemeChange`] does to the custom accent: three cases, since leaving the field out
/// and sending `null` mean different things.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum AccentChange {
    /// Keep it as it is: the field is left out.
    #[default]
    Keep,
    /// Back to the preset's own accent: `null`.
    Reset,
    /// This colour: `#RGB` or `#RRGGBB`, the `#` optional (the server writes it `#RRGGBB`).
    Set(String),
}

impl AccentChange {
    /// Whether it keeps the accent (and so is left out of the request).
    #[must_use]
    pub fn is_keep(&self) -> bool {
        matches!(self, Self::Keep)
    }
}

impl Serialize for AccentChange {
    /// `Set` writes the colour; `Reset` (and `Keep`, which a [`ThemeChange`] leaves out) write
    /// `null`.
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Set(colour) => serializer.serialize_str(colour),
            Self::Keep | Self::Reset => serializer.serialize_none(),
        }
    }
}

impl<'de> Deserialize<'de> for AccentChange {
    /// A colour reads as `Set`, `null` as `Reset`; a field left out takes the default, `Keep`.
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Option::<String>::deserialize(deserializer)
            .map(|colour| colour.map_or(Self::Reset, Self::Set))
    }
}

/// `#RGB` or `#RRGGBB`, the `#` optional, surrounding spaces allowed: what the server takes.
#[must_use]
pub fn is_hex_colour(text: &str) -> bool {
    let digits = text.trim();
    let digits = digits.strip_prefix('#').unwrap_or(digits);
    matches!(digits.len(), 3 | 6) && digits.bytes().all(|b| b.is_ascii_hexdigit())
}

/// `POST /api/theme`'s answer: `{ ok: true }` and the page as it is now (or would be, with
/// `preview`). A bad value is refused with 400 and an `ErrorBody`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ThemeSetReply {
    /// Always `true`.
    pub ok: bool,
    /// The page.
    #[serde(flatten)]
    pub page: ThemePage,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_colours() {
        for good in ["#3366FF", "3366ff", "#36f", " #36F "] {
            assert!(is_hex_colour(good), "{good}");
        }
        for bad in ["", "#", "#12345", "#GGGGGG", "blue"] {
            assert!(!is_hex_colour(bad), "{bad}");
        }
    }

    #[test]
    fn accent_changes_have_three_cases() {
        let keep: ThemeChange = serde_json::from_str("{}").unwrap();
        assert_eq!(keep.accent, AccentChange::Keep);
        assert_eq!(serde_json::to_string(&keep).unwrap(), "{}");
        let reset: ThemeChange = serde_json::from_str(r#"{"accent":null}"#).unwrap();
        assert_eq!(reset.accent, AccentChange::Reset);
        assert_eq!(serde_json::to_string(&reset).unwrap(), r#"{"accent":null}"#);
        let set: ThemeChange = serde_json::from_str(r##"{"accent":"#36f"}"##).unwrap();
        assert_eq!(set.accent, AccentChange::Set("#36f".to_owned()));
        assert_eq!(
            serde_json::to_string(&set).unwrap(),
            r##"{"accent":"#36f"}"##
        );
        assert!(!set.accent.is_keep());
    }

    #[test]
    fn the_default_choice_is_the_servers() {
        let choice = ThemeChoice::default();
        assert_eq!(choice.preset, "blue");
        assert_eq!(choice.appearance, Appearance::System);
        assert_eq!(Appearance::default(), Appearance::System);
    }
}
