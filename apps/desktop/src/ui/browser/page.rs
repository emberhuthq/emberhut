//! The browser window's page: who drives (a small pill above it), the page itself (the live
//! stream, or its latest picture where there is no web view), and under it what the assistant
//! asked while this device holds the browser, the page wanting a file, and what the server
//! said no to.
//!
//! Nothing sits over the page: a native web view is drawn above the window's own content, so
//! the pill and the notes have rows of their own.

use gpui_kit::assets::IconName as AssetIcon;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{Disableable as _, Icon};
use gpui_kit::prelude::*;
use std::time::Duration;

use emberhut_api::browser::BrowserSession;
use gpui_kit::{
    AnyElement, App, ClipboardItem, Div, Entity, FontWeight, ImageSource, ObjectFit, Window, div,
    img, px,
};

use super::BrowserWindow;
use crate::brand::APP_NAME;
use crate::store::Store;
use crate::store::browser::Link;
use crate::store::browser::shown::{self, Tone};
use crate::ui::a11y::Accessible as _;
use crate::ui::palette::Pal;
use crate::ui::widgets::dot;

/// The page and the rows around it.
pub(super) fn page(
    this: &mut BrowserWindow,
    p: &Pal,
    window: &mut Window,
    cx: &mut Context<'_, BrowserWindow>,
) -> Div {
    let body = body(this, p, window, cx);
    let store = this.store.read(cx);
    let info = store.browser().cloned().unwrap_or_default();
    let device = store.browser_device().clone();
    let pill = shown::pill(&info, &device);
    let ask = shown::ask(&info, &device);
    let wants_file = info.file_request.is_some();
    let sending = store.browser_sending().map(str::to_owned);
    let problem = store.browser_problem().map(str::to_owned);
    let mut notes = div().flex().flex_col().items_center().gap(px(6.));
    let mut any_note = false;
    if let Some(ask) = ask {
        any_note = true;
        notes = notes.child(div().id("browser-ask").announced(ask.clone()).child(note(
            p,
            AssetIcon::Hand,
            p.accent,
            ask,
        )));
    }
    if wants_file {
        any_note = true;
        notes = notes.child(file_note(p, sending.as_deref(), cx));
    }
    if let Some(problem) = problem {
        any_note = true;
        notes = notes.child(
            div()
                .id("browser-problem")
                .announced(problem.clone())
                .child(note(p, AssetIcon::TriangleAlert, p.danger, problem)),
        );
    }
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_h_0()
        .child(
            div()
                // Who drives, said as it changes hands.
                .id("browser-driver")
                .announced(pill.text.clone())
                .flex()
                .flex_none()
                .justify_center()
                .py(px(6.))
                .child(pill_view(p, &pill)),
        )
        .child(div().flex_1().min_h_0().bg(gpui_kit::black()).child(body))
        .when(any_note, |d| d.child(notes.py(px(6.))))
}

/// Who drives, small: a dot tinted by who has it, and the words.
fn pill_view(p: &Pal, pill: &shown::Pill) -> Div {
    let tint = match pill.tone {
        Tone::Person => p.accent,
        Tone::Agent => p.warn,
        Tone::Idle => p.faint,
    };
    div()
        .flex()
        .items_center()
        .gap(px(6.))
        .px(px(10.))
        .py(px(4.))
        .rounded_full()
        .bg(p.surface_raised)
        .border_1()
        .border_color(p.border)
        .text_size(px(11.5))
        .font_weight(FontWeight::SEMIBOLD)
        .child(dot(tint, 7.))
        .child(pill.text.clone())
}

/// A note under the page.
fn note(p: &Pal, icon: AssetIcon, tint: gpui_kit::Hsla, text: String) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .px(px(12.))
        .py(px(7.))
        .rounded(px(12.))
        .bg(p.surface_raised)
        .border_1()
        .border_color(p.border)
        .text_color(tint)
        .font_weight(FontWeight::MEDIUM)
        .child(Icon::new(icon).size(px(14.)).text_color(tint))
        .child(text)
}

/// The page wants a file: Choose… opens this device's file picker; while it goes, its name.
fn file_note(p: &Pal, sending: Option<&str>, cx: &mut Context<'_, BrowserWindow>) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(10.))
        .pl(px(12.))
        .pr(px(8.))
        .py(px(5.))
        .rounded_full()
        .bg(p.surface_raised)
        .border_1()
        .border_color(p.border)
        .font_weight(FontWeight::MEDIUM)
        .child(
            Icon::new(AssetIcon::Paperclip)
                .size(px(14.))
                .text_color(p.info),
        )
        .child(shown::file_note(sending))
        .child(
            Button::new("browser-choose-file")
                .primary()
                .small()
                .label("Choose…")
                .disabled(sending.is_some())
                .on_click(cx.listener(|this, _, _, cx| this.choose_file(cx))),
        )
}

