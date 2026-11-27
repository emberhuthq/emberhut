//! The pairing form: a one-time code from a signed-in device, or its pairing link pasted
//! (`https://emberhut.com/pair#…`), traded for this device's own session. Shown on the sign-in
//! screen and in Settings.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::prelude::*;
use gpui_kit::{Entity, Window, div, px};

use emberhut_identity::{PairingLink, PairingLinkError};

use super::palette::pal;
use super::sign_in::PAIRING_HINT;
use crate::store::Store;

/// The form.
pub(crate) struct PairingForm {
    store: Entity<Store>,
    code: Entity<InputState>,
    note: Option<String>,
    /// A code is on its way: a second click waits (a code works once).
    pairing: bool,
}

impl PairingForm {
    /// The form over `store`.
    pub(crate) fn new(
        store: Entity<Store>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Self {
        cx.observe(&store, |_, _, cx| cx.notify()).detach();
        let code = cx.new(|cx| InputState::new(window, cx).placeholder("ABCD-EFGH"));
        Self {
            store,
            code,
            note: None,
            pairing: false,
        }
    }

    fn pair(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.pairing {
            return;
        }
        let code = self.code.read(cx).value().trim().to_owned();
        if code.is_empty() {
            self.note = Some("Type the pairing code, or paste its link.".to_owned());
            cx.notify();
            return;
        }
        let link = match as_link(&code) {
            None => None,
            Some(Ok(link)) => Some(link),
            Some(Err(error)) => {
                self.note = Some(format!("That link can't be used: {error}."));
                cx.notify();
                return;
            }
        };
        self.note = Some("Checking…".to_owned());
        self.pairing = true;
        let form = cx.entity().downgrade();
        let done = move |outcome: Result<(), String>, cx: &mut Context<'_, Store>| {
            let note = match outcome {
                Ok(()) => "Signed in: this device has its own session now.".to_owned(),
                Err(reason) => reason,
            };
            let shown = form.update(cx, |form, cx| {
                form.note = Some(note);
                form.pairing = false;
                cx.notify();
            });
            if let Err(error) = shown {
                log::debug!("the pairing form closed before the answer came: {error}");
            }
        };
        self.store.update(cx, |store, cx| match link {
            Some(link) => store.pair_link(link, cx, done),
            None => store.pair(code, cx, done),
        });
        self.code
            .update(cx, |code, cx| code.set_value("", window, cx));
    }
}

/// The pairing link the field holds, read, or `None` for a code. A link is anything a code
/// can't be: an address (`://`), the link without its scheme (`emberhut.com/pair…`), or its
/// fragment alone (`#id=…`, `id=…`), so a link is never sent as if it were a code.
fn as_link(text: &str) -> Option<Result<PairingLink, PairingLinkError>> {
    if let Some(fragment) = text
        .strip_prefix('#')
        .or(Some(text))
        .filter(|fragment| fragment.starts_with("id="))
    {
        return Some(PairingLink::from_fragment(fragment));
    }
    if !(text.contains("://") || text.contains("emberhut.com/pair") || text.contains("#id=")) {
        return None;
    }
    let whole = if text.starts_with("emberhut.com/") {
        format!("https://{text}")
    } else {
        text.to_owned()
    };
    Some(PairingLink::parse(&whole))
}

impl Render for PairingForm {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let p = pal(cx);
        let signed_in = self.store.read(cx).is_signed_in();
        let line = if signed_in {
            "This device is signed in with its own session. To sign in again, type a new code."
        } else {
            PAIRING_HINT
        };
        div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(div().text_size(px(13.)).text_color(p.muted).child(line))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(div().w(px(200.)).child(Input::new(&self.code)))
                    .child(
                        Button::new("pair")
                            .primary()
                            .label("Sign in")
                            .on_click(cx.listener(|this, _, window, cx| this.pair(window, cx))),
                    ),
            )
            .when_some(self.note.clone(), |d, note| {
                d.child(div().text_size(px(12.5)).text_color(p.muted).child(note))
            })
    }
}

#[cfg(test)]
mod tests {
    use emberhut_identity::SecretKey;

    use super::*;

    #[test]
    fn a_link_is_told_from_a_code_however_it_is_pasted() {
        let id = SecretKey::from_bytes(&[7; 32]).machine_id();
        let link = PairingLink::new(id, "ABCD-EFGH", vec!["http://127.0.0.1:8791".into()])
            .unwrap()
            .to_url();
        let fragment = link.split_once('#').unwrap().1.to_owned();
        for pasted in [
            link.clone(),
            link.trim_start_matches("https://").to_owned(),
            format!("#{fragment}"),
            fragment,
        ] {
            let read = as_link(&pasted).expect("a link").expect("it reads");
            assert_eq!(read.id(), &id, "{pasted}");
            assert_eq!(read.code(), "ABCD-EFGH");
        }
        assert!(as_link("abcd efgh").is_none());
        assert!(as_link("ABCD-EFGH").is_none());
        assert!(
            matches!(as_link("https://example.com/pair#id=x"), Some(Err(_))),
            "another address is a link that doesn't read, never a code"
        );
    }
}
