//! An environment's page, over Settings in the column: its icon, name and state, a hint when
//! something is off, what it is and can do, its lists (accounts, sessions, folders, tickets)
//! with their states, the last error (or how long it has been fine since one), and its safe
//! actions: Check now, Reconnect through a link, Start, Unlink behind the user's yes. What an
//! action made shows under them: a Chrome profile's re-pair link, a one-time page to open, a
//! note. It stays live while it shows.

use emberhut_api::environments::{EnvAction, EnvActionKind, EnvFact, EnvList, Environment};
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{Icon, Sizable as _};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, Div, ElementId, Entity, FontWeight, Role, ScrollHandle, Stateful, div, px,
};
use jiff::Timestamp;
use jiff::tz::TimeZone;

use super::chrome::copy_link;
use super::parts::{note, problem_line, section, selectable, tag};
use crate::brand::APP_NAME;
use crate::store::Store;
use crate::store::devices::relative;
use crate::store::settings::words::clock;
use crate::store::working::environments::{EnvKey, Outcome, actions, actions_footer};
use crate::ui::a11y::{self, Accessible as _};
use crate::ui::palette::Pal;
use crate::ui::reads;
use crate::ui::widgets::{dot, scroller};
use crate::ui::working::look::{badge, state_color};

/// A fact longer than this reads under its label rather than beside it.
const SHORT_FACT: usize = 28;

/// The page of `key`'s environment.
pub(crate) fn page(
    store: &Entity<Store>,
    key: EnvKey,
    scroll: &ScrollHandle,
    p: &Pal,
    cx: &App,
) -> Div {
    let feed = store.read(cx).environments();
    let env = feed
        .id_of(key)
        .and_then(|id| Some((id, feed.page()?)))
        .map(|(id, page)| page.row(id));
    let body = div()
        .flex()
        .flex_col()
        .gap(px(18.))
        .px(px(24.))
        .py(px(18.))
        .max_w(px(720.));
    let body = match env {
        Some(Some(env)) => body.children(sections(store, env, p, cx)),
        Some(None) => body.child(note(p, "This one isn't listed anymore.")),
        None => body.child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .when(feed.problem().is_none(), |d| {
                    d.child(Spinner::new().small().color(p.muted))
                })
                .child(note(p, feed.problem().unwrap_or("Loading…").to_owned())),
        ),
    };
    div()
        .size_full()
        .flex()
        .flex_col()
        .child(scroller("environment-scroll", scroll).child(body))
}

/// The page's sections.
fn sections(store: &Entity<Store>, env: &Environment, p: &Pal, cx: &App) -> Vec<AnyElement> {
    let mut parts = vec![head(env, p).into_any_element()];
    if let Some(hint) = &env.hint {
        parts.push(
            div()
                .flex()
                .items_start()
                .gap(px(8.))
                .p(px(12.))
                .rounded(px(10.))
                .bg(p.warn.opacity(0.1))
                .child(Icon::new(IconName::Info).size(px(14.)).text_color(p.warn))
                .child(
                    div()
                        .flex_1()
                        .text_size(px(13.))
                        .when(reads::starts_right(hint), Styled::text_right)
                        .child(hint.clone()),
                )
                .into_any_element(),
        );
    }
    if !env.facts.is_empty() {
        let facts = env
            .facts
            .iter()
            .enumerate()
            .map(|(i, f)| fact(&env.id, i, f, p).into_any_element())
            .collect();
        parts.push(section(p, "Details", None, facts, None).into_any_element());
    }
    for list in &env.lists {
        parts.push(list_section(list, p).into_any_element());
    }
    if let Some(last) = last_error(env, p) {
        parts.push(last.into_any_element());
    }
    let feed = store.read(cx).environments();
    let outcome = feed.outcome(&env.id);
    if actions(env).next().is_some() || outcome.is_some() {
        parts.push(actions_section(store, env, outcome, p, cx).into_any_element());
    }
    parts
}

