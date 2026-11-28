//! The setup's ladder of five steps: each a numbered row with its name and its plain line, the
//! open one with what it asks under it, a done one folded to the line saying what came of it,
//! the optional ones marked. What each step asks is drawn here too.

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{Disableable as _, Icon, Sizable as _};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, Div, ElementId, Entity, FontWeight, ImageSource, Role, SharedString,
    TestSupportExt as _, Window, div, img, px,
};

use super::SetupView;
use crate::store::Store;
use crate::store::devices::NewCode;
use crate::store::settings::chrome::ChromeSheet;
use crate::store::setup::{Here, HomeChoice, SetupStep, StepState, Tile, TileState, WAITING};
use crate::ui::a11y;
use crate::ui::palette::Pal;
use crate::ui::settings::parts::{note, tag};

/// The line over the pairing form on the first step, for an ember on another computer. It
/// names where a code comes from in the app on that computer (Settings › Devices).
pub(crate) const FIND_HINT: &str = "On the computer your ember lives on, open Emberhut and \
    choose Settings › Devices › Pair another device, then type the code here.";

/// The title over that form.
pub(crate) const FIND_TITLE: &str = "Find your ember";

/// The three lines beside the phone's QR code.
pub(crate) const REACH_LINES: [&str; 3] = [
    "Point your phone's camera at the code.",
    "No app yet? It takes you to the store first.",
    "Works once, for ten minutes.",
];

/// The step "Share your ember" shows: where sharing is, as the app has it today.
pub(crate) const SHARE_LINE: &str =
    "Let someone else talk to it in Slack: early preview, in Settings › Sharing.";

/// The line under the world's tiles.
pub(crate) const WHATSAPP_NEXT: &str = "WhatsApp is coming next.";

/// "Here" on the first step, named for this computer.
fn here_title() -> &'static str {
    if cfg!(target_os = "macos") {
        "Here, on this Mac"
    } else {
        "Here, on this computer"
    }
}

/// What "Here" says under its name.
fn here_line() -> &'static str {
    if cfg!(target_os = "macos") {
        "Your hut runs here while your Mac is awake. About two minutes."
    } else {
        "Your hut runs here while this computer is awake. About two minutes."
    }
}

/// Why an always-on home matters, under the homes.
fn always_on_line() -> &'static str {
    if cfg!(target_os = "macos") {
        "More huts keep it lit. With its only hut on this Mac, your ember stops when the Mac \
         sleeps or is off; an always-on hut (a PC, a home server or a VPS) keeps it working."
    } else {
        "More huts keep it lit. With its only hut on this computer, your ember stops when it \
         sleeps or is off; an always-on hut (a PC, a home server or a VPS) keeps it working."
    }
}

/// What to do on the always-on computer, over the pairing form.
pub(crate) const ALWAYS_ON_HOW: &str = "On that hut, run Emberhut's server, then \
     emberhut-server pair: type the code it prints here.";

