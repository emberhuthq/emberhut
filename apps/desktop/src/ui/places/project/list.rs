//! A project page's work as a list: each lane's head (its area's state, done of all), its
//! open work with the steps nested, its findings (a row with the count of each state, opening
//! the open ones grouped by state), then its done work folded.

use emberhut_api::areas::{Area, FindingItem};
use emberhut_api::ids::TaskId;
use gpui_kit::assets::IconName;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, Div, ElementId, Entity, FontWeight, div, px};
use jiff::civil::Date;

use super::{fold_row, stage_mark};
use crate::prose::direction;
use crate::store::projects::{Item, Lane, Stage};
use crate::store::tasks::{counts, findings};
use crate::store::{Page, Store};
use crate::ui::a11y::{self, Accessible as _};
use crate::ui::column;
use crate::ui::controls::tab_stop;
use crate::ui::palette::Pal;
use crate::ui::tasks::marks::{deadline_chip, glyph, landed_mark};

/// A lane as the list shows it: its head (the area's state, done of all, its findings), its
/// open work with the steps nested, then its done work folded.
pub(super) fn lane(
    p: &Pal,
    store: &Entity<Store>,
    read: &Store,
    lane: &Lane<'_>,
    today: Date,
) -> Div {
    let (open, done): (Vec<&Item>, Vec<&Item>) = lane
        .items
        .iter()
        .partition(|item| item.stage != Stage::Done);
    let mut head = div()
        .flex()
        .items_center()
        .gap(px(8.))
        .children(lane.area.map(|area| {
            stage_mark(
                p,
                match area.state {
                    emberhut_api::areas::WorkStage::Done => Stage::Done,
                    emberhut_api::areas::WorkStage::NeedsYou => Stage::NeedsYou,
                    emberhut_api::areas::WorkStage::NotStarted => Stage::NotStarted,
                    _ => Stage::Working,
                },
            )
        }))
        .child(
            div()
                .flex_1()
                .text_size(px(13.))
                .font_weight(FontWeight::SEMIBOLD)
                .child(direction::marked(&lane.title)),
        );
    if let Some(area) = lane.area {
        head = head.child(
            div()
                .text_size(px(12.))
                .text_color(p.muted)
                .child(format!("{}/{}", area.counts.done, area.counts.total)),
        );
    }
    let mut block = div().flex().flex_col().pt(px(14.)).child(head);
    for item in open {
        block = item_rows(block, p, item, 0, today);
    }
    if let Some(area) = lane.area.filter(|area| area.findings.total > 0) {
        block = block.child(findings(p, store, read, area));
    }
    if !done.is_empty() {
        let key = format!("done-{}", lane.title);
        let open = read.places().is_flipped(&key);
        let (flip, flip_key) = (store.clone(), key.clone());
        block = block.child(
            fold_row(p, &key, format!("{} done", done.len()), open).on_click(move |_, _, cx| {
                flip.update(cx, |store, cx| store.flip_place_fold(&flip_key, cx));
            }),
        );
        if open {
            for item in done {
                block = item_rows(block, p, item, 0, today);
            }
        }
    }
    block
}

/// An item's row (and its steps under it, indented): its stage, its title, its action, its
/// day; a click opens its task.
fn item_rows(mut block: Div, p: &Pal, item: &Item, depth: u16, today: Date) -> Div {
    let id = item.id;
    let chip = counts::deadline_chip(item.day.as_ref(), false, today);
    block = block.child(
        div()
            .id(ElementId::Name(format!("project-item-{id}-{depth}").into()))
            // One line for the row: its title, its stage and what to do.
            .button_named(a11y::item(
                &item.title,
                item.stage.name(),
                item.action.as_deref(),
            ))
            .flex()
            .items_center()
            .gap(px(8.))
            .py(px(4.))
            .pl(px(f32::from(depth) * 16.))
            .cursor_pointer()
            .hover(|d| d.bg(p.hover))
            .child(stage_mark(p, item.stage))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .truncate()
                            .text_size(px(13.))
                            .text_color(if item.stage == Stage::Done {
                                p.muted
                            } else {
                                p.text
                            })
                            .child(direction::marked(&item.title)),
                    )
                    .children(item.action.clone().map(|action| {
                        div()
                            .truncate()
                            .text_size(px(12.))
                            .text_color(p.accent)
                            .child(action)
                    })),
            )
            .children(chip.map(|chip| deadline_chip(p, chip)))
            .on_click(move |_, window, cx| column::push(Page::Task(id), window, cx)),
    );
    for kid in &item.kids {
        block = item_rows(block, p, kid, depth + 1, today);
    }
    block
}

