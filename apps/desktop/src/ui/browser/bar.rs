//! The browser window's toolbar: Back, Forward and Reload; the address (the site with a lock
//! for https, the whole address while editing, Return goes there; the site as text while
//! watching); the tabs (their count, the one on screen ticked, New tab, Close this tab); More
//! (the browser's own controls); and Take over or Give back. Only Take over works while
//! watching.

use emberhut_api::browser::{BrowserInfo, BrowserInput};
use gpui_kit::assets::IconName as AssetIcon;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::Input;
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::{Disableable as _, Icon};
use gpui_kit::prelude::*;
use gpui_kit::{Div, div, px};

use super::{BrowserWindow, CONTEXT, GiveBack};
use crate::store::browser::shown;
use crate::ui::palette::Pal;

/// The most tabs the tabs button counts.
const MOST_COUNTED: usize = 50;

/// The toolbar.
pub(super) fn bar(this: &BrowserWindow, p: &Pal, cx: &mut Context<'_, BrowserWindow>) -> Div {
    let store = this.store.read(cx);
    let info = store.browser().cloned().unwrap_or_default();
    let mine = store.browser_is_mine();
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(8.))
        .h(px(44.))
        .px(px(10.))
        .bg(p.surface)
        .border_b_1()
        .border_color(p.border)
        .child(navigation(mine, cx))
        .child(
            div()
                .flex_1()
                .flex()
                .justify_center()
                .child(address(this, &info, mine, p)),
        )
        .child(tabs(&info, mine, cx))
        .child(more(&info, mine, cx))
        .child(drive(mine, cx))
}

/// Back, Forward and Reload, while this device drives.
fn navigation(mine: bool, cx: &mut Context<'_, BrowserWindow>) -> Div {
    let button = |id: &'static str, icon: AssetIcon, tip: &'static str, input: BrowserInput| {
        Button::new(id)
            .ghost()
            .small()
            .icon(Icon::new(icon))
            .accessibility_label(tip)
            .tooltip(tip)
            .disabled(!mine)
            .on_click(cx.listener(move |this, _, _, cx| this.act(input.clone(), cx)))
    };
    div()
        .flex()
        .flex_none()
        .gap(px(2.))
        .child(button(
            "browser-back",
            AssetIcon::ChevronLeft,
            "Back",
            BrowserInput::Back,
        ))
        .child(button(
            "browser-forward",
            AssetIcon::ChevronRight,
            "Forward",
            BrowserInput::Forward,
        ))
        .child(button(
            "browser-reload",
            AssetIcon::RotateCw,
            "Reload",
            BrowserInput::Reload,
        ))
}

/// The address: a field while this device drives, the site as text otherwise.
fn address(this: &BrowserWindow, info: &BrowserInfo, mine: bool, p: &Pal) -> Div {
    let lock = if shown::is_secure(info) {
        AssetIcon::Lock
    } else {
        AssetIcon::Globe
    };
    let field = div()
        .flex()
        .items_center()
        .gap(px(6.))
        .w_full()
        .min_w(px(320.))
        .max_w(px(640.))
        .h(px(28.))
        .px(px(10.))
        .rounded_full()
        .bg(p.hover)
        .child(Icon::new(lock).size(px(12.)).text_color(p.muted));
    if mine {
        field.child(
            div()
                .flex_1()
                .min_w_0()
                .child(Input::new(&this.address).appearance(false).small()),
        )
    } else {
        field.child(div().flex_1().min_w_0().truncate().child(shown::site(info)))
    }
}

/// The tabs: the count, then each to switch to (the one on screen ticked), New tab and Close
/// this tab, while this device drives.
fn tabs(info: &BrowserInfo, mine: bool, cx: &mut Context<'_, BrowserWindow>) -> impl IntoElement {
    let count = info.tabs.len().clamp(1, MOST_COUNTED);
    let window = cx.entity();
    let info = info.clone();
    Button::new("browser-tabs")
        .ghost()
        .small()
        .icon(Icon::new(AssetIcon::Layers))
        .label(count.to_string())
        .accessibility_label(if count == 1 {
            "1 tab".to_owned()
        } else {
            format!("{count} tabs")
        })
        .tooltip("Tabs")
        .disabled(!mine)
        .dropdown_menu(move |mut menu, _, _| {
            let act = |input: BrowserInput| {
                let window = window.clone();
                move |_: &gpui_kit::ClickEvent, _: &mut gpui_kit::Window, cx: &mut gpui_kit::App| {
                    window.update(cx, |this, cx| this.act(input.clone(), cx));
                }
            };
            for tab in &info.tabs {
                let title = if tab.title.is_empty() {
                    tab.url.clone()
                } else {
                    tab.title.clone()
                };
                menu = menu.item(
                    PopupMenuItem::new(title)
                        .checked(tab.active)
                        .on_click(act(BrowserInput::Tab { id: tab.id })),
                );
            }
            menu = menu
                .separator()
                .item(PopupMenuItem::new("New tab").on_click(act(BrowserInput::NewTab)));
            if let Some(tab) = info.shown_tab() {
                menu = menu.item(
                    PopupMenuItem::new("Close this tab")
                        .on_click(act(BrowserInput::CloseTab { id: tab.id })),
                );
            }
            menu
        })
}

/// More: the browser's own controls (its tab strip, address bar, menus and extensions), for
/// what the toolbar doesn't do; and where there is no web view here, the live view in the
/// default browser.
fn more(info: &BrowserInfo, mine: bool, cx: &mut Context<'_, BrowserWindow>) -> impl IntoElement {
    let window = cx.entity();
    let showing = info.chrome_ui;
    Button::new("browser-more")
        .ghost()
        .small()
        .icon(Icon::new(AssetIcon::Ellipsis))
        .accessibility_label("More")
        .tooltip("More")
        .dropdown_menu(move |menu, _, _| {
            let toggle = window.clone();
            let menu = menu.item(
                PopupMenuItem::new(if showing {
                    "Hide Chrome's controls"
                } else {
                    "Show Chrome's controls"
                })
                .disabled(!mine)
                .on_click(move |_, _, cx| {
                    toggle.update(cx, |this, cx| {
                        this.act(BrowserInput::Chrome { show: !showing }, cx);
                    });
                }),
            );
            super::page::more_items(menu, &window)
        })
}

/// Take over, or Give back (⌘Esc) while this device drives.
fn drive(mine: bool, cx: &mut Context<'_, BrowserWindow>) -> impl IntoElement {
    if mine {
        Button::new("browser-give-back")
            .primary()
            .small()
            .icon(Icon::new(AssetIcon::Undo2))
            .label("Give back")
            .tooltip_with_action("Give back", &GiveBack, Some(CONTEXT))
            .on_click(cx.listener(|this, _, window, cx| this.give_back(&GiveBack, window, cx)))
    } else {
        Button::new("browser-take-over")
            .primary()
            .small()
            .icon(Icon::new(AssetIcon::Hand))
            .label("Take over")
            .tooltip("Take over: the agents wait until you give it back")
            .on_click(cx.listener(|this, _, _, cx| this.take_over(cx)))
    }
}
