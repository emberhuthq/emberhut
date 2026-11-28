//! "Make it yours": the ember's name and its look, right after it is lit. The name is typed
//! (or a suggestion picked) and shows at once wherever the setup names the ember; it is kept in
//! `setup.json` until the server takes it. The look is the theme's own choice
//! ([`Store::pick_theme`]): the whole window takes it as it is picked. Both have their default
//! ("Ember", Paper), so Continue works at once.

use gpui_kit::Context;

use super::super::Store;
use super::SetupStep;

/// The name an ember has until it is given one.
pub(crate) const DEFAULT_NAME: &str = "Ember";

/// The names offered beside the field.
pub(crate) const SUGGESTIONS: [&str; 5] = [DEFAULT_NAME, "Ash", "Cinder", "Spark", "Flint"];

/// The longest name, in characters.
pub(crate) const NAME_MAX: usize = 24;

/// `typed` as a name, as the server takes it: no newlines or control characters, trimmed, at
/// most [`NAME_MAX`] characters, the default when empty.
#[must_use]
pub(crate) fn name_of(typed: &str) -> String {
    let name: String = typed
        .chars()
        .filter(|c| !c.is_control())
        .collect::<String>()
        .trim()
        .chars()
        .take(NAME_MAX)
        .collect();
    if name.is_empty() {
        DEFAULT_NAME.to_owned()
    } else {
        name
    }
}

impl Store {
    /// The ember's name as the setup shows it now.
    #[must_use]
    pub(crate) fn ember_name(&self) -> String {
        self.setup
            .kept
            .name
            .as_deref()
            .map_or_else(|| DEFAULT_NAME.to_owned(), name_of)
    }

    /// The person typed `typed` as the ember's name (or picked a suggestion).
    pub(crate) fn name_ember(&mut self, typed: &str, cx: &mut Context<'_, Self>) {
        let name = name_of(typed);
        if self.setup.kept.name.as_deref() == Some(name.as_str()) {
            return;
        }
        self.setup.kept.name = Some(name);
        cx.notify();
    }

    /// Continue on "Make it yours": the name is kept, and the step is done with what it is
    /// called and how it looks.
    pub(crate) fn yours_done(&mut self, cx: &mut Context<'_, Self>) {
        let name = self.ember_name();
        self.setup.kept.name = Some(name.clone());
        self.keep_setup_file(cx);
        let page = self.theme().page();
        let look = page
            .presets
            .iter()
            .find(|preset| preset.id == page.choice.preset)
            .map(|preset| preset.name.clone());
        let line = match look {
            Some(look) => format!("Called {name}, in {look}."),
            None => format!("Called {name}."),
        };
        self.step_done(SetupStep::Yours, line, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_is_trimmed_kept_short_and_never_empty() {
        assert_eq!(name_of("  Ash "), "Ash");
        assert_eq!(name_of("   "), DEFAULT_NAME);
        assert_eq!(name_of("Ash\nby\tthe fire"), "Ashbythe fire");
        assert_eq!(name_of(&"x".repeat(50)).chars().count(), NAME_MAX);
    }
}
