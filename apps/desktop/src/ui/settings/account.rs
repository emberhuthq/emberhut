//! Settings' Account group (signed out: Sign in; signed in: the account's machines with
//! Unlink, Add this computer, Sign out) and the sheet that shows the device grant's code.

use emberhut_api::directory::LinkedMachine;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{Disableable as _, Sizable as _};
use gpui_kit::prelude::*;
use gpui_kit::{App, Context, Div, ElementId, Entity, FocusHandle, FontWeight, div, px};
use jiff::Timestamp;

use super::parts::sheet;
use crate::account::model::{Account, Phase, SignIn};
use crate::store::devices::relative;
use crate::ui::palette::Pal;
use crate::ui::widgets::{dot, group};

/// What the account is, said once, while signed out.
const ABOUT: &str = "Your emberhut.com account lists your machines on every device you sign in on, \
    and lets them use the emberhut.com relay. It never lets anyone into a machine: devices still \
    pair with a code.";

/// The Account group.
pub(super) fn section(account: &Entity<Account>, p: &Pal, cx: &App) -> Div {
    let read = account.read(cx);
    let mut group = group(p, "ACCOUNT");
    group = match read.phase() {
        Phase::Reading => group.child(muted("Reading your account's sign-in…", p)),
        Phase::SignedOut => group.child(muted(ABOUT, p)).child(
            div().child(
                Button::new("account-sign-in")
                    .primary()
                    .small()
                    .label("Sign in")
                    .on_click(on(account, Account::sign_in)),
            ),
        ),
        Phase::SigningIn(_) => group.child(
            div()
                .flex()
                .items_center()
                .gap(px(10.))
                .child(muted("Signing in…", p))
                .child(
                    Button::new("account-sign-in-cancel")
                        .ghost()
                        .small()
                        .label("Cancel")
                        .on_click(on(account, Account::cancel_sign_in)),
                ),
        ),
        Phase::SignedIn => signed_in(group, account, read, p),
    };
    group.when_some(read.note().map(str::to_owned), |d, note| {
        d.child(div().text_size(px(12.)).text_color(p.muted).child(note))
    })
}

/// The signed-in group's rows: the machines, then the actions.
fn signed_in(mut group: Div, account: &Entity<Account>, read: &Account, p: &Pal) -> Div {
    if let Some(why) = read.unkept() {
        group = group.child(muted(format!("Signed in until the app quits: {why}."), p));
    }
    match read.machines() {
        None => group = group.child(muted("Fetching your machines…", p)),
        Some([]) => group = group.child(muted("No machines in your account yet.", p)),
        Some(machines) => {
            let now = Timestamp::now();
            for machine in machines {
                let asked = read.unlink_asked() == Some(machine.machine.id());
                group = group.child(machine_row(account, machine, asked, now, p));
            }
        }
    }
    let cant_add = read.cant_add_this_computer();
    let linking = read.is_linking();
    group
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .pt(px(4.))
                .child(
                    Button::new("account-add-this")
                        .small()
                        .label(if linking {
                            "Adding this computer…"
                        } else {
                            "Add this computer to your account"
                        })
                        .disabled(linking || cant_add.is_some())
                        .on_click(on(account, Account::add_this_computer)),
                )
                .child(
                    Button::new("account-refresh")
                        .ghost()
                        .small()
                        .label("Refresh")
                        .on_click(on(account, Account::refresh)),
                )
                .child(
                    Button::new("account-sign-out")
                        .ghost()
                        .small()
                        .label("Sign out of your account")
                        .on_click(on(account, Account::sign_out)),
                ),
        )
        .when_some(cant_add, |d, why| d.child(muted(why, p)))
}

