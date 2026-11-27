//! The viewer's bars. On top (in a sheet): the file's name and Open in a window. Below: the
//! file's uses (Share…, Save to Downloads, Save As…, Open in its app, Copy), the zoom (out,
//! fit, in) for a picture, a PDF or a note, and Done.

use gpui_kit::assets::IconName;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonGroup, ButtonVariants as _};
use gpui_kit::prelude::*;
use gpui_kit::{App, Div, Entity, FontWeight, div, px};

use super::{FileViewer, zoom};
use crate::store::Store;
use crate::store::files::{RemoteFile, Use};
use crate::ui::files::actions::FileAction;
use crate::ui::palette::Pal;

/// Saves `file` into Downloads (⌘S).
pub(super) fn save_to_downloads(store: &Entity<Store>, file: RemoteFile, cx: &mut App) {
    store.update(cx, |store, cx| {
        store.use_file(file, Use::SaveToDownloads, cx);
    });
}

/// The top bar: the name, and Open in a window.
pub(super) fn top(viewer: &FileViewer, p: &Pal, cx: &mut Context<'_, FileViewer>) -> Div {
    let name = viewer.file().map_or_default(|file| file.name.clone());
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(8.))
        .h(px(44.))
        .px(px(16.))
        .border_b_1()
        .border_color(p.border)
        .bg(p.surface_raised)
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .font_weight(FontWeight::SEMIBOLD)
                .text_size(px(14.))
                .child(crate::prose::direction::marked(&name)),
        )
        .child(
            Button::new("viewer-own-window")
                .ghost()
                .small()
                .icon(IconName::Maximize)
                .accessibility_label("Open in its own window")
                .tooltip(full_screen_tip())
                .on_click(
                    cx.listener(|this, _, window, cx| this.move_to_own_window(false, window, cx)),
                ),
        )
}

/// What Open in a window's tooltip says.
fn full_screen_tip() -> &'static str {
    if cfg!(target_os = "macos") {
        "Open in its own window (⌃⌘F: full screen)"
    } else {
        "Open in its own window (F11: full screen)"
    }
}

/// The bottom bar: the uses, the zoom, Done.
pub(super) fn bottom(viewer: &FileViewer, p: &Pal, cx: &mut Context<'_, FileViewer>) -> Div {
    let Some(open) = &viewer.open else {
        return div();
    };
    let file = open.file.clone();
    let uses = FileAction::offered(&file).into_iter().map(|action| {
        let (file, store) = (file.clone(), viewer.store.clone());
        Button::new(format!("viewer-{action:?}"))
            .small()
            .ghost()
            .icon(action.icon())
            .label(action.label(&file))
            .on_click(move |_, window, cx| action.run(file.clone(), &store, window, cx))
    });
    let zooms = open.kind.zooms().then(|| {
        ButtonGroup::new("viewer-zoom")
            .small()
            .child(
                Button::new("viewer-zoom-out")
                    .icon(IconName::ZoomOut)
                    .accessibility_label("Zoom Out")
                    .tooltip("Zoom Out")
                    .on_click(cx.listener(|this, _, _, cx| this.zoom(Some(zoom::OUT), cx))),
            )
            .child(
                Button::new("viewer-zoom-fit")
                    .icon(IconName::ArrowLeftRight)
                    .accessibility_label("Fit width")
                    .tooltip("Fit width")
                    .on_click(cx.listener(|this, _, _, cx| this.zoom(None, cx))),
            )
            .child(
                Button::new("viewer-zoom-in")
                    .icon(IconName::ZoomIn)
                    .accessibility_label("Zoom In")
                    .tooltip("Zoom In")
                    .on_click(cx.listener(|this, _, _, cx| this.zoom(Some(zoom::IN), cx))),
            )
    });
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(6.))
        .px(px(12.))
        .py(px(10.))
        .border_t_1()
        .border_color(p.border)
        .bg(p.surface_raised)
        .children(uses)
        .children(zooms)
        .child(div().flex_1())
        .when(!viewer.windowed, |d| {
            d.child(
                Button::new("viewer-done")
                    .primary()
                    .small()
                    .label("Done")
                    .on_click(cx.listener(|this, _, window, cx| this.done(window, cx))),
            )
        })
}
