//! A message's text as the chat draws it, from what was read of it once ([`Parsed`]):
//! markdown with its task keys and bare addresses linked (a click on a task opens its page, a
//! server path opens here, a web address in the browser), each quote with its own Copy,
//! tables as a grid or as cards when the column is too narrow, and pictures alone on a line:
//! one on the server in place, one on the web as a link (never fetched by itself). A message
//! too long to draw whole ends with "Show all", which shows all of it as plain text. Each
//! paragraph, list item, heading and quote reads the way its first letter does: Arabic from
//! the right edge.

use std::rc::Rc;
use std::sync::Arc;

use emberhut_api::ids::MessageId;
use gpui_kit::TestSupportExt as _;
use gpui_kit::assets::IconName as AssetIcon;
use gpui_kit::component::Icon;
use gpui_kit::component::text::TextView;
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, Div, ElementId, Entity, FontWeight, ImageSource, SharedString, Window, div,
    img, px,
};

use super::column;
use super::palette::Pal;
use super::viewer::OpenInViewer;
use crate::config::ServerUrl;
use crate::prose::direction::{self, Direction, Part, Run};
use crate::prose::links::LinkTarget;
use crate::prose::parsed::{Drawn, Parsed};
use crate::prose::tables::{Layout, Table};
use crate::store::files::RemoteFile;
use crate::store::{OpenReport, Page, Store};

/// The chat's text size, in pixels.
const TEXT: f32 = 14.;

/// What a quote's Copy does: copy that quote (its place among the message's quotes).
pub(crate) type OnCopy = Rc<dyn Fn(usize, &mut Window, &mut App)>;

/// What "Show all" does: show the whole message as plain text.
pub(crate) type OnShowAll = Rc<dyn Fn(&mut Window, &mut App)>;

/// Where a message's text is drawn.
#[derive(Clone)]
pub(crate) struct Place {
    /// The message (its elements' ids start with it).
    pub(crate) message: MessageId,
    /// How wide its text may be, in pixels (a table lays itself out in it).
    pub(crate) width: f32,
    /// The quote whose Copy was just pressed ("Copied" for a moment).
    pub(crate) copied: Option<usize>,
    /// What Copy on a quote does.
    pub(crate) on_copy: OnCopy,
    /// All of it is shown, as plain text ("Show all" was pressed).
    pub(crate) whole: bool,
    /// What "Show all" does.
    pub(crate) on_show_all: OnShowAll,
}

/// The assistant's text: its blocks, then "Show all" when some of it is left out.
pub(crate) fn answer(
    parsed: &Parsed,
    place: &Place,
    store: &Entity<Store>,
    p: &Pal,
    cx: &App,
) -> Div {
    let column = div().flex().flex_col().gap(px(8.));
    if let Some(whole) = parsed.whole.as_ref().filter(|_| place.whole) {
        return column.child(plain(whole));
    }
    let mut quotes = 0;
    let parts = parsed
        .blocks
        .iter()
        .enumerate()
        .map(|(at, block)| match block {
            Drawn::Markdown(markdown) => {
                markdown_view(id(place, at, "md"), Arc::clone(markdown), store).into_any_element()
            }
            Drawn::Directed(runs) => directed(place, at, runs, store, p),
            Drawn::Quote {
                shown, direction, ..
            } => {
                quotes += 1;
                let parts = quote_parts(place, at, quotes - 1);
                quote_view(parts, shown, *direction, store, p).into_any_element()
            }
            Drawn::Table(table) => table_view(place, at, table, store, p).into_any_element(),
            Drawn::Picture { url, alt } => {
                picture(place, at, url, alt, store, p, cx).into_any_element()
            }
        });
    column.children(parts).children(show_all(parsed, place, p))
}

/// The user's words: inline markdown and links, in the bubble's colours, from the right edge
/// when they start in Arabic, then "Show all" when some of them are left out.
pub(crate) fn mine(parsed: &Parsed, place: &Place, store: &Entity<Store>, p: &Pal) -> Div {
    let column = div().flex().flex_col().gap(px(6.));
    if let Some(whole) = parsed.whole.as_ref().filter(|_| place.whole) {
        return column.child(plain(whole));
    }
    let words = parsed.blocks.iter().find_map(|block| match block {
        Drawn::Markdown(markdown) => Some(markdown_view(
            ElementId::Name(format!("mine-{}", place.message).into()),
            Arc::clone(markdown),
            store,
        )),
        Drawn::Directed(_) | Drawn::Quote { .. } | Drawn::Table(_) | Drawn::Picture { .. } => None,
    });
    column
        .when(parsed.direction.is_rtl(), Styled::text_right)
        .children(words)
        .children(show_all(parsed, place, p))
}

