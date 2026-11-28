//! The person's Chrome profiles with the assistant's extension: each paired on its own (Work,
//! Personal...), so the assistant works in the account a task is about.
//!
//! The state carries them ([`crate::state::State::chrome`]), the pairing links still waiting
//! ([`crate::state::State::chrome_pairing`]) and the extension's newest version. `POST
//! /api/chrome` ([`ChromeAction`]) adds one (a link to open in that profile), pairs one again,
//! renames, unpairs, cancels a link, pauses the assistant there for a while, or hands the
//! profile back to it. Its 200 answer is [`ChromeReply`]; a request it can't do (no such
//! profile, one that isn't connected, a name already taken) is refused with 409 and an
//! [`crate::error::ErrorBody`] with `ok: false`.
//!
//! The server finds the profile of a pause or a hand-back by `id`, else by its short `name`;
//! this contract always sends the `id`.

use serde::{Deserialize, Serialize};

use crate::ids::{ChromeCode, ChromeProfileId, TaskId};
use crate::open_enum::open_enum;
use crate::serde_util::null_as_default;
use crate::time::Timestamp;

open_enum! {
    /// Who drives a profile now.
    #[derive(Default)]
    pub enum ChromeControl {
        /// The assistant (or a worker).
        #[default]
        Agent = "agent",
        /// The person: the assistant handed it over and waits (its word is in
        /// [`crate::wire`]).
        Person = crate::wire::PERSON,
    }
}

/// The assistant may work in a profile now: it is paired, online and not paused.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ChromeGrant {
    /// Until when; none: any time.
    #[serde(default)]
    pub until: Option<Timestamp>,
    /// The task it works on there.
    #[serde(default)]
    pub task: Option<TaskId>,
    /// What let it in (`paired`).
    pub by: String,
}

/// The person paused the assistant in a profile; it ends by itself.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ChromePause {
    /// Why, in words.
    pub why: String,
    /// When it ends, in milliseconds since the Unix epoch.
    pub until: f64,
}

/// A paired Chrome profile.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ChromeProfile {
    /// Its id.
    pub id: ChromeProfileId,
    /// Its short name, as the assistant's tools name it (`chrome:<name>`).
    pub name: String,
    /// What the person called it.
    pub label: String,
    /// The Google account it is signed in to.
    #[serde(default)]
    pub email: Option<String>,
    /// The machine it runs on.
    #[serde(default)]
    pub machine: Option<String>,
    /// That machine's system (`mac`, `linux`...).
    #[serde(default)]
    pub os: Option<String>,
    /// The extension's version.
    #[serde(default)]
    pub version: Option<String>,
    /// Chrome's own version, as its user agent gives it (`Chrome/131.0.0.0`).
    #[serde(default)]
    pub browser: Option<String>,
    /// A newer extension is out ([`crate::state::State::chrome_latest`]).
    #[serde(default)]
    pub outdated: bool,
    /// Online now.
    #[serde(default)]
    pub connected: bool,
    /// When it was last online.
    #[serde(default)]
    pub seen: Option<Timestamp>,
    /// When it was paired (`null` from a server that doesn't know).
    #[serde(default)]
    pub created: Option<Timestamp>,
    /// The extension's id.
    #[serde(default)]
    pub ext: Option<String>,
    /// That id is an old copy of the extension, from another folder.
    #[serde(default)]
    pub old_copy: bool,
    /// The assistant may work there now.
    #[serde(default)]
    pub grant: Option<ChromeGrant>,
    /// The person's pause there.
    #[serde(default)]
    pub paused: Option<ChromePause>,
    /// Who drives it.
    #[serde(default)]
    pub control: ChromeControl,
    /// What the assistant handed over for, while the person drives.
    #[serde(default)]
    pub handoff: Option<String>,
    /// The page on screen there.
    #[serde(default)]
    pub url: String,
    /// Its title.
    #[serde(default)]
    pub title: String,
    /// Who is driving it, in words.
    #[serde(default)]
    pub driver: Option<String>,
    /// The task the assistant works on there.
    #[serde(default)]
    pub task: Option<TaskId>,
    /// The tasks that use this profile by default.
    #[serde(default, deserialize_with = "null_as_default")]
    pub tasks: Vec<TaskId>,
}

