//! What each group page holds, built of the rows and sections Settings already had: Your
//! ember (on duty, what it thinks with and the usage, what it knows, the models under
//! Technical details), Devices, Connections (data sources and Chrome, the tool servers and the
//! skills folded), Sharing, Privacy (what it knows, archived rooms, the dark room, where data
//! goes), Advanced (updates, the server, diagnostics folded, where the agent works) and the
//! account.

use gpui_kit::TestSupportExt as _;
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{Icon, Sizable as _};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, Div, FontWeight, div, img, px};

use super::SettingsView;
use super::group::{Block, block, plain};
use super::parts::{nav_row, note, problem_line, section, tag};
use super::{account, places, switches, thinks_with_row, updates, usage};
use crate::assets::FACE;
use crate::store::settings::SettingsPane;
use crate::store::settings::chrome::{self as chrome_words, Health};
use crate::store::settings::entries::{Fold, duty_word};
use crate::store::settings::now::Now;
use crate::store::{Page, Store};
use crate::ui::column;
use crate::ui::palette::{Pal, category_color};
use crate::ui::settings_more;
use crate::ui::widgets::group;

/// Where the agent's model sends what it reads, said once on Privacy.
const DATA_FLOW: &str = "Your messages, and what your ember reads to answer them (facts about \
    you, mail, pages), go to the model you picked. Gmail and Slack are read from their own \
    servers. Phone alerts pass through Apple's push service.";

/// Sharing's one honest line.
const SHARING: &str = "Today sharing is set up through your server's API, for developers. \
    Inviting someone from here comes next.";

impl SettingsView {
    /// Your ember's blocks.
    pub(super) fn ember_blocks(
        &self,
        now: &Now,
        p: &Pal,
        cx: &mut Context<'_, Self>,
    ) -> Vec<Block> {
        let store = self.store.read(cx);
        let problem = store.on_duty_problem().map(str::to_owned);
        let hero = self.hero(now, p);
        let mut blocks = vec![block(
            &["on-duty"],
            div()
                .flex()
                .flex_col()
                .gap(px(6.))
                .child(hero)
                .children(problem.map(|problem| problem_line(p, problem))),
        )];
        blocks.push(block(
            &["thinks-with"],
            section(
                p,
                "What it thinks with",
                None,
                vec![thinks_with_row(store, p)],
                None,
            ),
        ));
        let has_usage = store.state().is_some_and(|state| !state.usage.is_empty());
        if has_usage {
            blocks.push(block(&["usage"], usage::card(&self.store, p, cx)));
        }
        let knows = match (now.facts, now.lessons) {
            (Some(facts), 0) => format!("{facts} things"),
            (Some(facts), lessons) => format!("{facts} things · {lessons} lessons to review"),
            (None, _) => "Facts, and lessons from your corrections".to_owned(),
        };
        let health = if now.lessons > 0 {
            Health::Warn
        } else {
            Health::Off
        };
        let about = nav_row(
            "about-you-row",
            p,
            (IconName::BookOpen, category_color(0)),
            crate::store::settings::about::TITLE,
            (knows, health),
            "",
        )
        .on_click(Self::opens(SettingsPane::AboutYou))
        .test_support();
        blocks.push(block(
            &["about-you", "lessons"],
            section(
                p,
                "What it knows",
                None,
                vec![about.into_any_element()],
                None,
            ),
        ));
        let models = now
            .models
            .clone()
            .unwrap_or_else(|| "Shows once a provider is saved".to_owned());
        let key = store
            .credentials()
            .saved()
            .and_then(|saved| saved.key_hint.clone())
            .map(|hint| format!("Key ending {hint}"));
        let mut rows = vec![fact_row("Models", models, p)];
        rows.extend(key.map(|key| fact_row("Key", key, p)));
        let details = self.fold(
            Fold::EmberDetails,
            ("Technical details", "the models it runs on".to_owned()),
            vec![section(p, "", None, rows, None).into_any_element()],
            p,
            cx,
        );
        blocks.push(block(&["models"], details));
        blocks
    }

