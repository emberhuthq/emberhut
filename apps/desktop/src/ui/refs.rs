//! What an answer is about, as chips under it: `label · title` (a task, a pull request, a
//! report; a card by its label only). A task's chip is tinted by where the task stands and
//! opens its page; a report opens beside the page; anything else its link. Right-click: Copy
//! Link, and a task's key.

use emberhut_api::chat::{ChatMessage, MessageRef, RefKind};
use emberhut_api::state::State;
use emberhut_api::tasks::TaskStatus;
use gpui_kit::TestSupportExt as _;
use gpui_kit::assets::IconName as AssetIcon;
use gpui_kit::component::Icon;
use gpui_kit::component::menu::{ContextMenuExt as _, PopupMenuItem};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::prelude::*;
use gpui_kit::{ClipboardItem, ElementId, Entity, Hsla, div, px};

use super::palette::Pal;
use super::prose::follow_link;
use crate::prose::links;
use crate::store::{OpenReport, Page, Store, lookup};

/// The most characters of a title a chip shows.
const TITLE_CHARS: usize = 34;

/// What a chip reads: its label (a task's key), ` · ` and its title, the title cut at
/// [`TITLE_CHARS`]; a card, or a chip with no title, by its label only.
#[must_use]
pub(crate) fn chip_text(reference: &MessageRef) -> String {
    match reference.title.as_deref() {
        Some(title) if !title.is_empty() && reference.kind != RefKind::Card => {
            let title = if title.chars().count() > TITLE_CHARS {
                let cut: String = title.chars().take(TITLE_CHARS - 1).collect();
                format!("{}…", cut.trim())
            } else {
                title.to_owned()
            };
            format!("{} · {title}", reference.label)
        }
        _ => reference.label.clone(),
    }
}

/// Where a task stands, as its chip's tint shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Tint {
    /// Not a task, or one the state doesn't have: muted.
    Plain,
    /// Done: green.
    Done,
    /// Waiting on the user: orange.
    Waiting,
    /// Being worked: blue.
    Working,
    /// Put away: muted.
    PutAway,
    /// To do: the accent.
    Open,
}

/// The tint of `reference`'s chip in `state`.
#[must_use]
pub(crate) fn tint(reference: &MessageRef, state: Option<&State>) -> Tint {
    let task = (reference.kind == RefKind::Task)
        .then_some(reference.task)
        .flatten()
        .zip(state)
        .and_then(|(task, state)| lookup::issue(state, task));
    match task.map(|task| &task.status) {
        None => Tint::Plain,
        Some(TaskStatus::Done) => Tint::Done,
        Some(TaskStatus::Waiting) => Tint::Waiting,
        Some(TaskStatus::Working) => Tint::Working,
        Some(TaskStatus::Canceled) => Tint::PutAway,
        Some(_) => Tint::Open,
    }
}

impl Tint {
    fn color(self, p: &Pal) -> Hsla {
        match self {
            Self::Plain | Self::PutAway => p.muted,
            Self::Done => p.success,
            Self::Waiting => p.warn,
            Self::Working => p.info,
            Self::Open => p.accent,
        }
    }
}

/// The link Copy Link copies: a task's own link, or a web address.
#[must_use]
pub(crate) fn copied_link(reference: &MessageRef) -> Option<String> {
    if reference.kind == RefKind::Task
        && let Some(task) = reference.task
    {
        return Some(links::task_url(task));
    }
    reference
        .url
        .clone()
        .filter(|url| url.starts_with("https://"))
}

fn kind_icon(kind: &RefKind) -> AssetIcon {
    match kind {
        RefKind::Task => AssetIcon::ListChecks,
        RefKind::Pr => AssetIcon::GitPullRequest,
        RefKind::Ticket => AssetIcon::Ticket,
        RefKind::Card => AssetIcon::Layers,
        RefKind::Report => AssetIcon::FileText,
        _ => AssetIcon::File,
    }
}

