//! An X post and the replies drafted for it: who posted it and how it does (a click opens the
//! post), what to check before posting, then each reply in full with why it works, `Reply on X`
//! (the reply on the clipboard and X's reply box open with it) and `Posted`; the one posted is
//! marked and the others fade. Full, it also shows what wins in that reply section. Compact, on
//! a Needs you card, the post is two lines.

use emberhut_api::ids::TaskId;
use emberhut_api::tasks::Artifact;
use emberhut_api::x_reply::{XReply, XReplyOption, XRoom};
use gpui_kit::assets::IconName;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::prelude::*;
use gpui_kit::{App, ClipboardItem, Div, ElementId, Entity, FontWeight, div, px};

use super::palette::Pal;
use super::tasks::marks::glyph;
use crate::store::Store;
use crate::store::x_posts;

/// Opens `url` (a post, a reply box, a top reply: the server's words) as the app opens any link
/// from the server ([`crate::config::ServerUrl::link`]): a web address or a server path only, so
/// a `file:`, `smb:` or another app's scheme never reaches the system.
fn open_link(store: &Entity<Store>, url: &str, cx: &mut App) {
    if let Some(link) = store.read(cx).server().link(url) {
        cx.open_url(&link);
    }
}

/// The post and its replies; `compact` on a card.
pub(crate) fn view<V: 'static>(
    p: &Pal,
    store: &Entity<Store>,
    artifact: &Artifact,
    task: TaskId,
    compact: bool,
    _: &Context<'_, V>,
) -> Div {
    let Some(post) = artifact.x.as_ref() else {
        return div();
    };
    let facts = x_posts::post_facts(post, jiff::Timestamp::now());
    let (address, opener) = (post.post_url.clone(), store.clone());
    let mut column = div().flex().flex_col().gap(px(10.)).child(
        div()
            .id(ElementId::Name(format!("x-post-{}", artifact.id).into()))
            .flex()
            .flex_col()
            .gap(px(3.))
            .cursor_pointer()
            .child(
                div()
                    .text_size(px(13.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .truncate()
                    .child(format!(
                        "Reply to @{}{}",
                        post.author.as_deref().unwrap_or("post"),
                        post.author_name
                            .as_deref()
                            .map_or_else(String::new, |name| format!(" · {name}"))
                    )),
            )
            .when(!facts.is_empty(), |d| {
                d.child(
                    div()
                        .text_size(px(11.5))
                        .text_color(p.faint)
                        .truncate()
                        .child(facts),
                )
            })
            .children(post.post_text.clone().map(|text| {
                div()
                    .text_size(px(13.))
                    .text_color(p.muted)
                    .line_clamp(if compact { 2 } else { 6 })
                    .child(text)
            }))
            .on_click(move |_, _, cx| {
                cx.stop_propagation();
                if !address.is_empty() {
                    open_link(&opener, &address, cx);
                }
            }),
    );
    if post.posted.is_none()
        && let Some(check) = post.check.as_deref()
    {
        column = column.child(
            div()
                .flex()
                .items_start()
                .gap(px(6.))
                .text_size(px(12.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(p.warn)
                .child(glyph(IconName::TriangleAlert, p.warn, 13.))
                .child(check.to_owned()),
        );
    }
    for (i, option) in post.replies.iter().enumerate() {
        column = column.child(reply(p, store, artifact, post, task, i, option));
    }
    if !compact
        && let Some(room) = post
            .room
            .as_ref()
            .filter(|room| !room.top_replies.is_empty())
    {
        column = column.child(winning(p, store, artifact, room));
    }
    column
}

/// One drafted reply: its label, the reply, why, and Reply on X / Posted (or that it was).
fn reply(
    p: &Pal,
    store: &Entity<Store>,
    artifact: &Artifact,
    post: &XReply,
    task: TaskId,
    index: usize,
    option: &XReplyOption,
) -> Div {
    let posted = post.posted.as_ref();
    let used = posted.is_some_and(|posted| usize::try_from(posted.option).ok() == Some(index));
    let mut card = div()
        .flex()
        .flex_col()
        .gap(px(8.))
        .p(px(12.))
        .rounded(px(12.))
        .bg(p.hover)
        .when(posted.is_some() && !used, |d| d.opacity(0.5));
    let label = [option.label.as_deref(), option.style.as_deref()]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ");
    if !label.is_empty() {
        card = card.child(
            div()
                .text_size(px(11.5))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(p.muted)
                .child(label),
        );
    }
    card = card.child(div().text_size(px(13.5)).child(option.text.clone()));
    if let Some(why) = &option.why {
        card = card.child(
            div()
                .text_size(px(12.))
                .text_color(p.muted)
                .child(why.clone()),
        );
    }
    if used {
        return card.child(
            div()
                .flex()
                .items_center()
                .gap(px(4.))
                .text_size(px(12.5))
                .font_weight(FontWeight::MEDIUM)
                .text_color(p.success)
                .child(glyph(IconName::CircleCheck, p.success, 14.))
                .child("Posted"),
        );
    }
    if posted.is_some() {
        return card;
    }
    let (text, url) = (option.text.clone(), x_posts::intent_url(post, option));
    let (notes, opener) = (store.clone(), store.clone());
    let (marks, artifact_id) = (store.clone(), artifact.id);
    let option_number = u32::try_from(index).unwrap_or(u32::MAX);
    card.child(
        div()
            .flex()
            .gap(px(8.))
            .child(
                Button::new(ElementId::Name(
                    format!("x-reply-{}-{index}-{task}", artifact.id).into(),
                ))
                .label("Reply on X")
                .icon(glyph(IconName::Reply, p.on_accent, 13.))
                .primary()
                .small()
                .tooltip("Copies the reply and opens X's reply box with it")
                .on_click(move |_, _, cx| {
                    cx.stop_propagation();
                    cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
                    open_link(&opener, &url, cx);
                    notes.update(cx, |store, cx| {
                        store.show_notice("Copied: paste it in X".to_owned(), cx);
                    });
                }),
            )
            .child(
                Button::new(ElementId::Name(
                    format!("x-posted-{}-{index}-{task}", artifact.id).into(),
                ))
                .label("Posted")
                .outline()
                .small()
                .on_click(move |_, _, cx| {
                    cx.stop_propagation();
                    marks.update(cx, |store, cx| {
                        store.mark_posted(artifact_id, option_number, cx);
                    });
                }),
            ),
    )
}

/// What wins in that reply section: its top replies, small, each opening on X.
fn winning(p: &Pal, store: &Entity<Store>, artifact: &Artifact, room: &XRoom) -> Div {
    let mut column = div().flex().flex_col().gap(px(6.)).child(
        div()
            .text_size(px(11.5))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(p.muted)
            .child(match &room.winners_pattern {
                Some(pattern) => format!("Winning there: {pattern}"),
                None => "Winning there".to_owned(),
            }),
    );
    for (i, top) in room.top_replies.iter().take(3).enumerate() {
        let (url, opener) = (top.url.clone(), store.clone());
        column = column.child(
            div()
                .id(ElementId::Name(format!("x-top-{}-{i}", artifact.id).into()))
                .text_size(px(11.5))
                .text_color(p.muted)
                .line_clamp(2)
                .cursor_pointer()
                .child(format!(
                    "@{}{}  {}",
                    top.author,
                    top.likes
                        .map_or_else(String::new, |likes| format!(" · {likes} likes")),
                    top.text
                ))
                .on_click(move |_, _, cx| {
                    if let Some(url) = &url {
                        open_link(&opener, url, cx);
                    }
                }),
        );
    }
    column
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use gpui_kit::TestAppContext;

    use super::*;
    use crate::backend::fake::FakeBackend;

    #[gpui_kit::test]
    fn only_web_links_from_a_post_reach_the_system(cx: &mut TestAppContext) {
        let store = cx.new(|_| Store::for_tests(Arc::new(FakeBackend::new([]))));
        for refused in [
            "file:///Applications/Calculator.app",
            "smb://example.com/share",
            "x-app://open",
        ] {
            cx.update(|cx| open_link(&store, refused, cx));
            assert_eq!(cx.opened_url(), None, "{refused}");
        }
        cx.update(|cx| open_link(&store, "https://x.com/example/status/1", cx));
        assert_eq!(
            cx.opened_url().as_deref(),
            Some("https://x.com/example/status/1")
        );
    }
}