/// An area's findings: a row with the count of each state; a click opens them grouped by
/// state (what each state means first, and a scan picker when they came from more than one
/// scan), each with its scan and the task that fixes it. The landed and superseded ones are
/// counted, not listed.
fn findings(p: &Pal, store: &Entity<Store>, read: &Store, area: &Area) -> Div {
    let key = format!("findings-{}", area.id);
    let open = read.places().is_flipped(&key);
    let groups = findings::area_groups(&area.findings);
    let mut counts = div().flex().items_center().gap(px(8.));
    for (state, _, count) in &groups {
        counts = counts.child(
            div()
                .flex()
                .items_center()
                .gap(px(3.))
                .text_size(px(12.))
                .text_color(p.muted)
                .child(landed_mark(p, state))
                .child(count.to_string()),
        );
    }
    let (flip, flip_key) = (store.clone(), key.clone());
    let row = div()
        .id(ElementId::Name(key.clone().into()))
        .flex()
        .items_center()
        .gap(px(8.))
        .py(px(4.))
        .cursor_pointer()
        .hover(|d| d.bg(p.hover))
        .text_size(px(13.))
        .child(glyph(IconName::FileSearch, p.muted, 12.))
        .child(div().flex_1().child("Findings"))
        .child(counts)
        .child(glyph(
            if open {
                IconName::ChevronDown
            } else {
                IconName::ChevronRight
            },
            p.faint,
            13.,
        ))
        .on_click(move |_, _, cx| {
            flip.update(cx, |store, cx| store.flip_place_fold(&flip_key, cx));
        });
    let mut block = div().flex().flex_col().child(tab_stop(row.border_1(), p));
    if !open {
        return block;
    }
    let mut legend = div().flex().flex_col().gap(px(2.)).py(px(4.)).pl(px(22.));
    for state in &findings::ORDER {
        legend = legend.child(
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .text_size(px(11.))
                .text_color(p.faint)
                .child(landed_mark(p, state))
                .child(findings::meaning(state)),
        );
    }
    block = block.child(legend);
    let scans = findings::scans(&area.findings);
    let scan = read
        .places()
        .area_scan(area.id)
        .filter(|picked| scans.iter().any(|(scan, _)| scan == picked));
    if scans.len() > 1 {
        block = block.child(scan_picker(store, area, &scans, scan));
    }
    for (state, rows, count) in findings::area_groups_of(&area.findings, scan) {
        block = block.child(
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .pt(px(8.))
                .pl(px(22.))
                .text_size(px(12.))
                .font_weight(FontWeight::SEMIBOLD)
                .child(landed_mark(p, &state))
                .child(findings::name(&state))
                .child(div().text_color(p.muted).child(count.to_string())),
        );
        if rows.is_empty() {
            block = block.child(
                div()
                    .pl(px(41.))
                    .py(px(2.))
                    .text_size(px(12.))
                    .text_color(p.muted)
                    .child("Counted, not listed here"),
            );
        }
        for finding in rows {
            block = block.child(finding_row(p, read, finding));
        }
    }
    block
}

/// The scan picker over an area's findings from more than one scan: Every scan, or one of
/// `scans` (`picked`).
fn scan_picker(
    store: &Entity<Store>,
    area: &Area,
    scans: &[(TaskId, &str)],
    picked: Option<TaskId>,
) -> Div {
    let label = picked
        .and_then(|picked| scans.iter().find(|(scan, _)| *scan == picked))
        .map_or("Every scan", |(_, title)| *title)
        .to_owned();
    let choices: Vec<(Option<TaskId>, String)> = std::iter::once((None, "Every scan".to_owned()))
        .chain(
            scans
                .iter()
                .map(|(scan, title)| (Some(*scan), (*title).to_owned())),
        )
        .collect();
    let (store, id) = (store.clone(), area.id);
    div().pl(px(22.)).pt(px(4.)).child(
        Button::new(ElementId::Name(format!("findings-{id}-scan").into()))
            .label(format!("Scan: {label}"))
            .ghost()
            .xsmall()
            .dropdown_menu(move |mut menu, _, _| {
                for (scan, title) in &choices {
                    let (store, scan) = (store.clone(), *scan);
                    menu = menu.item(
                        PopupMenuItem::new(title.clone())
                            .checked(scan == picked)
                            .on_click(move |_, _, cx| {
                                store.update(cx, |store, cx| store.pick_area_scan(id, scan, cx));
                            }),
                    );
                }
                menu
            }),
    )
}

/// One finding: its mark, its title and its scan, the task that fixes it (in the accent while
/// that one waits on the user); a click opens the finding.
fn finding_row(p: &Pal, read: &Store, finding: &FindingItem) -> AnyElement {
    let id = finding.id;
    let fix = finding.fixed_by.map(|fix| {
        let waiting = finding.fixed_by_status.as_deref() == Some("waiting");
        let tip = match &finding.fixed_by_status {
            Some(status) => format!("Fixed by {} ({status})", read.task_key(fix)),
            None => format!("Fixed by {}", read.task_key(fix)),
        };
        div()
            .id(ElementId::Name(format!("finding-{id}-fix").into()))
            .flex_none()
            .text_size(px(11.))
            .text_color(if waiting { p.accent } else { p.faint })
            .child(read.task_key(fix))
            .tooltip(move |window, cx| {
                gpui_kit::component::tooltip::Tooltip::new(tip.clone()).build(window, cx)
            })
    });
    div()
        .id(ElementId::Name(format!("finding-{id}").into()))
        .flex()
        .items_start()
        .gap(px(8.))
        .py(px(4.))
        .pl(px(22.))
        .cursor_pointer()
        .hover(|d| d.bg(p.hover))
        .child(div().pt(px(2.)).child(landed_mark(p, &finding.landed)))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .truncate()
                        .text_size(px(13.))
                        .child(finding.title.clone()),
                )
                .child(
                    div()
                        .truncate()
                        .text_size(px(11.5))
                        .text_color(p.muted)
                        .child(finding.scan_title.clone()),
                ),
        )
        .children(fix)
        .on_click(move |_, window, cx| column::push(Page::Task(id), window, cx))
        .into_any_element()
}
