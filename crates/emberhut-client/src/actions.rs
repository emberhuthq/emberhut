//! The person's changes and the pages behind Settings: `POST /api/settings` with one typed
//! action, the MCP servers' and skills' switches, what the assistant knows about the person,
//! and the Chrome profiles.
//!
//! Each route is one method taking its request type from [`emberhut_api`]; a new action of
//! `/api/settings` is one more variant of [`SettingsAction`], with no new method here.

use emberhut_api::chrome::{ChromeAction, ChromeReply};
use emberhut_api::error::OkReply;
use emberhut_api::facts::FactsPage;
use emberhut_api::paths;
use emberhut_api::settings::{SettingsAction, SettingsRequest};
use emberhut_api::switches::{SwitchAction, SwitchPage};

use crate::exchange;
use crate::{Client, Error};

impl Client {
    /// Makes one change (`POST /api/settings`): pause or resume the assistant, tick a step,
    /// correct a fact... The server answers with the state of the request's room as well,
    /// which this drops: the next poll has it.
    ///
    /// # Errors
    ///
    /// [`Error::Status`] 400 with the server's reason when the change can't be made (no such
    /// task, fact or rule); [`Error::SignInRequired`], the transport's and decoding errors.
    pub fn settings(&self, action: SettingsAction) -> Result<(), Error> {
        let request = SettingsRequest::from(action);
        let _: OkReply = exchange::json(self.post(paths::SETTINGS, &request)?)?;
        Ok(())
    }

    /// The MCP servers and skills with their switches (`GET /api/switches`).
    ///
    /// # Errors
    ///
    /// [`Error::Status`] 404 from a server without these routes; [`Error::SignInRequired`],
    /// the transport's and decoding errors.
    pub fn switches(&self) -> Result<SwitchPage, Error> {
        exchange::json(self.get(paths::SWITCHES, "")?)
    }

    /// Turns a server or a skill on or off, or reconnects a server (`POST /api/switches`); the
    /// page as it is after.
    ///
    /// # Errors
    ///
    /// [`Error::Status`] 400 with the server's reason (the assistant's own server, a server
    /// that is off); as [`Client::switches`].
    pub fn switch(&self, action: &SwitchAction) -> Result<SwitchPage, Error> {
        exchange::json(self.post(paths::SWITCHES, action)?)
    }

    /// What the assistant knows about the person, and its rules on trial (`GET /api/facts`).
    ///
    /// # Errors
    ///
    /// [`Error::SignInRequired`], [`Error::Status`], the transport's and decoding errors.
    pub fn facts(&self) -> Result<FactsPage, Error> {
        exchange::json(self.get(paths::FACTS, "")?)
    }

    /// Pairs, renames, unpairs or pauses a Chrome profile, or cancels a link
    /// (`POST /api/chrome`); the profiles and links as they are after, with the new link for
    /// [`ChromeAction::Add`] and [`ChromeAction::Repair`].
    ///
    /// # Errors
    ///
    /// [`Error::Status`] 409 with the server's reason (no such profile, one that isn't
    /// connected); [`Error::SignInRequired`], the transport's and decoding errors.
    pub fn chrome(&self, action: &ChromeAction) -> Result<ChromeReply, Error> {
        exchange::json(self.post(paths::CHROME, action)?)
    }
}