/// A reference's chip.
pub(crate) fn chip(
    message: &ChatMessage,
    i: usize,
    reference: &MessageRef,
    store: &Entity<Store>,
    p: &Pal,
    state: Option<&State>,
) -> impl IntoElement {
    let reference = reference.clone();
    let tint = tint(&reference, state).color(p);
    let tip = reference.title.clone().unwrap_or_default();
    let link = copied_link(&reference);
    let key = (reference.kind == RefKind::Task).then(|| reference.label.clone());
    let store = store.clone();
    div()
        .id(ElementId::Name(format!("ref-{}-{i}", message.id).into()))
        .flex()
        .items_center()
        .gap(px(6.))
        .px(px(10.))
        .py(px(5.))
        .rounded(px(9.))
        .border_1()
        .border_color(p.border)
        .bg(p.card)
        .cursor_pointer()
        .hover(|d| d.bg(p.hover))
        .text_size(px(12.5))
        .max_w(px(360.))
        .child(
            Icon::new(kind_icon(&reference.kind))
                .size(px(13.))
                .text_color(tint),
        )
        .child(div().truncate().child(chip_text(&reference)))
        // Seen by the headless tests (a no-op outside them).
        .test_support()
        .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
        .on_click({
            let reference = reference.clone();
            move |_, window, cx| open(&reference, &store, window, cx)
        })
        .context_menu(move |menu, _, _| {
            let menu = match link.clone() {
                Some(link) => menu.item(
                    PopupMenuItem::new("Copy Link")
                        .icon(Icon::new(AssetIcon::Link))
                        .on_click(move |_, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(link.clone()));
                        }),
                ),
                None => menu,
            };
            match key.clone() {
                Some(key) => menu.item(
                    PopupMenuItem::new(format!("Copy {key}"))
                        .icon(Icon::new(AssetIcon::Hash))
                        .on_click(move |_, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(key.clone()));
                        }),
                ),
                None => menu,
            }
        })
}

/// A chip clicked: a task's page (done and put away ones too) in the window's column, over
/// what it shows; a report beside the page; a file or link as a link in a message opens.
fn open(
    reference: &MessageRef,
    store: &Entity<Store>,
    window: &mut gpui_kit::Window,
    cx: &mut gpui_kit::App,
) {
    if reference.kind == RefKind::Task
        && let Some(task) = reference.task
    {
        super::column::push(Page::Task(task), window, cx);
        return;
    }
    let Some(url) = reference.url.as_deref() else {
        return;
    };
    if reference.kind == RefKind::Report && crate::config::ServerUrl::is_path(url) {
        let report = OpenReport {
            name: reference
                .name
                .clone()
                .or_else(|| reference.title.clone())
                .unwrap_or_else(|| reference.label.clone()),
            path: url.to_owned(),
        };
        store.update(cx, |store, cx| store.show_report(Some(report), cx));
        return;
    }
    follow_link(store, url, window, cx);
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::fixtures::{issue, state};
    use crate::wire::TASK_KEY_PREFIX;

    fn reference(value: serde_json::Value) -> MessageRef {
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn a_chip_reads_its_label_and_a_short_title() {
        let task = reference(
            json!({"kind": "task", "label": format!("{TASK_KEY_PREFIX}37"),
            "task": 37, "title": "Bakery: why the weekend menu sold out early (post-mortem)"}),
        );
        assert_eq!(
            chip_text(&task),
            format!("{TASK_KEY_PREFIX}37 · Bakery: why the weekend menu sold…")
        );
        let pr = reference(json!({"kind": "pr", "label": "PR #76", "title": "example/app"}));
        assert_eq!(chip_text(&pr), "PR #76 · example/app");
        let card = reference(json!({"kind": "card", "label": "CC card", "title": "bc349c528b"}));
        assert_eq!(chip_text(&card), "CC card");
        let bare = reference(json!({"kind": "report", "label": "Report"}));
        assert_eq!(chip_text(&bare), "Report");
    }

    #[test]
    fn a_tasks_chip_is_tinted_by_where_it_stands() {
        let lists = state(json!({"issues": [issue(1, "done", json!({})),
            issue(2, "waiting", json!({})), issue(3, "working", json!({})),
            issue(4, "canceled", json!({})), issue(5, "todo", json!({}))]}));
        let tints: Vec<Tint> = (1..=6)
            .map(|id| {
                tint(
                    &reference(json!({"kind": "task", "label": "x", "task": id})),
                    Some(&lists),
                )
            })
            .collect();
        assert_eq!(
            tints,
            [
                Tint::Done,
                Tint::Waiting,
                Tint::Working,
                Tint::PutAway,
                Tint::Open,
                Tint::Plain
            ]
        );
        let pr = reference(json!({"kind": "pr", "label": "PR #1", "task": 1}));
        assert_eq!(tint(&pr, Some(&lists)), Tint::Plain);
    }

    #[test]
    fn copy_link_copies_a_tasks_link_or_a_web_address() {
        let task =
            reference(json!({"kind": "task", "label": format!("{TASK_KEY_PREFIX}9"), "task": 9}));
        assert_eq!(
            copied_link(&task),
            Some(links::task_url(emberhut_api::ids::TaskId::new(9)))
        );
        let pr =
            reference(json!({"kind": "pr", "label": "PR #1", "url": "https://example.com/pull/1"}));
        assert_eq!(
            copied_link(&pr).as_deref(),
            Some("https://example.com/pull/1")
        );
        let report =
            reference(json!({"kind": "report", "label": "Report", "url": "/api/artifacts/3"}));
        assert_eq!(copied_link(&report), None);
    }
}
