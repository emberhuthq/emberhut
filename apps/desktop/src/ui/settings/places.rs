//! Settings' Data sources and Where the assistant works: each environment the server lists as
//! a compact row (its icon, name, state in its colour, what it is) that opens its page over
//! Settings, beside the Chrome profiles' row and the Machines row. A line while they load, or
//! why they don't. The filter narrows the rows by their state.

use emberhut_api::environments::Environment;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, Div, ElementId, Entity, div, px};

use super::parts::{nav_row, note, section};
use crate::brand::APP_NAME;
use crate::store::settings::chrome::Health;
use crate::store::working::environments::health;
use crate::store::working::settings_filter::{SettingsFilter, SettingsPart};
use crate::store::{Page, Store};
use crate::ui::column;
use crate::ui::palette::Pal;
use crate::ui::working::look::badge;

/// A row of Settings that isn't an environment (the Chrome profiles, the Machines), with its
/// health for the filter.
pub(super) struct OtherRow {
    /// The row.
    pub(super) row: AnyElement,
    /// How it is doing.
    pub(super) health: Health,
}

/// The Data sources section: the connections, then the Chrome profiles' row.
pub(super) fn sources(
    store: &Entity<Store>,
    filter: SettingsFilter,
    chrome: OtherRow,
    p: &Pal,
    cx: &App,
) -> Option<Div> {
    if !filter.shows(SettingsPart::Sources) {
        return None;
    }
    let (mut rows, count) = env_rows(store, filter, p, cx, |env| env.group.is_data_source());
    let count = count + usize::from(filter.keeps(chrome.health));
    if filter.keeps(chrome.health) {
        rows.push(chrome.row);
    }
    Some(section(
        p,
        "Data sources",
        Some(count),
        rows,
        Some(format!(
            "Reconnect and Unlink are in each one's page. Unlink deletes only its sign-in, \
             never what {APP_NAME} read."
        )),
    ))
}

/// Where the assistant works: the Machines row, then its runners, workers, browser and external threads.
pub(super) fn workplaces(
    store: &Entity<Store>,
    filter: SettingsFilter,
    machines: OtherRow,
    p: &Pal,
    cx: &App,
) -> Option<Div> {
    if !filter.shows(SettingsPart::Machines) {
        return None;
    }
    let (places, count) = env_rows(store, filter, p, cx, |env| env.group.is_workplace());
    let mut rows = Vec::new();
    if filter.keeps(machines.health) {
        rows.push(machines.row);
    }
    rows.extend(places);
    Some(section(
        p,
        format!("Where {APP_NAME} works"),
        Some(count),
        rows,
        None,
    ))
}

/// The rows `pick` picks in the state `filter` keeps, and how many; while the page hasn't
/// come, a line saying so (or why not), and none.
fn env_rows(
    store: &Entity<Store>,
    filter: SettingsFilter,
    p: &Pal,
    cx: &App,
    pick: impl Fn(&Environment) -> bool,
) -> (Vec<AnyElement>, usize) {
    let feed = store.read(cx).environments();
    let Some(page) = feed.page() else {
        return (vec![loading(feed.problem(), p).into_any_element()], 0);
    };
    let rows: Vec<AnyElement> = page
        .rows()
        .filter(|env| pick(env) && filter.keeps(health(&env.state)))
        .map(|env| env_row(store, env, p).into_any_element())
        .collect();
    let count = rows.len();
    (rows, count)
}

/// `Looking…` with a spinner, or why the page didn't come.
fn loading(problem: Option<&str>, p: &Pal) -> Div {
    match problem {
        Some(problem) => note(p, problem.to_owned()),
        None => div()
            .flex()
            .items_center()
            .gap(px(8.))
            .child(Spinner::new().small().color(p.muted))
            .child(note(p, "Looking…")),
    }
}

/// An environment's row: a click opens its page over Settings (Back returns here).
fn env_row(store: &Entity<Store>, env: &Environment, p: &Pal) -> impl IntoElement {
    let (store, id) = (store.clone(), env.id.clone());
    nav_row(
        ElementId::Name(format!("env-{}", env.id).into()),
        p,
        badge(env, p),
        env.name.clone(),
        (env.status.clone(), health(&env.state)),
        &env.detail,
    )
    .on_click(move |_, window, cx| {
        let key = store.update(cx, |store, _| store.environment_key(id.clone()));
        column::push(Page::Environment(key), window, cx);
    })
}
