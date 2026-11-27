//! The assistant's browser in a window of its own (the sidebar's Browser, `emberhut://browser`):
//! the whole screen of the browser on the server's machine, live, with a toolbar above it
//! (back, forward, reload, the address, the tabs, the browser's own controls, and Take over or
//! Give back), who drives over the page, and under it what the assistant asked and a file the
//! page wants. While an agent drives the person watches; Take over gives them the mouse,
//! keyboard and clipboard and the agents wait; Give back (⌘Esc) wakes the assistant.
//!
//! On macOS the page is the live stream in a web view (`stream`). Elsewhere (no web view in
//! this build) it is the page's latest picture while someone else drives, and while this device
//! drives the toolbar works and the live view opens in the default browser.

mod bar;
mod page;
#[cfg_attr(
    all(not(target_os = "macos"), not(test)),
    expect(dead_code, reason = "the policy is the macOS web view's")
)]
mod policy;
#[cfg_attr(
    all(not(target_os = "macos"), not(test)),
    expect(dead_code, reason = "the scripts run in the macOS web view only")
)]
mod scripts;
pub(crate) mod sidebar;
#[cfg(target_os = "macos")]
mod stream;
pub(crate) mod thumb;

use std::rc::Rc;

use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::prelude::*;
use gpui_kit::{App, Entity, FocusHandle, KeyBinding, PathPromptOptions, Window, actions, div, px};

use crate::assets::UI_FONT;
use crate::shell::WindowKind;
use crate::shell::windows;
use crate::store::Store;
use crate::store::browser::{Showing, shown};
use crate::ui::palette::pal;

/// The browser window's key context.
const CONTEXT: &str = "BrowserWindow";

actions!(
    browser,
    [
        /// Gives the browser back to the assistant (while this device drives it).
        GiveBack,
    ]
);

/// Binds the browser window's shortcuts.
pub(crate) fn bind_keys(cx: &mut App) {
    #[cfg(target_os = "macos")]
    cx.bind_keys([KeyBinding::new("cmd-escape", GiveBack, Some(CONTEXT))]);
    #[cfg(not(target_os = "macos"))]
    cx.bind_keys([KeyBinding::new("ctrl-escape", GiveBack, Some(CONTEXT))]);
}

/// Makes the Browser window available: the sidebar lists it and `emberhut://browser` opens it.
pub(crate) fn register(cx: &mut App) {
    windows::register_view(
        WindowKind::Browser,
        Rc::new(|store, window, cx| cx.new(|cx| BrowserWindow::new(store, window, cx)).into()),
        cx,
    );
}

/// The browser window's root view.
pub(crate) struct BrowserWindow {
    store: Entity<Store>,
    /// The address field, while this device drives.
    address: Entity<InputState>,
    /// The address field has the keyboard: it shows the whole address.
    editing: bool,
    /// The site the field showed last while not being edited.
    shown_site: String,
    focus: FocusHandle,
    /// The live view, on macOS.
    #[cfg(target_os = "macos")]
    stream: Option<stream::StreamWeb>,
    /// The live view couldn't be made for the session: its pictures show instead.
    #[cfg(target_os = "macos")]
    stream_failed: bool,
}

impl BrowserWindow {
    /// The window over `store`. Opening it claims a browser the assistant handed over.
    fn new(store: Entity<Store>, window: &mut Window, cx: &mut Context<'_, Self>) -> Self {
        let address =
            cx.new(|cx| InputState::new(window, cx).placeholder("Search or enter website"));
        cx.subscribe_in(&address, window, Self::address_event)
            .detach();
        cx.observe_in(&store, window, |this, _, window, cx| {
            this.follow_site(window, cx);
            cx.notify();
        })
        .detach();
        let closing = store.clone();
        cx.on_release(move |_, cx| closing.update(cx, Store::close_browser))
            .detach();
        // The live view is native, drawn above the window's own content, so the app lock's
        // cover (which takes the window's place) drops it rather than leaving it on screen; the
        // first frame after unlocking makes it again.
        #[cfg(target_os = "macos")]
        cx.observe_global::<crate::lock::Lock>(|this, cx| {
            if crate::lock::state(cx).covered() && this.stream.take().is_some() {
                cx.notify();
            }
        })
        .detach();
        store.update(cx, |store, cx| store.open_browser(showing(), cx));
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        Self {
            store,
            address,
            editing: false,
            shown_site: String::new(),
            focus,
            #[cfg(target_os = "macos")]
            stream: None,
            #[cfg(target_os = "macos")]
            stream_failed: false,
        }
    }

