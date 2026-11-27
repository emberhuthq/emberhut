//! One row of the chat: a time stamp when a gap comes before it, then the user's bubble, the
//! assistant's markdown with its pictures and chips ([`parts`]), or a run of the server's event
//! lines folded into one line.

use std::rc::Rc;

use gpui_kit::assets::IconName as AssetIcon;
use gpui_kit::component::menu::ContextMenuExt as _;
use gpui_kit::component::{Icon, IconName};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, ClickEvent, Div, ElementId, Entity, FontWeight, Stateful, Window, div, px,
};
use jiff::ZonedDateTime;

pub(crate) use self::parts::{RowState, bubble};
use super::chat::ChatView;
use super::chat::attachments;
use super::files::actions as file_actions;
use super::files::drag;
use super::palette::Pal;
use super::viewer::OpenInViewer;
use super::widgets::icon;
use crate::brand::APP_NAME;
use crate::config::ServerUrl;
use crate::heic;
use crate::prose::events::{self, EventLine};
use crate::prose::parsed::Parsed;
use crate::store::files::titles;
use crate::store::files::{Kind, RemoteFile};
use crate::store::{OpenReport, Opening, Store};
use crate::ui::a11y::{self, Accessible};
use emberhut_api::chat::{ChatFile, ChatMessage, FileKind};

mod parts;
mod picture;
mod video;

/// What a row of the chat shows, borrowed from the chat held.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Item<'a> {
    /// An event line that its run's first line already shows.
    Folded,
    /// A message.
    Message {
        /// The message.
        message: &'a ChatMessage,
        /// The time stamp above it, after a gap.
        stamp: Option<String>,
    },
    /// The first of a run of event lines, standing for the run.
    Events {
        /// The run.
        run: &'a [ChatMessage],
        /// The time stamp above it, after a gap.
        stamp: Option<String>,
    },
}

impl<'a> Item<'a> {
    /// The row for `messages[at]`, `now` placing its time stamp.
    #[must_use]
    pub(crate) fn at(messages: &'a [ChatMessage], at: usize, now: &ZonedDateTime) -> Self {
        let Some(message) = messages.get(at) else {
            return Self::Folded;
        };
        let stamp = stamp_of(messages, at, now);
        if !message.event {
            return Self::Message { message, stamp };
        }
        if !starts_run(messages, at) {
            return Self::Folded;
        }
        // The run goes on while events follow, across a gap too: only its start is stamped.
        let length = 1 + messages[at + 1..]
            .iter()
            .take_while(|next| next.event)
            .count();
        Self::Events {
            run: &messages[at..at + length],
            stamp,
        }
    }
}

/// The time stamp above `messages[at]`, after a gap from the one before.
fn stamp_of(messages: &[ChatMessage], at: usize, now: &ZonedDateTime) -> Option<String> {
    let previous = at.checked_sub(1).and_then(|before| messages.get(before));
    let message = messages.get(at)?;
    crate::format::time_stamp(
        previous.map(|m| m.at.as_str()),
        Some(message.at.as_str()),
        now,
    )
}

/// Whether the event `messages[at]` starts a run: the one before isn't an event.
fn starts_run(messages: &[ChatMessage], at: usize) -> bool {
    let previous = at.checked_sub(1).and_then(|before| messages.get(before));
    !previous.is_some_and(|previous| previous.event)
}

/// The event lines of a run, as each of its messages was read, up to
/// [`MOST_LINES`](events::MOST_LINES).
fn run_lines(run: &[Rc<Parsed>]) -> Vec<EventLine> {
    run.iter()
        .flat_map(|parsed| parsed.event_lines.iter().cloned())
        .take(events::MOST_LINES)
        .collect()
}

