//! The assistant's own browser: a real Chrome on the server's machine, streamed whole to the
//! person's devices. While the assistant (or a worker) drives, the person watches; when they take
//! over, the agents wait until they give it back.
//!
//! The state carries where it is and who drives ([`crate::state::State::browser`], a
//! [`BrowserInfo`]). `GET /api/browser/session?device=` ([`BrowserSessionQuery`]) gives the live
//! view's address and the password for what this device may do ([`BrowserSessionReply`]);
//! `POST /api/browser/input` ([`BrowserInput`]) takes over, gives back, and drives the toolbar
//! while the person holds it ([`BrowserReply`]); `POST /api/browser/upload?name=` (the raw file
//! as the body, [`crate::uploads::UploadQuery`]) puts a file into the page's file field when the
//! page asks for one; `GET /api/browser/frame.jpg` is the latest picture of the page, for a card.
//!
//! A request the server can't do (input before taking over, an upload nobody asked for) is
//! refused with 409 and an [`crate::error::ErrorBody`]; a session on a machine without the live
//! view is refused with 503 and one too.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::ids::{BrowserDeviceId, BrowserTabId, TaskId};
use crate::open_enum::open_enum;
use crate::time::Timestamp;

open_enum! {
    /// Who drives the browser now.
    #[derive(Default)]
    pub enum BrowserControl {
        /// The assistant or one of its workers ([`BrowserInfo::driver`] says which).
        #[default]
        Agent = "agent",
        /// The person: they took it over, or the assistant handed it to them (its word is in
        /// [`crate::wire`]).
        Person = crate::wire::PERSON,
    }
}

/// One of the browser's tabs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct BrowserTab {
    /// Its number, for switching to it or closing it.
    pub id: BrowserTabId,
    /// Its address.
    #[serde(default)]
    pub url: String,
    /// Its title.
    #[serde(default)]
    pub title: String,
    /// It is the one on screen.
    #[serde(default)]
    pub active: bool,
}

/// The page asks for a file (its upload field was clicked): the browser's own dialog is held
/// back, and the person's app offers its file picker instead.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct BrowserFileRequest {
    /// The field takes more than one file.
    #[serde(default)]
    pub multiple: bool,
    /// When the page asked.
    pub at: Timestamp,
}

/// Where the browser is and who drives it (the state's `browser`, and the answer of every
/// `/api/browser/*` change).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct BrowserInfo {
    /// The server is connected to the browser.
    #[serde(default)]
    pub connected: bool,
    /// The address of the page on screen.
    #[serde(default)]
    pub url: String,
    /// Its title.
    #[serde(default)]
    pub title: String,
    /// Who is driving: the assistant, one of its workers, or the person (`You`); none while
    /// nobody is.
    #[serde(default)]
    pub driver: Option<String>,
    /// The task the browsing is for.
    #[serde(default)]
    pub task: Option<TaskId>,
    /// Whether the assistant or the person has it.
    #[serde(default)]
    pub control: BrowserControl,
    /// Why the assistant handed it over, while it waits for the person.
    #[serde(default)]
    pub handoff: Option<String>,
    /// When control last changed.
    #[serde(default)]
    pub at: Option<Timestamp>,
    /// The tabs, the one on screen [`BrowserTab::active`].
    #[serde(default)]
    pub tabs: Vec<BrowserTab>,
    /// The browser's own controls (its tab strip, address bar, menus and extensions) are on
    /// screen; normally only the page is.
    #[serde(default, rename = "chromeUI")]
    pub chrome_ui: bool,
    /// Which of the person's devices drives: none, any (an app that doesn't say); empty, none
    /// yet (the assistant handed it over and the first device to open it takes it).
    #[serde(default)]
    pub device: Option<BrowserDeviceId>,
    /// The page asks for a file.
    #[serde(default)]
    pub file_request: Option<BrowserFileRequest>,
}

impl BrowserInfo {
    /// Whether the person drives it on `device`: they have it, and no other of their devices
    /// does (one controller at a time; their other devices watch).
    #[must_use]
    pub fn is_mine(&self, device: &BrowserDeviceId) -> bool {
        self.control == BrowserControl::Person
            && self.device.as_ref().is_none_or(|holder| holder == device)
    }

    /// Whether the assistant handed it over and none of the person's devices has taken it yet.
    #[must_use]
    pub fn is_unclaimed(&self) -> bool {
        self.control == BrowserControl::Person
            && self
                .device
                .as_ref()
                .is_some_and(|holder| holder.as_str().is_empty())
    }