/// A machine's row: its name, connected or when it was last seen, and Unlink (confirmed).
fn machine_row(
    account: &Entity<Account>,
    machine: &LinkedMachine,
    asked: bool,
    now: Timestamp,
    p: &Pal,
) -> Div {
    let id = *machine.machine.id();
    let name = machine.name.to_string();
    let (state, color) = if machine.connected {
        ("Connected".to_owned(), p.success)
    } else {
        let seen = machine
            .last_seen
            .and_then(|at| i64::try_from(at.get()).ok())
            .and_then(|millis| Timestamp::from_millisecond(millis).ok())
            .map_or_else(
                || "Never connected".to_owned(),
                |at| format!("Last seen {}", relative(at, now)),
            );
        (seen, p.muted)
    };
    let row = div()
        .flex()
        .items_center()
        .gap(px(10.))
        .child(dot(color, 8.))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .child(div().text_size(px(13.)).child(name.clone()))
                .child(div().text_size(px(11.5)).text_color(p.muted).child(state)),
        );
    if !asked {
        let account = account.clone();
        return row.child(
            Button::new(ElementId::Name(format!("account-unlink-{id}").into()))
                .ghost()
                .small()
                .label("Unlink")
                .on_click(move |_, _, cx| {
                    account.update(cx, |account, cx| account.ask_unlink(id, cx));
                }),
        );
    }
    let (cancel, confirm) = (account.clone(), account.clone());
    row.child(div().text_size(px(12.)).text_color(p.muted).child(format!(
        "Unlink {name}? It leaves the relay until it is added again."
    )))
    .child(
        Button::new(ElementId::Name(
            format!("account-unlink-cancel-{id}").into(),
        ))
        .ghost()
        .small()
        .label("Cancel")
        .on_click(move |_, _, cx| cancel.update(cx, Account::cancel_unlink)),
    )
    .child(
        Button::new(ElementId::Name(
            format!("account-unlink-confirm-{id}").into(),
        ))
        .danger()
        .small()
        .label("Unlink")
        .on_click(move |_, _, cx| {
            confirm.update(cx, |account, cx| account.unlink(id, name.clone(), cx));
        }),
    )
}

/// The sheet over Settings while a sign-in is under way: the device grant's code with the page
/// to type it on, or a word that the sign-in window is open.
pub(super) fn sheet_for(
    account: &Entity<Account>,
    focus: &FocusHandle,
    p: &Pal,
    cx: &App,
) -> Option<Div> {
    let Phase::SigningIn(step) = account.read(cx).phase() else {
        return None;
    };
    let body = match step {
        SignIn::Starting => div().child(muted("Asking your account for a sign-in code…", p)),
        SignIn::Window => div().child(muted("Finish signing in in the window that opened.", p)),
        SignIn::Code { user_code, page } => code_body(user_code, page, p),
    };
    let cancel = account.downgrade();
    Some(sheet(
        p,
        focus,
        "Sign in to your account",
        body,
        ("Cancel", move |_, cx| {
            if let Err(error) = cancel.update(cx, Account::cancel_sign_in) {
                log::debug!("the account went with its sheet: {error}");
            }
        }),
    ))
}

/// The code to type and the page to type it on.
fn code_body(user_code: &str, page: &str, p: &Pal) -> Div {
    let page = page.to_owned();
    div()
        .flex()
        .flex_col()
        .items_center()
        .gap(px(12.))
        .py(px(8.))
        .child(muted(
            "Open the sign-in page in your browser, sign in, and confirm this code there:",
            p,
        ))
        .child(
            div()
                .id("account-user-code")
                .text_size(px(28.))
                .font_weight(FontWeight::BOLD)
                .child(user_code.to_owned()),
        )
        .child(
            Button::new("account-open-page")
                .primary()
                .label("Open the sign-in page")
                .on_click(move |_, _, cx| {
                    // The session already refused any other; this is the last look before the
                    // system opens it.
                    if page.starts_with("https://") {
                        cx.open_url(&page);
                    }
                }),
        )
        .child(muted(
            "This sheet closes by itself once the code is confirmed.",
            p,
        ))
}

/// A line of muted text.
fn muted(text: impl Into<gpui_kit::SharedString>, p: &Pal) -> Div {
    div()
        .text_size(px(12.5))
        .text_color(p.muted)
        .child(text.into())
}

/// A click that runs `action` on the account.
fn on(
    account: &Entity<Account>,
    action: fn(&mut Account, &mut Context<'_, Account>),
) -> impl Fn(&gpui_kit::ClickEvent, &mut gpui_kit::Window, &mut App) + 'static {
    let account = account.clone();
    move |_, _, cx| account.update(cx, action)
}