impl SetupView {
    /// The ladder: every step's row, the open one with what it asks.
    pub(super) fn ladder(
        &mut self,
        open: SetupStep,
        p: &Pal,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> AnyElement {
        // Only the open step: the rail beside it shows every step and what each unlocks.
        let body = self.body(open, p, window, cx);
        let rows = vec![self.row(open, open, Some(body), p, cx).into_any_element()];
        div()
            .id("setup-ladder")
            .flex()
            .flex_col()
            .px(px(24.))
            .py(px(6.))
            .rounded(px(16.))
            .border_1()
            .border_color(p.border)
            .bg(p.surface)
            .shadow_sm()
            .children(rows)
            .into_any_element()
    }

    /// A step's row: its mark, its name (and "optional"), its line or what came of it, and
    /// `body` under them while it is open.
    fn row(
        &self,
        step: SetupStep,
        open: SetupStep,
        body: Option<AnyElement>,
        p: &Pal,
        cx: &Context<'_, Self>,
    ) -> impl IntoElement {
        let state = self.store.read(cx).setup().state(step).clone();
        let is_open = step == open;
        let (line, line_color) = match &state {
            StepState::Done(line) if !line.is_empty() => (line.clone(), p.muted),
            StepState::Done(_) | StepState::Ahead => (step.line().to_owned(), p.muted),
            StepState::Skipped => (step.line().to_owned(), p.faint),
        };
        let done = matches!(state, StepState::Done(_));
        let spoken = a11y::line(&[
            step.name(),
            &line,
            if done {
                "done"
            } else if is_open {
                "open"
            } else if step.optional() {
                "optional"
            } else {
                ""
            },
        ]);
        let head = div()
            .flex()
            .items_start()
            .justify_between()
            .gap(px(12.))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(3.))
                    .child(
                        div()
                            .text_size(px(16.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(if matches!(state, StepState::Skipped) {
                                p.muted
                            } else {
                                p.text
                            })
                            .child(step.name()),
                    )
                    .child(div().text_size(px(13.5)).text_color(line_color).child(line)),
            )
            .when(
                step == SetupStep::Home && done && self.store.read(cx).setup().keep().is_some(),
                |d| d.child(super::keep::change_button(&self.store).flex_none()),
            )
            .when(step.optional() && !done, |d| {
                d.child(
                    div()
                        .flex_none()
                        .text_size(px(12.))
                        .text_color(p.faint)
                        .child(if matches!(state, StepState::Skipped) {
                            "later"
                        } else {
                            "optional"
                        }),
                )
            });
        div()
            .id(ElementId::Name(
                format!("setup-step-{}", step.index() + 1).into(),
            ))
            .role(Role::Group)
            .aria_label(spoken)
            .flex()
            .gap(px(14.))
            .py(px(16.))
            .child(mark(step, &state, is_open, p))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .gap(px(14.))
                    .child(head)
                    .children(body),
            )
            .test_support()
    }