/// All of a message as plain text, up to its bound.
fn plain(whole: &Arc<str>) -> Div {
    div()
        .text_size(px(TEXT))
        .child(SharedString::from(Arc::clone(whole)))
}

/// "Show all", when some of the message is left out.
fn show_all(parsed: &Parsed, place: &Place, p: &Pal) -> Option<impl IntoElement> {
    parsed.whole.as_ref()?;
    let on_show_all = Rc::clone(&place.on_show_all);
    Some(
        div()
            .id(ElementId::Name(
                format!("show-all-{}", place.message).into(),
            ))
            // Seen by the headless tests (a no-op outside them).
            .test_support()
            .cursor_pointer()
            .text_size(px(12.5))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(p.accent)
            .child("Show all")
            .on_click(move |_, window, cx| on_show_all(window, cx)),
    )
}

/// A block of markdown, each of its runs read its own way: one view for a run that reads from
/// the left (all of it, most of the time), and each paragraph, heading and list item of a run
/// that reads from the right laid out from the right edge, a list item's marker at its right.
fn directed(place: &Place, at: usize, runs: &[Run], store: &Entity<Store>, p: &Pal) -> AnyElement {
    let runs = runs.iter().enumerate().map(|(r, run)| {
        let id =
            |part: usize| ElementId::Name(format!("md-{}-{at}-{r}-{part}", place.message).into());
        match run {
            Run::LeftToRight(text) => {
                markdown_view(id(0), Arc::clone(text), store).into_any_element()
            }
            Run::RightToLeft(parts) => div()
                .w_full()
                .flex()
                .flex_col()
                .gap(px(6.))
                .children(
                    parts
                        .iter()
                        .enumerate()
                        .map(|(at, part)| right_to_left(id(at), part, store, p)),
                )
                .into_any_element(),
        }
    });
    div()
        .flex()
        .flex_col()
        .gap(px(8.))
        .children(runs)
        .into_any_element()
}

/// A paragraph, heading or list item that reads from the right: its words against the right
/// edge, a list item's marker at their right and its nesting as space on the right.
fn right_to_left(id: ElementId, part: &Part, store: &Entity<Store>, p: &Pal) -> Div {
    let words = div().flex_1().min_w_0().text_right().child(markdown_view(
        id,
        Arc::clone(&part.markdown),
        store,
    ));
    let Some(marker) = &part.marker else {
        return div().w_full().flex().child(words);
    };
    #[expect(
        clippy::cast_precision_loss,
        reason = "a list's nesting is a handful of levels"
    )]
    let indent = 16. * part.depth as f32;
    div()
        .w_full()
        .flex()
        .flex_row_reverse()
        .items_start()
        .gap(px(6.))
        .pr(px(indent))
        .child(
            div()
                .flex_none()
                .text_size(px(TEXT))
                .text_color(p.muted)
                .child(direction::right_to_left(marker)),
        )
        .child(words)
}

/// Linked markdown, whose links open as the app's links do.
pub(crate) fn markdown_view(
    id: ElementId,
    linked: impl Into<SharedString>,
    store: &Entity<Store>,
) -> TextView {
    let store = store.clone();
    TextView::markdown(id, linked)
        .text_size(px(TEXT))
        .on_link_click(move |url, _, window, cx| follow_link(&store, url, window, cx))
}

/// A link clicked in a message, as [`LinkTarget`] says: a task's opens its page in `window`'s
/// column, over what it shows; anything else goes as [`follow_outside_column`] says.
pub(crate) fn follow_link(store: &Entity<Store>, url: &str, window: &mut Window, cx: &mut App) {
    if let Some(task) = follow_outside_column(store, url, window, cx) {
        column::push(Page::Task(task), window, cx);
    }
}

