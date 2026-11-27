//! Settings' MCP servers and Skills: each with its state and the user's switch. A row opens in
//! place to show the rest (a server's whole error, its note, its kind and source, Reconnect; a
//! skill's whole description).

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{Disableable as _, Icon, Sizable as _};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, Div, ElementId, Entity, div, px};

use super::parts::{note, section, selectable};
use crate::brand::APP_NAME;
use crate::store::Store;
use crate::store::settings::switches::{
    RowKind, StateTone, row_key, server_origin, server_status, server_tone, servers_footer,
    skills_footer,
};
use crate::ui::palette::Pal;
use crate::ui::widgets::dot;
use emberhut_api::switches::{McpServer, Skill, SkillSource, SwitchAction};

/// The MCP servers' section: a line while they load (or why they don't), then a row each.
pub(super) fn servers(store: &Entity<Store>, p: &Pal, cx: &App) -> Div {
    let feed = store.read(cx).switches();
    let Some(page) = feed.page() else {
        let waiting = feed.problem().map_or_else(
            || {
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(Spinner::new().small().color(p.muted))
                    .child(note(p, "Looking…"))
            },
            |problem| note(p, problem.to_owned()),
        );
        return section(
            p,
            "MCP servers",
            None,
            vec![waiting.into_any_element()],
            None,
        );
    };
    let rows: Vec<AnyElement> = page
        .mcp
        .iter()
        .map(|server| server_row(store, server, p, cx).into_any_element())
        .collect();
    section(
        p,
        "MCP servers",
        Some(rows.len()),
        rows,
        Some(servers_footer(page)),
    )
}

/// The skills' section, a row each (none while the page hasn't come).
pub(super) fn skills(store: &Entity<Store>, p: &Pal, cx: &App) -> Option<Div> {
    let page = store.read(cx).switches().page()?;
    let rows: Vec<AnyElement> = page
        .skills
        .iter()
        .map(|skill| skill_row(store, skill, p, cx).into_any_element())
        .collect();
    let footer = (!rows.is_empty()).then(skills_footer);
    Some(section(p, "Skills", Some(rows.len()), rows, footer))
}

/// The line under the lists after a failed fetch or switch, while a page is on hand.
pub(super) fn problem(store: &Entity<Store>, cx: &App) -> Option<String> {
    let feed = store.read(cx).switches();
    feed.page()?;
    feed.problem().map(str::to_owned)
}

fn tone_color(tone: StateTone, p: &Pal) -> gpui_kit::Hsla {
    match tone {
        StateTone::Good => p.success,
        StateTone::Bad => p.danger,
        StateTone::Waiting => p.warn,
        StateTone::Quiet => p.faint,
    }
}

/// A switch, or a spinner while its change is on its way; every switch waits meanwhile.
fn control(
    store: &Entity<Store>,
    id: String,
    name: &str,
    on: bool,
    busy: Option<&str>,
    make: fn(String, bool) -> SwitchAction,
) -> Div {
    let (store, owned) = (store.clone(), name.to_owned());
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .when(busy == Some(name), |d| d.child(Spinner::new().small()))
        .child(
            Switch::new(ElementId::Name(id.into()))
                .checked(on)
                .small()
                .disabled(busy.is_some())
                .on_click(move |on, _, cx| {
                    let action = make(owned.clone(), *on);
                    store.update(cx, |store, cx| store.switch(action, cx));
                }),
        )
}

