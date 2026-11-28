//! The person's changes and the pages behind Settings: `POST /api/settings` with one typed
//! action, the MCP servers' and skills' switches, what the assistant knows about the person
//! (and all of it as one zip), and the Chrome profiles.
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

    /// Everything the assistant keeps about the person, as one zip (`GET /api/memory/export`,
    /// a signed-in device only): its bytes, up to the client's body limit (64 MiB), and the
    /// file name the server gives it.
    ///
    /// # Errors
    ///
    /// [`Error::SignInRequired`], [`Error::Status`], the transport's errors.
    pub fn memory_export(&self) -> Result<Attachment, Error> {
        let mut response = self.get(paths::MEMORY_EXPORT, "")?;
        if !response.status().is_success() {
            return Err(exchange::refusal(response));
        }
        let name = response
            .headers()
            .get("content-disposition")
            .and_then(|value| value.to_str().ok())
            .and_then(attachment_name);
        let bytes = exchange::body_bytes(&mut response)?;
        Ok(Attachment { name, bytes })
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

/// A file the server sends to be saved: its bytes, and the name it gives it.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Attachment {
    /// Its name, from `Content-Disposition`: a bare file name (no folder, not hidden); none
    /// when the server gave none it could be saved as.
    pub name: Option<String>,
    /// Its bytes.
    pub bytes: Vec<u8>,
}

/// The file name of a `Content-Disposition` header (`attachment; filename="a.zip"`), when it
/// is one a file can be saved as: its last path part, neither blank, hidden nor `..`, without
/// control characters.
fn attachment_name(header: &str) -> Option<String> {
    let value = header.split(';').map(str::trim).find_map(|part| {
        let (key, value) = part.split_once('=')?;
        key.trim()
            .eq_ignore_ascii_case("filename")
            .then(|| value.trim().trim_matches('"'))
    })?;
    let name = value.rsplit(['/', '\\']).next()?.trim();
    let fine = !name.is_empty() && !name.starts_with('.') && !name.chars().any(char::is_control);
    fine.then(|| name.to_owned())
}

#[cfg(test)]
mod tests {
    use super::attachment_name;

    #[test]
    fn an_attachments_name_is_a_bare_file_name() {
        assert_eq!(
            attachment_name("attachment; filename=\"memory-2026-11-28.zip\"").as_deref(),
            Some("memory-2026-11-28.zip")
        );
        assert_eq!(
            attachment_name("attachment; FileName=plain.zip").as_deref(),
            Some("plain.zip")
        );
        assert_eq!(
            attachment_name("attachment; filename=\"../../etc/x.zip\"").as_deref(),
            Some("x.zip"),
            "no folder"
        );
        for refused in [
            "attachment",
            "attachment; filename=\"\"",
            "attachment; filename=\".hidden\"",
            "attachment; filename=\"a/..\"",
            "attachment; filename=\"a\u{7}b.zip\"",
        ] {
            assert_eq!(attachment_name(refused), None, "{refused:?}");
        }
    }
}