/// Its icon, name, Legacy tag, state and what it is.
fn head(env: &Environment, p: &Pal) -> Div {
    let (glyph, tint) = badge(env, p);
    let color = state_color(&env.state, p);
    div()
        .flex()
        .items_center()
        .gap(px(14.))
        .child(
            div()
                .size(px(46.))
                .rounded(px(11.))
                .bg(tint)
                .flex()
                .items_center()
                .justify_center()
                .child(
                    Icon::new(glyph)
                        .size(px(22.))
                        .text_color(Pal::readable_on(tint)),
                ),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(3.))
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .child(
                            div()
                                .text_size(px(17.))
                                .font_weight(FontWeight::BOLD)
                                .child(env.name.clone()),
                        )
                        .when(env.legacy == Some(true), |d| {
                            d.child(tag("Legacy", p.muted))
                        }),
                )
                .child(
                    div()
                        // Said as it changes (Connecting, Connected, Signed out).
                        .id("environment-status")
                        .announced(env.status.clone())
                        .flex()
                        .items_center()
                        .gap(px(5.))
                        .child(dot(color, 8.))
                        .child(
                            div()
                                .text_size(px(13.))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(color)
                                .child(env.status.clone()),
                        ),
                )
                .when(!env.detail.is_empty(), |d| {
                    d.child(note(p, env.detail.clone()))
                }),
        )
}

/// A fact: a short one beside its label, a sentence under it; in its state's colour.
fn fact(env: &emberhut_api::ids::EnvironmentId, index: usize, f: &EnvFact, p: &Pal) -> Div {
    let color = f
        .state
        .as_ref()
        .map_or(p.muted, |state| state_color(state, p));
    let value = div()
        .text_size(px(12.5))
        .text_color(color)
        .child(selectable(
            ElementId::Name(format!("env-{env}-fact-{index}").into()),
            &f.value,
        ));
    let label = div().text_size(px(13.)).child(f.label.clone());
    if f.value.chars().count() > SHORT_FACT {
        div()
            .flex()
            .flex_col()
            .gap(px(3.))
            .child(label)
            .child(value)
    } else {
        div()
            .flex()
            .items_center()
            .gap(px(12.))
            .child(label.flex_1())
            .child(value)
    }
}

/// One of its lists: each item with its state's dot, name, note and value.
fn list_section(list: &EnvList, p: &Pal) -> Div {
    let rows: Vec<AnyElement> = if list.items.is_empty() {
        vec![note(p, list.empty.clone().unwrap_or_else(|| "None".to_owned())).into_any_element()]
    } else {
        list.items
            .iter()
            .map(|item| {
                div()
                    .flex()
                    .items_start()
                    .gap(px(8.))
                    .when_some(item.state.as_ref(), |d, state| {
                        d.child(div().pt(px(5.)).child(dot(state_color(state, p), 7.)))
                    })
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .child(div().text_size(px(13.)).child(item.name.clone()))
                            .when_some(item.note.as_ref().filter(|n| !n.is_empty()), |d, n| {
                                d.child(note(p, n.clone()))
                            }),
                    )
                    .when_some(item.value.clone(), |d, value| {
                        d.child(div().text_size(px(12.5)).text_color(p.muted).child(value))
                    })
                    .into_any_element()
            })
            .collect()
    };
    section(p, list.title.clone(), None, rows, None)
}

/// The error now, or the last one and how long it has been fine since.
fn last_error(env: &Environment, p: &Pal) -> Option<Div> {
    let row = if let Some(error) = &env.error {
        div()
            .flex()
            .items_start()
            .gap(px(8.))
            .child(
                Icon::new(IconName::TriangleAlert)
                    .size(px(14.))
                    .text_color(p.danger),
            )
            .child(
                div()
                    .flex_1()
                    .text_size(px(13.))
                    .text_color(p.danger)
                    .child(selectable(
                        ElementId::Name(format!("env-{}-error", env.id).into()),
                        error,
                    )),
            )
    } else {
        let last = env.last_error.as_ref()?;
        let since = last.at.as_str().parse::<Timestamp>().map_or_else(
            |_| last.at.as_str().to_owned(),
            |at| relative(at, Timestamp::now()),
        );
        div()
            .flex()
            .flex_col()
            .gap(px(2.))
            .child(div().text_size(px(13.)).child(selectable(
                ElementId::Name(format!("env-{}-last-error", env.id).into()),
                &last.error,
            )))
            .child(note(p, format!("{since} · fine since")))
    };
    Some(section(
        p,
        "Last error",
        None,
        vec![row.into_any_element()],
        None,
    ))
}

/// An action's icon.
fn action_icon(kind: &EnvActionKind) -> IconName {
    match kind {
        EnvActionKind::Repair | EnvActionKind::Link | EnvActionKind::WaLink => {
            IconName::ExternalLink
        }
        EnvActionKind::Start => IconName::Play,
        EnvActionKind::Unlink => IconName::Minus,
        _ => IconName::RefreshCw,
    }
}

