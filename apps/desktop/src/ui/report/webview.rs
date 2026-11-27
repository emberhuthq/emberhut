//! A report's real HTML in a `WKWebView` (gpui-webview over Wry), locked down as
//! docs/adr/0006-report-webview.md sets out: loaded from the string under the reference client's
//! Content-Security-Policy, the guard script first, windows only from the user's click, no
//! navigation away (links go to the default browser), no bridge to Rust, no custom protocol,
//! private storage, no autoplay, downloads or drops, devtools in debug builds only.

use std::cell::Cell;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use gpui_kit::{AnyElement, App, AppContext as _, Entity, IntoElement as _, Task, Window};
use gpui_webview::WebView;
use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_web_kit::{WKAudiovisualMediaTypes, WKWebViewConfiguration, WKWebsiteDataStore};
use wry::{
    NewWindowResponse, PageLoadEvent, Theme, WebViewBuilder, WebViewBuilderExtDarwin as _,
    WebViewBuilderExtMacos as _,
};

use super::policy::{NavigationPolicy, decide};
use super::{WEB_VIEW_ENV, guard, sandbox, web_view_off};

/// How long a page may take to finish loading before the log says so: a page that never loads
/// (or whose `WebKit` process died) stays blank, and this is the trace it leaves.
const SLOW_LOAD: Duration = Duration::from_secs(15);

/// Why there is no web view for the report.
#[derive(Debug, thiserror::Error)]
pub(super) enum Error {
    /// The environment turned web views off.
    #[error("web views are off ({WEB_VIEW_ENV}=off)")]
    Off,
    /// Not on the main thread, where the native views are made.
    #[error("a web view can only be made on the main thread")]
    NotMainThread,
    /// Wry couldn't make it.
    #[error("the web view couldn't be made")]
    Build(#[source] wry::Error),
}

/// A report's page in a web view. Dropping it lets the view go: GPUI drops it at the end of the
/// update, which hides its native view, and Wry takes the native view out of the window when the
/// last frame's handles to it are gone.
pub(super) struct ReportWeb {
    view: Entity<WebView>,
    /// The appearance the page was last given: dark or not.
    dark: Cell<Option<bool>>,
    /// Whether it was last shown (not hidden under the file viewer's sheet).
    shown: Cell<bool>,
    /// Opens in the default browser the links the page's handlers pass on; it ends with the web
    /// view (whose handlers hold the channel's senders) or when this is dropped.
    _links: Task<()>,
    /// Logs a page that hasn't finished loading after [`SLOW_LOAD`].
    _watch: Task<()>,
}

impl ReportWeb {
    /// The page `html` in a new web view in `window`.
    ///
    /// # Errors
    ///
    /// When web views are off, or one can't be made.
    pub(super) fn open(html: &str, window: &mut Window, cx: &mut App) -> Result<Self, Error> {
        let made = Self::make(html, window, cx);
        match &made {
            Ok(_) => {}
            Err(error @ Error::Off) => log::info!("the report shows as rich text: {error}"),
            Err(error) => {
                let source = std::error::Error::source(error)
                    .map_or_else(String::new, |source| format!(": {source}"));
                log::warn!("the report shows as rich text: {error}{source}");
            }
        }
        made
    }