/// What a run's line says: `Emberhut noticed · 1 check-in · 2 new mails`, or its first line as it
/// is when none of it is an event line.
#[must_use]
fn event_summary(lines: &[EventLine], run: &[Rc<Parsed>]) -> String {
    let said = events::summary(lines).unwrap_or_else(|| {
        run.iter()
            .map(|parsed| parsed.first_line.as_str())
            .find(|line| !line.is_empty())
            .unwrap_or_default()
            .to_owned()
    });
    format!("{APP_NAME} noticed · {said}")
}

/// A row: centred, at most the chat's width, with its time stamp.
pub(crate) fn row(stamp: Option<String>, body: AnyElement, p: &Pal) -> Div {
    let column = div()
        .flex()
        .flex_col()
        .w_full()
        .max_w(px(880.))
        .px(px(28.))
        .py(px(4.))
        .when_some(stamp, |d, stamp| {
            d.child(
                div()
                    .flex()
                    .justify_center()
                    .pt(px(14.))
                    .pb(px(6.))
                    .text_size(px(11.5))
                    .text_color(p.faint)
                    .font_weight(FontWeight::MEDIUM)
                    .child(stamp),
            )
        })
        .child(body);
    div().w_full().flex().justify_center().child(column)
}

/// A run of event lines (each message as it was read, `parsed`): one muted line summing it up,
/// which lists each event when clicked.
pub(crate) fn event_line(
    run: &[ChatMessage],
    parsed: &[Rc<Parsed>],
    open: bool,
    p: &Pal,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Div {
    let id = run
        .first()
        .map_or_else(|| "events".to_owned(), |m| format!("event-{}", m.id));
    let lines = run_lines(parsed);
    let summary = event_summary(&lines, parsed);
    let opens = !lines.is_empty();
    div()
        .flex()
        .flex_col()
        .items_center()
        .w_full()
        .child(
            div()
                .id(ElementId::Name(id.into()))
                .flex()
                .items_center()
                .gap(px(5.))
                .max_w_full()
                .when(opens, |d| d.cursor_pointer().hover(|d| d.bg(p.hover)))
                .px(px(8.))
                .py(px(2.))
                .rounded(px(8.))
                .text_size(px(12.))
                .text_color(p.faint)
                .child(div().truncate().child(summary))
                .when(opens, |d| {
                    d.child(icon(
                        if open {
                            IconName::ChevronDown
                        } else {
                            IconName::ChevronRight
                        },
                        p.faint,
                        11.,
                    ))
                    .on_click(on_click)
                }),
        )
        .when(open && opens, |d| {
            d.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .w_full()
                    .max_w(px(440.))
                    .mt(px(4.))
                    .p(px(12.))
                    .rounded(px(14.))
                    .bg(p.card)
                    .text_size(px(12.))
                    .text_color(p.muted)
                    .children(lines.into_iter().map(|line| {
                        div()
                            .flex()
                            .items_start()
                            .gap(px(8.))
                            .child(
                                Icon::new(if line.is_check_in() {
                                    AssetIcon::AlarmClock
                                } else {
                                    AssetIcon::Circle
                                })
                                .size(px(if line.is_check_in() { 13. } else { 7. }))
                                .text_color(p.faint),
                            )
                            .child(div().flex_1().min_w_0().line_clamp(2).child(line.text))
                    })),
            )
        })
}

/// What the Apple apps name the still frames they send with a video, for the assistant only.
const VIDEO_FRAME: &str = " · frame ";

/// The files a message shows, with their places among all of its files: not the still frames
/// sent along with a video, which only the assistant looks at (as the Apple apps leave them
/// out).
fn shown_files(files: &[ChatFile]) -> impl Iterator<Item = (usize, &ChatFile)> {
    files
        .iter()
        .enumerate()
        .filter(|(_, file)| !file.name.contains(VIDEO_FRAME))
}

/// What a click on a chip with a link does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Click {
    /// Open the report in the panel beside the page (fetched from the server, signed in).
    Panel,
    /// Fetch the file from the server (signed in) and open it in the viewer.
    View,
    /// Hand the link to the browser (which `ServerUrl::link` allows only for web addresses).
    Browser,
    /// Nothing: there is no link.
    Nothing,
}

