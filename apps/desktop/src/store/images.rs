//! Pictures in the chat: fetched by path and decoded in the background when a page of chat
//! arrives (not when drawn, so each costs its decoded size from then on; see
//! [`super::retention`] for what that leaves unbounded), kept while [`super::retention`] says
//! so, and freed when it lets them go, GPUI's copies included.
//!
//! The store decodes them itself and hands GPUI the pixels ([`ImageSource::Render`]): GPUI's
//! own decoding of an [`Image`] would cache the pixels in its asset cache for the app's
//! lifetime, out of the store's reach. What GPUI still holds of a picture is its tiles in each
//! window's sprite atlas, which [`App::drop_image`] frees.
//!
//! A file that comes but can't be decoded isn't fetched again while it is held: the same bytes
//! would fail the same way. A fetch that failed is, once it is
//! [`RETRY_AFTER`](super::fetched::RETRY_AFTER) old.
//!
//! [`ImageSource::Render`]: gpui_kit::ImageSource::Render

use std::sync::Arc;
use std::time::Instant;

use gpui_kit::AppContext as _;
use gpui_kit::{App, Context, Image, ImageFormat, RenderImage, SvgRenderer};

use super::Store;
use super::fetched::{Failure, Fetched};
use super::retention::{RoomCache, Weigh};
use emberhut_api::chat::{ChatFile, FileKind};

use crate::config::ServerUrl;
use emberhut_api::ids::RoomId;

/// The chat's pictures, decoded.
pub(crate) type ImageCache = RoomCache<Arc<RenderImage>>;

impl Weigh for Arc<RenderImage> {
    /// Its pixels, every frame.
    fn bytes(&self) -> usize {
        (0..self.frame_count())
            .filter_map(|frame| self.as_bytes(frame))
            .map(<[u8]>::len)
            .sum()
    }
}

/// A picture's format, from its first bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Sniffed {
    /// PNG.
    Png,
    /// JPEG.
    Jpeg,
    /// GIF.
    Gif,
    /// WebP.
    Webp,
    /// HEIC or HEIF (an iPhone's photos), which only the platform reads ([`crate::heic`]).
    Heic,
}

impl Sniffed {
    /// The format `bytes` start with; PNG when they say nothing (GPUI's decoder tries anyway).
    #[must_use]
    pub(crate) fn of(bytes: &[u8]) -> Self {
        if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
            Self::Png
        } else if bytes.starts_with(&[0xFF, 0xD8]) {
            Self::Jpeg
        } else if bytes.starts_with(b"GIF") {
            Self::Gif
        } else if bytes.get(8..12) == Some(b"WEBP") {
            Self::Webp
        } else if bytes.get(4..8) == Some(b"ftyp")
            && bytes.get(8..12).is_some_and(|brand| {
                [
                    b"heic", b"heix", b"hevc", b"heim", b"heis", b"mif1", b"msf1",
                ]
                .iter()
                .any(|known| brand == *known)
            })
        {
            Self::Heic
        } else {
            Self::Png
        }
    }

    /// The format GPUI decodes it as (a HEIC picture once the platform turned it to TIFF).
    #[must_use]
    pub(crate) fn format(self) -> ImageFormat {
        match self {
            Self::Png => ImageFormat::Png,
            Self::Jpeg => ImageFormat::Jpeg,
            Self::Gif => ImageFormat::Gif,
            Self::Webp => ImageFormat::Webp,
            Self::Heic => ImageFormat::Tiff,
        }
    }
}

/// `bytes` as a picture GPUI reads: as they are, or turned into TIFF by the platform when they
/// are HEIC ([`crate::heic`]). It may decode the whole picture: call it off the UI thread.
///
/// # Errors
///
/// A HEIC picture this platform can't read, worded for the user.
pub(crate) fn readable(bytes: Vec<u8>) -> Result<Image, String> {
    let sniffed = Sniffed::of(&bytes);
    if sniffed != Sniffed::Heic {
        return Ok(Image::from_bytes(sniffed.format(), bytes));
    }
    crate::heic::to_tiff(&bytes)
        .map(|tiff| Image::from_bytes(ImageFormat::Tiff, tiff))
        .ok_or_else(|| "This device can't show HEIC pictures".to_owned())
}

/// `bytes` decoded for drawing (see [`readable`]).
///
/// # Errors
///
/// Why it can't be drawn, worded for the user.
pub(crate) fn decode(bytes: Vec<u8>, svg: SvgRenderer) -> Result<Arc<RenderImage>, String> {
    readable(bytes)?
        .to_image_data(svg)
        .map_err(|error| format!("couldn't read the picture: {error}"))
}

/// Whether a chat file is fetched as a picture: a picture, but not a HEIC one where the platform
/// can't read it (it shows as a file) nor a video's later still frames (only the first shows).
#[must_use]
fn is_fetched(file: &ChatFile) -> bool {
    file.kind == FileKind::Image
        && (crate::heic::CONVERTS || !crate::heic::is_heic(&file.name))
        && !super::stills::is_later_frame(&file.name)
}

impl Store {
    /// The picture at `path`, if it is here.
    #[must_use]
    pub(crate) fn picture(&self, path: &str) -> Option<Arc<RenderImage>> {
        match self.images.get(path) {
            Some(Fetched::Ready(image)) => Some(Arc::clone(image)),
            Some(Fetched::Loading | Fetched::Failed(_)) | None => None,
        }
    }

    /// How many pictures have come since the store began. It only grows: a picture that comes
    /// while another is freed still counts.
    #[must_use]
    pub(crate) fn pictures_come(&self) -> u64 {
        self.pictures_come
    }