    /// What the open `step` asks.
    fn body(
        &mut self,
        step: SetupStep,
        p: &Pal,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> AnyElement {
        match step {
            SetupStep::Home => self.home(p, cx).into_any_element(),
            SetupStep::Light => self.light(p, cx).into_any_element(),
            SetupStep::Yours => self.yours(p, window, cx).into_any_element(),
            SetupStep::Reach => self.reach(p, window, cx).into_any_element(),
            SetupStep::World => self.world(p, cx).into_any_element(),
            SetupStep::Share => share(&self.store, p).into_any_element(),
        }
    }

    /// Step 1: here or another computer, Continue, and how it goes.
    fn home(&self, p: &Pal, cx: &Context<'_, Self>) -> Div {
        let setup = self.store.read(cx).setup();
        let choice = setup.choice();
        let here = setup.here().clone();
        let busy = matches!(here, Here::Starting | Here::Pairing);
        let status: Option<AnyElement> = match &here {
            Here::Idle => None,
            Here::Starting => Some(working(p, "Starting your hut…").into_any_element()),
            Here::Pairing => Some(working(p, "Pairing this app with it…").into_any_element()),
            Here::TypeCode => Some(
                note(
                    p,
                    "Emberhut already runs here. Type a pairing code from it.",
                )
                .text_size(px(13.))
                .into_any_element(),
            ),
            Here::Failed(why) => Some(
                div()
                    .id("setup-home-problem")
                    .role(Role::Status)
                    .aria_label(why.clone())
                    .text_size(px(13.))
                    .text_color(p.warn)
                    .child(why.clone())
                    .test_support()
                    .into_any_element(),
            ),
        };
        let store = self.store.clone();
        let continue_button = Button::new("setup-continue")
            .primary()
            .label("Continue")
            .loading(busy)
            .disabled(busy)
            .on_click(move |_, _, cx| store.update(cx, Store::continue_home));
        let shows_form = setup.shows_elsewhere() || here == Here::TypeCode;
        div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(
                div()
                    .id("setup-home-choices")
                    .role(Role::RadioGroup)
                    .aria_label("Your ember's hut")
                    .flex()
                    .flex_col()
                    .gap(px(10.))
                    .child(choice_card(
                        &self.store,
                        HomeChoice::Here,
                        choice,
                        IconName::Laptop,
                        here_title(),
                        Some("Recommended"),
                        here_line(),
                        p,
                    ))
                    .child(choice_card(
                        &self.store,
                        HomeChoice::Elsewhere,
                        choice,
                        IconName::Server,
                        "Another hut",
                        None,
                        "A PC, home server or VPS that's always on. You'll need a code from it.",
                        p,
                    )),
            )
            .child(cloud_card(
                &self.store,
                self.store.read(cx).on_cloud_waitlist(),
                p,
            ))
            .child(always_on(&self.store, p))
            .children(status)
            .when(here != Here::TypeCode, |d| {
                d.child(div().flex().pt(px(4.)).child(continue_button))
            })
            .when(shows_form, |d| {
                d.child(
                    div()
                        .id("setup-find")
                        .flex()
                        .flex_col()
                        .gap(px(8.))
                        .pt(px(6.))
                        .child(
                            div()
                                .text_size(px(15.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(FIND_TITLE),
                        )
                        .child(note(p, ALWAYS_ON_HOW).text_size(px(12.5)))
                        .child(self.find.clone())
                        .test_support(),
                )
            })
    }

    /// Step 2: the picker of what the ember thinks with, and Later.
    fn light(&self, p: &Pal, cx: &Context<'_, Self>) -> Div {
        let store = self.store.clone();
        div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(self.plans(p, cx))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .child(
                        Button::new("setup-later")
                            .ghost()
                            .small()
                            .label("Later")
                            .on_click(move |_, _, cx| store.update(cx, Store::setup_later)),
                    )
                    .child(note(p, "Your ember can't answer until it's lit.")),
            )
    }

    /// Step 3: the pairing code's QR code and what to do with it, and Later.
    fn reach(&mut self, p: &Pal, window: &Window, cx: &mut Context<'_, Self>) -> Div {
        let code = self.store.read(cx).new_code().cloned();
        let qr = self.qr.of(code.as_ref(), window.scale_factor(), cx);
        let picture = match (&code, qr) {
            (_, Some(qr)) => img(ImageSource::Render(qr.picture))
                .id("setup-qr")
                .size(px(qr.side))
                .into_any_element(),
            (Some(NewCode::Made(code)), None) => div()
                .id("setup-code")
                .text_size(px(20.))
                .font_weight(FontWeight::BOLD)
                .child(code.code.clone())
                .test_support()
                .into_any_element(),
            (Some(NewCode::Making), None) => Spinner::new().color(p.muted).into_any_element(),
            (None, None) => {
                let again = self.store.clone();
                Button::new("setup-code-again")
                    .small()
                    .label("Try again")
                    .on_click(move |_, _, cx| again.update(cx, Store::make_pairing_code))
                    .into_any_element()
            }
        };
        let store = self.store.clone();
        div()
            .flex()
            .items_start()
            .gap(px(22.))
            .child(
                div()
                    .flex_none()
                    .min_w(px(120.))
                    .min_h(px(120.))
                    .p(px(8.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(12.))
                    .border_1()
                    .border_color(p.border)
                    .bg(p.surface)
                    .child(picture),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .pt(px(6.))
                    .children(
                        REACH_LINES
                            .map(|line| div().text_size(px(13.5)).text_color(p.muted).child(line)),
                    )
                    .child(
                        div().flex().pt(px(6.)).child(
                            Button::new("setup-later")
                                .outline()
                                .label("Later")
                                .on_click(move |_, _, cx| store.update(cx, Store::setup_later)),
                        ),
                    ),
            )
    }

    /// Step 4: a tile for each connection, WhatsApp next, and Later (Continue once one is
    /// connected).
    fn world(&self, p: &Pal, cx: &Context<'_, Self>) -> Div {
        let read = self.store.read(cx);
        let paired = !read.chrome_profiles().is_empty();
        let states: Vec<(Tile, TileState)> = Tile::ALL
            .into_iter()
            .map(|tile| (tile, read.setup().tile(tile, paired)))
            .collect();
        let any = states
            .iter()
            .any(|(_, state)| *state == TileState::Connected);
        let tiles: Vec<AnyElement> = states
            .into_iter()
            .map(|(tile, state)| world_tile(&self.store, tile, state, p).into_any_element())
            .collect();
        let store = self.store.clone();
        let next = Button::new("setup-world-next")
            .outline()
            .label(if any { "Continue" } else { "Later" })
            .on_click(move |_, _, cx| store.update(cx, Store::world_done));
        div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(div().flex().flex_wrap().gap(px(10.)).children(tiles))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(10.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(
                                Icon::new(IconName::MessageCircle)
                                    .size(px(15.))
                                    .text_color(p.muted),
                            )
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .text_color(p.muted)
                                    .child(WHATSAPP_NEXT),
                            ),
                    )
                    .child(next),
            )
    }
}

/// A step's mark: its number in a ring (the open one's in the accent), a check once done.
fn mark(step: SetupStep, state: &StepState, open: bool, p: &Pal) -> Div {
    let ring = div()
        .flex_none()
        .size(px(28.))
        .rounded_full()
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(13.))
        .font_weight(FontWeight::SEMIBOLD);
    match state {
        StepState::Done(_) => ring.bg(p.success).child(
            Icon::new(IconName::Check)
                .size(px(15.))
                .text_color(p.surface),
        ),
        _ if open => ring
            .border_2()
            .border_color(p.accent)
            .text_color(p.accent)
            .bg(p.accent.opacity(0.08))
            .child((step.index() + 1).to_string()),
        StepState::Skipped => ring
            .border_1()
            .border_color(p.border)
            .text_color(p.faint)
            .child((step.index() + 1).to_string()),
        StepState::Ahead => ring
            .border_1()
            .border_color(p.border)
            .text_color(p.muted)
            .child((step.index() + 1).to_string()),
    }
}