    /// Your ember's hero: its face, what it is doing, and the On duty switch.
    fn hero(&self, now: &Now, p: &Pal) -> Div {
        let on = now.on_duty;
        let duty = self.store.clone();
        div()
            .flex()
            .items_center()
            .gap(px(14.))
            .p(px(16.))
            .rounded(px(12.))
            .border_1()
            .border_color(p.border)
            .bg(p.card)
            .child(img(FACE).size(px(44.)).rounded_full().flex_none())
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_size(px(15.))
                            .child(now.doing.clone()),
                    )
                    .child(div().text_size(px(12.5)).text_color(p.muted).child(if on {
                        "Works on its own, and answers when you message it."
                    } else {
                        "Stops working on its own. Still answers when you message it."
                    })),
            )
            .child(
                div()
                    .text_size(px(13.))
                    .text_color(p.muted)
                    .child(duty_word(on)),
            )
            .child(
                Switch::new("on-duty-switch")
                    .checked(on)
                    .on_click(move |on, _, cx| {
                        duty.update(cx, |store, cx| store.set_on_duty(*on, cx));
                    }),
            )
    }

    /// Devices' blocks: the devices, pairing another, pairing this one, the huts.
    pub(super) fn devices_blocks(
        &mut self,
        scale: f32,
        p: &Pal,
        cx: &mut Context<'_, Self>,
    ) -> Vec<Block> {
        let qr = self.pairing_qr(scale, cx);
        let mut blocks = vec![block(
            &["devices"],
            settings_more::devices(&self.store, p, cx),
        )];
        if let Some(card) = settings_more::pair_card(&self.store, qr, p, cx) {
            blocks.push(block(&["pair"], card));
        }
        let store = self.store.read(cx);
        let this_one = if store.is_signed_in() {
            group(p, "THIS COMPUTER").child(note(
                p,
                "Signed in with its own session. To pair it again, sign it out from another \
                 device.",
            ))
        } else {
            group(p, "PAIR THIS COMPUTER").child(self.pairing.clone())
        };
        blocks.push(block(&["sign-in-here"], this_one));
        let (machines, names) = settings_more::machines_row(store);
        let row = nav_row(
            "machines-row",
            p,
            (IconName::Cpu, category_color(5)),
            "Huts",
            machines,
            &names,
        )
        // Over Settings in the column: Back returns here.
        .on_click(move |_, window, cx| column::push(Page::Machines, window, cx))
        .test_support();
        blocks.push(block(
            &["machines"],
            section(
                p,
                "Huts your ember works on",
                None,
                vec![row.into_any_element()],
                None,
            ),
        ));
        blocks
    }

    /// Connections' blocks: the data sources with Chrome, then the tool servers and the
    /// skills folded, then why a list didn't refresh.
    pub(super) fn connections_blocks(
        &self,
        now: &Now,
        p: &Pal,
        cx: &mut Context<'_, Self>,
    ) -> Vec<Block> {
        let store = self.store.read(cx);
        let profiles = chrome_words::summary(store.chrome_profiles());
        let chrome = nav_row(
            "chrome-profiles-row",
            p,
            (IconName::Globe, p.accent),
            "Chrome profiles",
            (profiles.status, profiles.health),
            &profiles.detail,
        )
        .on_click(Self::opens(SettingsPane::ChromeProfiles))
        .test_support();
        let sources = places::sources(&self.store, chrome.into_any_element(), p, cx);
        let mut blocks = vec![block(&["sources", "chrome"], sources)];
        let (tools, skills) = {
            let page = store.switches().page();
            let count = |on: usize, all: usize| format!("{on} of {all} on");
            let tools = page.map_or_else(String::new, |page| {
                let on = page.mcp.iter().filter(|server| server.on).count();
                let line = format!("tool servers (MCP) · {}", count(on, page.mcp.len()));
                match now.failing_tools.len() {
                    0 => line,
                    failing => format!("{line} · {failing} can't connect"),
                }
            });
            let skills = page.map_or_else(String::new, |page| {
                let on = page.skills.iter().filter(|skill| skill.on).count();
                format!(
                    "{} · what your ember knows how to do",
                    count(on, page.skills.len())
                )
            });
            (tools, skills)
        };
        let servers = vec![switches::servers(&self.store, p, cx).into_any_element()];
        let tools = self.fold(Fold::Tools, ("Tools", tools), servers, p, cx);
        blocks.push(block(&["tools"], tools));
        let skills_section = switches::skills(&self.store, p, cx)
            .into_iter()
            .map(IntoElement::into_any_element)
            .collect();
        let skills = self.fold(Fold::Skills, ("Skills", skills), skills_section, p, cx);
        blocks.push(block(&["skills"], skills));
        let store = self.store.read(cx);
        let places_problem = store
            .environments()
            .page()
            .and(store.environments().problem())
            .map(str::to_owned);
        for problem in [switches::problem(&self.store, cx), places_problem]
            .into_iter()
            .flatten()
        {
            blocks.push(plain(problem_line(p, problem)));
        }
        blocks
    }

    /// Sharing: only the user, today, and how it is set up for now.
    pub(super) fn sharing_blocks(p: &Pal) -> Vec<Block> {
        let card =
            div()
                .flex()
                .items_center()
                .gap(px(12.))
                .p(px(16.))
                .rounded(px(12.))
                .border_1()
                .border_color(p.border)
                .bg(p.card)
                .child(Icon::new(IconName::Users).size(px(18.)).text_color(p.muted))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .child(
                            div()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_size(px(14.))
                                .child("Only you can talk to your ember"),
                        )
                        .child(div().text_size(px(12.5)).text_color(p.muted).child(
                            "Sharing lets someone else ask it things, within limits you set.",
                        )),
                )
                .child(tag("Early preview", p.muted));
        vec![block(
            &["sharing"],
            div()
                .flex()
                .flex_col()
                .gap(px(10.))
                .child(card)
                .child(note(p, SHARING).px(px(4.))),
        )]
    }

    /// Privacy's blocks: what the agent knows, the archived rooms, the dark room, and where
    /// the user's data goes.
    pub(super) fn privacy_blocks(
        &self,
        now: &Now,
        p: &Pal,
        cx: &mut Context<'_, Self>,
    ) -> Vec<Block> {
        let knows = now.facts.map_or_else(
            || "Fix or forget any of it".to_owned(),
            |count| format!("{count} things · fix or forget any"),
        );
        let about = nav_row(
            "privacy-about-you-row",
            p,
            (IconName::BookOpen, category_color(0)),
            crate::store::settings::about::TITLE,
            (knows, Health::Off),
            "",
        )
        .on_click(Self::opens(SettingsPane::AboutYou))
        .test_support();
        let open = self.store.read(cx).archived_open();
        vec![
            block(
                &["about-you"],
                section(p, "", None, vec![about.into_any_element()], None),
            ),
            block(
                &["archived-rooms"],
                settings_more::archived(&self.store, open, p, cx),
            ),
            block(&["dark-room"], self.dark_room(p, cx)),
            block(
                &["data-flow"],
                note(p, DATA_FLOW).text_size(px(12.5)).px(px(4.)),
            ),
        ]
    }

    /// The dark room's history: Clear…, and once pressed its confirmation (it can't be undone).
    fn dark_room(&self, p: &Pal, cx: &mut Context<'_, Self>) -> Div {
        let asked = self.dark_clear_asked;
        let action: AnyElement = if asked {
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .child(
                    Button::new("dark-clear-cancel")
                        .ghost()
                        .small()
                        .label("Cancel")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.dark_clear_asked = false;
                            cx.notify();
                        })),
                )
                .child(
                    Button::new("dark-clear-confirm")
                        .danger()
                        .small()
                        .label("Clear history")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.dark_clear_asked = false;
                            this.store.update(cx, Store::clear_dark);
                            cx.notify();
                        })),
                )
                .into_any_element()
        } else {
            Button::new("dark-clear")
                .small()
                .label("Clear…")
                .on_click(cx.listener(|this, _, _, cx| {
                    this.dark_clear_asked = true;
                    cx.notify();
                }))
                .into_any_element()
        };
        let line = if asked {
            "Clear everything said in the dark room? It can't be brought back."
        } else {
            "Not remembered by your ember · kept on the server until you clear it"
        };
        let row = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(10.))
            .child(Icon::new(IconName::Moon).size(px(16.)).text_color(p.muted))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .child(div().text_size(px(13.)).child("Dark room"))
                    .child(div().text_size(px(12.)).text_color(p.muted).child(line)),
            )
            .child(action);
        section(p, "", None, vec![row.into_any_element()], None)
    }

    /// Advanced's blocks: updates, the server, diagnostics folded, where the agent works.
    pub(super) fn advanced_blocks(&self, p: &Pal, cx: &mut Context<'_, Self>) -> Vec<Block> {
        let mut blocks = vec![block(
            &["updates"],
            updates::section_for(&self.store, p, cx),
        )];
        let store = self.store.read(cx);
        let server: Vec<AnyElement> = settings_more::server_rows(store, p)
            .into_iter()
            .map(IntoElement::into_any_element)
            .collect();
        blocks.push(block(
            &["server", "phone-push"],
            section(p, "Your ember's home", None, server, None),
        ));
        let sign_in = settings_more::sign_in_rows(store, p);
        let metrics = super::metrics_line(store.metrics());
        let mut rows: Vec<AnyElement> = sign_in
            .into_iter()
            .map(IntoElement::into_any_element)
            .collect();
        rows.push(note(p, metrics).into_any_element());
        let diagnostics = self.fold(
            Fold::Diagnostics,
            ("Diagnostics", "sign-in mode, polling".to_owned()),
            vec![section(p, "", None, rows, None).into_any_element()],
            p,
            cx,
        );
        blocks.push(block(&["diagnostics"], diagnostics));
        blocks.push(block(
            &["workplaces"],
            places::workplaces(&self.store, p, cx),
        ));
        let store = self.store.read(cx);
        let places_problem = store
            .environments()
            .page()
            .and(store.environments().problem())
            .map(str::to_owned);
        blocks.extend(places_problem.map(|problem| plain(problem_line(p, problem))));
        blocks
    }

    /// The account's block.
    pub(super) fn account_blocks(p: &Pal, cx: &mut Context<'_, Self>) -> Vec<Block> {
        crate::account::installed(cx).map_or_else(
            || {
                vec![plain(note(
                    p,
                    "This build has no emberhut.com account: devices pair with a code.",
                ))]
            },
            |signed| vec![block(&["account"], account::section(&signed, p, cx))],
        )
    }
}

/// A labelled fact in a section: its name, then its value.
fn fact_row(label: &'static str, value: String, p: &Pal) -> AnyElement {
    div()
        .flex()
        .flex_wrap()
        .gap(px(12.))
        .text_size(px(13.))
        .child(div().w(px(110.)).child(label))
        .child(div().flex_1().min_w_0().text_color(p.muted).child(value))
        .into_any_element()
}
