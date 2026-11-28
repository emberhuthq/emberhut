//! The sheets over the Chrome profiles: a profile's page (`chrome_profile`), naming a new
//! profile (after adding the extension to Chrome, on the user's OK), and a link to pair (Copy,
//! and Share where the system has a share menu) with its pairing live; and the pieces they
//! share.

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::Input;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{Disableable as _, Icon, Sizable as _};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, Div, ElementId, FontWeight, Hsla, SharedString, div, px};
use jiff::tz::TimeZone;

use super::chrome::{ChromePane, account_line, badge, copy_link};
use super::chrome_profile;
use super::parts::{note, problem_line, selectable, sheet as frame};
use super::share;
use crate::brand::APP_NAME;
use crate::store::Store;
use crate::store::settings::chrome::{ChromeSheet, offered_names};
use crate::store::settings::chrome_install::{ChromeOs, InstallState, store_page};
use crate::store::settings::words::clock;
use crate::ui::palette::Pal;
use emberhut_api::chrome::{ChromeAction, ChromePairing, ChromeProfile};

/// The sheet open over the list, if one is.
pub(super) fn sheet(pane: &ChromePane, p: &Pal, cx: &mut Context<'_, ChromePane>) -> Option<Div> {
    let open = pane.store.read(cx).chrome().sheet().cloned()?;
    let store = pane.store.clone();
    let close = move |_: &mut gpui_kit::Window, cx: &mut gpui_kit::App| {
        store.update(cx, Store::close_chrome_sheet);
    };
    let ((title, body), button) = match &open {
        ChromeSheet::Profile(id) => (chrome_profile::page(pane, *id, p, cx), "Done"),
        // Naming a profile asks for nothing until Get link: leaving it cancels.
        ChromeSheet::Add => (add(pane, p, cx), "Cancel"),
        ChromeSheet::Pairing(pairing) => (pairing_page(pane, pairing, p, cx), "Done"),
    };
    Some(frame(p, &pane.focus, title, body, (button, close)))
}

