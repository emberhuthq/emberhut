//! HTML reports for the panel beside the page: fetched by path when one is opened, and kept as
//! text with the title read from the page.

use std::time::Instant;

use gpui_kit::AppContext as _;
use gpui_kit::{Context, SharedString};

use super::fetched::Fetched;
use super::{Store, update_or_log};

/// A fetched report: the page, and its own title.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ReportPage {
    /// The page's HTML, as the server sent it (invalid UTF-8 replaced).
    pub(crate) html: SharedString,
    /// The page's title (see [`title_of`]), if it has one.
    pub(crate) title: Option<SharedString>,
}

impl ReportPage {
    /// The report whose page is `html`.
    #[must_use]
    fn new(html: String) -> Self {
        let title = title_of(&html).map(SharedString::from);
        Self {
            html: html.into(),
            title,
        }
    }
}

impl Store {
    /// The report at `path`: loading until its fetch (started by [`Store::show_report`]) is
    /// back.
    #[must_use]
    pub(crate) fn report(&self, path: &str) -> Fetched<ReportPage> {
        self.reports.get(path).cloned().unwrap_or(Fetched::Loading)
    }

    /// Fetches the report at `path` unless it is here or on its way.
    pub(super) fn fetch_report(&mut self, path: String, cx: &mut Context<'_, Self>) {
        if !self.reports.begin(&path, Instant::now()) {
            return;
        }
        let bytes = self.file_bytes(&path, cx);
        cx.spawn(async move |this, cx| {
            // The title is read off the main thread too: a report can be megabytes.
            let outcome = match bytes.await {
                Ok(bytes) => {
                    cx.background_spawn(async move {
                        Ok(ReportPage::new(
                            String::from_utf8_lossy(&bytes).into_owned(),
                        ))
                    })
                    .await
                }
                Err(failure) => Err(failure),
            };
            update_or_log(&this, cx, |store, cx| {
                store.reports.finish(path, outcome, Instant::now());
                cx.notify();
            });
        })
        .detach();
    }
}

/// A page's title, as the reference client reads it: the text of its `<title>`, else of its first
/// `<h1>`, with tags dropped, the common entities decoded and white space collapsed; `None` when
/// it has neither or it is blank.
#[must_use]
pub(in crate::store) fn title_of(html: &str) -> Option<String> {
    // ASCII lowercasing keeps every byte offset, so positions found in `lower` cut `html`.
    let lower = html.to_ascii_lowercase();
    let found = element_text(html, &lower, "title").or_else(|| element_text(html, &lower, "h1"))?;
    let mut text = strip_tags(found);
    for (entity, char) in [
        ("&lt;", "<"),
        ("&gt;", ">"),
        ("&quot;", "\""),
        ("&#39;", "'"),
        ("&nbsp;", " "),
        ("&mdash;", "—"),
        ("&ndash;", "–"),
        ("&amp;", "&"),
    ] {
        text = text.replace(entity, char);
    }
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    (!text.is_empty()).then_some(text)
}

/// What is between the first `<tag …>` and the `</tag>` after it (`lower` is `html` in ASCII
/// lowercase).
fn element_text<'a>(html: &'a str, lower: &str, tag: &str) -> Option<&'a str> {
    let open = format!("<{tag}");
    let close = format!("</{tag}>");
    let mut from = 0;
    while let Some(at) = lower[from..].find(&open) {
        let name_end = from + at + open.len();
        // The whole name: `<title>` or `<title lang="en">`, not `<titles>`.
        let whole = lower[name_end..]
            .chars()
            .next()
            .is_none_or(|next| !(next.is_alphanumeric() || next == '_'));
        if whole {
            let content = name_end + lower[name_end..].find('>')? + 1;
            let end = content + lower[content..].find(&close)?;
            return Some(&html[content..end]);
        }
        from = name_end;
    }
    None
}

/// `text` without its tags (a `<` with no `>` after it stays).
fn strip_tags(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(lt) = rest.find('<') {
        out.push_str(&rest[..lt]);
        match rest[lt + 1..].find('>') {
            Some(gt) if gt > 0 => rest = &rest[lt + 1 + gt + 1..],
            _ => {
                out.push('<');
                rest = &rest[lt + 1..];
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::title_of;

    #[test]
    fn the_title_comes_first_then_the_first_heading() {
        let page = "<html><head><title>Weekly report</title></head><body><h1>Other</h1>";
        assert_eq!(title_of(page).as_deref(), Some("Weekly report"));
        let page = "<body><h1 class=\"top\">Sales</h1><h1>Second</h1></body>";
        assert_eq!(title_of(page).as_deref(), Some("Sales"));
    }

    #[test]
    fn a_blank_title_or_none_at_all_is_none() {
        assert_eq!(title_of("<p>No heading here</p>"), None);
        assert_eq!(title_of("<title> \n </title>"), None);
        assert_eq!(title_of("<title>never closed"), None);
    }

    #[test]
    fn a_blank_title_doesnt_fall_back_to_the_heading() {
        // As the reference client: an empty <title> is found, and is no title.
        assert_eq!(title_of("<title></title><h1>Heading</h1>"), None);
        // An unclosed one isn't found, so the heading is used.
        assert_eq!(
            title_of("<title>open<h1>Heading</h1>").as_deref(),
            Some("Heading")
        );
    }

    #[test]
    fn tags_go_entities_decode_and_white_space_collapses() {
        let page =
            "<TITLE lang=\"en\">\n  Q3 &amp; Q4 <b>sales</b> &mdash;\tdraft &lt;2&gt;  </TITLE>";
        assert_eq!(title_of(page).as_deref(), Some("Q3 & Q4 sales — draft <2>"));
        assert_eq!(
            title_of("<title>a < b</title>").as_deref(),
            Some("a < b"),
            "a lone < is text"
        );
        assert_eq!(title_of("<title>&amp;lt;</title>").as_deref(), Some("&lt;"));
    }

    #[test]
    fn only_the_whole_tag_name_counts() {
        let page = "<titles>no</titles><h1>Yes</h1>";
        assert_eq!(title_of(page).as_deref(), Some("Yes"));
        let page = "<h10>no</h10><h1>Yes</h1>";
        assert_eq!(title_of(page).as_deref(), Some("Yes"));
    }

    #[test]
    fn arabic_titles_keep_their_text() {
        let page = "<title>تقرير الأسبوع</title>";
        assert_eq!(title_of(page).as_deref(), Some("تقرير الأسبوع"));
    }
}