impl Click {
    /// The click for a chip linking to `url`; `report` when it is an HTML report. Only a path on
    /// the server opens beside the page (a report) or in the viewer (anything else): anything
    /// elsewhere goes to the browser.
    fn of(report: bool, url: Option<&str>) -> Self {
        match url {
            None => Self::Nothing,
            Some(url) if ServerUrl::is_path(url) && report => Self::Panel,
            Some(url) if ServerUrl::is_path(url) => Self::View,
            Some(_) => Self::Browser,
        }
    }
}

/// Whether a chat file shows as a picture: one this app draws (HEIC where the platform reads it).
pub(super) fn shows_as_picture(file: &ChatFile) -> bool {
    file.kind == FileKind::Image
        && (attachments::is_drawn(&file.name) || (heic::CONVERTS && heic::is_heic(&file.name)))
}

/// Whether a chat file is a video, which shows its first frame and plays in the viewer.
pub(super) fn is_video(file: &ChatFile) -> bool {
    file.kind == FileKind::Video || Kind::of(&file.name) == Kind::Video
}

/// The still frame sent with `video` that shows as its picture: its first.
pub(super) fn poster<'a>(files: &'a [ChatFile], video: &ChatFile) -> Option<&'a ChatFile> {
    let first = format!("{}{VIDEO_FRAME}1.jpg", video.name);
    files.iter().find(|file| file.name == first)
}

/// `file` as the viewer and its uses take it.
pub(super) fn remote(file: &ChatFile) -> RemoteFile {
    RemoteFile::new(file.url.clone(), file.name.clone())
}

/// Opens `file` in the viewer of `window`.
pub(super) fn view_in(window: &mut Window, file: RemoteFile, cx: &mut App) {
    window.dispatch_action(Box::new(OpenInViewer { file }), cx);
}

/// A file's chip: its icon, name and size (when it was sent from here: the server gives none),
/// or for an HTML report its card. A report opens beside the page; anything else in the viewer,
/// fetched first, the chip saying so meanwhile or why it failed. Its uses are on a right-click,
/// and it drags out as the file.
fn file_chip(
    message: &ChatMessage,
    i: usize,
    file: &ChatFile,
    store: &Entity<Store>,
    p: &Pal,
    cx: &Context<'_, ChatView>,
) -> impl IntoElement {
    let html = file.is_html();
    let held = store.read(cx);
    let note = match held.opening(&file.url) {
        Some(opening) => Some(opening.note()),
        None => held
            .attachments()
            .sent_size(&file.url)
            .map(crate::format::file_size),
    };
    let failed = matches!(held.opening(&file.url), Some(Opening::Failed(_)));
    let id = ElementId::Name(format!("file-{}-{i}", message.id).into());
    // The chip's name and what it is doing ("Downloading…"), said again as that changes.
    let spoken = a11y::line(&[&file.name, note.as_deref().unwrap_or_default()]);
    let live = held.opening(&file.url).is_some();
    let element = if html {
        let title = held
            .report_title(&file.url)
            .map_or_else(|| titles::stem(&file.name), ToString::to_string);
        report_card(id, &title, &file.name, p)
    } else {
        div()
            .id(id)
            .flex()
            .items_center()
            .gap(px(7.))
            .px(px(10.))
            .py(px(6.))
            .rounded(px(10.))
            .border_1()
            .border_color(p.border)
            .bg(p.card)
            .cursor_pointer()
            .hover(|d| d.bg(p.hover))
            .max_w(px(360.))
            .child(
                Icon::new(attachments::file_icon(&file.name))
                    .size(px(15.))
                    .text_color(p.muted),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .min_w_0()
                    .child(
                        div()
                            .truncate()
                            .text_size(px(12.5))
                            .child(file.name.clone()),
                    )
                    .children(note.map(|note| {
                        div()
                            .truncate()
                            .text_size(px(11.))
                            .text_color(if failed { p.danger } else { p.muted })
                            .child(note)
                    })),
            )
    };
    let element = element.button_named(spoken).when(live, Accessible::live);
    let report = OpenReport {
        name: file.name.clone(),
        path: file.url.clone(),
    };
    let (remote, store_for_click) = (remote(file), store.clone());
    let opened = remote.clone();
    drag::draggable(element, &remote, store)
        .on_click(
            move |_, window, cx| match Click::of(html, Some(&report.path)) {
                Click::Panel => {
                    store_for_click
                        .update(cx, |store, cx| store.show_report(Some(report.clone()), cx));
                }
                Click::View => view_in(window, opened.clone(), cx),
                Click::Browser => {
                    if let Some(link) = store_for_click.read(cx).server().link(&report.path) {
                        cx.open_url(&link);
                    }
                }
                Click::Nothing => {}
            },
        )
        .context_menu(file_actions::menu(&remote, store))
}

