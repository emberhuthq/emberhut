//! The reply box on a task's page: the user's last message on the task and what came back
//! since, the photos added (four at most), the field and Send, and where the message went. It
//! goes where an answer goes (the task's worker, its session, or the assistant in its room),
//! never to Main.

use std::path::{Path, PathBuf};

use emberhut_api::ids::TaskId;
use emberhut_api::tasks::TaskStatus;
use gpui_kit::assets::IconName;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::input::Textarea;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::prelude::*;
use gpui_kit::{App, Div, ElementId, Entity, PathPromptOptions, div, px};

use super::text::{Long, collapsible};
use super::{Part, TaskView};
use crate::store::Store;
use crate::store::tasks::replies::Reply;
use crate::store::tasks::{clock, history};
use crate::ui::tasks::marks::glyph;

/// The picture files a reply takes, by extension.
const PICTURES: [&str; 8] = ["png", "jpg", "jpeg", "gif", "webp", "heic", "heif", "tiff"];

/// Whether `path` names a picture.
fn is_picture(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| PICTURES.contains(&extension.to_lowercase().as_str()))
}

/// The section, on every task but an archived one.
pub(super) fn section_for(part: &Part<'_>) -> Option<&'static str> {
    let task = part.task;
    if task.status == TaskStatus::Canceled {
        return None;
    }
    let decision = task
        .decision
        .as_ref()
        .filter(|_| task.status == TaskStatus::Waiting);
    Some(match decision {
        None => "Message",
        Some(decision) if decision.options.is_empty() => "Your answer",
        Some(_) => "Or answer in your own words",
    })
}

/// The reply section.
pub(super) fn section(
    mut body: Div,
    part: &Part<'_>,
    view: &TaskView,
    cx: &Context<'_, TaskView>,
) -> Div {
    let Some(title) = section_for(part) else {
        return body;
    };
    let (p, task) = (part.p, part.task.id);
    body = body.child(super::section(p, title));
    body = exchange(body, part, view, cx);
    let reply = part.read.tasks().reply(task);
    body = photos(body, part, &reply);
    let text = view.reply.read(cx).value().to_string();
    let ready = reply.can_send(&text);
    let add = part.store.clone();
    let room = reply.room_for();
    body = body.child(
        div()
            .flex()
            .items_end()
            .gap(px(8.))
            .child(
                div()
                    .id("reply-photo")
                    .p(px(5.))
                    .rounded(px(6.))
                    .cursor_pointer()
                    .when(room == 0, |d| d.opacity(0.4))
                    .hover(|d| d.bg(p.hover))
                    .child(glyph(IconName::Image, p.muted, 17.))
                    .on_click(move |_, _, cx| {
                        if room > 0 {
                            pick_photos(&add, task, cx);
                        }
                    }),
            )
            .child(div().flex_1().min_w_0().child(Textarea::new(&view.reply)))
            .child(if reply.sending {
                div()
                    .p(px(4.))
                    .child(Spinner::new().color(p.muted))
                    .into_any_element()
            } else {
                div()
                    .id("reply-send")
                    .p(px(3.))
                    .cursor_pointer()
                    .child(glyph(
                        IconName::CircleArrowUp,
                        if ready { p.accent } else { p.faint },
                        24.,
                    ))
                    .on_click(cx.listener(|this, _, window, cx| this.send_reply(window, cx)))
                    .into_any_element()
            }),
    );
    if let Some(note) = reply.note {
        body = body.child(
            div()
                .pt(px(4.))
                .text_size(px(11.5))
                .text_color(p.muted)
                .child(note),
        );
    }
    body
}

/// The user's last message on the task and what came back since.
fn exchange(mut body: Div, part: &Part<'_>, view: &TaskView, cx: &Context<'_, TaskView>) -> Div {
    let p = part.p;
    if let Some(page) = part.page {
        for (i, moment) in history::exchange(&page.history).into_iter().enumerate() {
            let at = clock::instant(&moment.at)
                .map_or_else(String::new, |at| clock::day_and_time(at, part.now));
            let key = format!("exchange-{i}");
            body = body.child(
                div()
                    .flex()
                    .gap(px(10.))
                    .pb(px(8.))
                    .child(who(p, history::is_mine(moment)))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .child(collapsible(
                                p,
                                part.store,
                                Long {
                                    key: &key,
                                    text: &moment.text,
                                    lines: 4,
                                    title: None,
                                },
                                view,
                                cx,
                            ))
                            .child(div().text_size(px(11.)).text_color(p.faint).child(at)),
                    ),
            );
        }
    }
    body
}

/// The photos added to the reply, each with its ✕, and a spinner while one uploads.
fn photos(mut body: Div, part: &Part<'_>, reply: &Reply) -> Div {
    let (p, task) = (part.p, part.task.id);
    if !reply.files.is_empty() || reply.uploading > 0 {
        let mut photos = div().flex().flex_wrap().gap(px(6.)).pb(px(6.));
        for file in &reply.files {
            let (store, url) = (part.store.clone(), file.url.clone());
            photos = photos.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .px(px(8.))
                    .py(px(4.))
                    .rounded(px(8.))
                    .border_1()
                    .border_color(p.border)
                    .text_size(px(12.))
                    .child(glyph(IconName::Image, p.muted, 13.))
                    .child(file.name.clone())
                    .child(
                        div()
                            .id(ElementId::Name(format!("remove-{}", file.url).into()))
                            .cursor_pointer()
                            .child(glyph(IconName::X, p.muted, 12.))
                            .on_click(move |_, _, cx| {
                                store.update(cx, |store, cx| {
                                    store.remove_task_photo(task, &url, cx);
                                });
                            }),
                    ),
            );
        }
        if reply.uploading > 0 {
            photos = photos.child(Spinner::new().small().color(p.muted));
        }
        body = body.child(photos);
    }
    body
}

/// Who a step of the history is from: the user, or the assistant (and its workers).
pub(super) fn who(p: &crate::ui::palette::Pal, mine: bool) -> Div {
    div()
        .w(px(44.))
        .flex_none()
        .text_size(px(11.5))
        .text_color(if mine { p.accent } else { p.muted })
        .child(if mine { "You" } else { crate::brand::APP_NAME })
}

/// Asks for photos to add to `task`'s reply box.
fn pick_photos(store: &Entity<Store>, task: TaskId, cx: &mut App) {
    let picked = cx.prompt_for_paths(PathPromptOptions {
        files: true,
        directories: false,
        multiple: true,
        prompt: Some("Add".into()),
    });
    let store = store.clone();
    cx.spawn(async move |cx| {
        let paths: Vec<PathBuf> = match picked.await {
            Ok(Ok(Some(paths))) => paths.into_iter().filter(|path| is_picture(path)).collect(),
            Ok(Ok(None)) => return,
            Ok(Err(error)) => {
                log::warn!("the file picker failed: {error}");
                return;
            }
            Err(error) => {
                log::warn!("the file picker went away: {error}");
                return;
            }
        };
        store.update(cx, |store, cx| store.add_task_photos(task, paths, cx));
    })
    .detach();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_pictures_go_in_a_reply() {
        assert!(is_picture(Path::new("/tmp/shot.PNG")));
        assert!(is_picture(Path::new("photo.heic")));
        assert!(!is_picture(Path::new("notes.txt")));
        assert!(!is_picture(Path::new("no-extension")));
    }
}