    fn make(html: &str, window: &mut Window, cx: &mut App) -> Result<Self, Error> {
        if web_view_off(std::env::var_os(WEB_VIEW_ENV).as_deref()) {
            return Err(Error::Off);
        }
        let main_thread = MainThreadMarker::new().ok_or(Error::NotMainThread)?;
        let policy = Arc::new(Mutex::new(NavigationPolicy::default()));
        // WebKit calls the handlers on the main thread, possibly while GPUI is mid-update, so
        // they never touch the app: a link goes through this channel to a task of its own.
        let (links, to_open) = smol::channel::unbounded::<String>();
        let builder = WebViewBuilder::new()
            .with_webview_configuration(configuration(main_thread))
            .with_html(sandbox::sandboxed(html))
            // In every frame, so a frame the page makes gets it too.
            .with_initialization_script_for_main_only(guard::GUARD, false)
            .with_autoplay(false)
            // A force click's preview would load the linked page in WebKit, past the policy.
            .with_allow_link_preview(false)
            .with_devtools(cfg!(debug_assertions))
            .with_navigation_handler({
                let policy = Arc::clone(&policy);
                let links = links.clone();
                move |url| {
                    decide(
                        &policy,
                        &links,
                        url,
                        Instant::now(),
                        NavigationPolicy::navigation,
                    )
                }
            })
            .with_new_window_req_handler({
                let policy = Arc::clone(&policy);
                move |url, _features| {
                    decide(
                        &policy,
                        &links,
                        url,
                        Instant::now(),
                        NavigationPolicy::new_window,
                    );
                    NewWindowResponse::Deny
                }
            })
            .with_on_page_load_handler({
                let policy = Arc::clone(&policy);
                move |event, _url| {
                    if matches!(event, PageLoadEvent::Finished) {
                        lock(&policy).loaded();
                    }
                }
            })
            .with_download_started_handler(|url, _path| {
                log::info!("a report's download was refused: {url}");
                false
            })
            // True: Wry keeps the drop, so WebKit never loads a dropped file.
            .with_drag_drop_handler(|_event| true);
        let web = WebView::build(builder, window, cx).map_err(Error::Build)?;
        let view = cx.new(|_| web);
        let links = cx.spawn(async move |cx| {
            while let Ok(url) = to_open.recv().await {
                cx.update(|cx| cx.open_url(&url));
            }
        });
        let watch = cx.spawn(async move |cx| {
            cx.background_executor().timer(SLOW_LOAD).await;
            if !lock(&policy).is_loaded() {
                log::warn!(
                    "a report hasn't finished loading after {} s: a slow picture, or WebKit \
                     failed (the panel may stay blank; Open in browser shows it)",
                    SLOW_LOAD.as_secs()
                );
            }
        });
        Ok(Self {
            view,
            dark: Cell::new(None),
            shown: Cell::new(true),
            _links: links,
            _watch: watch,
        })
    }

    /// Gives the page Emberhut's own light or dark appearance (for `prefers-color-scheme` and its
    /// controls), when it changed.
    pub(super) fn follow_appearance(&self, dark: bool, cx: &App) {
        if self.dark.get() == Some(dark) {
            return;
        }
        self.dark.set(Some(dark));
        let theme = if dark { Theme::Dark } else { Theme::Light };
        if let Err(error) = self.view.read(cx).set_theme(theme) {
            log::warn!("the report couldn't follow the appearance: {error}");
        }
    }

    /// Shows the page, or hides it while something of the app's own lies over the panel (the
    /// file viewer's sheet), which its native view would cover.
    pub(super) fn set_shown(&self, shown: bool, cx: &App) {
        if self.shown.replace(shown) == shown {
            return;
        }
        if let Err(error) = self.view.read(cx).set_visible(shown) {
            log::warn!("the report couldn't be shown or hidden: {error}");
        }
    }

    /// The web view, to place in the panel.
    pub(super) fn element(&self) -> AnyElement {
        self.view.clone().into_any_element()
    }
}

/// The web view's `WebKit` configuration, where Wry has no setting of its own: a script may open
/// a window only while it handles the user's click or key; no media plays without the user;
/// and storage is private (Wry's incognito, which a configuration of our own replaces).
#[expect(
    unsafe_code,
    reason = "objc2-web-kit marks these six public WebKit calls unsafe: two constructors \
              (the configuration, the non-persistent data store), one getter (its preferences) \
              and three property setters, all on the main thread (the marker proves it) on \
              objects made here"
)]
fn configuration(main_thread: MainThreadMarker) -> Retained<WKWebViewConfiguration> {
    // SAFETY: see the `expect` above: main thread, live objects made here, documented public
    // API only (no private preferences: ADR 0006 rejects them).
    unsafe {
        let configuration = WKWebViewConfiguration::new(main_thread);
        configuration.setWebsiteDataStore(&WKWebsiteDataStore::nonPersistentDataStore(main_thread));
        configuration.setMediaTypesRequiringUserActionForPlayback(WKAudiovisualMediaTypes::All);
        configuration
            .preferences()
            .setJavaScriptCanOpenWindowsAutomatically(false);
        configuration
    }
}

/// The policy, whoever held it last (a panic in a handler leaves it usable).
fn lock(policy: &Mutex<NavigationPolicy>) -> std::sync::MutexGuard<'_, NavigationPolicy> {
    policy.lock().unwrap_or_else(PoisonError::into_inner)
}
