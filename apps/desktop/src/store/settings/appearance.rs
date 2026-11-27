//! Appearance's words: the main page's summary of the theme, a custom colour's contrast as the
//! server measured it, and which typed colour is worth a preview. Pure functions of the theme.

use emberhut_api::theme::{Appearance, ThemePage};

use crate::color::normalize_hex;
use crate::format::capitalized;

/// The theme as Settings' Appearance row says it: `Blue · System`, `Custom · Dark`.
#[must_use]
pub(crate) fn summary(page: &ThemePage) -> String {
    let choice = &page.choice;
    let colour = if choice.accent.is_some() {
        "Custom".to_owned()
    } else {
        page.presets
            .iter()
            .find(|preset| preset.id == choice.preset)
            .map_or_else(|| capitalized(&choice.preset), |preset| preset.name.clone())
    };
    let mode = match &choice.appearance {
        Appearance::System => "System".to_owned(),
        Appearance::Light => "Light".to_owned(),
        Appearance::Dark => "Dark".to_owned(),
        other => capitalized(other.as_str()),
    };
    format!("{colour} · {mode}")
}

/// The name of the preset under a custom colour: what "Use …'s own colour" goes back to.
#[must_use]
pub(crate) fn preset_name(page: &ThemePage) -> String {
    page.presets
        .iter()
        .find(|preset| preset.id == page.choice.preset)
        .map_or_else(|| "the preset".to_owned(), |preset| preset.name.clone())
}

/// A colour's contrast for the mode on screen (`dark`), as the server measured it: `On the
/// page 5.1:1 · text on it 6.2:1 · AA`, then a line on what it adjusted, if anything.
#[must_use]
pub(crate) fn contrast_line(page: &ThemePage, dark: bool) -> String {
    let contrast = &page.contrast;
    let side = if dark {
        &contrast.dark
    } else {
        &contrast.light
    };
    let on_page = side.accent_on_background.min(side.accent_on_surface);
    let mut line = format!(
        "On the page {on_page:.1}:1 · text on it {:.1}:1 · {}",
        side.on_accent,
        if contrast.aa { "AA" } else { "below AA" }
    );
    let changed: Vec<&str> = [
        contrast.light.adjusted.then_some("darkened for light mode"),
        contrast.dark.adjusted.then_some("lightened for dark mode"),
    ]
    .into_iter()
    .flatten()
    .collect();
    if !changed.is_empty() {
        line.push('\n');
        line.push_str(&capitalized(&changed.join(", ")));
        line.push_str(", so text stays readable.");
    }
    line
}

/// The typed colour worth asking the server to preview: a hex colour that isn't the custom
/// colour already in use.
#[must_use]
pub(crate) fn to_preview(typed: &str, current: Option<&str>) -> Option<String> {
    normalize_hex(typed).filter(|hex| Some(hex.as_str()) != current)
}

/// What the user is told when Apply meets text that isn't a colour.
pub(crate) const NOT_A_HEX: &str = "That isn't a hex colour (like #3366FF).";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::built_in;

    #[test]
    fn the_row_names_the_colour_and_the_mode() {
        let mut page = built_in();
        page.choice.appearance = Appearance::Dark;
        let preset = page.presets[0].name.clone();
        assert_eq!(summary(&page), format!("{preset} · Dark"));
        page.choice.accent = Some("#3366FF".into());
        assert_eq!(summary(&page), "Custom · Dark");
    }

    #[test]
    fn the_contrast_line_is_the_modes_on_screen() {
        let mut page = built_in();
        page.contrast.light.accent_on_background = 5.4;
        page.contrast.light.accent_on_surface = 5.1;
        page.contrast.light.on_accent = 6.24;
        page.contrast.aa = true;
        assert_eq!(
            contrast_line(&page, false),
            "On the page 5.1:1 · text on it 6.2:1 · AA"
        );
        page.contrast.aa = false;
        page.contrast.light.adjusted = true;
        page.contrast.dark.adjusted = true;
        assert_eq!(
            contrast_line(&page, false),
            "On the page 5.1:1 · text on it 6.2:1 · below AA\nDarkened for light mode, \
             lightened for dark mode, so text stays readable."
        );
    }

    #[test]
    fn only_a_new_colour_is_previewed() {
        assert_eq!(to_preview("#36f", None).as_deref(), Some("#3366FF"));
        assert_eq!(to_preview("#3366ff", Some("#3366FF")), None);
        assert_eq!(to_preview("blue", None), None);
    }
}