/// An HTML report's card: the page's own title, a Report label with the file's name, and a
/// chevron.
fn report_card(id: ElementId, title: &str, name: &str, p: &Pal) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .gap(px(12.))
        .p(px(10.))
        .max_w(px(380.))
        .rounded(px(14.))
        .border_1()
        .border_color(p.border)
        .bg(p.card)
        .cursor_pointer()
        .hover(|d| d.bg(p.hover))
        .child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .size(px(40.))
                .rounded(px(9.))
                .bg(p.accent.opacity(0.12))
                .child(
                    Icon::new(AssetIcon::FileChartColumn)
                        .size(px(20.))
                        .text_color(p.accent),
                ),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .gap(px(4.))
                .child(
                    div()
                        .line_clamp(2)
                        .text_size(px(13.5))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(title.to_owned()),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .min_w_0()
                        .child(
                            div()
                                .flex_none()
                                .px(px(6.))
                                .py(px(1.))
                                .rounded_full()
                                .bg(p.hover)
                                .text_size(px(10.5))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(p.muted)
                                .child("Report"),
                        )
                        .child(
                            div()
                                .truncate()
                                .text_size(px(11.5))
                                .text_color(p.muted)
                                .child(name.to_owned()),
                        ),
                ),
        )
        .child(icon(IconName::ChevronRight, p.faint, 13.))
}

#[cfg(test)]
mod tests {
    use jiff::Timestamp;
    use jiff::tz::TimeZone;

    use serde_json::json;

    use super::*;
    use crate::fixtures::message_at;
    use emberhut_api::chat::FileKind;

    fn message(id: i64, at: &str, event: bool, text: &str) -> ChatMessage {
        let at = if at.is_empty() {
            "2026-11-25T08:00:00Z"
        } else {
            at
        };
        message_at(id, at, json!({"event": event, "text": text}))
    }

    fn now() -> ZonedDateTime {
        let now: Timestamp = "2026-11-25T12:00:00Z".parse().unwrap();
        now.to_zoned_datetime(TimeZone::UTC)
    }

    #[test]
    fn a_run_of_events_shows_once_at_its_first() {
        let chat = [
            message(1, "2026-11-25T08:00:00Z", false, "Hi"),
            message(2, "2026-11-25T08:01:00Z", true, "- [check_in] morning"),
            message(3, "2026-11-25T08:02:00Z", true, "[deploy] done"),
            message(4, "2026-11-25T08:03:00Z", false, "Ok"),
        ];
        assert!(matches!(
            Item::at(&chat, 0, &now()),
            Item::Message { stamp: Some(_), .. }
        ));
        let Item::Events { run, stamp: None } = Item::at(&chat, 1, &now()) else {
            panic!("the run")
        };
        assert_eq!(run.len(), 2);
        assert_eq!(Item::at(&chat, 2, &now()), Item::Folded);
        assert!(matches!(
            Item::at(&chat, 3, &now()),
            Item::Message { stamp: None, .. }
        ));
        assert_eq!(Item::at(&chat, 9, &now()), Item::Folded);
    }

