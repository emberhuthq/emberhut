//! A Chrome profile's sheet: what it is, the assistant in it now (its grant, a pause, a
//! hand-off), its details, and rename, re-pair and unpair.

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::Input;
use gpui_kit::component::{Disableable as _, Sizable as _};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, Div, ElementId, FontWeight, div, px};
use jiff::tz::TimeZone;

use super::chrome::{ChromePane, account_line, badge, online, profile_tags};
use super::chrome_sheets::{act, detail, group, icon_line};
use super::parts::{note, problem_line, selectable};
use crate::brand::APP_NAME;
use crate::store::settings::chrome::{grant_line, machine_name, task_name};
use crate::store::settings::words::{clock_of_millis, day_and_time, seen_ago};
use crate::ui::palette::Pal;
use emberhut_api::chrome::{ChromeAction, ChromeControl, ChromeProfile};
use emberhut_api::ids::ChromeProfileId;
use emberhut_api::tasks::Issue;

/// The extension's version, and the newer one when it is out.
fn extension_line(profile: &ChromeProfile, latest: Option<&str>) -> String {
    profile.version.as_ref().map_or_else(
        || "Unknown".to_owned(),
        |version| {
            if profile.outdated {
                format!("v{version} · v{} is out", latest.unwrap_or("?"))
            } else {
                format!("v{version}")
            }
        },
    )
}

/// Online, when it was last seen, or that it never connected.
fn status_line(profile: &ChromeProfile) -> String {
    if profile.connected {
        return "Online".to_owned();
    }
    match profile.seen.as_ref() {
        Some(seen) => format!(
            "Last seen {}",
            seen_ago(Some(seen.as_str()), jiff::Timestamp::now()).to_lowercase()
        ),
        None => "Never connected".to_owned(),
    }
}

/// A profile's page; "Unpaired" once it is gone.
pub(super) fn page(
    pane: &ChromePane,
    id: ChromeProfileId,
    p: &Pal,
    cx: &mut Context<'_, ChromePane>,
) -> (String, Div) {
    let store = pane.store.read(cx);
    let Some(profile) = store.chrome_profile(id).cloned() else {
        return (
            "Unpaired".to_owned(),
            div().child(note(
                p,
                format!("This Chrome profile isn't paired with {APP_NAME} anymore."),
            )),
        );
    };
    let issues = store.state().map_or_default(|state| state.issues.clone());
    let latest = store.state().and_then(|state| state.chrome_latest.clone());
    let (busy, problem) = (
        store.chrome().is_busy(),
        store.chrome().problem().map(str::to_owned),
    );
    let here = assistant_rows(pane, &profile, &issues, busy, p);
    let body = div()
        .flex()
        .flex_col()
        .gap(px(16.))
        .child(head(&profile, p))
        .when(!here.is_empty(), |d| {
            d.child(group(p, format!("{APP_NAME} in this profile"), here))
        })
        .child(group(
            p,
            "Details",
            details(&profile, &issues, latest.as_deref(), p),
        ))
        .when(profile.outdated, |d| {
            d.child(icon_line(
                IconName::RefreshCw,
                p.warn,
                "Chrome updates the extension from the Chrome Web Store by itself; restart \
                 Chrome to have it now. A copy loaded unpacked updates with reload at \
                 chrome://extensions."
                    .to_owned(),
            ))
        })
        .child(name_group(pane, &profile, busy, p, cx))
        .child(pairing_actions(pane, &profile, busy, p, cx))
        .children(problem.map(|problem| problem_line(p, problem)));
    (profile.label, body)
}

/// The badge, the name with its tags, the account and whether it is online.
fn head(profile: &ChromeProfile, p: &Pal) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(14.))
        .child(badge(profile, 52., p))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(3.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .child(
                            div()
                                .font_weight(FontWeight::BOLD)
                                .text_size(px(16.))
                                .child(profile.label.clone()),
                        )
                        .children(profile_tags(profile, p)),
                )
                .child(account_line(profile, p))
                .child(online(profile, p)),
        )
}

/// The assistant in the profile now: until when it may work there, the user's pause, a
/// hand-off waiting on the user with Hand back, and Pause.
fn assistant_rows(
    pane: &ChromePane,
    profile: &ChromeProfile,
    issues: &[Issue],
    busy: bool,
    p: &Pal,
) -> Vec<AnyElement> {
    let (id, zone) = (profile.id, TimeZone::system());
    let mut rows = Vec::new();
    if let Some(grant) = &profile.grant {
        rows.push(icon_line(
            IconName::Hand,
            p.accent,
            grant_line(grant, issues, &zone),
        ));
    }
    if let Some(paused) = &profile.paused {
        let back = clock_of_millis(paused.until, &zone).unwrap_or_default();
        rows.push(icon_line(
            IconName::CirclePause,
            p.muted,
            format!(
                "Paused ({}). Back by itself at {back}, or when you hand back.",
                paused.why
            ),
        ));
    }
    if profile.control == ChromeControl::Person {
        let why = profile
            .handoff
            .as_ref()
            .map_or_default(|why| format!(": {why}"));
        rows.push(icon_line(
            IconName::UserRound,
            p.warn,
            format!("Handed to you{why}"),
        ));
        let store = pane.store.clone();
        rows.push(
            div()
                .child(
                    Button::new("chrome-handback")
                        .small()
                        .icon(gpui_kit::component::IconName::Undo2)
                        .label(format!("Hand back to {APP_NAME}"))
                        .disabled(busy)
                        .on_click(move |_, _, cx| act(&store, ChromeAction::Handback { id }, cx)),
                )
                .into_any_element(),
        );
    }
    if profile.grant.is_some() {
        let store = pane.store.clone();
        rows.push(
            div()
                .child(
                    Button::new("chrome-pause")
                        .small()
                        .danger()
                        .icon(gpui_kit::component::IconName::Pause)
                        .label(format!("Pause {APP_NAME} for 10 minutes"))
                        .disabled(busy)
                        .on_click(move |_, _, cx| {
                            act(&store, ChromeAction::Pause { id, minutes: None }, cx);
                        }),
                )
                .into_any_element(),
        );
    }
    rows
}