    /// The tab on screen, if the server listed the tabs.
    #[must_use]
    pub fn shown_tab(&self) -> Option<&BrowserTab> {
        self.tabs.iter().find(|tab| tab.active)
    }
}

/// `POST /api/browser/input`'s answer: the browser as it is after.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct BrowserReply {
    /// Always `true`: a refusal is an error body.
    pub ok: bool,
    /// The browser.
    #[serde(flatten)]
    pub browser: BrowserInfo,
}

/// `POST /api/browser/input`: take over, give back, and while the person drives, the toolbar.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "validate", derive(garde::Validate))]
pub enum BrowserInput {
    /// The person takes it over on `device`; the agents wait: `{ kind: "takeover", device }`.
    Takeover {
        /// The device that drives (the server keeps its first 64 characters).
        #[cfg_attr(feature = "validate", garde(skip))]
        device: BrowserDeviceId,
    },
    /// The person gives it back, which wakes the assistant: `{ kind: "handback" }`.
    Handback,
    /// Back in the tab on screen.
    Back,
    /// Forward in the tab on screen.
    Forward,
    /// Reload the tab on screen.
    Reload,
    /// Go to what was typed in the address field: `{ kind: "go", url }`. The server makes an
    /// address of a site's name, and a search of anything else.
    Go {
        /// What was typed.
        #[cfg_attr(feature = "validate", garde(skip))]
        url: String,
    },
    /// Bring a tab on screen: `{ kind: "tab", id }`.
    Tab {
        /// The tab.
        #[cfg_attr(feature = "validate", garde(dive))]
        id: BrowserTabId,
    },
    /// Open a new tab: `{ kind: "newtab" }`.
    #[serde(rename = "newtab")]
    NewTab,
    /// Close a tab: `{ kind: "closetab", id }`.
    #[serde(rename = "closetab")]
    CloseTab {
        /// The tab.
        #[cfg_attr(feature = "validate", garde(dive))]
        id: BrowserTabId,
    },
    /// Show or hide the browser's own controls: `{ kind: "chrome", show }`.
    Chrome {
        /// Show them.
        #[cfg_attr(feature = "validate", garde(skip))]
        show: bool,
    },
}

/// `GET /api/browser/session?device=`: which device asks, so the server knows whether it
/// drives.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema, utoipa::IntoParams), into_params(parameter_in = Query))]
#[cfg_attr(feature = "validate", derive(garde::Validate))]
pub struct BrowserSessionQuery {
    /// This device.
    #[cfg_attr(feature = "validate", garde(skip))]
    pub device: BrowserDeviceId,
}

impl BrowserSessionQuery {
    /// The query string, without the `?`: `device=` and the id percent-encoded.
    ///
    /// ```
    /// use emberhut_api::browser::BrowserSessionQuery;
    /// use emberhut_api::ids::BrowserDeviceId;
    ///
    /// let query = BrowserSessionQuery { device: BrowserDeviceId::new("a b") };
    /// assert_eq!(query.to_query(), "device=a%20b");
    /// ```
    #[must_use]
    pub fn to_query(&self) -> String {
        let mut query = String::from("device=");
        crate::query::push_encoded(&mut query, self.device.as_str());
        query
    }
}

open_enum! {
    /// What the live view lets this device do.
    pub enum StreamMode {
        /// Its mouse, keyboard and clipboard reach the browser.
        Control = "control",
        /// It watches only.
        View = "view",
    }
}

/// The size of the server's virtual screen, in pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct BrowserScreen {
    /// Its width.
    pub width: u32,
    /// Its height.
    pub height: u32,
}

/// The way into the live view: its address on the person's network and the sign-in for what
/// this device may do. The stream itself checks the password, so a new session comes whenever
/// control changes.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct BrowserSession {
    /// The live view's address.
    #[serde(default)]
    pub url: Option<String>,
    /// The user name its sign-in takes.
    pub user: String,
    /// The password for [`Self::mode`].
    pub password: String,
    /// Control or view only.
    pub mode: StreamMode,
    /// The server's screen now; none when it couldn't tell.
    #[serde(default)]
    pub screen: Option<BrowserScreen>,
}

