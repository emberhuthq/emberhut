//! A quoted message: a coloured bar, who said it and when, two lines of it, and its picture
//! small. It rides above the composer while a reply is written, then on top of the sent
//! bubble, where a click goes back to the original.

use emberhut_api::chat::{FileKind, Quote};
use gpui_kit::assets::IconName as AssetIcon;
use gpui_kit::component::Icon;
use gpui_kit::prelude::*;
use gpui_kit::{Div, FontWeight, Hsla, ImageSource, div, img, px};
use jiff::ZonedDateTime;

use super::palette::Pal;
use crate::brand::APP_NAME;
use crate::prose::copy;
use crate::store::Store;

/// Where a quote is drawn, which sets its colours.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Surface {
    /// Inside the user's own bubble, in its text colour.
    OnMine,
    /// On the chat or above the composer.
    Plain,
}

/// Its first line: `You · 09:41`, `Emberhut · 09:41`.
#[must_use]
pub(crate) fn who_and_when(quote: &Quote, now: &ZonedDateTime) -> String {
    let who = if quote.mine { "You" } else { APP_NAME };
    let when = quote
        .at
        .as_ref()
        .and_then(|at| at.as_str().parse::<jiff::Timestamp>().ok())
        .map(|at| {
            at.to_zoned_datetime(now.time_zone().clone())
                .strftime("%H:%M")
                .to_string()
        });
    match when {
        Some(when) => format!("{who} · {when}"),
        None => who.to_owned(),
    }
}

/// Its words as the user reads them, or what it showed when it was only a picture.
#[must_use]
pub(crate) fn words(quote: &Quote) -> String {
    let text = copy::preview(&quote.text);
    if !text.is_empty() {
        return text;
    }
    match quote.file.as_ref().map(|file| &file.kind) {
        Some(FileKind::Video) => "Video".to_owned(),
        _ => "Photo".to_owned(),
    }
}

/// The icon standing for a quoted file that isn't a picture: a video's, or a file's.
fn thumbnail_icon(kind: &FileKind) -> AssetIcon {
    match kind {
        FileKind::Video => AssetIcon::Video,
        _ => AssetIcon::File,
    }
}

/// The quote, drawn on `surface`, its time in `now`'s time zone (the chat's clock this frame).
pub(crate) fn view(
    quote: &Quote,
    surface: Surface,
    now: &ZonedDateTime,
    store: &Store,
    p: &Pal,
) -> Div {
    let (bar, text, background): (Hsla, Hsla, Hsla) = match surface {
        Surface::OnMine => (p.on_mine, p.on_mine.opacity(0.85), p.on_mine.opacity(0.18)),
        Surface::Plain if quote.mine => (p.mine, p.muted, p.hover),
        Surface::Plain => (p.warn, p.muted, p.hover),
    };
    let thumbnail = quote.file.as_ref().map(|file| match file.kind {
        FileKind::Image => match store.picture(&file.url) {
            Some(image) => img(ImageSource::Render(image))
                .size(px(36.))
                .rounded(px(6.))
                .into_any_element(),
            None => div()
                .size(px(36.))
                .rounded(px(6.))
                .bg(p.card)
                .into_any_element(),
        },
        _ => div()
            .size(px(36.))
            .rounded(px(6.))
            .bg(p.card)
            .flex()
            .items_center()
            .justify_center()
            .child(
                Icon::new(thumbnail_icon(&file.kind))
                    .size(px(14.))
                    .text_color(p.muted),
            )
            .into_any_element(),
    });
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .p(px(6.))
        .rounded(px(8.))
        .bg(background)
        .child(div().w(px(3.)).h(px(30.)).rounded(px(2.)).bg(bar))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .gap(px(2.))
                .child(
                    div()
                        .text_size(px(11.5))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(bar)
                        .child(who_and_when(quote, now)),
                )
                .child(
                    div()
                        .text_size(px(12.))
                        .line_clamp(2)
                        .text_color(text)
                        .child(words(quote)),
                ),
        )
        .children(thumbnail)
}

#[cfg(test)]
mod tests {
    use jiff::tz::TimeZone;
    use serde_json::json;

    use super::*;

    fn quote(value: serde_json::Value) -> Quote {
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn a_quote_says_who_when_and_what() {
        let now = "2026-11-25T12:00:00Z"
            .parse::<jiff::Timestamp>()
            .unwrap()
            .to_zoned_datetime(TimeZone::UTC);
        let mine = quote(json!({"id": 1, "mine": true, "at": "2026-11-25T09:41:00Z",
            "text": "The **plan**, see [it](https://example.com)", "file": null}));
        assert_eq!(who_and_when(&mine, &now), "You · 09:41");
        assert_eq!(words(&mine), "The plan, see it");
        let photo = quote(json!({"id": 2, "mine": false, "at": null, "text": " ",
            "file": {"name": "a.png", "url": "/uploads/a.png", "kind": "image"}}));
        assert_eq!(who_and_when(&photo, &now), "Emberhut");
        assert_eq!(words(&photo), "Photo");
        let video = quote(json!({"id": 3, "mine": false, "at": null, "text": "",
            "file": {"name": "a.mov", "url": "/uploads/a.mov", "kind": "video"}}));
        assert_eq!(words(&video), "Video");
    }

    #[test]
    fn a_quoted_file_shows_its_kinds_icon() {
        assert_eq!(thumbnail_icon(&FileKind::Video), AssetIcon::Video);
        assert_eq!(thumbnail_icon(&FileKind::File), AssetIcon::File);
    }
}