    /// The address field: editing shows the whole address, Return goes there, and leaving it
    /// shows the site again.
    fn address_event(
        &mut self,
        input: &Entity<InputState>,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        match event {
            InputEvent::Focus => {
                self.editing = true;
                let whole = self.store.read(cx).browser().map(shown::address);
                input.update(cx, |input, cx| {
                    input.set_value(whole.unwrap_or_default(), window, cx);
                });
            }
            InputEvent::Blur => {
                self.editing = false;
                self.shown_site.clear();
                self.follow_site(window, cx);
            }
            InputEvent::PressEnter { .. } => {
                let typed = input.read(cx).value().trim().to_owned();
                if !typed.is_empty() {
                    self.act(emberhut_api::browser::BrowserInput::Go { url: typed }, cx);
                }
                window.focus(&self.focus, cx);
            }
            InputEvent::Change => {}
        }
    }

    /// Shows the site in the address field when it changed and the field isn't being edited.
    fn follow_site(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.editing {
            return;
        }
        let site = self
            .store
            .read(cx)
            .browser()
            .map_or_else(String::new, shown::site);
        if site != self.shown_site {
            self.shown_site.clone_from(&site);
            self.address
                .update(cx, |input, cx| input.set_value(site, window, cx));
        }
    }

    /// Sends `input` to the browser.
    fn act(&self, input: emberhut_api::browser::BrowserInput, cx: &mut Context<'_, Self>) {
        self.store
            .update(cx, |store, cx| store.browser_act(input, cx));
    }

    /// Take over on this device.
    fn take_over(&mut self, cx: &mut Context<'_, Self>) {
        self.store.update(cx, Store::take_over_browser);
    }

    /// Give it back.
    fn give_back(&mut self, _: &GiveBack, _: &mut Window, cx: &mut Context<'_, Self>) {
        self.store.update(cx, Store::give_back_browser);
    }

    /// The page wants a file: this device's own file picker, and the file into the page.
    fn choose_file(&mut self, cx: &mut Context<'_, Self>) {
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Send".into()),
        });
        let store = self.store.clone();
        cx.spawn(async move |_, cx| {
            let path = match picked.await {
                Ok(Ok(Some(paths))) => paths.into_iter().next(),
                Ok(Ok(None)) => None,
                Ok(Err(error)) => {
                    log::warn!("the file picker failed: {error}");
                    None
                }
                Err(error) => {
                    log::warn!("the file picker went away: {error}");
                    None
                }
            };
            if let Some(path) = path {
                store.update(cx, |store, cx| store.send_to_browser(path, cx));
            }
        })
        .detach();
    }
}

/// How this build shows the page: the live stream where it has a web view.
fn showing() -> Showing {
    if cfg!(target_os = "macos") {
        Showing::Stream
    } else {
        Showing::Stills
    }
}

impl Render for BrowserWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let p = pal(cx);
        let bar = bar::bar(self, &p, cx);
        let page = page::page(self, &p, window, cx);
        div()
            .id("browser-window")
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::give_back))
            .size_full()
            .flex()
            .flex_col()
            .bg(p.background)
            .text_color(p.text)
            .font_family(UI_FONT)
            .text_size(px(13.))
            .child(bar)
            .child(page)
    }
}