impl BrowserSession {
    /// The live view's address with the sign-in in it (`https://user:password@host/`, both
    /// percent-encoded), for a web view to sign in with; none without an address, or for one
    /// that isn't `https`: over plain `http` the password would cross the network in the clear
    /// ([`Self::is_secure`] tells which).
    ///
    /// ```
    /// use emberhut_api::browser::{BrowserSession, StreamMode};
    ///
    /// let session = BrowserSession {
    ///     url: Some("https://example.org/".to_owned()),
    ///     user: "viewer".to_owned(),
    ///     password: "p@ss word".to_owned(),
    ///     mode: StreamMode::View,
    ///     screen: None,
    /// };
    /// assert_eq!(
    ///     session.signed_url().as_deref(),
    ///     Some("https://viewer:p%40ss%20word@example.org/")
    /// );
    /// ```
    #[must_use]
    pub fn signed_url(&self) -> Option<String> {
        let url = self.url.as_deref()?;
        let (scheme, rest) = url.split_once("://")?;
        if !scheme.eq_ignore_ascii_case("https") {
            return None;
        }
        let mut signed = String::with_capacity(url.len() + self.user.len() + 32);
        signed.push_str(scheme);
        signed.push_str("://");
        crate::query::push_encoded(&mut signed, &self.user);
        signed.push(':');
        crate::query::push_encoded(&mut signed, &self.password);
        signed.push('@');
        signed.push_str(rest);
        Some(signed)
    }

    /// Whether the live view's address is `https`, so its sign-in may be sent to it.
    #[must_use]
    pub fn is_secure(&self) -> bool {
        self.url
            .as_deref()
            .and_then(|url| url.split_once("://"))
            .is_some_and(|(scheme, _)| scheme.eq_ignore_ascii_case("https"))
    }

    /// Whether this session lets the device drive.
    #[must_use]
    pub fn is_control(&self) -> bool {
        self.mode == StreamMode::Control
    }
}

/// The password stays out of logs.
impl fmt::Debug for BrowserSession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BrowserSession")
            .field("url", &self.url)
            .field("user", &self.user)
            .field("password", &"…")
            .field("mode", &self.mode)
            .field("screen", &self.screen)
            .finish()
    }
}

/// `GET /api/browser/session`'s answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct BrowserSessionReply {
    /// Always `true`: a refusal is an error body.
    pub ok: bool,
    /// The session.
    #[serde(flatten)]
    pub session: BrowserSession,
}

/// `GET /chrome/pair/status?code=`: how a pairing link stands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ChromePairStatus {
    /// `waiting`, `paired` or `expired`.
    pub state: String,
    /// The profile's label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Its short name, once paired.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Its Google account, once paired.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    /// Its machine, once paired.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub machine: Option<String>,
}

/// `GET /chrome/pair/status`'s query.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema, utoipa::IntoParams))]
#[cfg_attr(feature = "openapi", into_params(parameter_in = Query))]
pub struct ChromePairQuery {
    /// The pairing code.
    #[serde(default)]
    pub code: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(control: BrowserControl, device: Option<&str>) -> BrowserInfo {
        BrowserInfo {
            control,
            device: device.map(BrowserDeviceId::new),
            ..BrowserInfo::default()
        }
    }

    #[test]
    fn one_device_drives_and_the_others_watch() {
        let here = BrowserDeviceId::new("here");
        assert!(!info(BrowserControl::Agent, None).is_mine(&here));
        assert!(
            info(BrowserControl::Person, None).is_mine(&here),
            "any device"
        );
        assert!(info(BrowserControl::Person, Some("here")).is_mine(&here));
        assert!(!info(BrowserControl::Person, Some("other")).is_mine(&here));
        assert!(
            !info(BrowserControl::Person, Some("")).is_mine(&here),
            "handed over, not taken yet"
        );
    }

    #[test]
    fn handed_over_and_not_taken_is_unclaimed() {
        assert!(info(BrowserControl::Person, Some("")).is_unclaimed());
        assert!(!info(BrowserControl::Person, Some("here")).is_unclaimed());
        assert!(!info(BrowserControl::Person, None).is_unclaimed());
        assert!(!info(BrowserControl::Agent, Some("")).is_unclaimed());
    }

    #[test]
    fn a_session_signs_in_only_over_https() {
        let mut session = BrowserSession {
            url: Some("HTTPS://example.org:8447/".to_owned()),
            user: "viewer".to_owned(),
            password: "secret".to_owned(),
            mode: StreamMode::Control,
            screen: None,
        };
        assert_eq!(
            session.signed_url().as_deref(),
            Some("HTTPS://viewer:secret@example.org:8447/")
        );
        assert!(session.is_secure() && session.is_control());
        session.url = Some("http://example.org:8447/".to_owned());
        assert_eq!(session.signed_url(), None, "never in the clear");
        assert!(!session.is_secure());
        session.url = Some("file:///etc/passwd".to_owned());
        assert_eq!(session.signed_url(), None);
        session.url = None;
        assert_eq!(session.signed_url(), None);
        assert!(!session.is_secure());
        assert!(
            !format!("{session:?}").contains("secret"),
            "no password in logs"
        );
    }
}