/// One of the first step's two choices, as a card with a radio mark.
#[expect(
    clippy::too_many_arguments,
    reason = "a card's parts, each said once by the step that draws it"
)]
fn choice_card(
    store: &Entity<Store>,
    choice: HomeChoice,
    picked: HomeChoice,
    glyph: IconName,
    title: &'static str,
    tag_words: Option<&'static str>,
    line: &'static str,
    p: &Pal,
) -> impl IntoElement {
    let on = choice == picked;
    let store = store.clone();
    let id = match choice {
        HomeChoice::Here => "setup-home-here",
        HomeChoice::Elsewhere => "setup-home-elsewhere",
    };
    div()
        .id(id)
        .role(Role::RadioButton)
        .aria_label(a11y::line(&[title, tag_words.unwrap_or_default(), line]))
        .aria_selected(on)
        .flex()
        .items_center()
        .gap(px(12.))
        .px(px(16.))
        .py(px(12.))
        .rounded(px(12.))
        .border_1()
        .border_color(if on { p.accent } else { p.border })
        .bg(if on { p.accent_soft } else { p.surface })
        .cursor_pointer()
        .hover(|d| d.bg(p.hover))
        .on_click(move |_, _, cx| store.update(cx, |store, cx| store.choose_home(choice, cx)))
        .child(
            Icon::new(if on {
                IconName::CircleDot
            } else {
                IconName::Circle
            })
            .size(px(17.))
            .text_color(if on { p.text } else { p.faint }),
        )
        .child(Icon::new(glyph).size(px(17.)).text_color(p.muted))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .gap(px(2.))
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap(px(8.))
                        .child(
                            div()
                                .text_size(px(14.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(title),
                        )
                        .children(tag_words.map(|words| tag(words, p.accent))),
                )
                .child(div().text_size(px(12.5)).text_color(p.muted).child(line)),
        )
        .test_support()
}

