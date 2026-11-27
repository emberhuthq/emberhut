//! Results: what the task produced that no run shows. Pictures and videos as tiles, PDFs,
//! decks, sheets and notes as rows with their icons (a click opens one in the viewer), links (a
//! pull request has its own icon), HTML reports as report cards opening in the panel beside the
//! page; each file with Save to Downloads and Copy link, its uses on a right-click, and dragged
//! out as the file. Under an ask, they fold into `Files and results · n`, below the answers.

use emberhut_api::tasks::{Artifact, TaskStatus};
use gpui_kit::assets::IconName;
use gpui_kit::component::menu::ContextMenuExt as _;
use gpui_kit::prelude::*;
use gpui_kit::{App, ClipboardItem, Div, ElementId, Entity, FontWeight, Window, div, px};

use super::{Part, TaskView, fold, fold_row, text};
use crate::store::files::RemoteFile;
use crate::store::tasks::runs;
use crate::store::{OpenReport, Store};
use crate::ui::files::actions::{self as file_actions, FileAction};
use crate::ui::files::drag;
use crate::ui::inline_video;
use crate::ui::palette::Pal;
use crate::ui::tasks::marks::glyph;
use crate::ui::viewer::OpenInViewer;

/// How a result shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Shape {
    /// A picture or a video, as a tile.
    Tile,
    /// A link.
    Link,
    /// An HTML report, as a card.
    Report,
    /// Any other file, as a row.
    File,
}

fn shape(artifact: &Artifact) -> Shape {
    if artifact.thumb.is_some() {
        Shape::Tile
    } else if artifact.kind == "link" {
        Shape::Link
    } else if artifact.kind == "html" {
        Shape::Report
    } else {
        Shape::File
    }
}

/// A file row's icon: a PDF, a deck, a sheet, a document or a note.
fn file_icon(artifact: &Artifact) -> IconName {
    let extension = artifact
        .name
        .rsplit_once('.')
        .map_or_else(String::new, |(_, extension)| extension.to_lowercase());
    if artifact.kind == "pdf" {
        IconName::FileText
    } else if ["pptx", "ppt", "key"].contains(&extension.as_str()) {
        IconName::Presentation
    } else if ["xlsx", "xls", "numbers", "csv"].contains(&extension.as_str()) {
        IconName::Sheet
    } else {
        IconName::File
    }
}

/// The results, folded under an ask or open after Where it stands.
pub(super) fn section(
    mut body: Div,
    part: &Part<'_>,
    view: &TaskView,
    cx: &Context<'_, TaskView>,
) -> Div {
    let runs = runs::timeline(part.page);
    let artifacts = if part.task.artifacts.is_empty() {
        part.page.map_or(&[][..], |page| &page.task.artifacts)
    } else {
        &part.task.artifacts
    };
    // A decision's cards show their own pictures, and an X post shows above: not again here.
    let on_cards = crate::store::decision::card_media(part.task);
    let mut results = runs::unclaimed(artifacts, &runs, part.now);
    results.retain(|artifact| !on_cards.contains(&artifact.id) && artifact.x.is_none());
    if results.is_empty() {
        return body;
    }
    let p = part.p;
    let asks = part.task.status == TaskStatus::Waiting && part.task.decision.is_some();
    if asks {
        let open = view.is_open(fold::RESULTS);
        body = body.child(
            fold_row(
                p,
                "results-fold",
                format!("Files and results · {}", results.len()),
                open,
            )
            .on_click(cx.listener(|this, _, _, cx| {
                this.toggle_open(fold::RESULTS);
                cx.notify();
            })),
        );
        if !open {
            return body;
        }
    } else {
        body = body.child(super::section(p, "Results"));
    }
    let tiles: Vec<&Artifact> = results
        .iter()
        .copied()
        .filter(|a| shape(a) == Shape::Tile)
        .collect();
    if !tiles.is_empty() {
        let mut row = div().flex().flex_wrap().gap(px(10.)).pb(px(8.));
        for artifact in tiles {
            row = row.child(tile(p, part.store, artifact, cx));
        }
        body = body.child(row);
    }
    for artifact in results.iter().copied().filter(|a| shape(a) != Shape::Tile) {
        body = body.child(file_row(p, part.store, artifact));
    }
    body
}

