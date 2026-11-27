//! Settings' App section, as the reference client's update rows read: the version and build,
//! when it last checked (and how old the server's feed was), "Ready to install Build N" with
//! Restart to Update, or Check for updates, and how updates work. Off the Mac it shows the
//! version and says updates come with the Mac app.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{Disableable as _, Sizable as _};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, Div, Entity, div, px};

use super::parts::{note, section};
use crate::store::Store;
use crate::store::updates::{HOW_IT_WORKS, Rows};
use crate::ui::palette::Pal;

/// The section, over `store`.
pub(super) fn section_for(store: &Entity<Store>, p: &Pal, cx: &App) -> Div {
    let rows = store.read(cx).updates().rows(std::time::SystemTime::now());
    let footer = rows.last_checked.is_some().then(|| HOW_IT_WORKS.to_owned());
    section(p, "App", None, lines(&rows, store, p), footer)
}

fn lines(rows: &Rows, store: &Entity<Store>, p: &Pal) -> Vec<AnyElement> {
    let mut lines = vec![fact("Version", rows.version.clone(), p).into_any_element()];
    if let Some(checked) = &rows.last_checked {
        let value = match &rows.published {
            Some(published) => format!("{checked} · feed {published}"),
            None => checked.clone(),
        };
        lines.push(fact("Last checked", value, p).into_any_element());
    }
    if let Some(ready) = &rows.ready {
        lines.push(fact("Ready to install", ready.clone(), p).into_any_element());
        let store = store.clone();
        lines.push(
            div()
                .child(
                    Button::new("settings-restart-to-update")
                        .primary()
                        .small()
                        .label("Restart to Update")
                        .on_click(move |_, _, cx| store.update(cx, Store::restart_to_update)),
                )
                .into_any_element(),
        );
    } else if rows.last_checked.is_some() {
        lines.push(
            div()
                .child(
                    Button::new("settings-check-for-updates")
                        .small()
                        .label("Check for updates")
                        .disabled(!rows.can_check)
                        .on_click(|_, window, cx| {
                            crate::ui::update_dialog::check_for_updates(window, cx);
                        }),
                )
                .into_any_element(),
        );
    }
    if let Some(status) = &rows.status {
        lines.push(note(p, status.clone()).into_any_element());
    }
    lines
}

/// A row: its label, and its value in the secondary colour.
fn fact(label: &'static str, value: String, p: &Pal) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(12.))
        .text_size(px(13.))
        .child(div().w(px(120.)).child(label))
        .child(div().text_color(p.muted).child(value))
}