    /// Fetches the pictures `room`'s held chat shows that aren't here yet.
    pub(super) fn fetch_pictures(&mut self, room: RoomId, cx: &mut Context<'_, Self>) {
        // Its files, the pictures its quotes show, and the pictures alone on a line of its text.
        let paths: Vec<String> = self
            .chats
            .messages(room)
            .iter()
            .flat_map(|message| {
                message
                    .files
                    .iter()
                    .chain(message.reply.as_ref().and_then(|quote| quote.file.as_ref()))
                    .filter(|file| is_fetched(file))
                    .map(|file| file.url.clone())
                    .chain(crate::prose::blocks::pictures(&message.text))
            })
            .filter(|url| ServerUrl::is_path(url))
            .collect();
        for path in paths {
            self.fetch_picture(room, path, cx);
        }
        // Its reports' cards show their pages' titles.
        self.fetch_report_titles(room, cx);
    }

    /// `room` is opened: the pictures of the rooms that are no longer kept are freed.
    pub(super) fn keep_pictures_for(&mut self, room: RoomId, cx: &mut Context<'_, Self>) {
        let dropped = self.images.open(room);
        self.free_pictures(dropped, cx);
    }

    pub(super) fn fetch_picture(&mut self, room: RoomId, path: String, cx: &mut Context<'_, Self>) {
        if !self.images.begin(room, &path, Instant::now()) {
            return;
        }
        let bytes = self.file_bytes(&path, cx);
        let svg = cx.svg_renderer();
        cx.spawn(async move |this, cx| {
            let outcome = match bytes.await {
                // The same bytes would fail the same way: not fetched again.
                Ok(bytes) => {
                    cx.background_spawn(async move { decode(bytes, svg).map_err(Failure::lasting) })
                        .await
                }
                Err(failure) => Err(failure),
            }
            .inspect_err(|error| log::warn!("couldn't show the picture {path}: {error}"));
            super::update_or_log(&this, cx, |store, cx| {
                let dropped = store.images.finish(path.clone(), outcome, Instant::now());
                if store.picture(&path).is_some() {
                    store.pictures_come += 1;
                }
                store.free_pictures(dropped, cx);
                cx.notify();
            });
        })
        .detach();
    }

    /// Frees `pictures`' tiles in every window's sprite atlas, then their pixels (the last
    /// reference goes with them). Deferred until no window is being updated, as `App` doesn't
    /// reach the one that is.
    fn free_pictures(&self, pictures: Vec<Arc<RenderImage>>, cx: &mut App) {
        if pictures.is_empty() {
            return;
        }
        log::debug!(
            "freeing {} pictures; {} held, {} bytes",
            pictures.len(),
            self.images.len(),
            self.images.bytes()
        );
        cx.defer(move |cx| {
            for picture in pictures {
                cx.drop_image(picture, None);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use gpui_kit::TestAppContext;
    use serde_json::json;

    use super::*;
    use crate::backend::fake::{Call, FakeBackend};
    use crate::fixtures::{message_at, state};

    #[gpui_kit::test]
    fn a_messages_own_pictures_and_its_quotes_are_fetched_never_one_on_the_web(
        cx: &mut TestAppContext,
    ) {
        let backend = Arc::new(FakeBackend::new([]));
        let shared = Arc::clone(&backend) as Arc<dyn crate::backend::Backend>;
        let store = cx.new(|_| Store::for_tests(shared));
        let answer = message_at(
            1,
            "2026-11-25T08:00:00Z",
            json!({"text": "Here:\n![chart](/uploads/chart.png)\n![pixel](https://example.com/p.png)\nin a line ![x](/uploads/inline.png) stays text",
                "reply": {"id": 0, "mine": true, "at": null, "text": "",
                    "file": {"name": "q.png", "url": "/uploads/q.png", "kind": "image"}}}),
        );
        store.update(cx, |store, cx| {
            store.take_state_for_tests(state(json!({"chat": [answer]})), cx);
        });
        cx.run_until_parked();
        let fetched: Vec<String> = backend
            .calls()
            .into_iter()
            .filter_map(|call| match call {
                Call::File(path) => Some(path),
                _ => None,
            })
            .collect();
        assert_eq!(fetched, ["/uploads/q.png", "/uploads/chart.png"]);
    }

    #[test]
    fn formats_are_known_by_their_first_bytes() {
        assert_eq!(Sniffed::of(b"\x89PNG\r\n\x1a\n...."), Sniffed::Png);
        assert_eq!(Sniffed::of(&[0xFF, 0xD8, 0xFF, 0xE0]), Sniffed::Jpeg);
        assert_eq!(Sniffed::of(b"GIF89a"), Sniffed::Gif);
        assert_eq!(Sniffed::of(b"RIFF\0\0\0\0WEBPVP8 "), Sniffed::Webp);
        assert_eq!(Sniffed::of(b"\0\0\0\x18ftypheic\0\0"), Sniffed::Heic);
        assert_eq!(
            Sniffed::of(b"\0\0\0\x18ftypmp42\0\0"),
            Sniffed::Png,
            "a video isn't"
        );
        assert_eq!(Sniffed::of(b""), Sniffed::Png);
    }

    #[test]
    fn only_the_pictures_that_show_are_fetched() {
        let file = |name: &str, kind| ChatFile {
            name: name.to_owned(),
            url: format!("/uploads/{name}"),
            kind,
            path: None,
        };
        assert!(is_fetched(&file("shot.png", FileKind::Image)));
        assert!(is_fetched(&file("clip.mov · frame 1.jpg", FileKind::Image)));
        assert!(!is_fetched(&file(
            "clip.mov · frame 2.jpg",
            FileKind::Image
        )));
        assert!(!is_fetched(&file("notes.pdf", FileKind::File)));
        assert_eq!(
            is_fetched(&file("IMG_0001.HEIC", FileKind::Image)),
            crate::heic::CONVERTS
        );
    }
}
