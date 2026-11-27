//! The assistant's browser on this device: whether its window is open, the live view's
//! session for this device (asked again whenever control changes, since the stream's password
//! decides what it lets the device do), taking it over and giving it back, its toolbar, a file
//! for its page, and its latest picture for a window without a web view and for the card of
//! the task it browses for.
//!
//! The browser's state itself is the server's ([`State::browser`](emberhut_api::state::State)),
//! polled with everything else; an input's answer replaces it at once.

mod device;
mod frames;
pub(crate) mod shown;

use std::path::PathBuf;
use std::sync::Arc;

use emberhut_api::browser::{BrowserControl, BrowserInfo, BrowserInput, BrowserSession};
use emberhut_api::ids::BrowserDeviceId;
use gpui_kit::AppContext as _;
use gpui_kit::{Context, RenderImage};

use super::poll::{self, Kicked, Kicker};
use super::{Store, update_or_log};
use crate::config::AppFiles;

/// How the open window shows the page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Showing {
    /// The live stream, in a web view.
    Stream,
    /// The latest picture, every few seconds (no web view here): watching only.
    Stills,
}

/// The live view's session for this device.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Link {
    /// Asked for, not here yet.
    Connecting,
    /// Here.
    Ready(BrowserSession),
    /// The server has none to give: its machine has no live view, or it couldn't be reached.
    Unavailable,
}

/// The browser's part of the store.
pub(crate) struct BrowserWork {
    /// This device, as the browser knows it.
    device: BrowserDeviceId,
    /// The window is open, and how it shows the page.
    open: Option<Showing>,
    /// The session for the window.
    link: Link,
    /// Whether this device drove when the session was asked for: when that changes, a new one
    /// is asked for.
    asked_for: Option<bool>,
    /// Numbers the sessions asked for, so a late answer to an older one is dropped.
    asks: u64,
    /// The file on its way to the page.
    sending: Option<String>,
    /// What the server said no to last, worded for the user.
    problem: Option<String>,
    /// The page's latest picture.
    frame: Option<Arc<RenderImage>>,
    /// Wakes the pictures' loop; the loop's end of it waits here until the loop starts.
    kicker: Kicker,
    kicked: Option<Kicked>,
}

impl BrowserWork {
    /// The browser's part for this device (its id kept in `files`).
    pub(crate) fn new(files: Option<&AppFiles>) -> Self {
        let (kicker, waiting) = poll::kicker();
        Self {
            device: device::load(files),
            open: None,
            link: Link::Connecting,
            asked_for: None,
            asks: 0,
            sending: None,
            problem: None,
            frame: None,
            kicker,
            kicked: Some(waiting),
        }
    }
}

impl Store {
    /// The assistant's browser, from a server that has one.
    #[must_use]
    pub(crate) fn browser(&self) -> Option<&BrowserInfo> {
        self.state.as_ref()?.browser.as_ref()
    }

    /// This device, as the browser knows it.
    #[must_use]
    pub(crate) fn browser_device(&self) -> &BrowserDeviceId {
        &self.browser.device
    }

    /// Whether this device drives the browser.
    #[must_use]
    pub(crate) fn browser_is_mine(&self) -> bool {
        self.browser()
            .is_some_and(|info| info.is_mine(&self.browser.device))
    }

    /// The live view's session for the open window.
    #[must_use]
    pub(crate) fn browser_link(&self) -> &Link {
        &self.browser.link
    }

    /// The file on its way into the page, by its name.
    #[must_use]
    pub(crate) fn browser_sending(&self) -> Option<&str> {
        self.browser.sending.as_deref()
    }

    /// What the server last said no to, worded for the user.
    #[must_use]
    pub(crate) fn browser_problem(&self) -> Option<&str> {
        self.browser.problem.as_deref()
    }

    /// The page's latest picture, while someone else drives.
    #[must_use]
    pub(crate) fn browser_frame(&self) -> Option<Arc<RenderImage>> {
        self.browser.frame.clone()
    }

    /// The browser's window opened, showing the page as `showing` says. If the assistant
    /// handed the browser over and none of the person's devices has it yet, this one takes it.
    pub(crate) fn open_browser(&mut self, showing: Showing, cx: &mut Context<'_, Self>) {
        self.browser.open = Some(showing);
        self.browser.problem = None;
        if self.browser().is_some_and(BrowserInfo::is_unclaimed) {
            self.take_over_browser(cx);
        }
        self.ask_browser_session(cx);
        self.browser_follows_state(cx);
    }