/// A picture or video tile: its thumbnail (a play mark on a video, which plays it in place on
/// macOS), its caption, Save.
fn tile(p: &Pal, store: &Entity<Store>, artifact: &Artifact, cx: &App) -> Div {
    let thumb = artifact
        .thumb
        .as_deref()
        .and_then(|thumb| store.read(cx).picture(thumb));
    let id = artifact.id.get();
    let shown = super::thumb::thumb(thumb, (200., 124.), 10., p.card, move || {
        format!("result-picture-{id}")
    });
    let video = artifact.kind == "video";
    let (url, name) = (artifact.url.clone(), artifact.name.clone());
    let file = remote(artifact);
    // A video playing in place, in its tile.
    if video && let Some(playing) = inline_video::playing(&file, (200., 124.), store, cx) {
        return div()
            .flex()
            .flex_col()
            .gap(px(4.))
            .w(px(200.))
            .child(
                drag::draggable(
                    div()
                        .id(ElementId::Name(format!("result-{}", artifact.id).into()))
                        .child(playing),
                    &file,
                    store,
                )
                .context_menu(file_actions::menu(&file, store)),
            )
            .child(caption_row(p, store, artifact));
    }
    let (opened, player) = (file.clone(), store.clone());
    div()
        .flex()
        .flex_col()
        .gap(px(4.))
        .w(px(200.))
        .child(
            drag::draggable(
                div()
                    .id(ElementId::Name(format!("result-{}", artifact.id).into()))
                    .relative()
                    .cursor_pointer()
                    .child(shown)
                    .when(video, |d| {
                        d.child(
                            div()
                                .absolute()
                                .inset_0()
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(glyph(IconName::CirclePlay, p.on_accent, 36.)),
                        )
                    }),
                &file,
                store,
            )
            .on_click(move |_, window, cx| {
                // A video plays in place where it can, else in the viewer.
                if !(video && inline_video::play(&player, &opened, cx)) {
                    view_in(window, &url, &name, cx);
                }
            })
            .context_menu(file_actions::menu(&file, store)),
        )
        .child(caption_row(p, store, artifact))
}

/// A tile's caption (its name when it has none) and its Save and Copy buttons.
fn caption_row(p: &Pal, store: &Entity<Store>, artifact: &Artifact) -> Div {
    let caption = artifact
        .caption
        .clone()
        .unwrap_or_else(|| artifact.name.clone());
    div()
        .flex()
        .items_center()
        .gap(px(4.))
        .child(
            div()
                .flex_1()
                .truncate()
                .text_size(px(11.5))
                .text_color(p.muted)
                .child(caption),
        )
        .child(file_buttons(p, store, artifact))
}

/// A link, a report card or a file row.
fn file_row(p: &Pal, store: &Entity<Store>, artifact: &Artifact) -> Div {
    let kind = shape(artifact);
    let icon = match kind {
        Shape::Link if artifact.url.contains("/pull/") => IconName::GitPullRequest,
        Shape::Link => IconName::Link,
        Shape::Report => IconName::FileChartColumn,
        Shape::File | Shape::Tile => file_icon(artifact),
    };
    let title = artifact
        .caption
        .clone()
        .unwrap_or_else(|| artifact.name.clone());
    let under = artifact.caption.as_ref().map(|_| artifact.name.clone());
    let (open, url, name) = (store.clone(), artifact.url.clone(), artifact.name.clone());
    let row = div()
        .id(ElementId::Name(format!("result-{}", artifact.id).into()))
        .flex()
        .items_center()
        .gap(px(10.))
        .flex_1()
        .min_w_0()
        .py(px(6.))
        .cursor_pointer()
        .child(glyph(icon, p.accent, 16.))
        .child(
            div()
                .flex()
                .flex_col()
                .min_w_0()
                .child(
                    div()
                        .truncate()
                        .text_size(px(13.))
                        .when(kind == Shape::Report, |d| d.font_weight(FontWeight::MEDIUM))
                        .child(title),
                )
                .children(under.map(|name| {
                    div()
                        .truncate()
                        .text_size(px(11.))
                        .text_color(p.muted)
                        .child(name)
                })),
        )
        .on_click(move |_, window, cx| match kind {
            Shape::Link => text::follow(&open, &url, window, cx),
            Shape::Report => open.update(cx, |store, cx| {
                store.show_report(
                    Some(OpenReport {
                        name: name.clone(),
                        path: url.clone(),
                    }),
                    cx,
                );
            }),
            Shape::File | Shape::Tile => view_in(window, &url, &name, cx),
        });
    let row = if kind == Shape::Link {
        row.into_any_element()
    } else {
        let file = remote(artifact);
        drag::draggable(row, &file, store)
            .context_menu(file_actions::menu(&file, store))
            .into_any_element()
    };
    let mut line = div().flex().items_center().gap(px(6.)).child(row);
    if kind == Shape::Report {
        line = line
            .border_1()
            .border_color(p.border)
            .rounded(px(10.))
            .px(px(10.))
            .my(px(3.))
            .bg(p.card);
    }
    if kind != Shape::Link {
        line = line.child(file_buttons(p, store, artifact));
    }
    line
}

