//! "Light it", plans first: the plans the person already pays for, each with its API key
//! folded under it ("Use an API key instead"), then more ways. Claude signs in through
//! Anthropic's own page on Emberhut here, and lists its accounts: who, which plan, which takes
//! the next work, and whether the ember moves to one with room when one runs out. `ChatGPT`'s
//! plan sign-in is coming; Google doesn't let other apps use a Gemini plan's sign-in, so
//! Gemini takes a key.

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{Disableable as _, Icon, Sizable as _};
use gpui_kit::prelude::*;
use gpui_kit::{Context, Div, FontWeight, Role, SharedString, TestSupportExt as _, div, img, px};

use super::SetupView;
use crate::assets::CLAUDE_LOGO;
use crate::hut::HutAccount;
use crate::store::Store;
use crate::ui::palette::Pal;
use crate::ui::settings::parts::{note, tag};
use emberhut_api::agent_provider::ProviderKind;

/// Gemini's OpenAI-style address, for its key through "Another service".
pub(crate) const GEMINI_ADDRESS: &str = "https://generativelanguage.googleapis.com/v1beta/openai";

/// Which way with a key is unfolded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Fold {
    /// An Anthropic API key.
    Claude,
    /// An `OpenAI` API key.
    ChatGpt,
    /// A Gemini API key, through "Another service".
    Gemini,
    /// `DeepSeek`, `OpenRouter`, Ollama, any other service.
    More,
}

impl Fold {
    /// The provider the picker opens on.
    fn kind(self) -> Option<ProviderKind> {
        match self {
            Self::Claude => Some(ProviderKind::Anthropic),
            Self::ChatGpt => Some(ProviderKind::OpenAi),
            Self::Gemini => Some(ProviderKind::OpenAiCompatible),
            Self::More => None,
        }
    }
}

impl SetupView {
    /// Step 2's plans, the key folded under each, and the picker when one is unfolded.
    pub(super) fn plans(&self, p: &Pal, cx: &Context<'_, Self>) -> impl IntoElement {
        div()
            .id("setup-plans")
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(
                div()
                    .text_size(px(13.))
                    .text_color(p.muted)
                    .child("Sign in with a plan you already pay for."),
            )
            .child(self.claude_card(p, cx))
            .child(plan_card(
                "setup-plan-chatgpt",
                p,
                Icon::new(IconName::Bot)
                    .size(px(18.))
                    .text_color(p.muted)
                    .into_any_element(),
                "ChatGPT",
                "Plus or Pro",
                Some("Coming soon"),
                "Signing in with your ChatGPT plan is on its way.",
                self.fold_link(Fold::ChatGpt, "Use an OpenAI API key instead", p, cx),
                None,
            ))
            .child(plan_card(
                "setup-plan-gemini",
                p,
                Icon::new(IconName::Sparkles)
                    .size(px(18.))
                    .text_color(p.muted)
                    .into_any_element(),
                "Google Gemini",
                "",
                None,
                "Google doesn't let other apps use a Gemini plan's sign-in, so this one takes a \
                 Gemini API key.",
                self.fold_link(Fold::Gemini, "Use a Gemini API key", p, cx),
                None,
            ))
            .child(div().flex().pt(px(2.)).child(self.fold_link(
                Fold::More,
                "More ways: DeepSeek, OpenRouter, Ollama, any other service",
                p,
                cx,
            )))
            .when_some(self.fold, |d, fold| {
                d.child(
                    div()
                        .id("setup-key-picker")
                        .flex()
                        .flex_col()
                        .gap(px(8.))
                        .when(fold == Fold::Gemini, |d| {
                            d.child(note(
                                p,
                                format!(
                                    "Under Another service, choose OpenAI-style, the address \
                                     {GEMINI_ADDRESS}, and your key from aistudio.google.com."
                                ),
                            ))
                        })
                        .child(self.credentials.clone()),
                )
            })
    }

    /// Claude's card: sign in with the plan, its accounts, more accounts, and the key folded.
    fn claude_card(&self, p: &Pal, cx: &Context<'_, Self>) -> impl IntoElement {
        let store = self.store.read(cx);
        let plans = store.plans().clone();
        let here = store.signs_in_plans();
        let body: Div = if here {
            self.claude_here(&plans, store.setup_is_lit(), p)
        } else {
            div().child(note(
                p,
                "Your ember lives on another computer: sign Claude Code in there \
                 (claude auth login), or use an API key.",
            ))
        };
        plan_card(
            "setup-plan-claude",
            p,
            img(CLAUDE_LOGO).size(px(18.)).into_any_element(),
            "Claude",
            "Pro or Max",
            None,
            "",
            self.fold_link(Fold::Claude, "Use an Anthropic API key instead", p, cx),
            Some(body),
        )
    }