/// A link clicked in a message, but for a task's: another app link opens its place (a page, a
/// room, a window of its own); an HTML report on the server opens beside the page, another
/// file on the server in the viewer of `window`, as a file chip's does; an email address opens
/// in the mail app, a web address in the browser. Anything else stays shut. A task's link is
/// left to the caller: its task.
#[must_use]
pub(crate) fn follow_outside_column(
    store: &Entity<Store>,
    url: &str,
    window: &mut Window,
    cx: &mut App,
) -> Option<emberhut_api::ids::TaskId> {
    match LinkTarget::of(url) {
        LinkTarget::Task(task) => return Some(task),
        LinkTarget::Place => crate::shell::open_link(url, cx),
        LinkTarget::Mail => cx.open_url(url),
        LinkTarget::Report { name } => {
            let report = OpenReport {
                name,
                path: url.to_owned(),
            };
            store.update(cx, |store, cx| store.show_report(Some(report), cx));
        }
        LinkTarget::File { name } => {
            let file = RemoteFile::new(url, name);
            window.dispatch_action(Box::new(OpenInViewer { file }), cx);
        }
        LinkTarget::Web => {
            if let Some(link) = store.read(cx).server().link(url) {
                cx.open_url(&link);
            }
        }
        LinkTarget::Refused => log::warn!("not following a link from a message: {url}"),
    }
    None
}

/// A link clicked in a window of its own that has no column or report panel to open the app's
/// places in (Party, the Dark room): an email address opens in the mail app, a web address (or
/// a path on the server) in the browser, as [`ServerUrl::link`] lets it. Anything else (a
/// file, another app's scheme, the app's own) stays shut, whatever the text that held it says.
pub(crate) fn follow_web_link(store: &Entity<Store>, url: &str, cx: &mut App) {
    if LinkTarget::of(url) == LinkTarget::Mail {
        cx.open_url(url);
    } else if let Some(link) = store.read(cx).server().link(url) {
        cx.open_url(&link);
    }
}

fn id(place: &Place, at: usize, kind: &str) -> ElementId {
    ElementId::Name(format!("{kind}-{}-{at}", place.message).into())
}

/// Something a click does.
type Action = Box<dyn Fn(&mut Window, &mut App)>;

/// A quote's place: its ids, whether it was just copied, and what its Copy does.
struct QuoteParts {
    id: ElementId,
    copy_id: ElementId,
    copied: bool,
    on_copy: Action,
}

fn quote_parts(place: &Place, at: usize, index: usize) -> QuoteParts {
    let on_copy = Rc::clone(&place.on_copy);
    QuoteParts {
        id: id(place, at, "quote"),
        copy_id: id(place, at, "quote-copy"),
        copied: place.copied == Some(index),
        on_copy: Box::new(move |window, cx| on_copy(index, window, cx)),
    }
}

/// A quote: a draft the user may send as their own, with a bar, and a Copy that copies its
/// words only.
fn quote_view(
    parts: QuoteParts,
    quote: &Arc<str>,
    direction: Direction,
    store: &Entity<Store>,
    p: &Pal,
) -> Div {
    let QuoteParts {
        id,
        copy_id,
        copied,
        on_copy,
    } = parts;
    // A quote in Arabic has its bar and its Copy on the right.
    let rtl = direction.is_rtl();
    div()
        .flex()
        .flex_col()
        .gap(px(6.))
        .child(
            div()
                .when(rtl, |d| d.pr(px(10.)).border_r_2().text_right())
                .when(!rtl, |d| d.pl(px(10.)).border_l_2())
                .border_color(p.faint)
                .text_color(p.muted)
                .child(markdown_view(id, Arc::clone(quote), store)),
        )
        .child(
            div()
                .flex()
                .when(rtl, |d| d.justify_end().pr(px(10.)))
                .when(!rtl, |d| d.pl(px(10.)))
                .child(
                    div()
                        .id(copy_id)
                        .flex()
                        .items_center()
                        .gap(px(5.))
                        .px(px(9.))
                        .py(px(3.))
                        .rounded(px(8.))
                        .border_1()
                        .border_color(p.border)
                        .cursor_pointer()
                        .hover(|d| d.bg(p.hover))
                        .text_size(px(12.))
                        .text_color(if copied { p.success } else { p.text })
                        .child(if copied { "Copied" } else { "Copy" })
                        .on_click(move |_, window, cx| on_copy(window, cx)),
                ),
        )
}