/// Save to Downloads and Copy link, for a file.
fn file_buttons(p: &Pal, store: &Entity<Store>, artifact: &Artifact) -> Div {
    let (save, file) = (store.clone(), remote(artifact));
    let link = store.clone();
    let link_url = artifact.url.clone();
    div()
        .flex()
        .gap(px(2.))
        .flex_none()
        .child(
            div()
                .id(ElementId::Name(format!("save-{}", artifact.id).into()))
                .p(px(3.))
                .rounded(px(5.))
                .cursor_pointer()
                .hover(|d| d.bg(p.hover))
                .child(glyph(IconName::Download, p.muted, 13.))
                .on_click(move |_, window, cx| {
                    FileAction::SaveToDownloads.run(file.clone(), &save, window, cx);
                }),
        )
        .child(
            div()
                .id(ElementId::Name(format!("link-{}", artifact.id).into()))
                .p(px(3.))
                .rounded(px(5.))
                .cursor_pointer()
                .hover(|d| d.bg(p.hover))
                .child(glyph(IconName::Share2, p.muted, 13.))
                .on_click(move |_, _, cx| {
                    if let Some(address) = link.read(cx).server().link(&link_url) {
                        cx.write_to_clipboard(ClipboardItem::new_string(address));
                    }
                }),
        )
}

/// `artifact` as the viewer and its uses take it.
fn remote(artifact: &Artifact) -> RemoteFile {
    RemoteFile::new(artifact.url.clone(), artifact.name.clone())
}

/// Opens the result at `url`, called `name`, in the viewer of `window`.
fn view_in(window: &mut Window, url: &str, name: &str, cx: &mut App) {
    let file = RemoteFile::new(url, name);
    window.dispatch_action(Box::new(OpenInViewer { file }), cx);
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn artifact(kind: &str, name: &str, thumb: bool) -> Artifact {
        serde_json::from_value(json!({"id": 1, "kind": kind, "caption": null,
            "at": "2026-11-25T08:00:00Z", "name": name, "url": "/api/artifacts/1",
            "thumb": thumb.then_some("/api/artifacts/1/thumb")}))
        .unwrap()
    }

    #[test]
    fn each_result_shows_by_its_kind() {
        assert_eq!(shape(&artifact("image", "a.png", true)), Shape::Tile);
        assert_eq!(shape(&artifact("link", "PR", false)), Shape::Link);
        assert_eq!(shape(&artifact("html", "week.html", false)), Shape::Report);
        assert_eq!(shape(&artifact("pdf", "a.pdf", false)), Shape::File);
        assert!(matches!(
            file_icon(&artifact("file", "deck.PPTX", false)),
            IconName::Presentation
        ));
        assert!(matches!(
            file_icon(&artifact("file", "data.xlsx", false)),
            IconName::Sheet
        ));
        assert!(matches!(
            file_icon(&artifact("pdf", "a.pdf", false)),
            IconName::FileText
        ));
    }
}