impl ChromeProfile {
    /// Chrome's major version (`131`), from its user agent.
    #[must_use]
    pub fn chrome_major(&self) -> Option<&str> {
        let version = self.browser.as_deref()?.strip_prefix("Chrome/")?;
        version.split('.').next().filter(|major| !major.is_empty())
    }
}

/// A pairing link: open it in the profile to pair, once, before `until`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ChromePairing {
    /// Its code, as shown.
    pub code: ChromeCode,
    /// The link.
    pub link: String,
    /// The profile's name, as the person gave it.
    pub label: String,
    /// Its short name.
    pub name: String,
    /// The paired profile it pairs again; none for a new one.
    #[serde(default)]
    pub device: Option<ChromeProfileId>,
    /// When the link stops working.
    pub until: Timestamp,
    /// Used: its profile is paired.
    #[serde(default)]
    pub paired: bool,
}

/// `POST /api/chrome`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "validate", derive(garde::Validate))]
pub enum ChromeAction {
    /// A link for a new profile: `{ kind: "add", label }`. The answer carries it.
    Add {
        /// The profile's name (Work, Personal...).
        #[cfg_attr(feature = "validate", garde(custom(crate::rules::chrome_label)))]
        label: String,
    },
    /// A new link for a paired profile: `{ kind: "repair", id }`.
    Repair {
        /// The profile.
        #[cfg_attr(feature = "validate", garde(dive))]
        id: ChromeProfileId,
    },
    /// Rename a profile: `{ kind: "rename", id, label }`.
    Rename {
        /// The profile.
        #[cfg_attr(feature = "validate", garde(dive))]
        id: ChromeProfileId,
        /// Its new name.
        #[cfg_attr(feature = "validate", garde(custom(crate::rules::chrome_label)))]
        label: String,
    },
    /// Unpair a profile: `{ kind: "unpair", id }`.
    Unpair {
        /// The profile.
        #[cfg_attr(feature = "validate", garde(dive))]
        id: ChromeProfileId,
    },
    /// Cancel a link not used yet: `{ kind: "cancel", code }`.
    Cancel {
        /// The link's code.
        #[cfg_attr(feature = "validate", garde(skip))]
        code: ChromeCode,
    },
    /// Pause the assistant in a connected profile: `{ kind: "pause", id, minutes? }`, for 1 to
    /// 10 minutes (10 when left out or 0). `kind: "stop"`, an older client's word, reads as
    /// this.
    #[serde(alias = "stop")]
    Pause {
        /// The profile.
        #[cfg_attr(feature = "validate", garde(dive))]
        id: ChromeProfileId,
        /// For how long.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "validate", garde(range(max = 10)))]
        minutes: Option<u8>,
    },
    /// Hand a connected profile back to the assistant: `{ kind: "handback", id }`.
    Handback {
        /// The profile.
        #[cfg_attr(feature = "validate", garde(dive))]
        id: ChromeProfileId,
    },
}

/// `POST /api/chrome`'s answer: `ok`, the profiles and links as they are now, and the new
/// link for `add` and `repair`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ChromeReply {
    /// Always `true`.
    pub ok: bool,
    /// The paired profiles.
    #[serde(default)]
    pub chrome: Vec<ChromeProfile>,
    /// The links.
    #[serde(default)]
    pub chrome_pairing: Vec<ChromePairing>,
    /// The extension's newest version (`0` when the server doesn't know it), as in
    /// [`crate::state::State::chrome_latest`].
    #[serde(default)]
    pub chrome_latest: Option<String>,
    /// The link asked for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pairing: Option<ChromePairing>,
}

