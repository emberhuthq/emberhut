//! `GET /api/environments`: every place the assistant works (its runners, its workers, its own
//! browser, external threads) and every connection it reads through (mail, chat apps, the phone's health,
//! the calendar, the Claude accounts), each with a live state, what it is, its lists, its last
//! error and the safe actions it offers. `POST /api/environments/check` runs one of those
//! actions and answers with the page as it is after, plus what the action made (a Chrome
//! profile's re-pair link, a one-time sign-in link, a note).
//!
//! The server draws the rows: their names, states, words and actions are its own, so a client
//! shows them as they come. A request the server can't do (an unknown action, an id that has
//! no such action) is refused with 400 and an [`crate::error::ErrorBody`].

use serde::{Deserialize, Serialize};

use crate::chrome::ChromePairing;
use crate::ids::EnvironmentId;
use crate::open_enum::open_enum;
use crate::time::Timestamp;

open_enum! {
    /// How a place or a connection is.
    pub enum EnvState {
        /// Fine.
        Ok = "ok",
        /// It wants a look.
        Warn = "warn",
        /// Down: it can't be reached or doesn't work.
        Down = "down",
        /// Not set up, or turned off.
        Off = "off",
    }
}

open_enum! {
    /// The group a row is in.
    pub enum EnvGroupId {
        /// The runners that run sessions.
        Runners = "runners",
        /// The assistant's workers.
        Workers = "workers",
        /// The assistant's own browser.
        Browser = "browser",
        /// the coding-thread app.
        CodingApp = "t3",
        /// The connections the assistant reads through.
        Connections = "connections",
        /// The paired Chrome profiles.
        Chrome = "chrome",
    }
}

impl EnvGroupId {
    /// Whether its rows are places the assistant works: runners, workers, its browser, external threads.
    #[must_use]
    pub fn is_workplace(&self) -> bool {
        matches!(
            self,
            Self::Runners | Self::Workers | Self::Browser | Self::CodingApp
        )
    }

    /// Whether its rows are data sources: the connections the assistant reads through.
    #[must_use]
    pub fn is_data_source(&self) -> bool {
        matches!(self, Self::Connections)
    }
}

open_enum! {
    /// What an action of a row does.
    pub enum EnvActionKind {
        /// Look again now.
        Check = "check",
        /// A Chrome profile's re-pair link.
        Repair = "repair",
        /// Start the assistant's browser.
        Start = "start",
        /// Send the phone's health now (the phone's own action).
        HealthSync = "health-sync",
        /// A one-time link to sign in again (mail) or to give a new token (chat).
        Link = "link",
        /// A pairing code for the assistant's own chat number.
        WaLink = "wa-link",
        /// Delete the connection's sign-in (what it already read stays).
        Unlink = "unlink",
    }
}

impl Default for EnvActionKind {
    /// [`Self::Check`], as the server reads a request that names none.
    fn default() -> Self {
        Self::Check
    }
}

/// The page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct EnvPage {
    /// When it was made.
    pub at: Timestamp,
    /// How many rows are fine, want a look and are down.
    pub summary: EnvSummary,
    /// The groups, each with its rows; a group with no rows is left out.
    pub groups: Vec<EnvGroup>,
    /// After a re-pair: the link to open in the profile.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pairing: Option<ChromePairing>,
    /// After an action: what it did, in a line.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// After a link action: the one-time page to open (a sign-in, a token's page, a pairing
    /// code).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
}

impl EnvPage {
    /// Every row, group by group.
    pub fn rows(&self) -> impl Iterator<Item = &Environment> {
        self.groups.iter().flat_map(|group| &group.items)
    }

    /// The row `id`.
    #[must_use]
    pub fn row(&self, id: &EnvironmentId) -> Option<&Environment> {
        self.rows().find(|row| &row.id == id)
    }
}

