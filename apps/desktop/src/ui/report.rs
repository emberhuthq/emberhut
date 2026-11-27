//! The report panel beside the page: an HTML report from the chat, with a link to open it in
//! the browser.
//!
//! On macOS the report shows as its real HTML in a web view, locked down
//! (docs/adr/0006-report-webview.md). Elsewhere, and when the web view can't be made, it shows
//! as gpui-kit's rich text (also when `EMBERHUT_REPORT_WEBVIEW=off`). A web view that was made but
//! whose page then fails (its `WebKit` process dies) isn't noticed: Wry 0.53 doesn't say so, and
//! the panel stays blank, with a warning in the log when the page hasn't loaded after a while.
//! The panel holds one web view at most ([`slot`]).
//!
//! The panel's inner edge resizes it ([`edge`]), within bounds that leave the report and the chat
//! usable ([`width`]), and the width is kept for the next launch ([`kept`]). It can also fill the
//! content area and come back ([`layout`]). The web view follows each new size as it is laid out.

mod edge;
#[cfg(any(target_os = "macos", test))]
mod guard;
mod kept;
mod layout;
#[cfg(not(target_os = "macos"))]
mod no_webview;
#[cfg(any(target_os = "macos", test))]
mod policy;
#[cfg(any(target_os = "macos", test))]
mod sandbox;
mod size;
#[cfg(test)]
mod size_tests;
mod slot;
#[cfg(test)]
mod tests;
#[cfg(target_os = "macos")]
mod webview;
mod width;

#[cfg(not(target_os = "macos"))]
use self::no_webview as webview;

#[cfg(any(target_os = "macos", test))]
use std::ffi::OsStr;

use gpui_kit::assets::IconName as AssetIcon;
use gpui_kit::component::text::TextView;
use gpui_kit::component::{Icon, IconName};
use gpui_kit::prelude::*;
use gpui_kit::{
    App, Div, Entity, FontWeight, Pixels, ScrollHandle, SharedString, Stateful, Window, div, px,
};

pub(crate) use self::size::ReportSize;
use self::slot::Slot;
use self::webview::ReportWeb;
use self::width::Space;
pub(crate) use self::width::{MIN_CHAT, MIN_PANEL};
use super::a11y::Accessible as _;
use super::files::actions::FileAction;
use super::palette::{self, pal};
use super::toast;
use super::widgets::icon;
use super::window::MainWindow;
use crate::store::files::RemoteFile;
use crate::store::gate::Gate;
use crate::store::{Fetched, OpenReport, ReportPage, Store};

/// The environment variable that turns the web view off (`off`): reports then show as rich text,
/// to check the fallback and to measure what the web view costs.
#[cfg(target_os = "macos")]
const WEB_VIEW_ENV: &str = "EMBERHUT_REPORT_WEBVIEW";

/// Whether `value`, that of `EMBERHUT_REPORT_WEBVIEW`, turns the web view off.
#[cfg(any(target_os = "macos", test))]
fn web_view_off(value: Option<&OsStr>) -> bool {
    value.is_some_and(|value| value.eq_ignore_ascii_case("off"))
}

/// The rich text body's selector, for the tests (a no-op in a normal build).
const RICH_TEXT: &str = "report-rich-text";

/// The panel's selector, for the tests (a no-op in a normal build).
const PANEL: &str = "report-panel";

/// The body's selector (the page, its rich text, or the loading or failure line), for the tests
/// (a no-op in a normal build).
const BODY: &str = "report-body-box";

/// The expand button's selector, for the tests (a no-op in a normal build).
const EXPAND: &str = "report-expand";

/// The panel's web view, kept across frames while the same report shows.
#[derive(Default)]
pub(crate) struct ReportBody {
    slot: Slot<ReportWeb, webview::Error>,
    /// The name of the header button under the pointer, said on the header's second line.
    hovered: Option<SharedString>,
}

impl ReportBody {
    /// Drops the web view, if one is up. Its view is native, drawn above whatever GPUI draws:
    /// a screen that takes the window's place (the app lock's cover) drops it rather than only
    /// leaving it undrawn. The next frame that shows the report makes it again.
    pub(crate) fn clear(&mut self) {
        self.slot.clear();
    }
}