/// The page itself, by where the session stands.
fn body(
    this: &mut BrowserWindow,
    p: &Pal,
    window: &mut Window,
    cx: &mut Context<'_, BrowserWindow>,
) -> AnyElement {
    let link = this.store.read(cx).browser_link().clone();
    match link {
        Link::Connecting => {
            drop_stream(this);
            centred(
                div()
                    .id("browser-connecting")
                    .announced(format!("Connecting to {APP_NAME}'s browser…"))
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(8.))
                    .child(Spinner::new().color(p.muted))
                    .child(format!("Connecting to {APP_NAME}'s browser…")),
                p,
            )
        }
        Link::Unavailable => {
            drop_stream(this);
            centred(
                div().child(format!(
                    "{APP_NAME}'s browser isn't running on the server yet."
                )),
                p,
            )
        }
        Link::Ready(session) => live(this, &session, p, window, cx),
    }
}

/// Words in the middle of the page's black.
fn centred(content: impl IntoElement, p: &Pal) -> AnyElement {
    div()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .text_color(p.faint)
        .text_size(px(13.5))
        .child(content)
        .into_any_element()
}

/// The live stream in a web view, made afresh for each session (control or view only).
#[cfg(target_os = "macos")]
fn live(
    this: &mut BrowserWindow,
    session: &BrowserSession,
    p: &Pal,
    window: &mut Window,
    cx: &mut Context<'_, BrowserWindow>,
) -> AnyElement {
    // No web view (an `http` address among the reasons: its sign-in would cross the network
    // in the clear): the pictures.
    if this.stream_failed {
        return stills(this, session, p, cx);
    }
    if !this
        .stream
        .as_ref()
        .is_some_and(|stream| stream.shows(session))
    {
        this.stream = None;
        match super::stream::StreamWeb::open(session, window, cx) {
            Ok(stream) => this.stream = Some(stream),
            Err(error) => {
                log::warn!("the browser shows pictures: {error}");
                this.stream_failed = true;
                this.store.update(cx, |store, cx| {
                    store.browser_shows(crate::store::browser::Showing::Stills, cx);
                });
                return stills(this, session, p, cx);
            }
        }
    }
    this.stream.as_ref().map_or_else(
        || stills(this, session, p, cx),
        super::stream::StreamWeb::element,
    )
}

/// Without a web view: the page's latest picture.
#[cfg(not(target_os = "macos"))]
fn live(
    this: &mut BrowserWindow,
    session: &BrowserSession,
    p: &Pal,
    _: &mut Window,
    cx: &mut Context<'_, BrowserWindow>,
) -> AnyElement {
    stills(this, session, p, cx)
}

/// The live view's web view goes (the window shows something else).
#[cfg(target_os = "macos")]
fn drop_stream(this: &mut BrowserWindow) {
    this.stream = None;
}

/// Without a web view there is none to drop.
#[cfg(not(target_os = "macos"))]
fn drop_stream(_: &mut BrowserWindow) {}

/// The page's latest picture while someone else drives; while this device drives (no picture
/// is sent then), how to see it live.
fn stills(
    this: &BrowserWindow,
    session: &BrowserSession,
    p: &Pal,
    cx: &mut Context<'_, BrowserWindow>,
) -> AnyElement {
    let store = this.store.read(cx);
    if store.browser_is_mine() {
        return driving_here(session, live_view_link(store, session), p);
    }
    if let Some(frame) = store.browser_frame() {
        return img(ImageSource::Render(frame))
            .size_full()
            .object_fit(ObjectFit::Contain)
            .into_any_element();
    }
    let info = store.browser().cloned().unwrap_or_default();
    let said = shown::no_picture(&info, store.browser_device());
    centred(div().max_w(px(460.)).text_center().child(said), p)
}

/// Why there is no live view for an `http` stream address.
fn not_secure() -> String {
    format!(
        "You're driving. The live view's address isn't secure (http), so {APP_NAME} doesn't \
         send its password there; the toolbar still drives the browser."
    )
}

/// How long the stream's password stays on the clipboard after Copy password.
const PASSWORD_KEPT: Duration = Duration::from_secs(60);

/// Where the live view opens in the default browser: its address as the app's links go out
/// ([`ServerUrl::link`](crate::config::ServerUrl::link): a web address, or a path on the
/// server); none for any other scheme the server may have sent (`file:`, another app's).
fn live_view_link(store: &Store, session: &BrowserSession) -> Option<String> {
    session
        .url
        .as_deref()
        .and_then(|url| store.server().link(url))
}