    /// Claude's card on Emberhut here: the accounts, a sign-in under way, Continue once lit,
    /// sign in (or add another), and the switch to move between them.
    fn claude_here(&self, plans: &crate::store::setup::Plans, lit: bool, p: &Pal) -> Div {
        let entity = self.store.clone();
        let signed_in = plans.signed_in().count();
        let busy = plans.waiting.is_some() || plans.restarting;
        let sign_in = Button::new("setup-claude-sign-in")
            .label(if plans.accounts.is_empty() {
                "Sign in with Claude"
            } else {
                "Add another Claude account"
            })
            .disabled(busy)
            .on_click(move |_, _, cx| entity.update(cx, Store::sign_in_with_claude));
        let sign_in = if plans.accounts.is_empty() {
            sign_in.primary()
        } else {
            sign_in.ghost()
        };
        let cancel_store = self.store.clone();
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .children(
                plans.accounts.iter().enumerate().map(|(at, account)| {
                    account_row(&self.store, account, at, plans, signed_in, p)
                }),
            )
            .when_some(plans.waiting.as_ref(), |d, _| {
                d.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .child(working_line(
                            p,
                            "Finish signing in on Anthropic's page in your browser…",
                        ))
                        .child(
                            Button::new("setup-claude-cancel")
                                .ghost()
                                .small()
                                .label("Cancel")
                                .on_click(move |_, _, cx| {
                                    cancel_store.update(cx, Store::stop_waiting_for_sign_in);
                                }),
                        ),
                )
            })
            .when(plans.restarting, |d| {
                d.child(working_line(p, "Starting your ember on your plan…"))
            })
            .when_some(plans.problem.clone(), |d, why| {
                d.child(
                    div()
                        .id("setup-claude-problem")
                        .role(Role::Status)
                        .text_size(px(12.5))
                        .text_color(p.warn)
                        .child(why),
                )
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .when(signed_in > 0 && lit && !busy, |d| {
                        let store = self.store.clone();
                        d.child(
                            Button::new("setup-plans-continue")
                                .primary()
                                .label("Continue")
                                .on_click(move |_, _, cx| store.update(cx, Store::continue_lit)),
                        )
                    })
                    .child(sign_in),
            )
            .when(signed_in > 1, |d| {
                d.child(moves_row(&self.store, plans.moves, busy, p))
            })
            .child(note(
                p,
                "Opens Anthropic's sign-in in your browser. Emberhut never sees your password.",
            ))
    }

    /// A link that unfolds `fold` (or folds it again), opening the picker on its provider.
    fn fold_link(
        &self,
        fold: Fold,
        words: &'static str,
        p: &Pal,
        cx: &Context<'_, Self>,
    ) -> gpui_kit::AnyElement {
        let open = self.fold == Some(fold);
        let view = cx.entity().downgrade();
        let store = self.store.clone();
        let id = match fold {
            Fold::Claude => "setup-fold-claude",
            Fold::ChatGpt => "setup-fold-chatgpt",
            Fold::Gemini => "setup-fold-gemini",
            Fold::More => "setup-fold-more",
        };
        div()
            .id(id)
            .role(Role::Button)
            .aria_label(words)
            .aria_expanded(open)
            .flex()
            .items_center()
            .gap(px(4.))
            .text_size(px(12.5))
            .text_color(p.accent)
            .cursor_pointer()
            .child(
                Icon::new(if open {
                    IconName::ChevronDown
                } else {
                    IconName::ChevronRight
                })
                .size(px(13.)),
            )
            .child(words)
            .test_support()
            .on_click(move |_, _, cx| {
                let next = if open { None } else { Some(fold) };
                view.update(cx, |view, cx| {
                    view.fold = next;
                    cx.notify();
                })
                .ok();
                if let Some(kind) = next.and_then(Fold::kind) {
                    store.update(cx, |store, cx| store.pick_provider(&kind, cx));
                }
            })
            .into_any_element()
    }
}

/// A plan's card: its logo, name and plan, a tag, its line, what it holds and its key's link.
#[expect(
    clippy::too_many_arguments,
    reason = "a card's parts, each said once by the step that draws it"
)]
fn plan_card(
    id: &'static str,
    p: &Pal,
    logo: gpui_kit::AnyElement,
    name: &'static str,
    plan: &'static str,
    tag_words: Option<&'static str>,
    line: &'static str,
    key: gpui_kit::AnyElement,
    body: Option<Div>,
) -> impl IntoElement {
    div()
        .id(id)
        .flex()
        .flex_col()
        .gap(px(8.))
        .px(px(16.))
        .py(px(14.))
        .rounded(px(12.))
        .border_1()
        .border_color(p.border)
        .bg(p.surface)
        .child(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap(px(10.))
                .child(logo)
                .child(
                    div()
                        .text_size(px(14.5))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(name),
                )
                .when(!plan.is_empty(), |d| {
                    d.child(div().text_size(px(12.5)).text_color(p.muted).child(plan))
                })
                .children(tag_words.map(|words| tag(words, p.muted))),
        )
        .when(!line.is_empty(), |d| {
            d.child(div().text_size(px(12.5)).text_color(p.muted).child(line))
        })
        .children(body)
        .child(div().flex().child(key))
        .test_support()
}