/// A titled group inside a sheet.
pub(super) fn group(p: &Pal, title: impl Into<SharedString>, rows: Vec<AnyElement>) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(6.))
        .child(
            div()
                .text_size(px(12.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(p.muted)
                .child(title.into()),
        )
        .children(rows)
}

/// A detail's row: its label, then its value.
pub(super) fn detail(
    p: &Pal,
    label: impl Into<SharedString>,
    value: impl IntoElement,
) -> AnyElement {
    div()
        .flex()
        .gap(px(12.))
        .text_size(px(12.5))
        .child(
            div()
                .w(px(110.))
                .flex_none()
                .text_color(p.muted)
                .child(label.into()),
        )
        .child(div().flex_1().min_w_0().child(value))
        .into_any_element()
}

/// A line with an icon in a colour.
pub(super) fn icon_line(glyph: IconName, color: Hsla, text: String) -> AnyElement {
    div()
        .flex()
        .items_start()
        .gap(px(8.))
        .text_size(px(12.5))
        .text_color(color)
        .child(Icon::new(glyph).size(px(14.)).text_color(color))
        .child(div().flex_1().child(text))
        .into_any_element()
}

pub(super) fn act(store: &gpui_kit::Entity<Store>, action: ChromeAction, cx: &mut gpui_kit::App) {
    store.update(cx, |store, cx| store.chrome_act(action, cx));
}

/// Naming a new profile: the name (with the usual ones not taken yet), then Get link.
fn add(pane: &ChromePane, p: &Pal, cx: &mut Context<'_, ChromePane>) -> (String, Div) {
    let install = install_group(pane, p, cx);
    let store = pane.store.read(cx);
    let offered = offered_names(store.chrome_profiles());
    let (busy, problem) = (
        store.chrome().is_busy(),
        store.chrome().problem().map(str::to_owned),
    );
    let typed = pane.name.read(cx).value().trim().to_owned();
    let chips = offered.into_iter().map(|name| {
        Button::new(ElementId::Name(format!("chrome-name-{name}").into()))
            .small()
            .label(name)
            .on_click(cx.listener(move |this, _, window, cx| {
                this.name
                    .update(cx, |input, cx| input.set_value(name, window, cx));
            }))
    });
    let (get, label) = (pane.store.clone(), typed.clone());
    let body = div()
        .flex()
        .flex_col()
        .gap(px(10.))
        .child(install)
        .child(
            div()
                .text_size(px(12.))
                .text_color(p.muted)
                .child("Which profile?"),
        )
        .child(Input::new(&pane.name))
        .child(div().flex().flex_wrap().gap(px(8.)).children(chips))
        .child(note(
            p,
            format!(
                "{APP_NAME} gives you a link. Open it in that Chrome profile once the extension \
                 is on: it asks you to pair, then this waits until it has."
            ),
        ))
        .child(
            div().flex().justify_end().child(
                Button::new("chrome-get-link")
                    .primary()
                    .label("Get link")
                    .disabled(typed.is_empty() || busy)
                    .on_click(move |_, _, cx| {
                        get.update(cx, |store, cx| store.add_chrome(&label, cx));
                    }),
            ),
        )
        .children(problem.map(|problem| problem_line(p, problem)));
    ("Add a Chrome profile".to_owned(), body)
}

/// A link to pair: the link, Copy, how to use it, and its pairing as it happens.
fn pairing_page(
    pane: &ChromePane,
    pairing: &ChromePairing,
    p: &Pal,
    cx: &mut Context<'_, ChromePane>,
) -> (String, Div) {
    let store = pane.store.read(cx);
    let now = store
        .state()
        .and_then(|state| {
            state
                .chrome_pairing
                .iter()
                .find(|link| link.code == pairing.code)
        })
        .cloned();
    let paired = now
        .as_ref()
        .filter(|link| link.paired)
        .and_then(|link| link.device)
        .and_then(|device| store.chrome_profile(device))
        .cloned();
    let until = clock(pairing.until.as_str(), &TimeZone::system()).unwrap_or_default();
    let status = pairing_status(paired.as_ref(), now.is_some(), p);
    let body = div()
        .flex()
        .flex_col()
        .gap(px(16.))
        .child(group(
            p,
            format!("Open this in your {} Chrome profile", pairing.label),
            vec![
                selectable(
                    ElementId::Name(format!("chrome-pair-{}-link", pairing.code).into()),
                    &pairing.link,
                )
                .into_any_element(),
                div()
                    .flex()
                    .gap(px(10.))
                    .child(copy_link(
                        ElementId::Name(format!("chrome-pair-{}-copy", pairing.code).into()),
                        &pairing.link,
                        true,
                    ))
                    .children(share::button(
                        ElementId::Name(format!("chrome-pair-{}-share", pairing.code).into()),
                        &pairing.link,
                    ))
                    .into_any_element(),
                note(
                    p,
                    format!(
                        "Paste it in a window of that profile (on the Mac: the profile's own \
                         window). Code {} · works once, until {until}.",
                        pairing.code
                    ),
                )
                .into_any_element(),
            ],
        ))
        .child(group(p, "Status", vec![status.into_any_element()]))
        .child(group(
            p,
            "If the extension isn't in that profile yet",
            steps(p),
        ));
    let title = match &paired {
        Some(_) => "Paired".to_owned(),
        None => format!("Pair {}", pairing.label),
    };
    (title, body)
}

/// Where a link's pairing is: paired (with the profile's account), expired, or waiting.
fn pairing_status(paired: Option<&ChromeProfile>, live: bool, p: &Pal) -> Div {
    match (paired, live) {
        (Some(profile), _) => div()
            .flex()
            .items_center()
            .gap(px(12.))
            .child(badge(profile, 38., p))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(format!("Paired: {}", profile.label)),
                    )
                    .child(account_line(profile, p)),
            )
            .child(
                Icon::new(IconName::CircleCheck)
                    .size(px(20.))
                    .text_color(p.success),
            ),
        (None, false) => div().child(icon_line(
            IconName::Clock,
            p.warn,
            "This link has expired. Ask for a new one.".to_owned(),
        )),
        (None, true) => div()
            .flex()
            .items_center()
            .gap(px(10.))
            .child(Spinner::new().small().color(p.muted))
            .child(note(p, "Waiting for that Chrome profile…")),
    }
}

