//! The app around its windows: the main windows and the windows of their own over the one
//! store ([`windows`]), where the app is for the user ([`presence`]), and the way in from
//! outside: links the system hands over and the command line's place, both taken to a
//! [`Target`] ([`go`]). On macOS the app outlives its last window, as Mac apps do, and the
//! Dock icon opens a main window again; elsewhere closing the last window quits.

mod kind;
pub(crate) mod presence;
pub(crate) mod restore;
pub(crate) mod windows;

use gpui_kit::{App, AsyncApp};
use smol::channel::{Receiver, Sender};

pub(crate) use self::kind::WindowKind;
use crate::links::{self, Target};
use crate::store::Page;
use crate::ui::window::MainWindow;

/// The links the system hands the app, possibly before its windows exist (a link that
/// launched it): sent here as they come, taken by [`follow_links`].
#[derive(Clone, Debug)]
pub(crate) struct LinkSender(Sender<Vec<String>>);

/// The receiving end of [`LinkSender`].
#[derive(Debug)]
pub(crate) struct LinkReceiver(Receiver<Vec<String>>);

/// A channel for the system's links.
#[must_use]
pub(crate) fn link_channel() -> (LinkSender, LinkReceiver) {
    let (sender, receiver) = smol::channel::unbounded();
    (LinkSender(sender), LinkReceiver(receiver))
}

impl LinkSender {
    /// Hands over the links the system opened the app with.
    pub(crate) fn send(&self, urls: Vec<String>) {
        if self.0.try_send(urls).is_err() {
            log::debug!("a link came after the app stopped taking them");
        }
    }
}

/// Takes the app to each link the system hands over, for as long as the app runs.
pub(crate) fn follow_links(links: LinkReceiver, cx: &mut App) {
    cx.spawn(async move |cx: &mut AsyncApp| {
        while let Ok(urls) = links.0.recv().await {
            cx.update(|cx| {
                for url in urls {
                    open_link(&url, cx);
                }
            });
        }
    })
    .detach();
}

/// Takes the app to `url`, if it is one of the app's links (another is logged and left).
pub(crate) fn open_link(url: &str, cx: &mut App) {
    match links::parse(url) {
        Ok(target) => go(&target, cx),
        Err(error) => log::warn!("not opening {url}: {error}"),
    }
}

/// Takes the app to `target`: the main window used last comes forward (one opens if none is
/// open) showing it, or the window of its own it names opens.
pub(crate) fn go(target: &Target, cx: &mut App) {
    match *target {
        Target::Chat(None) => windows::with_main(cx, |_, _, _| {}),
        Target::Chat(Some(room)) => {
            windows::with_main(cx, |main, window, cx| {
                main.open(Page::Room(room), window, cx);
            });
        }
        Target::Page(page) => {
            windows::with_main(cx, |main, window, cx| main.push(page, window, cx));
        }
        Target::Task(task) => {
            windows::with_main(cx, |main, window, cx| main.open_task(task, window, cx));
        }
        Target::Talk(ref text) => talk_in_main(text.clone(), cx),
        Target::Window(kind) => open_window(kind, cx),
        Target::Party(party) => {
            crate::ui::party::open_party(emberhut_api::ids::PartyId::new(party), cx);
        }
    }
}

/// Opens the window of its own `kind`; without one in this build, the main window comes
/// forward instead (the link isn't lost on a closed app).
fn open_window(kind: WindowKind, cx: &mut App) {
    if !windows::open_single(kind, cx) {
        log::info!("this build has no {} window", kind.title());
        windows::with_main(cx, |_, _, _| {});
    }
}

/// Talk from a window of its own (or an `emberhut://chat?text=` link): the main window comes forward
/// with `text` started in its chat's composer.
pub(crate) fn talk_in_main(text: String, cx: &mut App) {
    windows::with_main(cx, move |main, window, cx| {
        main.start_message(&text, window, cx);
    });
}

/// Talk pressed in `window` once its words are started in the store
/// ([`crate::store::Store::start_message`]): from a window of its own (a task's page in the
/// Sessions window), the main window comes forward with its composer focused, so its chat shows
/// them; in a main window the chat beside the page shows them already.
pub(crate) fn talk_shown(window: &gpui_kit::Window, cx: &mut App) {
    if windows::is_main(window.window_handle(), cx) {
        return;
    }
    // After the click's own window is done with: the main window is updated next.
    cx.defer(|cx| windows::with_main(cx, MainWindow::focus_composer));
}

#[cfg(test)]
mod tests;