    /// The open window shows the page as `showing` says from now on (pictures, when its web
    /// view couldn't be made).
    #[cfg_attr(
        not(target_os = "macos"),
        expect(dead_code, reason = "only the macOS web view can fail to be made")
    )]
    pub(crate) fn browser_shows(&mut self, showing: Showing, cx: &mut Context<'_, Self>) {
        if self.browser.open.is_some() && self.browser.open != Some(showing) {
            self.browser.open = Some(showing);
            self.browser.kicker.kick();
            cx.notify();
        }
    }

    /// The browser's window closed: its session goes (nothing of it is kept).
    pub(crate) fn close_browser(&mut self, cx: &mut Context<'_, Self>) {
        self.browser.open = None;
        self.browser.link = Link::Connecting;
        self.browser.asked_for = None;
        self.browser.asks += 1;
        self.browser.kicker.kick();
        cx.notify();
    }

    /// Takes the browser over on this device: the agents wait.
    pub(crate) fn take_over_browser(&mut self, cx: &mut Context<'_, Self>) {
        let device = self.browser.device.clone();
        self.browser_act(BrowserInput::Takeover { device }, cx);
    }

    /// Gives the browser back, which wakes the assistant; only what this device holds.
    pub(crate) fn give_back_browser(&mut self, cx: &mut Context<'_, Self>) {
        if self.browser_is_mine() {
            self.browser_act(BrowserInput::Handback, cx);
        }
    }

    /// Sends `input` to the browser; its answer is the browser's state at once, and the next
    /// poll brings the rest.
    pub(crate) fn browser_act(&mut self, input: BrowserInput, cx: &mut Context<'_, Self>) {
        let backend = Arc::clone(&self.backend);
        cx.spawn(async move |this, cx| {
            let sent = input.clone();
            let reply = cx
                .background_spawn(async move { backend.browser_input(&sent) })
                .await;
            update_or_log(&this, cx, |store, cx| {
                store.browser_answered(reply, cx);
            });
        })
        .detach();
    }

    /// Sends the file at `path` into the page's file field, which asked for one.
    pub(crate) fn send_to_browser(&mut self, path: PathBuf, cx: &mut Context<'_, Self>) {
        if self.browser.sending.is_some() {
            return;
        }
        let name = path.file_name().map_or_else(
            || "file".to_owned(),
            |name| name.to_string_lossy().into_owned(),
        );
        self.browser.sending = Some(name.clone());
        self.browser.problem = None;
        cx.notify();
        let backend = Arc::clone(&self.backend);
        cx.spawn(async move |this, cx| {
            let reply = cx
                .background_spawn(async move {
                    let bytes = std::fs::read(&path)
                        .map_err(|error| crate::backend::Error::Local(Box::new(error)))?;
                    backend.browser_upload(&name, &bytes)
                })
                .await;
            update_or_log(&this, cx, |store, cx| {
                store.browser.sending = None;
                store.browser_answered(reply, cx);
            });
        })
        .detach();
    }

    /// An input's or an upload's answer: the browser as it is now, or what went wrong.
    fn browser_answered(
        &mut self,
        reply: Result<BrowserInfo, crate::backend::Error>,
        cx: &mut Context<'_, Self>,
    ) {
        match reply {
            Ok(info) => {
                self.browser.problem = None;
                if let Some(state) = &mut self.state {
                    state.browser = Some(info);
                }
                self.browser_follows_state(cx);
            }
            Err(error) => {
                log::warn!("the browser didn't take that: {error}");
                self.browser.problem = Some(problem(&error));
            }
        }
        self.kicker.kick();
        cx.notify();
    }

    /// A new state: a new session when control changed for this device while the window is
    /// open, and the pictures' loop told.
    pub(super) fn browser_follows_state(&mut self, cx: &mut Context<'_, Self>) {
        if self.browser.open.is_some() && self.browser.asked_for != Some(self.browser_is_mine()) {
            self.ask_browser_session(cx);
        }
        if self
            .browser()
            .is_some_and(|info| info.control == BrowserControl::Person)
            && let Some(frame) = self.browser.frame.take()
        {
            // The person may have a password on screen: no picture of it is kept.
            frames::free(frame, cx);
        }
        self.start_browser_frames(cx);
        self.browser.kicker.kick();
    }

    /// Asks for the live view's session for what this device may do now.
    fn ask_browser_session(&mut self, cx: &mut Context<'_, Self>) {
        self.browser.asked_for = Some(self.browser_is_mine());
        self.browser.asks += 1;
        let ask = self.browser.asks;
        let device = self.browser.device.clone();
        let backend = Arc::clone(&self.backend);
        cx.spawn(async move |this, cx| {
            let session = cx
                .background_spawn(async move { backend.browser_session(&device) })
                .await;
            update_or_log(&this, cx, |store, cx| {
                if store.browser.asks != ask {
                    return;
                }
                store.browser.link = match session {
                    Ok(session) => Link::Ready(session),
                    Err(error) => {
                        log::info!("no live view of the browser: {error}");
                        Link::Unavailable
                    }
                };
                cx.notify();
            });
        })
        .detach();
    }
}

/// What a refused input says: the server's own words, else what went wrong.
fn problem(error: &crate::backend::Error) -> String {
    error.server_message().map_or_else(
        || {
            if error.is_unreachable() {
                "Can't reach the server.".to_owned()
            } else {
                format!("That didn't work: {error}.")
            }
        },
        str::to_owned,
    )
}

#[cfg(test)]
mod tests;
