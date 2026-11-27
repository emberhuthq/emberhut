//! What the viewer shows of a file once it is here, by its kind: read off the UI thread (a
//! picture decoded, a note read, a PDF's pages measured), then drawn.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui_kit::TestSupportExt as _;
use gpui_kit::assets::IconName;
use gpui_kit::component::Icon;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::text::TextView;
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, Entity, RenderImage, SharedString, SvgRenderer, Window, div, px};

use super::native::Native;
use super::pdf_view::PdfPages;
use super::{FileViewer, Open, free_image, note, picture};
use crate::store::files::{Kind, RemoteFile, Use};
use crate::store::{Store, images};
use crate::ui::a11y::Accessible as _;
use crate::ui::files::actions::FileAction;
use crate::ui::palette::pal;
use crate::ui::prose;
use crate::ui::reads;

/// The note's text size before zooming.
const NOTE_TEXT: f32 = 14.;

/// What the viewer shows.
pub(super) enum Body {
    /// The file is coming down.
    Loading,
    /// It couldn't be had or read, worded for the user.
    Failed(String),
    /// A picture.
    Picture(Arc<RenderImage>),
    /// A note or text file, as markdown.
    Note(SharedString),
    /// A PDF's pages.
    Pdf(PdfPages),
    /// A file that could run as a program: never previewed or opened, only shown in its
    /// folder.
    Program,
    /// A video or a document, shown by the platform where it can (made once drawn), else
    /// opened in its app.
    Media {
        /// The file, kept here.
        path: PathBuf,
        /// The platform's view of it, once made.
        native: Option<Native>,
    },
}

impl Body {
    /// Frees what it holds (a picture's tiles, a PDF's pages).
    pub(super) fn free(self, cx: &mut App) {
        match self {
            Self::Picture(image) => free_image(image, cx),
            Self::Pdf(pages) => pages.free(cx),
            Self::Loading
            | Self::Failed(_)
            | Self::Note(_)
            | Self::Program
            | Self::Media { .. } => {}
        }
    }
}

/// What a load came back with.
enum Loaded {
    Failed(String),
    Picture(Arc<RenderImage>),
    Note(String),
    Pdf(PathBuf, Vec<super::pages::PageSize>),
    Program,
    Media(PathBuf),
}

/// Fetches `file` (a `kind`) for open `number`, then reads it off the UI thread for showing.
pub(super) fn load(
    store: &Entity<Store>,
    file: RemoteFile,
    kind: Kind,
    number: u64,
    cx: &mut Context<'_, FileViewer>,
) {
    let fetching = store.update(cx, |store, cx| store.fetch_file(&file, cx));
    let svg = cx.svg_renderer();
    cx.spawn(async move |this, cx| {
        let loaded = match fetching.kept().await {
            Err(why) => Loaded::Failed(why),
            // Its text is shown (a script reads as a note), but nothing else of it is made.
            Ok(_) if file.runs_as_program() && kind != Kind::Note => Loaded::Program,
            Ok(path) => {
                cx.background_spawn(async move { read(kind, &file.name, path, svg) })
                    .await
            }
        };
        let body = match loaded {
            Loaded::Failed(why) => Body::Failed(why),
            Loaded::Picture(image) => Body::Picture(image),
            Loaded::Note(text) => Body::Note(text.into()),
            Loaded::Pdf(path, sizes) => Body::Pdf(PdfPages::new(path, sizes)),
            Loaded::Program => Body::Program,
            Loaded::Media(path) => Body::Media { path, native: None },
        };
        if let Err(error) = this.update(cx, |viewer, cx| viewer.loaded(number, body, cx)) {
            log::debug!("the viewer closed before its file was read: {error}");
        }
    })
    .detach();
}

/// Reads the kept file at `path` for showing as a `kind`.
fn read(kind: Kind, name: &str, path: PathBuf, svg: SvgRenderer) -> Loaded {
    match kind {
        // A picture this platform can't read opens in its app.
        Kind::Picture if crate::heic::is_heic(name) && !crate::heic::CONVERTS => {
            Loaded::Media(path)
        }
        Kind::Picture => match std::fs::read(&path) {
            Ok(bytes) => images::decode(bytes, svg).map_or_else(Loaded::Failed, Loaded::Picture),
            Err(error) => unreadable(&path, &error),
        },
        Kind::Note => match std::fs::read(&path) {
            Ok(bytes) => Loaded::Note(note::markdown_for(name, &String::from_utf8_lossy(&bytes))),
            Err(error) => unreadable(&path, &error),
        },
        Kind::Pdf => match crate::pdf::pages(&path) {
            Ok(sizes) => Loaded::Pdf(path, sizes),
            #[cfg(not(target_os = "macos"))]
            Err(crate::pdf::Error::NoRenderer) => Loaded::Media(path),
            Err(error) => Loaded::Failed(error.to_string()),
        },
        Kind::Video | Kind::Document | Kind::Report => Loaded::Media(path),
    }
}