/// The panel, when a report is open.
pub(crate) fn panel(
    this: &mut MainWindow,
    window: &mut Window,
    cx: &mut Context<'_, MainWindow>,
) -> Option<Div> {
    let p = pal(cx);
    let viewing = this.viewer_shows(cx);
    let store = this.store.read(cx);
    let open = open_report(store);
    let in_browser = open
        .as_ref()
        .and_then(|(report, _)| store.server().link(&report.path));
    let page = sync(&mut this.report_body.slot, open.as_ref(), |html| {
        // The theme's colours as the page is made: an appearance change follows in the page
        // (its style picks by prefers-color-scheme), a preset change on its next opening.
        let colours = palette::web_colours(this.store.read(cx).theme().page());
        ReportWeb::open(html, &colours, window, cx)
    })
    .and_then(Result::ok)
    .map(|page| {
        page.follow_appearance(p.dark, cx);
        // The viewer's sheet lies over the panel: the page's native view would cover it.
        page.set_shown(!viewing, cx);
        page.element()
    });
    let (report, fetched) = open?;
    let expanded = this.report_size.is_expanded();
    let title = match &fetched {
        Fetched::Ready(page) => page.title.clone(),
        Fetched::Loading | Fetched::Failed(_) => None,
    };
    let body = match (page, fetched) {
        (Some(page), _) => div().flex_1().min_h_0().child(page).into_any_element(),
        // No web view here, or it couldn't be made (the reason is logged): rich text.
        (None, Fetched::Ready(page)) => scrolling(&this.report_scroll)
            .debug_selector(|| RICH_TEXT.to_owned())
            .child(TextView::html("report-html", page.html).text_size(px(14.)))
            .into_any_element(),
        (None, Fetched::Failed(reason)) => scrolling(&this.report_scroll)
            .child(div().text_color(p.muted).child(reason))
            .into_any_element(),
        (None, Fetched::Loading) => scrolling(&this.report_scroll)
            .child(div().text_color(p.muted).child("Loading…"))
            .into_any_element(),
    };
    let body = div()
        .debug_selector(|| BODY.to_owned())
        .flex()
        .flex_col()
        .flex_1()
        .min_h_0()
        .child(body);
    // Expanded, the page and its answer toast are gone: the toast comes under the report, out of
    // the web view's bounds (its native view would cover it).
    let toast = expanded
        .then(|| toast::toast(this, cx).map(toast::under_report))
        .flatten();
    let content = div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .h_full()
        .child(header(
            report,
            title,
            in_browser,
            expanded,
            this.report_body.hovered.clone(),
            &this.store,
            cx,
        ))
        .child(body)
        .children(toast);
    let panel = div()
        .debug_selector(|| PANEL.to_owned())
        .flex()
        .h_full()
        .bg(p.surface);
    // Expanded, it takes the chat's place too: it has no edge to drag, and the sidebar's border
    // is its own.
    Some(if expanded {
        panel.flex_1().min_w_0().child(content)
    } else {
        let width = this.report_size.pixels(space(this, window, cx));
        panel
            .flex_none()
            .w(width)
            .child(edge::edge(cx))
            .child(content)
    })
}

/// The width the panel shares in `window`: the window's beside the sidebar, less the column's
/// while it shows (see [`MainWindow::column_width`]). The window's root row holds the sidebar,
/// the page (the chat and the column) and the panel, and nothing else (see
/// `MainWindow::render`).
pub(crate) fn space(this: &MainWindow, window: &Window, cx: &App) -> Space {
    let width = window.viewport_size().width;
    let column = this.column_width(window, cx).unwrap_or(Pixels::ZERO);
    Space::new(width, this.beside_sidebar(window, cx) - column)
}

/// The report open beside the page, and where its fetch stands. It shows beside the app only:
/// none beside the sign-in, connecting and "not your device" screens.
fn open_report(store: &Store) -> Option<(OpenReport, Fetched<ReportPage>)> {
    if store.gate() != Gate::Open {
        return None;
    }
    let report = store.open_report()?.clone();
    let fetched = store.report(&report.path);
    Some((report, fetched))
}

/// Brings `slot` in line with what the panel shows: the view (made by `make` from the page's
/// HTML) for a fetched report, and none, dropping any, while it loads, when it failed, and when
/// no report is open.
fn sync<'s, V, E>(
    slot: &'s mut Slot<V, E>,
    open: Option<&(OpenReport, Fetched<ReportPage>)>,
    make: impl FnOnce(&str) -> Result<V, E>,
) -> Option<Result<&'s V, &'s E>> {
    match open {
        Some((report, Fetched::Ready(page))) => Some(slot.show(&report.path, || make(&page.html))),
        Some((_, Fetched::Loading | Fetched::Failed(_))) | None => {
            slot.clear();
            None
        }
    }
}

