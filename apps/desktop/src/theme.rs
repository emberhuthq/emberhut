//! The app's own side of the theme: the built-in default for before the server ever answered,
//! and how a choice is applied from the presets at hand. No colours are drawn here; see
//! `ui::palette`. (When the design tokens come from `packages/design-tokens`, `built_in` reads
//! them instead of the literals below.)

use emberhut_api::theme::{
    Contrast, ContrastFacts, DEFAULT_PRESET, Preset, ThemeChoice, ThemePage, TokenPair, Tokens,
};

use crate::color::{contrast_ratio, parse_hex};

/// The built-in theme, blue on graphite (the server's default). The one place the app has
/// colour values of its own.
#[must_use]
pub(crate) fn built_in() -> ThemePage {
    let light = tokens([
        "#2563EB", "#E5ECFD", "#FFFFFF", "#F5F6F8", "#FFFFFF", "#ECEEF1", "#D9DCE1", "#14171C",
        "#5B6270", "#8A909B", "#15803D", "#C2410C", "#DC2626", "#0E7490",
    ]);
    let dark = tokens([
        "#60A5FA", "#27384E", "#0B0D10", "#0E1013", "#17191D", "#202328", "#2C3036", "#E9EBEE",
        "#9BA1AB", "#6B717B", "#4ADE80", "#FB923C", "#F87171", "#22D3EE",
    ]);
    let contrast = Contrast {
        light: facts(&light),
        dark: facts(&dark),
        aa: true,
    };
    let aa = contrast.light.accent_on_surface >= 4.5 && contrast.dark.accent_on_surface >= 4.5;
    ThemePage {
        version: 0,
        choice: ThemeChoice::default(),
        presets: vec![Preset {
            id: DEFAULT_PRESET.into(),
            name: "Blue".into(),
            light: light.clone(),
            dark: dark.clone(),
        }],
        tokens: TokenPair { light, dark },
        contrast: Contrast { aa, ..contrast },
        names: Vec::new(),
    }
}

/// The accent's contrast in one appearance, as the server would measure it.
fn facts(tokens: &Tokens) -> ContrastFacts {
    let colour = |hex: &str| parse_hex(hex).unwrap_or_default();
    let accent = colour(&tokens.accent);
    // Two decimals, as the server sends them (and so the cache reads back exactly).
    let ratio = |a, b| (contrast_ratio(a, b) * 100.0).round() / 100.0;
    ContrastFacts {
        accent_on_background: ratio(accent, colour(&tokens.background)),
        accent_on_surface: ratio(accent, colour(&tokens.surface)),
        on_accent: ratio(colour(&tokens.on_accent), accent),
        adjusted: false,
        asked: tokens.accent.clone(),
    }
}

/// The theme as `choice` would make it from `page`'s presets; `None` when only the server can
/// make it (an unknown preset, or a custom accent other than the current one, which the server
/// holds to WCAG AA).
#[must_use]
pub(crate) fn with_choice(page: &ThemePage, choice: &ThemeChoice) -> Option<ThemePage> {
    let preset_tokens = page.preset_tokens(&choice.preset)?;
    if choice.accent.is_some() && choice.accent != page.choice.accent {
        return None;
    }
    let mut out = page.clone();
    if choice.accent.is_none() {
        out.tokens = preset_tokens;
    }
    out.choice = choice.clone();
    Some(out)
}

/// Tokens in the order the type lists them.
fn tokens(hex: [&str; 14]) -> Tokens {
    let [
        accent,
        accent_soft,
        on_accent,
        background,
        surface,
        surface_raised,
        border,
        text,
        muted,
        faint,
        success,
        warn,
        danger,
        info,
    ] = hex.map(String::from);
    Tokens {
        accent,
        accent_soft,
        on_accent,
        background,
        surface,
        surface_raised,
        border,
        text,
        muted,
        faint,
        success,
        warn,
        danger,
        info,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use emberhut_api::theme::Appearance;

    fn with_green() -> ThemePage {
        let mut page = built_in();
        let mut green = page.presets[0].clone();
        green.id = "green".into();
        green.light.accent = "#15803D".into();
        page.presets.push(green);
        page
    }

    #[test]
    fn every_built_in_token_is_a_colour() {
        let page = built_in();
        for tokens in [&page.tokens.light, &page.tokens.dark] {
            let json = serde_json::to_value(tokens).unwrap();
            for (name, value) in json.as_object().unwrap() {
                let hex = value.as_str().unwrap();
                assert!(parse_hex(hex).is_some(), "{name} = {hex}");
            }
        }
    }

    #[test]
    fn a_known_preset_is_made_without_the_server() {
        let choice = ThemeChoice {
            preset: "green".into(),
            appearance: Appearance::Dark,
            ..ThemeChoice::default()
        };
        let next = with_choice(&with_green(), &choice).unwrap();
        assert_eq!(next.tokens.light.accent, "#15803D");
        assert_eq!(next.choice.appearance, Appearance::Dark);
    }

    #[test]
    fn an_unknown_preset_or_a_new_accent_needs_the_server() {
        let page = with_green();
        let unknown = ThemeChoice {
            preset: "violet".into(),
            ..ThemeChoice::default()
        };
        assert_eq!(with_choice(&page, &unknown), None);
        let custom = ThemeChoice {
            preset: "blue".into(),
            accent: Some("#123456".into()),
            ..ThemeChoice::default()
        };
        assert_eq!(with_choice(&page, &custom), None);
    }

    #[test]
    fn the_built_in_contrast_is_measured() {
        let page = built_in();
        assert!(page.contrast.light.accent_on_surface > 4.5);
        assert!(page.contrast.dark.accent_on_surface > 4.5);
        assert!(page.contrast.aa);
    }

    #[test]
    fn the_built_in_theme_round_trips_through_its_cache_format() {
        let page = built_in();
        let json = serde_json::to_vec(&page).unwrap();
        let back: ThemePage = serde_json::from_slice(&json).unwrap();
        assert_eq!(back, page);
    }
}
