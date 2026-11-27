//! A message's menu (right-click on its bubble): Reply, Copy (the whole message, ready to paste
//! into a chat app), Copy Link for each web link in it (up to three, named by their host when
//! there are several), and on an answer, Explain simply. The items are worked out from the
//! message's text when the menu opens, not as the chat is drawn. Beside the message under the
//! pointer a Reply button shows.

use std::rc::Rc;

use gpui_kit::assets::IconName as AssetIcon;
use gpui_kit::component::Icon;
use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::prelude::*;
use gpui_kit::{
    App, ClipboardItem, Div, ElementId, Entity, SharedString, Stateful, Window, div, px,
};

use super::palette::Pal;
use crate::prose::copy;
use crate::store::Store;
use crate::ui::a11y::Accessible as _;

/// What Reply does for one message.
pub(crate) type OnReply = Rc<dyn Fn(&mut Window, &mut App)>;

/// One item of the menu, as data (the view turns it into a menu item).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Item {
    /// Quote it above the composer.
    Reply,
    /// Copy the whole message, ready to paste.
    Copy(String),
    /// Copy one of its links: the item's name and what it copies.
    CopyLink(String, String),
    /// Ask the assistant to say it again in simpler words: what is sent.
    ExplainSimply(String),
}

/// The menu's items for a message saying `text` (trimmed); `mine` when it is the user's.
#[must_use]
pub(crate) fn items(text: &str, mine: bool) -> Vec<Item> {
    let mut items = vec![Item::Reply];
    if !text.is_empty() {
        items.push(Item::Copy(copy::for_pasting(text)));
        let links = copy::web_links(text);
        let shown = links.len().min(copy::MENU_LINKS);
        items.extend(
            links
                .iter()
                .take(copy::MENU_LINKS)
                .map(|link| Item::CopyLink(copy::link_item(link, shown), link.copied.clone())),
        );
        if !mine {
            items.push(Item::ExplainSimply(copy::explain_simply(text)));
        }
    }
    items
}

/// The menu for a message saying `text`, as the context menu builds it when it opens.
pub(crate) fn menu(
    text: Rc<str>,
    mine: bool,
    store: &Entity<Store>,
    on_reply: OnReply,
) -> impl Fn(PopupMenu, &mut Window, &mut Context<'_, PopupMenu>) -> PopupMenu + 'static {
    let store = store.clone();
    move |menu, _, _| {
        items(&text, mine).into_iter().fold(menu, |menu, item| {
            let entry = match item {
                Item::Reply => {
                    let on_reply = Rc::clone(&on_reply);
                    PopupMenuItem::new("Reply")
                        .icon(Icon::new(AssetIcon::Reply))
                        .on_click(move |_, window, cx| on_reply(window, cx))
                }
                Item::Copy(text) => PopupMenuItem::new("Copy")
                    .icon(Icon::new(AssetIcon::Copy))
                    .on_click(move |_, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
                    }),
                Item::CopyLink(name, link) => PopupMenuItem::new(name)
                    .icon(Icon::new(AssetIcon::Link))
                    .on_click(move |_, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(link.clone()));
                    }),
                Item::ExplainSimply(asked) => {
                    let store = store.clone();
                    PopupMenuItem::new("Explain simply")
                        .icon(Icon::new(AssetIcon::MessageSquareText))
                        .on_click(move |_, _, cx| {
                            store.update(cx, |store, cx| store.say(&asked, cx));
                        })
                }
            };
            menu.item(entry)
        })
    }
}

/// The Reply button beside a message: shown while the pointer is over the message's row (its
/// hover `group`), which takes a hover style, not a redraw of the chat.
pub(crate) fn reply_button(
    id: ElementId,
    group: SharedString,
    p: &Pal,
    on_reply: OnReply,
) -> Stateful<Div> {
    div()
        .id(id)
        .flex_none()
        .size(px(26.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(6.))
        .opacity(0.)
        .group_hover(group, |style| style.opacity(1.))
        .button_named("Reply")
        .cursor_pointer()
        .hover(|d| d.bg(p.hover))
        .child(
            Icon::new(AssetIcon::Reply)
                .size(px(15.))
                .text_color(p.muted),
        )
        .on_click(move |_, window, cx| on_reply(window, cx))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_answers_menu_copies_it_its_links_and_explains_it() {
        let items = items(
            "The **plan** is in [the doc](https://example.com/doc) and on www.example.org.",
            false,
        );
        assert_eq!(items[0], Item::Reply);
        assert_eq!(
            items[1],
            Item::Copy(
                "The *plan* is in the doc (https://example.com/doc) and on www.example.org.".into()
            )
        );
        assert_eq!(
            items[2],
            Item::CopyLink("Copy example.com".into(), "https://example.com/doc".into())
        );
        assert_eq!(
            items[3],
            Item::CopyLink(
                "Copy www.example.org".into(),
                "https://www.example.org".into()
            )
        );
        assert!(
            matches!(&items[4], Item::ExplainSimply(asked) if asked.contains("The **plan** is in the doc and on"))
        );
    }

    #[test]
    fn the_users_own_message_has_no_explain_and_one_link_is_copy_link() {
        assert_eq!(
            items("see https://example.com/a", true),
            [
                Item::Reply,
                Item::Copy("see https://example.com/a".into()),
                Item::CopyLink("Copy Link".into(), "https://example.com/a".into()),
            ]
        );
        assert_eq!(items("", false), [Item::Reply], "a photo alone");
    }

    #[test]
    fn at_most_three_links_are_offered() {
        let many = "https://a.example.com https://b.example.com https://c.example.com https://d.example.com";
        let links = items(many, true)
            .into_iter()
            .filter(|item| matches!(item, Item::CopyLink(..)))
            .count();
        assert_eq!(links, 3);
    }
}
