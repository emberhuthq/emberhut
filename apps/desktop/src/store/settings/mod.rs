//! Settings' data: the MCP servers' and skills' switches, what the assistant knows about the
//! user, the Chrome profiles' changes, and the assistant's on-duty
//! switch. Each submodule hides one of them; their words and filters are pure functions,
//! tested on their own.

pub(crate) mod about;
pub(crate) mod appearance;
pub(crate) mod chrome;
mod on_duty;
pub(crate) mod switches;
pub(crate) mod words;

use gpui_kit::Context;

use self::about::FactsFeed;
use self::chrome::{ChromeFeed, ChromeSheet};
use self::on_duty::OnDuty;
use self::switches::SwitchesFeed;
use super::Store;
use emberhut_api::ids::ChromeProfileId;

/// The pages Settings' rows open, each over Settings in a window's column (Back returns to it),
/// so every window has its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SettingsPane {
    /// Appearance: mode, colour, a custom colour and its preview.
    Appearance,
    /// The Chrome profiles.
    ChromeProfiles,
    /// What the assistant knows about the user.
    AboutYou,
}

/// Settings' data.
#[derive(Debug, Default)]
pub(crate) struct SettingsData {
    switches: SwitchesFeed,
    facts: FactsFeed,
    chrome: ChromeFeed,
    on_duty: OnDuty,
}

impl Store {
    /// One of Settings' pages shows afresh in a window's column: what it shows is fetched.
    pub(super) fn settings_pane_shown(&mut self, pane: SettingsPane, cx: &mut Context<'_, Self>) {
        match pane {
            SettingsPane::AboutYou => self.load_facts(cx),
            SettingsPane::Appearance => self.load_theme(cx),
            SettingsPane::ChromeProfiles => {}
        }
    }

    /// Shows a Chrome profile's sheet over the profiles (`--chrome-profile`), once a window's
    /// column shows them.
    pub(crate) fn open_chrome_profile(&mut self, id: ChromeProfileId, cx: &mut Context<'_, Self>) {
        self.show_chrome_sheet(Some(ChromeSheet::Profile(id)), cx);
    }

    /// Settings' main page shows afresh in a column: its switches are fetched at once.
    pub(super) fn settings_opened(&mut self) {
        self.settings.switches.kick();
    }

    /// The window came to the front: the switches are fetched at once if Settings shows them.
    pub(super) fn settings_forward(&self) {
        self.settings.switches.kick();
    }

    /// Takes `state` in as if the poll had brought it, for the views' tests.
    #[cfg(test)]
    pub(crate) fn put_state_for_tests(&mut self, state: emberhut_api::state::State) {
        self.state = Some(state);
    }
}

#[cfg(test)]
mod tests;