/// Renting a home in the cloud: coming, not a choice yet. "Tell me when it's ready" puts the
/// person on the waitlist (kept on their server), and then says so.
fn cloud_card(store: &Entity<Store>, joined: bool, p: &Pal) -> impl IntoElement {
    let store = store.clone();
    let action: AnyElement = if joined {
        div()
            .id("setup-cloud-joined")
            .role(Role::Status)
            .flex()
            .items_center()
            .gap(px(6.))
            .text_size(px(12.5))
            .text_color(p.success)
            .child(Icon::new(IconName::CircleCheck).size(px(14.)))
            .child("You're on the list")
            .into_any_element()
    } else {
        Button::new("setup-cloud-notify")
            .ghost()
            .small()
            .label("Tell me when it's ready")
            .on_click(move |_, _, cx| store.update(cx, Store::join_cloud_waitlist))
            .into_any_element()
    };
    div()
        .id("setup-home-cloud")
        .role(Role::Group)
        .aria_label("Rent a hut in the cloud, coming soon")
        .flex()
        .items_center()
        .gap(px(12.))
        .px(px(16.))
        .py(px(12.))
        .rounded(px(12.))
        .border_1()
        .border_dashed()
        .border_color(p.border)
        .child(div().w(px(17.)))
        .child(Icon::new(IconName::Cloud).size(px(17.)).text_color(p.faint))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .gap(px(2.))
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap(px(8.))
                        .child(
                            div()
                                .text_size(px(14.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(p.muted)
                                .child("Rent a hut in the cloud"),
                        )
                        .child(tag("Coming soon", p.muted)),
                )
                .child(
                    div()
                        .text_size(px(12.5))
                        .text_color(p.muted)
                        .child("An always-on hut we run for you. Nothing to set up."),
                ),
        )
        .child(action)
        .test_support()
}

/// Why an always-on home matters: a small picture (this computer asleep, an always-on one
/// lit) and a line.
fn always_on(store: &Entity<Store>, p: &Pal) -> impl IntoElement {
    let store = store.clone();
    let place =
        |glyph: IconName, badge: Option<IconName>, color, badge_color, words: &'static str| {
            div()
                .flex()
                .flex_col()
                .items_center()
                .gap(px(3.))
                .child(
                    div()
                        .relative()
                        .size(px(30.))
                        .child(Icon::new(glyph).size(px(26.)).text_color(color))
                        .child(
                            div()
                                .absolute()
                                .top(px(-4.))
                                .right(px(-6.))
                                .child(match badge {
                                    // Asleep: a moon.
                                    Some(badge) => Icon::new(badge)
                                        .size(px(13.))
                                        .text_color(badge_color)
                                        .into_any_element(),
                                    // On: a lit dot (a flame on a server would read as a fire).
                                    None => div()
                                        .mt(px(3.))
                                        .size(px(8.))
                                        .rounded_full()
                                        .bg(badge_color)
                                        .into_any_element(),
                                }),
                        ),
                )
                .child(div().text_size(px(10.5)).text_color(p.muted).child(words))
        };
    div()
        .id("setup-always-on")
        .role(Role::Note)
        .aria_label(always_on_line())
        .flex()
        .items_center()
        .gap(px(14.))
        .px(px(4.))
        .pt(px(2.))
        .child(
            div()
                .flex()
                .items_end()
                .gap(px(14.))
                .flex_none()
                .child(place(
                    IconName::Laptop,
                    Some(IconName::Moon),
                    p.faint,
                    p.muted,
                    "Asleep",
                ))
                .child(place(
                    IconName::Server,
                    None,
                    p.text,
                    p.success,
                    "Always on",
                )),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_size(px(12.5))
                .text_color(p.muted)
                .child(always_on_line()),
        )
        .child(
            Button::new("setup-add-hut")
                .ghost()
                .small()
                .label("Add another hut")
                .tooltip("Pair an always-on computer with a code from it")
                .on_click(move |_, _, cx| {
                    store.update(cx, |store, cx| {
                        store.choose_home(HomeChoice::Elsewhere, cx);
                        store.continue_home(cx);
                    });
                }),
        )
        .test_support()
}