    #[test]
    fn a_run_stays_whole_across_a_gap_stamped_at_its_start() {
        let chat = [
            message(1, "2026-11-25T08:00:00Z", true, "[a] x"),
            message(2, "2026-11-25T09:00:00Z", true, "[b] y"),
            message(3, "2026-11-25T10:00:00Z", false, "Later"),
        ];
        let Item::Events { run, stamp } = Item::at(&chat, 0, &now()) else {
            panic!("the run")
        };
        assert_eq!(run.len(), 2, "the later event is part of the run");
        assert!(stamp.is_some(), "stamped at its start");
        assert_eq!(Item::at(&chat, 1, &now()), Item::Folded);
        assert!(matches!(
            Item::at(&chat, 2, &now()),
            Item::Message { stamp: Some(_), .. }
        ));
    }

    fn summed_up(run: &[ChatMessage]) -> String {
        let parsed: Vec<Rc<Parsed>> = run.iter().map(|m| Rc::new(Parsed::of(m))).collect();
        event_summary(&run_lines(&parsed), &parsed)
    }

    #[test]
    fn a_run_is_summed_up_in_one_line() {
        let run = [
            message(1, "", true, "\n- [checkin] morning"),
            message(
                2,
                "",
                true,
                "- [new_mail] from the bank\n- [new_mail] from the school",
            ),
        ];
        assert_eq!(
            summed_up(&run),
            "Emberhut noticed · 1 check-in · 2 new mails"
        );
        let plain = [message(3, "", true, "\nSomething else happened")];
        assert_eq!(
            summed_up(&plain),
            "Emberhut noticed · Something else happened"
        );
    }

    #[test]
    fn only_a_file_on_the_server_opens_here() {
        assert_eq!(Click::of(true, Some("/r/week.html")), Click::Panel);
        for elsewhere in [
            "https://example.com/r.html",
            "@evil.example/x",
            ".evil.net/x",
            "//evil/x",
        ] {
            assert_eq!(
                Click::of(true, Some(elsewhere)),
                Click::Browser,
                "{elsewhere}"
            );
        }
        assert_eq!(Click::of(false, Some("/uploads/a.pdf")), Click::View);
        assert_eq!(
            Click::of(false, Some("https://example.com/a.pdf")),
            Click::Browser
        );
        assert_eq!(Click::of(true, None), Click::Nothing);
    }

    #[test]
    fn a_video_shows_its_first_still_and_pictures_show_where_they_can_be_drawn() {
        let file = |name: &str, kind: FileKind| ChatFile {
            name: name.to_owned(),
            url: format!("/uploads/{name}"),
            kind,
            path: None,
        };
        let files = [
            file("clip.mov", FileKind::Video),
            file("clip.mov · frame 2.jpg", FileKind::Image),
            file("clip.mov · frame 1.jpg", FileKind::Image),
        ];
        assert!(is_video(&files[0]));
        assert_eq!(
            poster(&files, &files[0]).map(|still| still.name.as_str()),
            Some("clip.mov · frame 1.jpg")
        );
        assert!(shows_as_picture(&file("shot.png", FileKind::Image)));
        assert_eq!(
            shows_as_picture(&file("IMG_0001.HEIC", FileKind::Image)),
            heic::CONVERTS
        );
        assert!(!shows_as_picture(&file("notes.pdf", FileKind::File)));
    }

    #[test]
    fn a_videos_still_frames_are_not_shown() {
        let file = |name: &str| ChatFile {
            name: name.to_owned(),
            url: format!("/uploads/{name}"),
            kind: FileKind::Image,
            path: None,
        };
        let files = [
            file("clip.mov"),
            file("clip.mov · frame 1.jpg"),
            file("clip.mov · frame 2.jpg"),
            file("photo.jpg"),
        ];
        let shown: Vec<_> = shown_files(&files)
            .map(|(i, f)| (i, f.name.as_str()))
            .collect();
        assert_eq!(shown, [(0, "clip.mov"), (3, "photo.jpg")]);
    }
}
