//! The app's own side of the theme: the built-in default for before the server ever answered,
//! and how a choice is applied from the presets at hand. No colours are drawn here; see
//! `ui::palette`. The built-in colours are worked out by `emberhut-core`'s theme rules, the
//! server's own, so they are the server's default to the last digit.

use emberhut_api::theme::{
    Contrast, ContrastFacts, DEFAULT_PRESET, EmberTint, Preset, ThemeChoice, ThemePage, TokenPair,
    Tokens,
};
use emberhut_core::theme as rules;

/// The presets the built-in theme offers: the default, paper, and blue beside it.
const BUILT_IN_PRESETS: [&str; 2] = [DEFAULT_PRESET, "blue"];

/// The built-in theme: the server's default (paper, as the system is, never set), with blue
/// to pick as well, and the ember's tints.
#[must_use]
pub(crate) fn built_in() -> ThemePage {
    let (_, light) = rules::resolve(DEFAULT_PRESET, None, false);
    let (_, dark) = rules::resolve(DEFAULT_PRESET, None, true);
    let aa = light.is_aa() && dark.is_aa();
    ThemePage {
        version: 0,
        choice: ThemeChoice::default(),
        presets: rules::PRESETS
            .iter()
            .filter(|preset| BUILT_IN_PRESETS.contains(&preset.id))
            .map(|preset| {
                let TokenPair { light, dark } = preset_tokens(preset.id);
                Preset {
                    id: preset.id.into(),
                    name: preset.name.into(),
                    light,
                    dark,
                }
            })
            .collect(),
        tokens: preset_tokens(DEFAULT_PRESET),
        contrast: Contrast {
            light: facts(light),
            dark: facts(dark),
            aa,
        },
        names: rules::NAMES.iter().map(|name| (*name).to_owned()).collect(),
        tints: rules::TINTS
            .iter()
            .map(|tint| EmberTint {
                id: tint.id.into(),
                name: tint.name.into(),
            })
            .collect(),
    }
}

/// A preset's own colours in both appearances, as the server resolves them.
#[must_use]
pub(crate) fn preset_tokens(preset: &str) -> TokenPair {
    TokenPair {
        light: tokens(rules::resolve(preset, None, false).0),
        dark: tokens(rules::resolve(preset, None, true).0),
    }
}

/// The accent's contrast in one appearance, as the server sends it.
fn facts(facts: rules::Contrast) -> ContrastFacts {
    ContrastFacts {
        accent_on_background: facts.accent_on_background,
        accent_on_surface: facts.accent_on_surface,
        on_accent: facts.on_accent,
        adjusted: facts.adjusted,
        asked: facts.asked,
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

/// One appearance's colours, as the API carries them.
fn tokens(palette: rules::Palette) -> Tokens {
    let rules::Palette {
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
    } = palette;
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
    use crate::color::parse_hex;
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
    fn the_built_in_theme_is_paper_with_blue_to_pick() {
        let page = built_in();
        assert_eq!(page.choice.preset, "paper");
        assert_eq!(page.tokens.light.background, "#F6F0E6");
        assert_eq!(page.tokens.light.accent, "#B83D0C");
        assert_eq!(page.tokens.dark.background, "#15110E");
        assert_eq!(page.tokens.dark.accent, "#FF9A5A");
        let ids: Vec<&str> = page.presets.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, ["paper", "blue"]);
        assert_eq!(page.preset_tokens("paper").unwrap(), page.tokens);
        let blue = ThemeChoice {
            preset: "blue".into(),
            ..ThemeChoice::default()
        };
        let blue = with_choice(&page, &blue).unwrap();
        assert_eq!(blue.tokens.light.accent, "#2563EB");
        assert_eq!(blue.tokens.light.background, "#F5F6F8");
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