/// One Claude account: who, which plan, how it stands, and what can be done with it.
fn account_row(
    store: &gpui_kit::Entity<Store>,
    account: &HutAccount,
    at: usize,
    plans: &crate::store::setup::Plans,
    signed_in: usize,
    p: &Pal,
) -> impl IntoElement {
    let who: SharedString = account
        .email
        .clone()
        .unwrap_or_else(|| "Claude account".to_owned())
        .into();
    let waiting = plans.waiting.as_deref() == Some(account.id.as_str());
    let first_signed_in = plans
        .signed_in()
        .next()
        .is_some_and(|first| first.id == account.id);
    let (state, color) = account_state(account, plans, waiting, first_signed_in, p);
    let (first_store, remove_store, again_store) = (store.clone(), store.clone(), store.clone());
    let (first_id, remove_id, again_id) =
        (account.id.clone(), account.id.clone(), account.id.clone());
    let busy = plans.waiting.is_some() || plans.restarting;
    div()
        .id(SharedString::from(format!("setup-account-{}", account.id)))
        .role(Role::ListItem)
        .aria_label(SharedString::from(format!(
            "{who}, {}, {state}",
            account.plan_name()
        )))
        .flex()
        .items_center()
        .gap(px(10.))
        .px(px(12.))
        .py(px(8.))
        .rounded(px(10.))
        .bg(p.surface_raised)
        .child(div().size(px(7.)).rounded_full().bg(if account.signed_in {
            p.success
        } else {
            p.faint
        }))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .child(div().text_size(px(13.)).truncate().child(who))
                        .child(tag(account.plan_name(), p.muted)),
                )
                .child(div().text_size(px(11.5)).text_color(color).child(state)),
        )
        .when(!account.signed_in && !waiting, |d| {
            d.child(
                Button::new(SharedString::from(format!("setup-account-again-{at}")))
                    .ghost()
                    .small()
                    .label("Sign in again")
                    .disabled(busy)
                    .on_click(move |_, _, cx| {
                        again_store.update(cx, |store, cx| store.sign_in_again(&again_id, cx));
                    }),
            )
        })
        .when(
            account.signed_in && !first_signed_in && signed_in > 1,
            |d| {
                d.child(
                    Button::new(SharedString::from(format!("setup-account-first-{at}")))
                        .ghost()
                        .small()
                        .label("Use first")
                        .disabled(busy)
                        .on_click(move |_, _, cx| {
                            first_store.update(cx, |store, cx| store.put_plan_first(&first_id, cx));
                        }),
                )
            },
        )
        .child(
            Button::new(SharedString::from(format!("setup-account-remove-{at}")))
                .ghost()
                .small()
                .icon(IconName::X)
                .tooltip("Remove this account")
                .disabled(busy)
                .on_click(move |_, _, cx| {
                    remove_store.update(cx, |store, cx| store.remove_plan(&remove_id, cx));
                }),
        )
}

/// How account `account` stands, in words and their colour.
fn account_state(
    account: &HutAccount,
    plans: &crate::store::setup::Plans,
    waiting: bool,
    first_signed_in: bool,
    p: &Pal,
) -> (String, gpui_kit::Hsla) {
    let resting = plans.resting_until(&account.id).map(clock);
    if waiting {
        ("Signing in…".to_owned(), p.muted)
    } else if !account.signed_in {
        ("Not signed in".to_owned(), p.warn)
    } else if let Some(until) = resting {
        (
            until.map_or_else(
                || "Resting: at its limit".to_owned(),
                |until| format!("Resting until {until}"),
            ),
            p.warn,
        )
    } else if first_signed_in {
        ("Next work goes here".to_owned(), p.success)
    } else if plans.moves {
        ("Ready when the first runs out".to_owned(), p.muted)
    } else {
        ("Ready".to_owned(), p.muted)
    }
}

/// The switch that lets the ember move to an account with room when one runs out.
fn moves_row(store: &gpui_kit::Entity<Store>, on: bool, busy: bool, p: &Pal) -> Div {
    let store = store.clone();
    div()
        .flex()
        .items_center()
        .gap(px(10.))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .child(
                    div()
                        .text_size(px(13.))
                        .child("When one runs out, move to an account with room"),
                )
                .child(note(
                    p,
                    "Off at first: Anthropic's limits assume one person's ordinary use.",
                )),
        )
        .child(
            Switch::new("setup-plans-move")
                .checked(on)
                .small()
                .disabled(busy)
                .on_click(move |on, _, cx| {
                    let on = *on;
                    store.update(cx, |store, cx| store.set_plans_move(on, cx));
                }),
        )
}

/// `at` (the server's time) as a time of day here, `15:00`.
fn clock(at: &str) -> Option<String> {
    let now = jiff::ZonedDateTime::now();
    let at: jiff::Timestamp = at.parse().ok()?;
    Some(
        at.to_zoned_datetime(now.time_zone().clone())
            .strftime("%H:%M")
            .to_string(),
    )
}

/// A line with a spinner: something under way.
fn working_line(p: &Pal, text: &'static str) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .text_size(px(12.5))
        .text_color(p.muted)
        .child(gpui_kit::component::spinner::Spinner::new().small())
        .child(text)
}
