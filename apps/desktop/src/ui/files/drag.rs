//! Dragging a file out of the app (a chip, a picture, a task's result) into Finder, the
//! desktop or another app, as the real file under its real name. GPUI hands the platform a
//! file on disk when the drag leaves the window, so the drag starts the file's download, and a
//! file that is here by then goes out (one still coming down is not dropped: drag it again once
//! the status line says it is here).

use gpui_kit::prelude::*;
use gpui_kit::{
    Div, Entity, ExternalDragPayload, FileDragPaths, FontWeight, SharedString, Stateful, Window,
    div, px,
};

use crate::store::Store;
use crate::store::files::RemoteFile;
use crate::ui::palette::pal;

/// A file being dragged.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DraggedFile(RemoteFile);

/// What follows the pointer while a file is dragged inside the window: its name.
pub(crate) struct DragLabel(SharedString);

impl Render for DragLabel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let p = pal(cx);
        div()
            .px(px(10.))
            .py(px(5.))
            .rounded(px(8.))
            .bg(p.card)
            .border_1()
            .border_color(p.border)
            .shadow_md()
            .text_size(px(12.5))
            .font_weight(FontWeight::MEDIUM)
            .text_color(p.text)
            .child(self.0.clone())
    }
}

/// `element`, draggable out of the app as `file`.
pub(crate) fn draggable(
    element: Stateful<Div>,
    file: &RemoteFile,
    store: &Entity<Store>,
) -> Stateful<Div> {
    let (fetch, out) = (store.clone(), store.clone());
    element
        .on_drag(DraggedFile(file.clone()), move |dragged, _, _, cx| {
            // The download starts with the drag, so the file may be here when it leaves.
            fetch.update(cx, |store, cx| drop(store.fetch_file(&dragged.0, cx)));
            cx.new(|_| DragLabel(dragged.0.name.clone().into()))
        })
        .external_drag_payload(move |dragged: &DraggedFile, _, cx| {
            let path = out.read(cx).kept_file(&dragged.0)?;
            Some(ExternalDragPayload::Files(FileDragPaths::new([(
                path, false,
            )])))
        })
}
