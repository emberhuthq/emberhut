//! "Make it yours": the ember's name and its look, right after it is lit. The name is typed
//! (or a suggestion picked) and shows at once wherever the setup names the ember; Continue
//! saves it the way Settings does ([`Store::rename_ember`]: `POST /api/ember/name`, shown at
//! once, back to the one before if the server refuses). The look is the theme's own choice
//! ([`Store::pick_theme`]): the whole window takes it as it is picked. Both have their default
//! ("Ember", Paper), so Continue works at once; "Ember" is saved too, since a name the person
//! never chose reads "Your ember".

use emberhut_api::ember_name::clean_name;
use gpui_kit::Context;

use super::super::Store;
use super::SetupStep;

/// The name the setup offers first.
pub(crate) const SUGGESTED_NAME: &str = "Ember";

/// The names offered beside the field.
pub(crate) const SUGGESTIONS: [&str; 5] = [SUGGESTED_NAME, "Ash", "Cinder", "Spark", "Flint"];

impl Store {
    /// The ember's name as the setup shows it now: what was typed, else its name on the
    /// server, else "Ember".
    #[must_use]
    pub(crate) fn setup_name(&self) -> String {
        self.setup
            .kept
            .name
            .clone()
            .or_else(|| self.ember_custom_name().map(str::to_owned))
            .unwrap_or_else(|| SUGGESTED_NAME.to_owned())
    }

    /// Why the name typed can't be the ember's (as the server would say), while it can't.
    #[must_use]
    pub(crate) fn setup_name_problem(&self) -> Option<String> {
        self.setup
            .name_problem
            .clone()
            .or_else(|| self.ember_name_problem().map(str::to_owned))
    }

    /// The person typed `typed` as the ember's name (or picked a suggestion): shown at once
    /// when it is one the server takes; blank goes back to "Ember".
    pub(crate) fn name_ember(&mut self, typed: &str, cx: &mut Context<'_, Self>) {
        match clean_name(typed) {
            Ok(name) => {
                self.setup.name_problem = None;
                self.setup.kept.name = Some(name.unwrap_or_else(|| SUGGESTED_NAME.to_owned()));
            }
            Err(refusal) => self.setup.name_problem = Some(refusal.to_string()),
        }
        cx.notify();
    }

    /// Continue on "Make it yours": the name is saved on the server, and the step is done with
    /// what it is called and how it looks.
    pub(crate) fn yours_done(&mut self, cx: &mut Context<'_, Self>) {
        let name = self.setup_name();
        self.setup.kept.name = Some(name.clone());
        self.keep_setup_file(cx);
        self.rename_ember(&name, cx);
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