/// This device drives where it has no web view: the toolbar works, and the live view opens in
/// the default browser (at `address`, [`live_view_link`]), where the person signs in with the
/// session's user and password.
///
/// The password goes on the clipboard: the only way into the stream from here, as a signed
/// address handed to the default browser would put it on the command line of the program that
/// opens it and in the browser's history. It is cleared from the clipboard after
/// [`PASSWORD_KEPT`] unless something else was copied since.
fn driving_here(session: &BrowserSession, address: Option<String>, p: &Pal) -> AnyElement {
    let password = session.password.clone();
    let secure = session.is_secure();
    let said = if !secure {
        not_secure()
    } else if cfg!(target_os = "macos") {
        format!(
            "You're driving. The live picture couldn't be shown here; the toolbar drives the \
             browser, or open the live view in your browser and sign in as {}. The password is \
             cleared from the clipboard a minute after you copy it.",
            session.user
        )
    } else {
        format!(
            "You're driving. The live picture shows in {APP_NAME} on macOS; here the toolbar \
             drives the browser, or open the live view in your browser and sign in as {}. The \
             password is cleared from the clipboard a minute after you copy it.",
            session.user
        )
    };
    let words = div().max_w(px(460.)).text_center().child(said);
    let buttons = div()
        .flex()
        .gap(px(8.))
        .child(
            Button::new("browser-open-live")
                .primary()
                .small()
                .icon(Icon::new(AssetIcon::ExternalLink))
                .label("Open in browser")
                .disabled(address.is_none())
                .on_click(move |_, _, cx| {
                    if let Some(address) = &address {
                        cx.open_url(address);
                    }
                }),
        )
        .when(secure, |d| {
            d.child(
                Button::new("browser-copy-password")
                    .outline()
                    .small()
                    .icon(Icon::new(AssetIcon::KeyRound))
                    .label("Copy password")
                    .on_click(move |_, _, cx| copy_for_a_while(password.clone(), cx)),
            )
        });
    centred(
        div()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(12.))
            .child(words)
            .child(buttons),
        p,
    )
}

/// Puts `secret` on the clipboard, and takes it off after [`PASSWORD_KEPT`] if it is still
/// there.
fn copy_for_a_while(secret: String, cx: &mut App) {
    cx.write_to_clipboard(ClipboardItem::new_string(secret.clone()));
    cx.spawn(async move |cx| {
        cx.background_executor().timer(PASSWORD_KEPT).await;
        cx.update(|cx| {
            let still = cx
                .read_from_clipboard()
                .and_then(|item| item.text())
                .is_some_and(|text| text == secret);
            if still {
                cx.write_to_clipboard(ClipboardItem::new_string(String::new()));
            }
        });
    })
    .detach();
}

/// More's items besides the browser's own controls: the live view in the default browser.
pub(super) fn more_items(menu: PopupMenu, window: &Entity<BrowserWindow>) -> PopupMenu {
    let window = window.clone();
    menu.separator().item(
        PopupMenuItem::new("Open the live view in your browser").on_click(move |_, _, cx| {
            let store = window.read(cx).store.read(cx);
            let address = if let Link::Ready(session) = store.browser_link() {
                live_view_link(store, session)
            } else {
                None
            };
            if let Some(address) = address {
                cx.open_url(&address);
            }
        }),
    )
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use emberhut_api::browser::StreamMode;
    use gpui_kit::TestAppContext;

    use super::*;
    use crate::backend::fake::FakeBackend;

    #[gpui_kit::test]
    fn the_live_view_opens_in_the_browser_only_from_a_web_address(cx: &mut TestAppContext) {
        let store = cx.new(|_| Store::for_tests(Arc::new(FakeBackend::new([]))));
        let session = |url: &str| BrowserSession {
            url: Some(url.to_owned()),
            user: "viewer".to_owned(),
            password: "example".to_owned(),
            mode: StreamMode::Control,
            screen: None,
        };
        store.read_with(cx, |store, _| {
            for shut in [
                "file:///etc/passwd",
                "smb://example.com/share",
                "x-example-app://open",
            ] {
                assert_eq!(live_view_link(store, &session(shut)), None, "{shut}");
            }
            assert_eq!(
                live_view_link(store, &session("https://example.org/")).as_deref(),
                Some("https://example.org/")
            );
            let none = BrowserSession {
                url: None,
                ..session("https://example.org/")
            };
            assert_eq!(live_view_link(store, &none), None);
        });
    }

    fn clipboard(cx: &TestAppContext) -> Option<String> {
        cx.read_from_clipboard().and_then(|item| item.text())
    }

    #[gpui_kit::test]
    fn a_copied_password_leaves_the_clipboard_after_a_while(cx: &mut TestAppContext) {
        cx.update(|cx| copy_for_a_while("example".to_owned(), cx));
        assert_eq!(clipboard(cx).as_deref(), Some("example"));
        cx.executor().advance_clock(PASSWORD_KEPT);
        cx.run_until_parked();
        assert_eq!(clipboard(cx).unwrap_or_default(), "", "cleared");

        cx.update(|cx| copy_for_a_while("example".to_owned(), cx));
        cx.write_to_clipboard(ClipboardItem::new_string("mine".to_owned()));
        cx.executor().advance_clock(PASSWORD_KEPT);
        cx.run_until_parked();
        assert_eq!(
            clipboard(cx).as_deref(),
            Some("mine"),
            "what was copied since stays"
        );
    }
}
