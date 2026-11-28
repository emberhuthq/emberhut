//! Settings' data: every setting described once ([`entries`]) for search ([`search`]) and the
//! front door's suggestions ([`suggest`]) over how things stand ([`now`]); the MCP servers'
//! and skills' switches, what the assistant knows about the user, the Chrome profiles' changes,
//! and the assistant's on-duty switch. Each submodule hides one of them; their words and
//! filters are pure functions, tested on their own.

pub(crate) mod about;
pub(crate) mod appearance;
pub(crate) mod chrome;
pub(crate) mod ember_name;
pub(crate) mod entries;
pub(crate) mod now;
mod on_duty;
pub(crate) mod search;
pub(crate) mod suggest;
pub(crate) mod switches;
pub(crate) mod words;

use gpui_kit::Context;

use self::about::FactsFeed;
use self::chrome::{ChromeFeed, ChromeSheet};
use self::ember_name::EmberNaming;
use self::on_duty::OnDuty;
use self::switches::SwitchesFeed;
use super::{Page, Store};
use emberhut_api::ids::ChromeProfileId;

/// The pages Settings opens, each over its front door in a window's column (Back returns to
/// it), so every window has its own: a group's page, or a page one of their rows opens.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum SettingsPane {
    /// Your ember: on duty, what it thinks with and its usage, what it knows.
    Ember,
    /// Appearance: mode, colour, a custom colour and its preview.
    Appearance,
    /// Devices: the signed-in devices, pairing another, this one's pairing, the machines.
    Devices,
    /// Connections: the data sources, Chrome, the tool servers and the skills.
    Connections,
    /// Sharing: who can talk to the agent.
    Sharing,
    /// Privacy: what the agent knows, archived rooms, the dark room, where data goes.
    Privacy,
    /// Advanced: updates, the server, diagnostics, where the agent works.
    Advanced,
    /// The emberhut.com account, in builds with it.
    Account,
    /// The Chrome profiles.
    ChromeProfiles,
    /// What the assistant knows about the user.
    AboutYou,
    /// What the ember thinks with: the model provider the server's agent runs on.
    Credentials,
}

impl SettingsPane {
    /// Its title, over the page and on Close's tip.
    #[must_use]
    pub(crate) fn title(self) -> &'static str {
        match self {
            Self::ChromeProfiles => "Chrome profiles",
            Self::AboutYou => about::TITLE,
            Self::Credentials => "What your ember thinks with",
            group => entries::Group::of_pane(group).map_or("Settings", entries::Group::name),
        }
    }
}

/// Settings' data.
#[derive(Debug, Default)]
pub(crate) struct SettingsData {
    switches: SwitchesFeed,
    facts: FactsFeed,
    chrome: ChromeFeed,
    on_duty: OnDuty,
    naming: EmberNaming,
}

impl Store {
    /// One of Settings' pages shows afresh in a window's column: what it shows is fetched.
    pub(super) fn settings_pane_shown(&mut self, pane: SettingsPane, cx: &mut Context<'_, Self>) {
        match pane {
            SettingsPane::AboutYou | SettingsPane::Privacy => self.load_facts(cx),
            SettingsPane::Appearance => self.load_theme(cx),
            SettingsPane::Credentials => self.open_credentials(cx),
            SettingsPane::Ember => {
                self.load_credentials(cx);
                self.load_facts(cx);
            }
            SettingsPane::Connections => self.settings.switches.kick(),
            SettingsPane::Devices => self.machines.kick(),
            SettingsPane::ChromeProfiles
            | SettingsPane::Sharing
            | SettingsPane::Advanced
            | SettingsPane::Account => {}
        }
    }

    /// Whether a window's column shows Settings' front door, or one of `panes`.
    #[must_use]
    pub(crate) fn shows_settings(&self, panes: &[SettingsPane]) -> bool {
        self.columns.shows_any(|page| match page {
            Page::Settings => true,
            Page::SettingsPane(pane) => panes.contains(&pane),
            _ => false,
        })
    }

    /// Shows a Chrome profile's sheet over the profiles (`--chrome-profile`), once a window's
    /// column shows them.
    pub(crate) fn open_chrome_profile(&mut self, id: ChromeProfileId, cx: &mut Context<'_, Self>) {
        self.show_chrome_sheet(Some(ChromeSheet::Profile(id)), cx);
    }

    /// Settings' front door shows afresh in a column: what its suggestions speak of is read
    /// at once (the switches, what the ember thinks with, what it knows and its lessons).
    pub(super) fn settings_opened(&mut self, cx: &mut Context<'_, Self>) {
        self.settings.switches.kick();
        self.load_credentials(cx);
        self.load_facts(cx);
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