/// A table: a grid while its columns have room to read, else a card per row; then how many
/// rows are left out.
fn table_view(place: &Place, at: usize, table: &Table, store: &Entity<Store>, p: &Pal) -> Div {
    let cell = |row: usize, column: usize, text: &str| {
        markdown_view(
            ElementId::Name(format!("cell-{}-{at}-{row}-{column}", place.message).into()),
            text.to_owned(),
            store,
        )
    };
    let rows = &table.rows;
    let drawn =
        match table.layout(place.width) {
            Layout::Grid(widths) => {
                let lines =
                    rows.iter().enumerate().map(|(r, row)| {
                        div()
                            .flex()
                            .when(r + 1 < rows.len(), |d| {
                                d.border_b_1().border_color(p.border)
                            })
                            .when(r == 0, |d| {
                                d.font_weight(FontWeight::SEMIBOLD).text_color(p.muted)
                            })
                            .children(row.iter().zip(&widths).enumerate().map(
                                |(c, (text, &width))| {
                                    div()
                                        .w(px(width))
                                        .flex_none()
                                        .px(px(8.))
                                        .py(px(5.))
                                        .child(cell(r, c, text))
                                },
                            ))
                    });
                div().flex().flex_col().children(lines)
            }
            Layout::Cards => {
                let cards = table.cards.iter().enumerate().map(|(r, card)| {
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(4.))
                        .py(px(6.))
                        .when(r > 0, |d| d.border_t_1().border_color(p.border))
                        .children(card.title.as_ref().map(|title| {
                            div()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(cell(r, 0, title))
                        }))
                        .children(card.lines.iter().enumerate().map(|(c, (label, value))| {
                            div()
                                .flex()
                                .gap(px(4.))
                                .children(label.as_ref().map(|label| {
                                    div()
                                        .flex_none()
                                        .text_color(p.muted)
                                        .text_size(px(TEXT))
                                        .child(format!("{label}:"))
                                }))
                                .child(div().flex_1().min_w_0().child(cell(r, c + 1, value)))
                        }))
                });
                div().flex().flex_col().children(cards)
            }
        };
    drawn.when(table.more_rows > 0, |d| {
        let rows = if table.more_rows == 1 { "row" } else { "rows" };
        d.child(
            div()
                .px(px(8.))
                .py(px(5.))
                .text_size(px(12.5))
                .text_color(p.muted)
                .child(format!("… {} more {rows}", table.more_rows)),
        )
    })
}

/// How a picture alone on its line is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum InlinePicture {
    /// On the server: fetched with this device's sign-in and drawn in place.
    InPlace,
    /// Anywhere else: a link to open in the browser; nothing is fetched by itself, so a
    /// message can't make the app call any address (a tracking pixel, a machine on the LAN).
    Link,
}

impl InlinePicture {
    /// How the picture at `url` is drawn.
    #[must_use]
    pub(crate) fn of(url: &str) -> Self {
        if ServerUrl::is_path(url) {
            Self::InPlace
        } else {
            Self::Link
        }
    }
}

