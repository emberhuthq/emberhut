//! The browser's latest picture (`GET /api/browser/frame.jpg`), every few seconds while
//! something shows it: the browser's window where it has no web view, and the card of a task
//! being browsed for on Needs you. While the person drives, the server sends none (a password
//! may be on screen), and none is asked for.

use std::sync::Arc;
use std::time::Duration;

use emberhut_api::browser::{BrowserControl, BrowserInfo};
use emberhut_api::paths;
use emberhut_api::state::State;
use gpui_kit::AppContext as _;
use gpui_kit::{App, Context, Image, ImageFormat, RenderImage};

use super::Showing;
use super::shown::is_browsing;
use crate::store::poll::Kicked;
use crate::store::presence::Presence;
use crate::store::{Page, Store, update_or_log};

/// How often the picture is asked for while it shows (the reference client's card: every 3 s).
pub(super) const EVERY: Duration = Duration::from_secs(3);

/// Whether a picture is wanted now: something shows it (`window`: the browser's window
/// shows stills; `cards`: a card of the browsed task is on screen), the app is on screen, and
/// an agent, not the person, has the browser.
#[must_use]
pub(super) fn wanted(
    window: bool,
    cards: bool,
    presence: Presence,
    info: Option<&BrowserInfo>,
) -> bool {
    (window || cards)
        && presence != Presence::Hidden
        && info.is_some_and(|info| info.control != BrowserControl::Person)
}

/// Whether a card on Needs you shows the browsed task: one of the asks waiting on the person
/// is the browser's task, while someone drives.
#[must_use]
pub(super) fn on_a_card(state: &State) -> bool {
    state
        .working
        .iter()
        .any(|row| row.needs_you() && is_browsing(row, state.browser.as_ref()))
}

impl Store {
    /// Whether a picture of the browser is [`wanted`] now.
    fn frames_wanted(&self) -> bool {
        let window = self.browser.open == Some(Showing::Stills) && !self.browser_is_mine();
        let cards = self.shows_page(Page::Needs) && self.state.as_ref().is_some_and(on_a_card);
        wanted(window, cards, self.presence(), self.browser())
    }

    /// Starts the pictures' loop (once).
    pub(super) fn start_browser_frames(&mut self, cx: &mut Context<'_, Self>) {
        if let Some(kicked) = self.browser.kicked.take() {
            run(kicked, cx);
        }
    }

    /// Takes a fetched picture in: it replaces the last one; none (404: the person drives,
    /// or no picture yet) drops it.
    fn take_frame(&mut self, frame: Option<Arc<RenderImage>>, cx: &mut Context<'_, Self>) {
        let old = std::mem::replace(&mut self.browser.frame, frame);
        if let Some(old) = old {
            free(old, cx);
        }
        cx.notify();
    }
}

/// Frees a picture's tiles in every window, then its pixels, once no window is being updated.
pub(super) fn free(frame: Arc<RenderImage>, cx: &mut App) {
    cx.defer(move |cx| cx.drop_image(frame, None));
}

/// Runs the pictures' loop for the store in `cx`: a picture when one is [`wanted`], then
/// [`EVERY`] (or a kick) while it still is, and only a kick while it isn't.
fn run(kicked: Kicked, cx: &mut Context<'_, Store>) {
    let svg = cx.svg_renderer();
    cx.spawn(async move |this, cx| {
        loop {
            let Some(due) = update_or_log(&this, cx, |store, _| {
                store.frames_wanted().then(|| Arc::clone(&store.backend))
            }) else {
                return;
            };
            if let Some(backend) = due {
                let svg = svg.clone();
                let frame = cx
                    .background_spawn(async move {
                        let bytes = backend.file(paths::BROWSER_FRAME).ok()?;
                        Image::from_bytes(ImageFormat::Jpeg, bytes)
                            .to_image_data(svg)
                            .inspect_err(|error| {
                                log::debug!("the browser's picture didn't read: {error}");
                            })
                            .ok()
                    })
                    .await;
                update_or_log(&this, cx, |store, cx| store.take_frame(frame, cx));
            }
            let Some(again) = update_or_log(&this, cx, |store, _| store.frames_wanted()) else {
                return;
            };
            if again {
                let timer = cx.background_executor().timer(EVERY);
                smol::future::or(
                    async {
                        timer.await;
                    },
                    kicked.wait(),
                )
                .await;
            } else {
                kicked.wait().await;
            }
        }
    })
    .detach();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_picture_is_wanted_while_it_shows_and_an_agent_has_the_browser() {
        let agent = BrowserInfo::default();
        let person = BrowserInfo {
            control: BrowserControl::Person,
            ..BrowserInfo::default()
        };
        assert!(wanted(true, false, Presence::Front, Some(&agent)));
        assert!(wanted(false, true, Presence::Behind, Some(&agent)));
        assert!(
            !wanted(false, false, Presence::Front, Some(&agent)),
            "nothing shows it"
        );
        assert!(
            !wanted(true, true, Presence::Hidden, Some(&agent)),
            "out of sight"
        );
        assert!(
            !wanted(true, true, Presence::Front, Some(&person)),
            "the person drives"
        );
        assert!(!wanted(true, true, Presence::Front, None), "no browser");
    }
}
