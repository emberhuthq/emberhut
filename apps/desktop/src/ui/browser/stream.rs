//! The live view in a `WKWebView` (gpui-webview over Wry), macOS only: the stream's own page,
//! signed in with the session's user and password, nothing of it kept on this device (a
//! private data store), the scripts of [`super::scripts`] ahead of its own, and while this
//! device drives, its clipboard bridged to this device's (for the main frame's bridge only, on
//! the stream's own address).
//!
//! Unlike a report's page (docs/adr/0006-report-webview.md), this one is the stream's, from the
//! address the server gives on the person's own network, and it needs what the report's guard
//! takes away: `WebRTC` for the picture, the keyboard and the mouse. It may not navigate away
//! from the stream's address, open windows or download, and a dropped file isn't loaded. It asks
//! for no camera or microphone, and the app declares neither, so macOS gives it neither.

use emberhut_api::browser::BrowserSession;
use gpui_kit::{
    AnyElement, App, AppContext as _, ClipboardItem, Entity, IntoElement as _, Task, Window,
};
use gpui_webview::WebView;
use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_web_kit::{WKWebViewConfiguration, WKWebsiteDataStore};
use wry::{
    NewWindowResponse, WebViewBuilder, WebViewBuilderExtDarwin as _, WebViewBuilderExtMacos as _,
};

use super::policy;
use super::scripts::{self, PageMessage, Token};

/// Why there is no web view for the live view.
#[derive(Debug, thiserror::Error)]
pub(super) enum Error {
    /// The session has no address a web view may load.
    #[error("the live view has no web address")]
    NoAddress,
    /// The address isn't `https`: its sign-in would cross the network in the clear.
    #[error("the live view's address isn't https, so its sign-in isn't sent")]
    NotSecure,
    /// Not on the main thread, where the native views are made.
    #[error("a web view can only be made on the main thread")]
    NotMainThread,
    /// Wry couldn't make it.
    #[error("the web view couldn't be made")]
    Build(#[source] wry::Error),
}

/// The live view for one session. Dropping it lets the native view go.
pub(super) struct StreamWeb {
    view: Entity<WebView>,
    /// The session it shows: a new session (control changed) is a new view.
    session: BrowserSession,
    /// Answers the page's clipboard calls; it ends with the web view.
    _bridge: Task<()>,
}

impl StreamWeb {
    /// The live view of `session`, in a new web view in `window`.
    ///
    /// # Errors
    ///
    /// When the session has no address, or the web view can't be made.
    pub(super) fn open(
        session: &BrowserSession,
        window: &mut Window,
        cx: &mut App,
    ) -> Result<Self, Error> {
        if session.url.is_some() && !session.is_secure() {
            return Err(Error::NotSecure);
        }
        let signed = session.signed_url().ok_or(Error::NoAddress)?;
        let home = session
            .url
            .as_deref()
            .and_then(policy::origin)
            .ok_or(Error::NoAddress)?;
        let main_thread = MainThreadMarker::new().ok_or(Error::NotMainThread)?;
        let bridge = session.is_control().then(Token::new).flatten();
        let scripts = scripts::for_session(session, bridge.as_ref());
        let bridge_home = home.clone();
        // WebKit calls the handlers on the main thread, possibly mid-update, so they never touch
        // the app: the page's messages go through this channel to a task of their own.
        let (to_app, from_page) = smol::channel::unbounded::<PageMessage>();
        let mut builder = WebViewBuilder::new()
            .with_webview_configuration(configuration(main_thread))
            .with_url(signed)
            .with_devtools(cfg!(debug_assertions))
            .with_allow_link_preview(false)
            .with_ipc_handler(move |request| {
                // The frame that posted it must be the stream's own, and the message the
                // bridge's (its token).
                let from = request.uri().to_string();
                if policy::origin(&from).as_deref() != Some(bridge_home.as_str()) {
                    log::info!("a message from outside the stream was dropped");
                    return;
                }
                if let Some(token) = &bridge
                    && let Some(message) = PageMessage::parse(request.body(), token)
                    && to_app.try_send(message).is_err()
                {
                    log::debug!("the live view's message came after it closed");
                }
            })
            .with_navigation_handler(move |url| {
                let allowed = policy::stays_on(&home, &url);
                if !allowed {
                    log::info!("the live view may not leave the stream");
                }
                allowed
            })
            .with_new_window_req_handler(|_, _| NewWindowResponse::Deny)
            .with_download_started_handler(|_, _| false)
            // True: Wry keeps the drop, so WebKit never loads a dropped file.
            .with_drag_drop_handler(|_| true);
        for script in scripts {
            builder = builder.with_initialization_script_for_main_only(script, true);
        }
        let web = WebView::build(builder, window, cx).map_err(Error::Build)?;
        let view = cx.new(|_| web);
        let page = view.downgrade();
        let bridge = cx.spawn(async move |cx| {
            while let Ok(message) = from_page.recv().await {
                let page = page.clone();
                cx.update(|cx| match message {
                    PageMessage::Write { text } => {
                        cx.write_to_clipboard(ClipboardItem::new_string(text));
                    }
                    PageMessage::Read { id } => {
                        let text = cx
                            .read_from_clipboard()
                            .and_then(|item| item.text())
                            .unwrap_or_default();
                        let Some(page) = page.upgrade() else {
                            return;
                        };
                        let answer = scripts::clipboard_answer(id, &text);
                        if let Err(error) = page.read(cx).raw().evaluate_script(&answer) {
                            log::warn!("the live view didn't get the clipboard: {error}");
                        }
                    }
                });
            }
        });
        Ok(Self {
            view,
            session: session.clone(),
            _bridge: bridge,
        })
    }

    /// Whether this view shows `session`.
    #[must_use]
    pub(super) fn shows(&self, session: &BrowserSession) -> bool {
        &self.session == session
    }

    /// The web view, to place in the window.
    pub(super) fn element(&self) -> AnyElement {
        self.view.clone().into_any_element()
    }
}

/// The web view's `WebKit` configuration: storage is private, so nothing of the session (its
/// sign-in, the stream's settings) is kept on this device.
#[expect(
    unsafe_code,
    reason = "objc2-web-kit marks these three public WebKit calls unsafe: two constructors \
              (the configuration, the non-persistent data store) and one property setter, all \
              on the main thread (the marker proves it) on objects made here"
)]
fn configuration(main_thread: MainThreadMarker) -> Retained<WKWebViewConfiguration> {
    // SAFETY: see the `expect` above: main thread, live objects made here, documented public
    // API only.
    unsafe {
        let configuration = WKWebViewConfiguration::new(main_thread);
        configuration.setWebsiteDataStore(&WKWebsiteDataStore::nonPersistentDataStore(main_thread));
        configuration
    }
}
