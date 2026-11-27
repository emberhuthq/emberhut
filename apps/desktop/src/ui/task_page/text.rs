//! Long text on a task's page: folded to a few lines with Show more and Show less, `Read full`
//! to read all of it (selectable) over the page, and task keys and links in it made clickable
//! as in a message's text: a key opens that task's page in the column, any other link goes
//! through the app's link rules.

use gpui_kit::component::text::TextView;
use gpui_kit::prelude::*;
use gpui_kit::{App, Div, ElementId, Entity, FontWeight, SharedString, Window, div, px};

use super::TaskView;
use crate::prose::direction;
use crate::prose::links;
use crate::store::Store;
use crate::ui::palette::Pal;
use crate::ui::prose;

/// Characters a line holds, roughly, for telling whether a text is long.
const LINE_CHARS: usize = 55;

/// Whether `text` is longer than `lines` lines.
#[must_use]
pub(super) fn is_long(text: &str, lines: usize) -> bool {
    text.chars().count() > lines * LINE_CHARS || text.split('\n').count() > lines
}

/// A link clicked in the page's text, the way a link in a message goes
/// ([`prose::follow_link`]): a task's opens that task's page over this one in the column (Back
/// returns here), and only the other links the app lets out open.
pub(super) fn follow(store: &Entity<Store>, url: &str, window: &mut Window, cx: &mut App) {
    prose::follow_link(store, url, window, cx);
}

/// `text` as the page draws it: markdown, selectable, keys and links clickable, at most
/// `lines` lines unless `open`.
pub(super) fn rich(
    id: impl Into<ElementId>,
    text: &str,
    lines: Option<usize>,
    store: &Entity<Store>,
) -> TextView {
    let store = store.clone();
    let view = TextView::markdown(id, links::linked(text))
        .selectable(true)
        .on_link_click(move |url, _, window, cx| follow(&store, url, window, cx));
    match lines {
        Some(lines) => view.max_lines(lines),
        None => view,
    }
}

/// A long text on the page: its key among the page's texts (which are open is the page's to
/// keep), the text, how many lines it folds to, and the title `Read full` shows it under.
#[derive(Clone, Copy, Debug)]
pub(super) struct Long<'a> {
    /// Its key.
    pub(super) key: &'a str,
    /// The text.
    pub(super) text: &'a str,
    /// The lines it folds to.
    pub(super) lines: usize,
    /// Given, a long one also opens whole (`Read full`).
    pub(super) title: Option<&'a str>,
}

/// Long text kept to its lines, with Show more / Show less under it and, given a title, Read
/// full.
pub(super) fn collapsible(
    p: &Pal,
    store: &Entity<Store>,
    long: Long<'_>,
    view: &TaskView,
    cx: &Context<'_, TaskView>,
) -> Div {
    let Long {
        key,
        text,
        lines,
        title,
    } = long;
    let folds = is_long(text, lines);
    let open = view.is_open(key);
    let mut column = div().flex().flex_col().gap(px(4.)).child(
        div()
            .text_size(px(13.))
            // Mostly Arabic, it starts at the right edge.
            .when(direction::of_most_letters(text).is_rtl(), |d| {
                d.text_right()
            })
            .child(rich(
                ElementId::Name(format!("text-{key}").into()),
                text,
                (folds && !open).then_some(lines),
                store,
            )),
    );
    if folds {
        let toggle_key = key.to_owned();
        let mut links = div().flex().gap(px(16.)).child(
            link_button(
                p,
                ElementId::Name(format!("more-{key}").into()),
                if open { "Show less" } else { "Show more" },
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                this.toggle_open(&toggle_key);
                cx.notify();
            })),
        );
        if let Some(title) = title {
            let (title, text) = (title.to_owned(), text.to_owned());
            links = links.child(
                link_button(
                    p,
                    ElementId::Name(format!("read-{key}").into()),
                    "Read full",
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.read_full(title.clone(), text.clone());
                    cx.notify();
                })),
            );
        }
        column = column.child(links);
    }
    column
}

/// A small link-styled button.
pub(super) fn link_button(
    p: &Pal,
    id: ElementId,
    label: impl Into<SharedString>,
) -> gpui_kit::Stateful<Div> {
    div()
        .id(id)
        .text_size(px(12.5))
        .font_weight(FontWeight::MEDIUM)
        .text_color(p.accent)
        .cursor_pointer()
        .child(label.into())
}