/// The actions, what the last one made, Unlink's question, and the footer.
fn actions_section(
    store: &Entity<Store>,
    env: &Environment,
    outcome: Option<&Outcome>,
    p: &Pal,
    cx: &App,
) -> Div {
    let feed = store.read(cx).environments();
    let busy = (feed.busy().is_some(), feed.busy() == Some(&env.id));
    let mut rows: Vec<AnyElement> = actions(env)
        .enumerate()
        .map(|(i, action)| action_row(store, env, i, action, busy, p).into_any_element())
        .collect();
    if feed.is_unlinking(&env.id) {
        rows.push(unlink_question(store, env, p).into_any_element());
    }
    if let Some(outcome) = outcome {
        rows.extend(made(store, env, outcome, p, cx));
    }
    if let Some(problem) = feed.problem() {
        rows.push(
            div()
                .id("environment-problem")
                .announced(problem.to_owned())
                .child(problem_line(p, problem.to_owned()))
                .into_any_element(),
        );
    }
    section(p, "", None, rows, Some(actions_footer(env)))
}

/// One action: its icon and label (Unlink in red; muted while any action runs), a spinner
/// while this row's runs (`running`).
fn action_row(
    store: &Entity<Store>,
    env: &Environment,
    index: usize,
    action: &EnvAction,
    (busy, running): (bool, bool),
    p: &Pal,
) -> impl IntoElement {
    let unlink = action.id == EnvActionKind::Unlink;
    let color = if busy {
        p.muted
    } else if unlink {
        p.danger
    } else {
        p.accent
    };
    let (store, id, kind) = (store.clone(), env.id.clone(), action.id.clone());
    crate::ui::controls::tab_stop(
        div()
            .id(ElementId::Name(
                format!("env-{}-action-{index}", env.id).into(),
            ))
            .button_named(a11y::line(&[
                &action.label,
                if running { "running" } else { "" },
            ]))
            .border_1()
            .rounded(px(6.))
            .flex()
            .items_center()
            .gap(px(8.))
            .cursor_pointer()
            .text_size(px(13.))
            .text_color(color)
            .child(Icon::new(action_icon(&action.id)).size(px(14.)))
            .child(div().flex_1().child(action.label.clone()))
            .when(running, |d| d.child(Spinner::new().small().color(p.muted)))
            .on_click(move |_, _, cx| {
                let (id, kind) = (id.clone(), kind.clone());
                store.update(cx, |store, cx| {
                    if store.environments().busy().is_some() {
                        return;
                    }
                    if kind == EnvActionKind::Unlink {
                        store.ask_unlink(Some(id), cx);
                    } else {
                        store.act_on_environment(id, kind, cx);
                    }
                });
            }),
        p,
    )
}

/// Unlink's question: what goes and what stays, Unlink and Cancel.
fn unlink_question(store: &Entity<Store>, env: &Environment, p: &Pal) -> Stateful<Div> {
    let (yes, no, id) = (store.clone(), store.clone(), env.id.clone());
    div()
        .id("unlink-question")
        .read_as(Role::AlertDialog, format!("Unlink {}?", env.name))
        .flex()
        .flex_col()
        .gap(px(8.))
        .p(px(10.))
        .rounded(px(8.))
        .bg(p.danger.opacity(0.08))
        .child(
            div()
                .text_size(px(13.))
                .font_weight(FontWeight::SEMIBOLD)
                .child(format!("Unlink {}?", env.name)),
        )
        .child(note(
            p,
            format!(
                "{APP_NAME} stops reading it and its sign-in is deleted from {APP_NAME}'s \
                 server. The messages it already read stay. Reconnect signs it in again."
            ),
        ))
        .child(
            div()
                .flex()
                .gap(px(8.))
                .child(
                    Button::new(ElementId::Name(format!("env-{}-unlink-yes", env.id).into()))
                        .small()
                        .danger()
                        .label("Unlink")
                        .on_click(move |_, _, cx| {
                            let id = id.clone();
                            yes.update(cx, |store, cx| {
                                store.act_on_environment(id, EnvActionKind::Unlink, cx);
                            });
                        }),
                )
                .child(
                    Button::new(ElementId::Name(format!("env-{}-unlink-no", env.id).into()))
                        .small()
                        .ghost()
                        .label("Cancel")
                        .on_click(move |_, _, cx| {
                            no.update(cx, |store, cx| store.ask_unlink(None, cx));
                        }),
                ),
        )
}