fn server_row(store: &Entity<Store>, server: &McpServer, p: &Pal, cx: &App) -> Div {
    let feed = store.read(cx).switches();
    let key = row_key(RowKind::Server, &server.name);
    let open = feed.is_open(&key);
    let busy = feed.busy();
    let failed = server_tone(server) == StateTone::Bad;
    let toggle = store.clone();
    let head = div()
        .flex()
        .items_center()
        .gap(px(10.))
        .child(
            div()
                .id(ElementId::Name(format!("{key}-row").into()))
                .flex()
                .items_center()
                .gap(px(10.))
                .flex_1()
                .min_w_0()
                .cursor_pointer()
                .child(dot(tone_color(server_tone(server), p), 7.))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .min_w_0()
                        .child(
                            div()
                                .text_size(px(13.))
                                .truncate()
                                .child(server.name.clone()),
                        )
                        .child(
                            div()
                                .text_size(px(12.))
                                .truncate()
                                .text_color(if failed { p.danger } else { p.muted })
                                .child(server_status(server)),
                        ),
                )
                .on_click(move |_, _, cx| {
                    let key = key.clone();
                    toggle.update(cx, |store, cx| store.toggle_switch_row(key, cx));
                }),
        )
        .child(if server.fixed {
            div()
                .id(ElementId::Name(format!("mcp:{}-lock", server.name).into()))
                .tooltip(|window, cx| {
                    gpui_kit::component::tooltip::Tooltip::new(format!(
                        "{APP_NAME}'s own tools: always on"
                    ))
                    .build(window, cx)
                })
                .child(Icon::new(IconName::Lock).size(px(12.)).text_color(p.faint))
                .into_any_element()
        } else {
            control(
                store,
                format!("mcp:{}-switch", server.name),
                &server.name,
                server.on,
                busy,
                |name, on| SwitchAction::Mcp { name, on },
            )
            .into_any_element()
        });
    div()
        .flex()
        .flex_col()
        .gap(px(6.))
        .child(head)
        .when(open, |d| {
            d.child(server_details(store, server, busy.is_some(), p))
        })
}

/// A server's details, open in place.
fn server_details(store: &Entity<Store>, server: &McpServer, busy: bool, p: &Pal) -> Div {
    let origin = server_origin(server);
    let reconnect = server.can_reconnect().then(|| {
        let (store, name) = (store.clone(), server.name.clone());
        Button::new(ElementId::Name(
            format!("mcp:{}-reconnect", server.name).into(),
        ))
        .ghost()
        .small()
        .icon(gpui_kit::component::IconName::Redo)
        .label("Reconnect")
        .disabled(busy)
        .on_click(move |_, _, cx| {
            let action = SwitchAction::Reconnect { name: name.clone() };
            store.update(cx, |store, cx| store.switch(action, cx));
        })
    });
    div()
        .pl(px(17.))
        .flex()
        .flex_col()
        .gap(px(4.))
        .text_size(px(12.))
        .when_some(server.error.clone(), |d, error| {
            d.child(
                div()
                    .id(ElementId::Name(format!("mcp:{}-error", server.name).into()))
                    .text_color(p.danger)
                    .child(selectable(
                        ElementId::Name(format!("mcp:{}-error-text", server.name).into()),
                        &error,
                    )),
            )
        })
        .when_some(server.note.clone(), |d, note| {
            d.child(div().text_color(p.muted).child(note))
        })
        .when(!origin.is_empty(), |d| {
            d.child(div().text_color(p.faint).child(origin))
        })
        .children(reconnect.map(|button| div().pt(px(2.)).child(button)))
}

fn skill_row(store: &Entity<Store>, skill: &Skill, p: &Pal, cx: &App) -> Div {
    let feed = store.read(cx).switches();
    let key = row_key(RowKind::Skill, &skill.name);
    let open = feed.is_open(&key);
    let busy = feed.busy();
    let toggle = store.clone();
    let source = (skill.source != SkillSource::Yours).then(|| skill.source.as_str().to_owned());
    let description = (!skill.description.is_empty()).then(|| {
        let text = if open {
            skill.description.clone()
        } else {
            skill
                .description
                .lines()
                .next()
                .unwrap_or_default()
                .to_owned()
        };
        let line = div().text_size(px(12.)).text_color(p.muted).child(text);
        if open { line } else { line.truncate() }
    });
    div()
        .flex()
        .items_start()
        .gap(px(10.))
        .child(
            div()
                .id(ElementId::Name(format!("{key}-row").into()))
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .cursor_pointer()
                .child(
                    div()
                        .flex()
                        .items_baseline()
                        .gap(px(6.))
                        .child(
                            div()
                                .text_size(px(13.))
                                .truncate()
                                .text_color(if skill.on { p.text } else { p.muted })
                                .child(skill.name.clone()),
                        )
                        .when_some(source, |d, source| {
                            d.child(div().text_size(px(11.)).text_color(p.faint).child(source))
                        }),
                )
                .children(description)
                .on_click(move |_, _, cx| {
                    let key = key.clone();
                    toggle.update(cx, |store, cx| store.toggle_switch_row(key, cx));
                }),
        )
        .child(control(
            store,
            format!("skill:{}-switch", skill.name),
            &skill.name,
            skill.on,
            busy,
            |name, on| SwitchAction::Skill { name, on },
        ))
}