fn unreadable(path: &Path, error: &std::io::Error) -> Loaded {
    log::warn!("couldn't read {}: {error}", path.display());
    Loaded::Failed("Couldn't read the file".to_owned())
}

/// The body, as the viewer draws it.
pub(super) fn render(
    viewer: &mut FileViewer,
    window: &mut Window,
    cx: &mut Context<'_, FileViewer>,
) -> AnyElement {
    let p = pal(cx);
    let store = viewer.store.clone();
    let Some(open) = &mut viewer.open else {
        return div().into_any_element();
    };
    match &mut open.body {
        Body::Loading => centred(div().text_color(p.muted).child("Loading…")),
        Body::Failed(why) => centred(
            div()
                .id("viewer-failed")
                .announced(why.clone())
                .text_color(p.danger)
                .child(why.clone()),
        ),
        Body::Picture(image) => picture::view(open.area, open.zoom, &open.scroll, image, cx),
        // Its links go the way the chat's do (`prose::follow_link`): a file on the server opens
        // in this viewer, a web address in the browser, and nothing else opens at all.
        Body::Note(text) => div()
            .id("viewer-note")
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&open.scroll)
            .px(px(24.))
            .py(px(18.))
            .when(reads::starts_right(text), Styled::text_right)
            .child(
                TextView::markdown("viewer-note-text", text.clone())
                    .selectable(true)
                    .text_size(px(NOTE_TEXT * open.zoom.scale()))
                    .on_link_click(move |url, _, window, cx| {
                        prose::follow_link(&store, url, window, cx);
                    }),
            )
            .into_any_element(),
        Body::Pdf(pages) => {
            let (area, zoom) = (open.area, open.zoom);
            pages.view(area, zoom, &open.scroll, window, cx)
        }
        Body::Program => program(open, &store, cx),
        Body::Media { path, native } => {
            if native.is_none() {
                *native = Native::make(open.kind, path, window);
            }
            match native {
                Some(native) => native.element(),
                None => outside(open, &store, cx),
            }
        }
    }
}

/// A file that could run as a program: its name, why it isn't opened, and Show in Finder.
fn program(open: &Open, store: &Entity<Store>, cx: &mut Context<'_, FileViewer>) -> AnyElement {
    let p = pal(cx);
    let (file, store) = (open.file.clone(), store.clone());
    centred(
        div()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(12.))
            .max_w(px(420.))
            .child(Icon::new(IconName::File).size(px(44.)).text_color(p.muted))
            .child(div().text_size(px(15.)).child(open.file.name.clone()))
            .child(
                div()
                    .id(PROGRAM_NOTE)
                    // Seen by the headless tests (a no-op outside them).
                    .test_support()
                    .text_size(px(13.))
                    .text_color(p.muted)
                    .child(program_note()),
            )
            .child(
                Button::new("viewer-show-in-folder")
                    .primary()
                    .label(FileAction::ShowInFolder.label(&open.file))
                    .on_click(move |_, _, cx| {
                        let file = file.clone();
                        store.update(cx, |store, cx| store.use_file(file, Use::ShowInFolder, cx));
                    }),
            ),
    )
}

/// The element id of what the viewer says of a file that could run as a program.
pub(super) const PROGRAM_NOTE: &str = "viewer-program-note";

/// What the viewer says of a file that could run as a program.
fn program_note() -> String {
    let manager = if cfg!(target_os = "macos") {
        "Finder"
    } else {
        "its folder"
    };
    format!("This file runs as a program: it is only shown in {manager}, never opened.")
}

/// `content` in the middle of the body.
fn centred(content: impl IntoElement) -> AnyElement {
    div()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .child(content)
        .into_any_element()
}

/// A file only its app shows here: its kind, its name, and Open in its app.
fn outside(open: &Open, store: &Entity<Store>, cx: &mut Context<'_, FileViewer>) -> AnyElement {
    let p = pal(cx);
    let (mark, said) = match open.kind {
        Kind::Video => (IconName::Film, "This video plays in its app."),
        Kind::Pdf => (
            IconName::FileText,
            "PDFs show here once Poppler's tools (pdftoppm) are installed.",
        ),
        Kind::Picture | Kind::Report | Kind::Note | Kind::Document => {
            (IconName::File, "This file previews in its app.")
        }
    };
    let (file, store) = (open.file.clone(), store.clone());
    centred(
        div()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(12.))
            .max_w(px(420.))
            .child(Icon::new(mark).size(px(44.)).text_color(p.muted))
            .child(div().text_size(px(15.)).child(open.file.name.clone()))
            .child(div().text_size(px(13.)).text_color(p.muted).child(said))
            .child(
                Button::new("viewer-open-in-app")
                    .primary()
                    .label(FileAction::OpenInApp.label(&open.file))
                    .on_click(move |_, _, cx| {
                        let file = file.clone();
                        store.update(cx, |store, cx| store.use_file(file, Use::OpenInApp, cx));
                    }),
            ),
    )
}
