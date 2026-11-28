//! "Set up your ember": the first-run setup, in the window's place while the gate says so
//! ([`crate::store::gate::Gate::Setup`]). On paper: the ember on the left at its glow with a
//! short note under it, and on the right the eyebrow, the headline and the five steps' ladder,
//! the open step's questions in it. Once every step is behind, "Your ember is lit." fills the
//! window, with Say hello.
//!
//! The steps reuse what the app has: the pairing form (another computer's code), the provider
//! picker (Light it), the pairing code and its QR code (Let it reach you) and the Chrome
//! profiles' sheet (Show it your world). Its data is the store's ([`crate::store::setup`]).
//! Step 1's done line has Change when the app keeps Emberhut here lit: a sheet with how.

pub(crate) mod banner;
mod ember;
mod keep;
mod plans;
mod steps;
#[cfg(test)]
mod tests;

use emberhut_api::ids::RoomId;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, Div, Entity, FocusHandle, FontWeight, ScrollHandle, TestSupportExt as _, Window,
    div, px, relative,
};

use self::steps::FIND_HINT;

use super::ember::credentials::CredentialsView;
use super::pairing::PairingForm;
use super::palette::{Pal, pal};
use super::qr::PairingQr;
use super::routes;
use super::settings::ChromePane;
use crate::motion;
use crate::store::Store;
use crate::store::settings::SettingsPane;
use crate::store::setup::{BRIGHTEST, SetupStep};

/// The welcome's eyebrow, and every other step's.
pub(crate) const WELCOME: &str = "Welcome to Emberhut";
/// See [`WELCOME`].
pub(crate) const SETTING_UP: &str = "Set up your ember";

/// What the welcome says under its headline.
pub(crate) const LEAD: &str = "Your ember is a personal agent that lives on a computer you own \
    and keeps working while you're away. Two steps get it going; the rest can wait.";

/// The line under the welcome's steps.
pub(crate) const FOOTER: &str =
    "Your ember thinks with Claude, on your own Claude account. No account with us. Open source.";

/// The ember's width beside the steps, and on the "lit" screen.
const EMBER: f32 = 150.;
/// See [`EMBER`].
const EMBER_LIT: f32 = 180.;

/// The setup's view.
pub(crate) struct SetupView {
    store: Entity<Store>,
    /// "Light it": the provider picker, as the step shows it.
    credentials: Entity<CredentialsView>,
    /// "Find your ember": the pairing form for an ember on another computer.
    find: Entity<PairingForm>,
    /// The Chrome profiles' sheets, over the setup while one is open.
    chrome: Entity<ChromePane>,
    /// The phone's pairing code as a QR code.
    qr: PairingQr,
    scroll: ScrollHandle,
    /// The keep-lit sheet's focus, and whether it took it since the sheet opened.
    keep_focus: (FocusHandle, bool),
    /// "Light it": which way with a key is unfolded.
    fold: Option<plans::Fold>,
}

