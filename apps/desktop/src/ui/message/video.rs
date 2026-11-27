//! A video in a message: its first still frame (sent with it) under a play mark, playing in its
//! row on a click on macOS ([`inline_video`]), in the viewer elsewhere.

use emberhut_api::chat::{ChatFile, ChatMessage};
use gpui_kit::assets::IconName as AssetIcon;
use gpui_kit::component::Icon;
use gpui_kit::component::menu::ContextMenuExt as _;
use gpui_kit::prelude::*;
use gpui_kit::{ElementId, Entity, ImageSource, div, img, px};

use super::{poster, remote, view_in};
use crate::store::Store;
use crate::ui::chat::ChatView;
use crate::ui::files::actions as file_actions;
use crate::ui::files::drag;
use crate::ui::inline_video;

/// A video in a message: its first frame (the still sent with it), shaped like the video, with
/// a play mark; a click plays it in place on macOS ([`inline_video`]), in the viewer elsewhere.
/// Without a still, a dark frame with its name.
pub(super) fn video(
    message: &ChatMessage,
    i: usize,
    file: &ChatFile,
    store: &Entity<Store>,
    cx: &Context<'_, ChatView>,
) -> impl IntoElement {
    let still = poster(&message.files, file).and_then(|still| store.read(cx).picture(&still.url));
    let id = ElementId::Name(format!("video-{}-{i}", message.id).into());
    let remote_file = remote(file);
    let natural = still.as_deref().map(inline_video::still_size);
    let menu = file_actions::menu(&remote_file, store);
    if let Some(playing) = inline_video::playing(
        &remote_file,
        inline_video::fit(natural, (420., 320.)),
        store,
        cx,
    ) {
        return drag::draggable(div().id(id).child(playing), &remote_file, store)
            .context_menu(menu)
            .into_any_element();
    }
    let frame = match still {
        Some(image) => img(ImageSource::Render(image))
            .max_w(px(420.))
            .max_h(px(320.))
            .rounded(px(12.))
            .into_any_element(),
        None => div()
            .w(px(320.))
            .h(px(180.))
            .rounded(px(12.))
            .bg(gpui_kit::black())
            .flex()
            .items_end()
            .p(px(10.))
            .child(
                div()
                    .truncate()
                    .text_size(px(12.))
                    .text_color(gpui_kit::white().opacity(0.8))
                    .child(file.name.clone()),
            )
            .into_any_element(),
    };
    let element = div().id(id).relative().cursor_pointer().child(frame).child(
        div()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .child(
                Icon::new(AssetIcon::CirclePlay)
                    .size(px(46.))
                    .text_color(gpui_kit::white()),
            ),
    );
    let (opened, player) = (remote_file.clone(), store.clone());
    drag::draggable(element, &remote_file, store)
        .on_click(move |_, window, cx| {
            if !inline_video::play(&player, &opened, cx) {
                view_in(window, opened.clone(), cx);
            }
        })
        .context_menu(menu)
        .into_any_element()
}