/// Adding the extension to Chrome, before a profile's link: what happens, then Add to Chrome
/// (nothing is written before it); once written, where and what Chrome does next, with the
/// store's page as the way that works at once.
fn install_group(pane: &ChromePane, p: &Pal, cx: &mut Context<'_, ChromePane>) -> Div {
    let state = pane.store.read(cx).chrome().install().clone();
    let open_store = || {
        Button::new("chrome-store-page")
            .small()
            .label("Open the Chrome Web Store page")
            .on_click(|_, _, cx| cx.open_url(&store_page()))
            .into_any_element()
    };
    let rows = match state {
        InstallState::NotWritten => {
            let store = pane.store.clone();
            vec![
                note(p, install_line(ChromeOs::this())).into_any_element(),
                div()
                    .flex()
                    .gap(px(10.))
                    .child(
                        Button::new("chrome-install")
                            .primary()
                            .label("Add to Chrome")
                            .on_click(move |_, _, cx| {
                                store.update(cx, Store::install_chrome_extension);
                            }),
                    )
                    .child(open_store())
                    .into_any_element(),
            ]
        }
        InstallState::Writing => vec![
            div()
                .flex()
                .items_center()
                .gap(px(10.))
                .child(Spinner::new().small().color(p.muted))
                .child(note(p, "Adding it to Chrome…"))
                .into_any_element(),
        ],
        InstallState::Written(written) => vec![
            icon_line(
                IconName::CircleCheck,
                p.success,
                format!(
                    "{} Chrome's list ({}). {}",
                    if written.already {
                        "Already in"
                    } else {
                        "Added to"
                    },
                    written.place,
                    written.next
                ),
            ),
            note(
                p,
                "Chrome open now? Add it from its Web Store page instead, at once.",
            )
            .into_any_element(),
            open_store(),
        ],
        InstallState::Failed(why) => vec![problem_line(p, why).into_any_element(), open_store()],
    };
    group(p, "Add the extension to Chrome", rows)
}

/// What Add to Chrome does on `os`, said before the user agrees.
#[must_use]
pub(super) fn install_line(os: ChromeOs) -> String {
    let then = match os {
        ChromeOs::Linux => {
            "Your computer asks for your password, and Chrome adds it by itself when it next \
             starts."
        }
        _ => "Chrome then asks you to turn it on: click Enable.",
    };
    format!("{APP_NAME} adds its extension from the Chrome Web Store. {then}")
}

/// How to put the extension in a profile that lacks it.
fn steps(p: &Pal) -> Vec<AnyElement> {
    let step = |n: &'static str, text: &'static str| {
        div()
            .flex()
            .items_start()
            .gap(px(10.))
            .child(
                div()
                    .size(px(20.))
                    .flex_none()
                    .rounded_full()
                    .bg(p.accent)
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(px(11.))
                    .font_weight(FontWeight::BOLD)
                    .text_color(p.on_accent)
                    .child(n),
            )
            .child(div().flex_1().text_size(px(12.5)).child(text))
            .into_any_element()
    };
    vec![
        step(
            "1",
            "Add Emberhut from the Chrome Web Store in that profile (or click Enable when \
             Chrome says “New extension added”).",
        ),
        step("2", "Open the link there again: it asks you to pair."),
        step("3", "Press Pair: this page sees it at once."),
        div()
            .flex()
            .gap(px(10.))
            .child(
                Button::new("chrome-pair-store-page")
                    .small()
                    .label("Open the Chrome Web Store page")
                    .on_click(|_, _, cx| cx.open_url(&store_page())),
            )
            .into_any_element(),
        note(
            p,
            "Working on the extension itself? Load its build unpacked at chrome://extensions \
             (Developer mode).",
        )
        .into_any_element(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_to_chrome_says_what_happens_before_the_user_agrees() {
        let mac = install_line(ChromeOs::Mac);
        assert!(mac.contains("Chrome Web Store") && mac.contains("click Enable"));
        assert_eq!(install_line(ChromeOs::Windows), mac);
        let linux = install_line(ChromeOs::Linux);
        assert!(linux.contains("password") && linux.contains("by itself"));
    }
}