/// A line with a spinner: something under way.
fn working(p: &Pal, text: &'static str) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .child(Spinner::new().small().color(p.muted))
        .child(div().text_size(px(13.)).text_color(p.muted).child(text))
}

/// A connection's tile: its name and what it does, and Connect, Connected, or where it waits.
/// Chrome's Connect opens the Chrome profiles' sheet for a new profile; the others have nothing
/// to connect from here yet, so theirs says where they wait.
fn world_tile(store: &Entity<Store>, tile: Tile, state: TileState, p: &Pal) -> impl IntoElement {
    let glyph = match tile {
        Tile::Calendar => IconName::Calendar,
        Tile::Gmail => IconName::Mail,
        Tile::Slack => IconName::Hash,
        Tile::Chrome => IconName::Globe,
    };
    let action: AnyElement = match state {
        TileState::Connected => tag("Connected", p.success).into_any_element(),
        TileState::Failing => tag("Needs a look", p.warn).into_any_element(),
        TileState::Waiting => div().into_any_element(),
        TileState::Open => {
            let store = store.clone();
            Button::new(ElementId::Name(
                format!("setup-connect-{}", tile.name()).into(),
            ))
            .outline()
            .small()
            .label("Connect")
            .on_click(move |_, _, cx| {
                store.update(cx, |store, cx| match tile {
                    Tile::Chrome => store.show_chrome_sheet(Some(ChromeSheet::Add), cx),
                    _ => store.ask_tile(tile, cx),
                });
            })
            .into_any_element()
        }
    };
    let line: SharedString = if state == TileState::Waiting {
        WAITING.into()
    } else {
        tile.line().into()
    };
    div()
        .id(ElementId::Name(
            format!("setup-tile-{}", tile.name()).into(),
        ))
        .role(Role::Group)
        .aria_label(a11y::line(&[tile.name(), &line]))
        .w(px(268.))
        .flex()
        .items_center()
        .gap(px(12.))
        .px(px(14.))
        .py(px(12.))
        .rounded(px(12.))
        .border_1()
        .border_color(p.border)
        .bg(p.surface)
        .child(Icon::new(glyph).size(px(20.)).text_color(p.muted))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .gap(px(2.))
                .child(
                    div()
                        .text_size(px(14.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(tile.name()),
                )
                .child(div().text_size(px(12.5)).text_color(p.muted).child(line)),
        )
        .child(action)
        .test_support()
}

/// Step 5: where sharing is, and Done.
fn share(store: &Entity<Store>, p: &Pal) -> Div {
    let store = store.clone();
    div()
        .flex()
        .flex_col()
        .gap(px(12.))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(12.))
                .px(px(16.))
                .py(px(12.))
                .rounded(px(12.))
                .border_1()
                .border_color(p.border)
                .bg(p.surface)
                .child(Icon::new(IconName::Users).size(px(18.)).text_color(p.muted))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_size(px(13.5))
                        .child(SHARE_LINE),
                )
                .child(tag("Early preview", p.muted)),
        )
        .child(
            div().flex().child(
                Button::new("setup-share-done")
                    .primary()
                    .label("Done")
                    .on_click(move |_, _, cx| store.update(cx, Store::finish_setup)),
            ),
        )
}
