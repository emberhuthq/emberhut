//! The sign-in screen, shown when the server wants this device signed in, or asks it to while
//! it only reports sessions (then "Not now" lets the user in for this launch).

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::prelude::*;
use gpui_kit::{Div, Entity, FontWeight, div, img, px};

use super::pairing::PairingForm;
use super::palette::Pal;
use crate::assets::FACE;
use crate::brand::APP_NAME;
use crate::store::Store;

/// What the sign-in screen (and Settings, while signed out) says about getting a pairing code.
///
/// It names where codes really come from (`POST /api/auth/pair/new`, docs/api.md): a device that
/// is already signed in (this app makes one in Settings › Devices), or the server's own command,
/// `emberhut-server pair`.
pub(crate) const PAIRING_HINT: &str = "This app opens only for you. Get a pairing code from a \
    device that's already signed in (Settings › Devices › Pair another device), or on the \
    server with emberhut-server pair, and type it here, or paste its pairing link. It works once, \
    for 10 minutes.";

/// The screen; `can_wait` offers "Not now".
pub(crate) fn screen(
    form: &Entity<PairingForm>,
    store: &Entity<Store>,
    can_wait: bool,
    p: &Pal,
) -> Div {
    let store = store.clone();
    div()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .bg(p.surface)
        .child(
            div()
                .flex()
                .flex_col()
                .items_center()
                .gap(px(14.))
                .w(px(460.))
                .child(img(FACE).size(px(84.)).rounded_full())
                .child(
                    div()
                        .text_size(px(22.))
                        .font_weight(FontWeight::BOLD)
                        .child(format!("Sign in to {APP_NAME}")),
                )
                .child(form.clone())
                .when(can_wait, |d| {
                    d.child(
                        Button::new("sign-in-later")
                            .ghost()
                            .label("Not now")
                            .on_click(move |_, _, cx| store.update(cx, Store::sign_in_later)),
                    )
                }),
        )
}

#[cfg(test)]
mod tests {
    use super::PAIRING_HINT;

    #[test]
    fn the_pairing_hint_names_where_codes_really_come_from() {
        for source in [
            "Settings › Devices › Pair another device",
            "emberhut-server pair",
        ] {
            assert!(PAIRING_HINT.contains(source), "the hint misses {source:?}");
        }
        assert!(PAIRING_HINT.contains("works once, for 10 minutes"));
    }

    #[test]
    fn the_pairing_hint_sends_nobody_to_a_screen_that_does_not_exist() {
        for missing in ["Pair a device", "Mac or iPhone", "Android"] {
            assert!(
                !PAIRING_HINT.contains(missing),
                "the hint still says {missing:?}"
            );
        }
    }
}