/// The body's scrolling column, for rich text and the loading and failure lines.
fn scrolling(scroll: &ScrollHandle) -> Stateful<Div> {
    div()
        .id("report-body")
        .flex_1()
        .overflow_y_scroll()
        .track_scroll(scroll)
        .px(px(22.))
        .py(px(16.))
}

/// The header: the page's own title (its file name under it), the page's uses (Share… where the
/// platform has a share menu, Save to Downloads, Save As…, Open in its app, Copy), Open in
/// browser, Expand (or Restore) and Close.
///
/// They are buttons of their own rather than a menu: anything that opened over the page would
/// sit under the web view's native view. For the same reason the name of the button under the
/// pointer (`hovered`) takes the second line rather than a tooltip, and so does the file name: a
/// tooltip would open over the page, and the web view's native view covers GPUI content in its
/// bounds.
fn header(
    report: OpenReport,
    title: Option<SharedString>,
    in_browser: Option<String>,
    expanded: bool,
    hovered: Option<SharedString>,
    store: &Entity<Store>,
    cx: &mut Context<'_, MainWindow>,
) -> Div {
    let p = pal(cx);
    let (resize, resize_name) = if expanded {
        (IconName::Minimize, "Restore")
    } else {
        (IconName::Maximize, "Expand")
    };
    let file = RemoteFile::new(report.path.clone(), report.name.clone());
    let uses: Vec<_> = FileAction::offered(&file)
        .into_iter()
        .map(|action| {
            let (file, store) = (file.clone(), store.clone());
            header_button(
                SharedString::from(format!("report-{action:?}")),
                Icon::new(action.icon()),
                action.label(&file).into(),
                cx,
            )
            .on_click(move |_, window, cx| action.run(file.clone(), &store, window, cx))
        })
        .collect();
    let open_browser = header_button(
        "open-browser".into(),
        Icon::new(AssetIcon::Globe),
        OPEN_IN_BROWSER.into(),
        cx,
    )
    .on_click(move |_, _, cx| {
        if let Some(link) = &in_browser {
            cx.open_url(link);
        }
    });
    let expand = header_button(
        "expand-report".into(),
        Icon::new(resize),
        resize_name.into(),
        cx,
    )
    .debug_selector(|| EXPAND.to_owned())
    .on_click(cx.listener(|this, _, window, cx| {
        this.toggle_report_expanded(window, cx);
    }));
    let close = header_button(
        "close-report".into(),
        Icon::new(IconName::Close),
        "Close".into(),
        cx,
    )
    .on_click(cx.listener(|this, _, _, cx| {
        this.report_body.hovered = None;
        this.store
            .update(cx, |store, cx| store.show_report(None, cx));
    }));
    let (first, second) = match title {
        Some(title) => (title, Some(SharedString::from(report.name))),
        None => (report.name.into(), None),
    };
    let second = hovered.or(second);
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .h(px(48.))
        .px(px(14.))
        .border_b_1()
        .border_color(p.border)
        .child(icon(IconName::FileText, p.muted, 15.))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .child(
                    div()
                        .truncate()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_size(px(13.))
                        .child(first),
                )
                .children(second.map(|line| {
                    div()
                        .truncate()
                        .text_size(px(11.))
                        .text_color(p.muted)
                        .child(line)
                })),
        )
        .children(uses)
        .child(open_browser)
        .child(expand)
        .child(close)
}

/// What the header's Open in browser button is called.
const OPEN_IN_BROWSER: &str = "Open in Browser";

/// A header button showing `mark`, named `name` for the screen reader and, while the pointer is
/// on it, on the header's second line.
fn header_button(
    id: SharedString,
    mark: Icon,
    name: SharedString,
    cx: &mut Context<'_, MainWindow>,
) -> Stateful<Div> {
    let p = pal(cx);
    let said = name.clone();
    div()
        .id(id)
        .button_named(name)
        .cursor_pointer()
        .p(px(4.))
        .rounded(px(5.))
        .hover(|d| d.bg(p.hover))
        .child(mark.size(px(15.)).text_color(p.muted))
        .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
            let shown = &mut this.report_body.hovered;
            if *hovered {
                *shown = Some(said.clone());
            } else if shown.as_ref() == Some(&said) {
                *shown = None;
            } else {
                return;
            }
            cx.notify();
        }))
}