/// The whole of a long text over the page, selectable, with Close.
pub(super) fn read_full_sheet(
    p: &Pal,
    store: &Entity<Store>,
    title: &str,
    text: &str,
    cx: &Context<'_, TaskView>,
) -> Div {
    div()
        .absolute()
        .inset_0()
        .flex()
        .flex_col()
        .bg(p.surface)
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .h(px(44.))
                .px(px(14.))
                .border_b_1()
                .border_color(p.border)
                .child(
                    div()
                        .flex_1()
                        .truncate()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_size(px(13.))
                        .child(title.to_owned()),
                )
                .child(
                    link_button(p, "read-full-close".into(), "Close").on_click(cx.listener(
                        |this, _, _, cx| {
                            this.close_reading();
                            cx.notify();
                        },
                    )),
                ),
        )
        .child(
            div()
                .id("read-full")
                .flex_1()
                .overflow_y_scroll()
                .p(px(20.))
                .text_size(px(14.))
                .child(rich("read-full-text", text, None, store)),
        )
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::sync::Arc;

    use emberhut_api::ids::TaskId;
    use gpui_kit::{TestAppContext, VisualTestContext};

    use super::*;
    use crate::backend::fake::FakeBackend;
    use crate::store::files::RemoteFile;
    use crate::ui::viewer::OpenInViewer;
    use crate::wire::{TASK_KEY_PREFIX, TASK_LINK};

    #[test]
    fn only_task_keys_become_task_links() {
        let text = format!(
            "UTF-8, SHA-256, ISO-9001 and COVID-19 stay; {TASK_KEY_PREFIX}12 opens, \
             `{TASK_KEY_PREFIX}3` stays."
        );
        assert_eq!(
            links::linked(&text),
            format!(
                "UTF-8, SHA-256, ISO-9001 and COVID-19 stay; [{TASK_KEY_PREFIX}12]({TASK_LINK}12) \
                 opens, `{TASK_KEY_PREFIX}3` stays."
            )
        );
    }

    /// Follows `url` as the page's text does; a task's link is left to the column: its task.
    fn click(store: &Entity<Store>, url: &str, cx: &mut VisualTestContext) -> Option<TaskId> {
        let task = cx.update(|window, cx| prose::follow_outside_column(store, url, window, cx));
        cx.run_until_parked();
        task
    }

    #[gpui_kit::test]
    fn a_link_opens_only_as_the_app_lets_it(cx: &mut TestAppContext) {
        let store = cx.new(|_| Store::for_tests(Arc::new(FakeBackend::new([]))));
        let viewed = Rc::new(RefCell::new(Vec::<RemoteFile>::new()));
        let noted = Rc::clone(&viewed);
        cx.update(|cx| {
            cx.on_action(move |view: &OpenInViewer, _| noted.borrow_mut().push(view.file.clone()));
        });
        let cx = cx.add_empty_window();
        for shut in [
            "file:///etc/hosts",
            "smb://example.com/share",
            "javascript:alert(1)",
            "example-app://open",
        ] {
            click(&store, shut, cx);
            assert_eq!(cx.opened_url(), None, "{shut}");
        }

        click(&store, "/api/artifacts/7/report.html", cx);
        click(&store, "/api/artifacts/8/notes.pdf", cx);
        assert_eq!(
            cx.opened_url(),
            None,
            "a server path never goes to the system"
        );
        cx.read(|cx| {
            let store = store.read(cx);
            assert_eq!(
                store.open_report().map(|report| report.path.as_str()),
                Some("/api/artifacts/7/report.html")
            );
        });
        assert_eq!(
            *viewed.borrow(),
            [RemoteFile::new("/api/artifacts/8/notes.pdf", "notes.pdf")],
            "a file opens in the viewer"
        );

        assert_eq!(
            [4, 9].map(|id| click(&store, &links::task_url(TaskId::new(id)), cx)),
            [Some(TaskId::new(4)), Some(TaskId::new(9))],
            "a task's link opens its page over this one in the column"
        );

        click(&store, "https://example.com/plan", cx);
        assert_eq!(cx.opened_url().as_deref(), Some("https://example.com/plan"));
    }

    #[test]
    fn a_text_is_long_past_its_lines() {
        assert!(!is_long("short", 4));
        assert!(is_long("a\nb\nc\nd\ne", 4));
        assert!(is_long(&"x".repeat(4 * LINE_CHARS + 1), 4));
    }
}