/// The browser extension's protocol (`apps/extension`) on `/api/chrome/ws`.
///
/// The extension's first message (`pair` or `hello`) carries `"protocol": 1`
/// ([`PROTOCOL`](crate::chrome::extension::PROTOCOL)) in its params. Besides the names here, the
/// socket carries `pair`, `hello`, `ping`, `pong`, `extension.initialized` and the relayed
/// `chrome.*` calls and events.
pub mod extension {
    /// The protocol version a first message names.
    pub const PROTOCOL: u64 = 1;
    /// The extension's fixed id: its manifest's `key` gives it (Chrome's rule, in
    /// `emberhut_core::chrome::extension_id`). A zip whose key gives another id is not this
    /// extension.
    pub const ID: &str = "omnhmmnnbibfdohdhcbcomdhbgpehfnj";
    /// Its page on the Chrome Web Store, where every Chrome profile adds it from (loading it
    /// unpacked is the developers' way).
    pub const STORE_PAGE: &str =
        "https://chromewebstore.google.com/detail/omnhmmnnbibfdohdhcbcomdhbgpehfnj";
    /// The extension's zip's file name, before its version.
    pub const ZIP_NAME: &str = "emberhut-extension";

    /// Server to extension: start working (attach the assistant's tabs, report them).
    pub const START: &str = "emberhut.start";
    /// Server to extension, after its hello: its name, label and the latest version.
    pub const HELLO: &str = "emberhut.hello";
    /// Server to extension: paired, with its token.
    pub const PAIRED: &str = "emberhut.paired";
    /// Server to extension: the owner paused the assistant there (`why`, `minutes`).
    pub const PAUSE: &str = "emberhut.pause";
    /// Server to extension: the pause is over.
    pub const RESUME: &str = "emberhut.resume";
    /// Server to extension: its new name.
    pub const NAME: &str = "emberhut.name";
    /// Server to extension: a step the assistant took (its popup lists them).
    pub const STEP: &str = "emberhut.step";
    /// Server to extension: the assistant handed the browser to the owner.
    pub const HANDOFF: &str = "emberhut.handoff";
    /// Both ways: the owner's turn is over.
    pub const HANDBACK: &str = "emberhut.handback";
    /// Server to extension: its folder holds `version` now; it reloads itself from it once it
    /// holds no tab.
    pub const UPDATE: &str = "emberhut.update";
    /// Extension to server: the owner paused it (`why`, `until`, `turn`), or it's over (`null`).
    pub const STOPPED: &str = "emberhut.stopped";
    /// Extension to server: the owner unpaired it there.
    pub const UNPAIR: &str = "emberhut.unpair";
    /// Extension to server: Chrome cut the link to a tab (another extension's frame), still
    /// open.
    pub const CUT: &str = "emberhut.cut";
    /// Extension to server: a tab went to a blocked site; the assistant let go of it.
    pub const BLOCKED: &str = "emberhut.blocked";
    /// Extension to server: Chrome won't let the extension into a tab yet (another extension's
    /// frame is in the page); it offers the tab again.
    pub const FRAMED: &str = "emberhut.framed";
    /// Who closed a tab, or handed back: the assistant itself.
    pub const BY_ASSISTANT: &str = "assistant";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn actions_are_tagged_by_kind() {
        let pause = ChromeAction::Pause {
            id: ChromeProfileId::new(2),
            minutes: None,
        };
        assert_eq!(
            serde_json::to_string(&pause).unwrap(),
            r#"{"kind":"pause","id":2}"#
        );
        let cancel: ChromeAction =
            serde_json::from_str(r#"{"kind":"cancel","code":"AB12"}"#).unwrap();
        assert_eq!(
            cancel,
            ChromeAction::Cancel {
                code: ChromeCode::new("AB12")
            }
        );
    }

    #[test]
    fn chromes_major_version_comes_from_its_user_agent() {
        let mut profile: ChromeProfile = serde_json::from_str(
            r#"{"id":1,"name":"work","label":"Work","created":"2026-01-01T00:00:00.000Z",
                "browser":"Chrome/131.0.6778.85","tasks":null}"#,
        )
        .unwrap();
        assert_eq!(profile.chrome_major(), Some("131"));
        assert_eq!(profile.tasks, []);
        profile.browser = Some("Firefox/1".into());
        assert_eq!(profile.chrome_major(), None);
    }
}