/// What the last action made: a re-pair link, a one-time page (Open, Copy), a note. The page
/// opens as any link from the server does ([`crate::config::ServerUrl::link`]): a link the app
/// won't hand to the system (`file:`, `smb:`, another app's scheme) has no Open, only Copy.
fn made(
    store: &Entity<Store>,
    env: &Environment,
    outcome: &Outcome,
    p: &Pal,
    cx: &App,
) -> Vec<AnyElement> {
    let mut rows = Vec::new();
    if let Some(pairing) = &outcome.pairing {
        let until = clock(pairing.until.as_str(), &TimeZone::system()).unwrap_or_default();
        rows.push(
            div()
                .flex()
                .flex_col()
                .gap(px(8.))
                .child(div().text_size(px(13.)).child(format!(
                    "Open this link in Chrome, in the {} profile, until {until}:",
                    pairing.label
                )))
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(p.muted)
                        .child(selectable(
                            ElementId::Name(format!("env-{}-pairing", env.id).into()),
                            &pairing.link,
                        )),
                )
                .child(copy_link(
                    ElementId::Name(format!("env-{}-pairing-copy", env.id).into()),
                    &pairing.link,
                    false,
                ))
                .into_any_element(),
        );
    }
    if let Some(link) = &outcome.link {
        let open = store.read(cx).server().link(link);
        rows.push(
            div()
                .flex()
                .flex_col()
                .gap(px(8.))
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(p.muted)
                        .child(selectable(
                            ElementId::Name(format!("env-{}-link", env.id).into()),
                            link,
                        )),
                )
                .child(
                    div()
                        .flex()
                        .gap(px(10.))
                        .children(open.map(|open| {
                            Button::new(ElementId::Name(format!("env-{}-link-open", env.id).into()))
                                .small()
                                .icon(gpui_kit::component::IconName::ExternalLink)
                                .label("Open")
                                .on_click(move |_, _, cx| cx.open_url(&open))
                        }))
                        .child(copy_link(
                            ElementId::Name(format!("env-{}-link-copy", env.id).into()),
                            link,
                            false,
                        )),
                )
                .into_any_element(),
        );
    }
    if let Some(text) = &outcome.note {
        rows.push(
            div()
                .id("environment-outcome")
                .announced(text.clone())
                .child(note(p, text.clone()))
                .into_any_element(),
        );
    }
    rows
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{TestAppContext, Window};
    use serde_json::json;

    use super::*;
    use crate::backend::fake::FakeBackend;
    use crate::ui::palette::pal;

    /// What an action made for a row, drawn alone.
    struct Made {
        store: Entity<Store>,
        env: Environment,
        outcome: Outcome,
    }

    impl Render for Made {
        fn render(&mut self, _: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
            let p = pal(cx);
            div().children(made(&self.store, &self.env, &self.outcome, &p, cx))
        }
    }

    #[gpui_kit::test]
    fn a_link_the_app_wont_open_has_copy_and_no_open(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            cx.set_global(Pal::new(&crate::theme::built_in(), false));
        });
        let store = cx.new(|_| Store::for_tests(Arc::new(FakeBackend::new([]))));
        let env: Environment = serde_json::from_value(json!({"id": "gmail:work",
            "group": "connections", "name": "Mail", "icon": "envelope.fill", "tint": "red",
            "state": "ok", "status": "Fine", "detail": "", "at": null, "error": null,
            "hint": null, "facts": [], "lists": [], "actions": [], "lastError": null}))
        .unwrap();
        for (link, opens) in [
            ("file:///Applications/Calculator.app", false),
            ("smb://example.com/share", false),
            ("x-app://open", false),
            ("https://example.com/once", true),
        ] {
            let outcome = Outcome {
                link: Some(link.to_owned()),
                ..Outcome::default()
            };
            let (store, env) = (store.clone(), env.clone());
            let window = cx.add_window(|_, _| Made {
                store,
                env,
                outcome,
            });
            cx.update_window(window.into(), |_, window, cx| {
                window.render_frame(cx);
                assert_eq!(
                    window.try_find("env-gmail:work-link-open").is_some(),
                    opens,
                    "{link}"
                );
                assert!(
                    window.try_find("env-gmail:work-link-copy").is_some(),
                    "{link}"
                );
            })
            .unwrap();
        }
    }
}