/// The account, machine, extension, Chrome, status, when it was paired, the name the
/// assistant's tools use, and the tasks it is the default for.
fn details(
    profile: &ChromeProfile,
    issues: &[Issue],
    latest: Option<&str>,
    p: &Pal,
) -> Vec<AnyElement> {
    let not_known = || "Not known yet".to_owned();
    let mut rows = vec![
        detail(
            p,
            "Account",
            profile.email.clone().unwrap_or_else(not_known),
        ),
        detail(
            p,
            "Machine",
            profile
                .machine
                .as_deref()
                .map_or_else(not_known, machine_name),
        ),
        detail(p, "Extension", extension_line(profile, latest)),
    ];
    if let Some(major) = profile.chrome_major() {
        rows.push(detail(p, "Chrome", major.to_owned()));
    }
    rows.push(detail(p, "Status", status_line(profile)));
    if let Some(paired) = profile
        .created
        .as_ref()
        .and_then(|at| day_and_time(at.as_str(), &TimeZone::system()))
    {
        rows.push(detail(p, "Paired", paired));
    }
    rows.push(detail(
        p,
        format!("{APP_NAME} calls it"),
        selectable(
            ElementId::Name(format!("chrome-{}-tool-name", profile.id).into()),
            &format!("chrome:{}", profile.name),
        ),
    ));
    if !profile.tasks.is_empty() {
        let tasks = profile
            .tasks
            .iter()
            .map(|task| task_name(*task, issues))
            .collect::<Vec<_>>()
            .join(", ");
        rows.push(detail(p, "Default for", tasks));
    }
    rows
}

/// The name field with Rename.
fn name_group(
    pane: &ChromePane,
    profile: &ChromeProfile,
    busy: bool,
    p: &Pal,
    cx: &mut Context<'_, ChromePane>,
) -> Div {
    let typed = pane.rename.read(cx).value().trim().to_owned();
    let can_rename = !typed.is_empty() && typed != profile.label && !busy;
    group(
        p,
        "Name",
        vec![
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(div().flex_1().child(Input::new(&pane.rename)))
                .child(
                    Button::new("chrome-rename")
                        .label("Rename")
                        .disabled(!can_rename)
                        .on_click(cx.listener(|this, _, _, cx| this.save_name(cx))),
                )
                .into_any_element(),
            note(
                p,
                format!("What you and {APP_NAME} call this profile, like Work or Personal."),
            )
            .into_any_element(),
        ],
    )
}

/// Re-pair and Unpair (after its confirmation).
fn pairing_actions(
    pane: &ChromePane,
    profile: &ChromeProfile,
    busy: bool,
    p: &Pal,
    cx: &mut Context<'_, ChromePane>,
) -> Div {
    let id = profile.id;
    let (repair, unpair_ask) = (pane.store.clone(), cx.entity().downgrade());
    div()
        .flex()
        .flex_col()
        .gap(px(8.))
        .child(
            div()
                .flex()
                .gap(px(8.))
                .child(
                    Button::new("chrome-repair")
                        .icon(gpui_kit::component::IconName::ExternalLink)
                        .label("Re-pair with a new link")
                        .disabled(busy)
                        .on_click(move |_, _, cx| act(&repair, ChromeAction::Repair { id }, cx)),
                )
                .child(
                    Button::new("chrome-unpair")
                        .danger()
                        .label("Unpair")
                        .disabled(busy)
                        .on_click(move |_, _, cx| {
                            if let Err(error) = unpair_ask.update(cx, |this, cx| {
                                this.unpairing = true;
                                cx.notify();
                            }) {
                                log::debug!("the profiles page is gone: {error}");
                            }
                        }),
                ),
        )
        .when(pane.unpairing, |d| {
            d.child(unpair_confirm(pane, profile, p, cx))
        })
        .child(note(
            p,
            format!(
                "Re-pair if the extension was removed or reinstalled in that profile. Unpair \
                 cuts it off: {APP_NAME} can't reach it until you pair it again."
            ),
        ))
}

/// "Unpair …?" with its two answers.
fn unpair_confirm(
    pane: &ChromePane,
    profile: &ChromeProfile,
    p: &Pal,
    cx: &mut Context<'_, ChromePane>,
) -> Div {
    let (store, id) = (pane.store.clone(), profile.id);
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .px(px(10.))
        .py(px(8.))
        .rounded(px(9.))
        .bg(p.danger.opacity(0.08))
        .child(div().flex_1().text_size(px(12.5)).child(format!(
            "Unpair {}? {APP_NAME} can't use this Chrome profile until you pair it again.",
            profile.label
        )))
        .child(
            Button::new("chrome-unpair-confirm")
                .small()
                .danger()
                .label("Unpair")
                .on_click(move |_, _, cx| act(&store, ChromeAction::Unpair { id }, cx)),
        )
        .child(
            Button::new("chrome-unpair-cancel")
                .small()
                .ghost()
                .label("Cancel")
                .on_click(cx.listener(|this, _, _, cx| {
                    this.unpairing = false;
                    cx.notify();
                })),
        )
}