/// How many rows are fine, want a look and are down.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct EnvSummary {
    /// Fine.
    pub ok: u32,
    /// Wanting a look.
    pub warn: u32,
    /// Down.
    pub down: u32,
}

/// A group of rows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct EnvGroup {
    /// Its id.
    pub id: EnvGroupId,
    /// Its title.
    pub title: String,
    /// A line under it.
    pub footer: Option<String>,
    /// Its rows.
    pub items: Vec<Environment>,
}

/// A place the assistant works, or a connection it reads through.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Environment {
    /// Its id (`runner`, `browser`, `gmail:work`, `slack:example`).
    pub id: EnvironmentId,
    /// Its group.
    pub group: EnvGroupId,
    /// Its name.
    pub name: String,
    /// Its icon, as an SF Symbols name (`server.rack`, `globe`).
    pub icon: String,
    /// Its icon's colour: a colour's name (`blue`, `teal`), or `accent` for the theme's.
    pub tint: String,
    /// Kept only while work moves off it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub legacy: Option<bool>,
    /// How it is.
    pub state: EnvState,
    /// Its state in words (`Online`, `Offline`, `Read yes · send no`).
    pub status: String,
    /// What it is, in a line.
    pub detail: String,
    /// When it was last looked at.
    pub at: Option<Timestamp>,
    /// What is wrong with it now.
    pub error: Option<String>,
    /// What to do about it, when something is off.
    pub hint: Option<String>,
    /// What it is and can do, each a label and a value.
    pub facts: Vec<EnvFact>,
    /// Its lists: accounts, sessions, folders, tickets.
    pub lists: Vec<EnvList>,
    /// The actions it offers.
    pub actions: Vec<EnvAction>,
    /// The last thing that went wrong there, kept after it recovered.
    pub last_error: Option<EnvLastError>,
}

/// A labelled fact about a row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct EnvFact {
    /// Its label.
    pub label: String,
    /// Its value (a word, or a sentence).
    pub value: String,
    /// Its state, when it has one: the value shows in its colour.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<EnvState>,
}

/// One of a row's lists.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct EnvList {
    /// Its title.
    pub title: String,
    /// Its items.
    pub items: Vec<EnvListItem>,
    /// What it says when it has none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub empty: Option<String>,
}

/// An item of a list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct EnvListItem {
    /// Its name.
    pub name: String,
    /// A value beside it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    /// A line under it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Its state, shown as a dot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<EnvState>,
}

/// An action a row offers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct EnvAction {
    /// What it does.
    pub id: EnvActionKind,
    /// Its label (`Check now`, `Reconnect: a Google sign-in link`).
    pub label: String,
}

/// The last thing that went wrong at a row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct EnvLastError {
    /// What went wrong.
    pub error: String,
    /// When.
    pub at: Timestamp,
}

/// `POST /api/environments/check`: one row's action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct EnvActionRequest {
    /// The row.
    pub id: EnvironmentId,
    /// What to do (look again, by default).
    #[serde(default)]
    pub kind: EnvActionKind,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_tell_places_from_sources() {
        assert!(EnvGroupId::Runners.is_workplace());
        assert!(EnvGroupId::CodingApp.is_workplace());
        assert!(!EnvGroupId::Chrome.is_workplace());
        assert!(EnvGroupId::Connections.is_data_source());
        assert!(!EnvGroupId::from("later").is_data_source());
    }

    #[test]
    fn an_action_is_its_id_and_kind() {
        let request = EnvActionRequest {
            id: EnvironmentId::new("gmail:work"),
            kind: EnvActionKind::Unlink,
        };
        assert_eq!(
            serde_json::to_string(&request).unwrap(),
            r#"{"id":"gmail:work","kind":"unlink"}"#
        );
        let bare: EnvActionRequest = serde_json::from_str(r#"{"id":"runner"}"#).unwrap();
        assert_eq!(bare.kind, EnvActionKind::Check);
    }
}