impl SetupView {
    /// The setup over `store`.
    pub(crate) fn new(
        store: Entity<Store>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Self {
        cx.observe(&store, |_, _, cx| cx.notify()).detach();
        let lit = store.clone();
        let credentials = cx.new(|cx| {
            CredentialsView::new(
                store.clone(),
                move |_, cx| lit.update(cx, Store::setup_lit),
                window,
                cx,
            )
            .in_setup()
        });
        let find = cx.new(|cx| PairingForm::new(store.clone(), window, cx).hint(FIND_HINT));
        let chrome = cx.new(|cx| ChromePane::new(store.clone(), window, cx).sheets_only());
        Self {
            store,
            credentials,
            find,
            chrome,
            qr: PairingQr::default(),
            scroll: ScrollHandle::new(),
            keep_focus: (cx.focus_handle(), false),
            fold: None,
        }
    }

    /// The keep-lit sheet while it shows, taking the focus once as it opens.
    fn keep_sheet(
        &mut self,
        p: &Pal,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Option<Div> {
        let shows = self.store.read(cx).setup().shows_keep_sheet();
        let (focus, focused) = &mut self.keep_focus;
        if !shows {
            *focused = false;
            return None;
        }
        if !*focused {
            *focused = true;
            window.focus(focus, cx);
        }
        Some(keep::keep_sheet(&self.store, focus, p, cx))
    }

    /// The steps' side: the eyebrow, the headline (and the welcome's lead), the ladder.
    fn steps(
        &mut self,
        open: SetupStep,
        p: &Pal,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> AnyElement {
        let welcome = open == SetupStep::Home && !self.store.read(cx).setup().is_done(open);
        let eyebrow = if welcome { WELCOME } else { SETTING_UP };
        let (words, word) = open.headline();
        let ladder = self.ladder(open, p, window, cx);
        div()
            .id("setup-scroll")
            .flex_1()
            .min_w_0()
            .h_full()
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .child(
                div()
                    .w_full()
                    .flex()
                    .flex_col()
                    .gap(px(14.))
                    .px(px(48.))
                    .py(px(44.))
                    .max_w(px(800.))
                    .child(
                        div()
                            .text_size(px(12.5))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(p.accent)
                            .child(eyebrow.to_uppercase()),
                    )
                    .child(headline(words, word, 36., p))
                    .when(welcome, |d| {
                        d.child(div().text_size(px(16.)).text_color(p.muted).child(LEAD))
                    })
                    .child(div().pt(px(6.)).child(ladder))
                    .when(welcome, |d| {
                        d.child(div().text_size(px(12.5)).text_color(p.faint).child(FOOTER))
                    }),
            )
            .into_any_element()
    }

    /// "Your ember is lit.": the ember at its brightest, what it is now, Say hello and Share
    /// your ember, and what waits in Settings.
    fn lit(&self, p: &Pal, reduced: bool, cx: &Context<'_, Self>) -> impl IntoElement {
        let read = self.store.read(cx);
        let summary = read.setup_summary();
        let waiting = read.setup_waiting();
        let (hello, share) = (self.store.clone(), self.store.clone());
        div()
            .id("setup-lit")
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(22.))
            .px(px(48.))
            .child(ember::picture(BRIGHTEST, EMBER_LIT, p, reduced))
            .child(headline("Your ember is", "lit.", 44., p))
            .child(
                div()
                    .id("setup-summary")
                    .max_w(px(620.))
                    .text_center()
                    .text_size(px(16.))
                    .text_color(p.muted)
                    .child(summary)
                    .test_support(),
            )
            .child(
                div()
                    .flex()
                    .gap(px(12.))
                    .child(
                        Button::new("setup-say-hello")
                            .primary()
                            .label("Say hello")
                            .on_click(move |_, _, cx| {
                                hello.update(cx, |store, cx| {
                                    store.finish_setup(cx);
                                    store.switch_room(RoomId::MAIN, cx);
                                });
                            }),
                    )
                    .child(
                        Button::new("setup-share")
                            .outline()
                            .label("Share your ember")
                            .on_click(move |_, window, cx| {
                                share.update(cx, Store::finish_setup);
                                routes::open_settings(Some(SettingsPane::Sharing), window, cx);
                            }),
                    ),
            )
            .children(
                waiting.map(|waiting| div().text_size(px(12.5)).text_color(p.faint).child(waiting)),
            )
            .test_support()
    }
}

/// A headline: `words`, then the ember's `word` in the accent, slanted.
fn headline(words: &'static str, word: &'static str, size: f32, p: &Pal) -> Div {
    div()
        .flex()
        .flex_wrap()
        .items_end()
        .gap(px(size * 0.24))
        .text_size(px(size))
        .line_height(relative(1.1))
        .font_weight(FontWeight::MEDIUM)
        .child(words.trim_end())
        .child(div().italic().text_color(p.accent).child(word))
}

impl Render for SetupView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let p = pal(cx);
        let reduced = motion::reduced(cx);
        let read = self.store.read(cx);
        let (open, glow, note) = (
            read.setup().open(),
            read.setup().glow(),
            read.setup().note(),
        );
        let sheet = read.chrome().sheet().is_some();
        let root = div()
            .id("setup")
            .size_full()
            .relative()
            .bg(p.background)
            .text_color(p.text);
        let Some(open) = open else {
            return root.child(self.lit(&p, reduced, cx));
        };
        let steps = self.steps(open, &p, window, cx);
        let keep_sheet = self.keep_sheet(&p, window, cx);
        root.flex()
            .child(
                div()
                    .w(relative(0.34))
                    .max_w(px(420.))
                    .h_full()
                    .flex_none()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap(px(30.))
                    .border_r_1()
                    .border_dashed()
                    .border_color(p.border)
                    .child(ember::picture(glow, EMBER, &p, reduced))
                    .child(
                        div()
                            .id("setup-note")
                            .italic()
                            .text_size(px(24.))
                            .text_color(p.accent)
                            .child(note)
                            .test_support(),
                    ),
            )
            .child(steps)
            .when(sheet, |d| {
                d.child(div().absolute().inset_0().child(self.chrome.clone()))
            })
            .children(keep_sheet)
    }
}