/// A picture alone on its line: one on the server in place (opening in its default app on a
/// click), any other as a link to it.
fn picture(
    place: &Place,
    at: usize,
    url: &str,
    alt: &str,
    store: &Entity<Store>,
    p: &Pal,
    cx: &App,
) -> impl IntoElement {
    let shown =
        (InlinePicture::of(url) == InlinePicture::InPlace).then(|| store.read(cx).picture(url));
    let (link, store) = (url.to_owned(), store.clone());
    let follow = move |_: &gpui_kit::ClickEvent, window: &mut Window, cx: &mut App| {
        follow_link(&store, &link, window, cx);
    };
    let Some(fetched) = shown else {
        let words = if alt.is_empty() { url } else { alt };
        return div()
            .id(id(place, at, "picture-link"))
            .flex()
            .items_center()
            .gap(px(6.))
            .max_w_full()
            .cursor_pointer()
            .text_size(px(13.))
            .text_color(p.accent)
            .child(Icon::new(AssetIcon::Link).size(px(13.)))
            .child(div().truncate().child(words.to_owned()))
            .on_click(follow);
    };
    let shown: AnyElement = match fetched {
        Some(image) => img(ImageSource::Render(image))
            .max_w(px(320.))
            .max_h(px(240.))
            .rounded(px(10.))
            .into_any_element(),
        None => div()
            .w(px(240.))
            .h(px(140.))
            .rounded(px(10.))
            .bg(p.card)
            .into_any_element(),
    };
    div()
        .id(id(place, at, "picture"))
        .cursor_pointer()
        .child(shown)
        .when(!alt.is_empty(), |d| {
            d.child(
                div()
                    .pt(px(3.))
                    .text_size(px(11.5))
                    .text_color(p.muted)
                    .child(alt.to_owned()),
            )
        })
        .on_click(follow)
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::sync::Arc;

    use emberhut_api::ids::TaskId;
    use gpui_kit::TestAppContext;

    use super::*;
    use crate::backend::fake::{Call, FakeBackend};
    use crate::wire::TASK_LINK;

    #[gpui_kit::test]
    fn a_link_in_a_window_of_its_own_opens_only_on_the_web_or_in_mail(cx: &mut TestAppContext) {
        let backend = Arc::new(FakeBackend::new([]));
        let store =
            cx.new(|_| Store::for_tests(Arc::clone(&backend) as Arc<dyn crate::backend::Backend>));
        let follow = |url: &str, cx: &mut TestAppContext| {
            cx.update(|cx| follow_web_link(&store, url, cx));
            cx.run_until_parked();
        };
        for refused in [
            "file:///etc/passwd",
            "smb://example.com/x",
            "x-app://open",
            "emberhut://settings",
            &format!("{TASK_LINK}12"),
        ] {
            follow(refused, cx);
            assert_eq!(cx.opened_url(), None, "{refused}");
        }
        follow("mailto:me@example.com", cx);
        assert_eq!(cx.opened_url().as_deref(), Some("mailto:me@example.com"));
        follow("https://example.com/plan", cx);
        assert_eq!(cx.opened_url().as_deref(), Some("https://example.com/plan"));
        assert!(backend.calls().is_empty(), "nothing fetched");
    }

    #[gpui_kit::test]
    fn a_link_in_a_message_goes_where_its_kind_goes(cx: &mut TestAppContext) {
        let backend = Arc::new(FakeBackend::new([]));
        let shared = Arc::clone(&backend) as Arc<dyn crate::backend::Backend>;
        let store = cx.new(|_| Store::for_tests(shared));
        let viewed = Rc::new(RefCell::new(Vec::<RemoteFile>::new()));
        let noted = Rc::clone(&viewed);
        cx.update(|cx| {
            cx.on_action(move |view: &OpenInViewer, _| noted.borrow_mut().push(view.file.clone()));
        });
        let window = cx.add_empty_window();
        let follow = |url: &str, window: &mut gpui_kit::VisualTestContext| {
            let task = window.update(|window, cx| follow_outside_column(&store, url, window, cx));
            window.run_until_parked();
            task
        };
        assert_eq!(
            follow(&format!("{TASK_LINK}12"), window),
            Some(TaskId::new(12)),
            "left to the column"
        );
        follow("mailto:me@example.com", window);
        assert_eq!(
            window.opened_url().as_deref(),
            Some("mailto:me@example.com")
        );
        follow("https://example.com/plan", window);
        assert_eq!(
            window.opened_url().as_deref(),
            Some("https://example.com/plan")
        );
        for refused in ["file:///etc/passwd", "smb://example.com/x", "x-app://open"] {
            follow(refused, window);
            assert_eq!(
                window.opened_url().as_deref(),
                Some("https://example.com/plan"),
                "{refused}"
            );
        }
        follow("/reports/week.html", window);
        store.read_with(window, |store, _| {
            assert_eq!(
                store.open_report().map(|report| report.name.as_str()),
                Some("week.html")
            );
        });
        follow("/uploads/x.command", window);
        assert_eq!(
            *viewed.borrow(),
            [RemoteFile::new("/uploads/x.command", "x.command")],
            "a file on the server goes to the viewer, as a file chip's does"
        );
        assert!(
            !backend
                .calls()
                .iter()
                .any(|call| matches!(call, Call::Download(_))),
            "nothing fetched to open elsewhere: {:?}",
            backend.calls()
        );
    }

    #[test]
    fn only_a_picture_on_the_server_is_drawn_in_place() {
        assert_eq!(
            InlinePicture::of("/uploads/chart.png"),
            InlinePicture::InPlace
        );
        for elsewhere in [
            "https://example.com/pixel.gif",
            "http://intranet.example/x.png",
            "file:///etc/x.png",
            "//example.com/x.png",
            "chart.png",
        ] {
            assert_eq!(
                InlinePicture::of(elsewhere),
                InlinePicture::Link,
                "{elsewhere}"
            );
        }
    }
}
