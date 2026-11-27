//! What a paste into the composer is, as the Apple apps tell: files copied in a file manager
//! attach as files, a picture attaches as a picture, and anything else pastes as text into the
//! field, however long it is (those apps never turn text into a file).
//!
//! What reaches the clipboard differs by platform (GPUI's readers), and macOS gives less than
//! the Apple apps see. There GPUI reads copied files, then text, then a picture: a copy that
//! holds both text and a picture is text, so a picture copied in a browser with its address
//! (Safari's Copy Image) pastes as the address, where the Apple apps attach the picture; a
//! screenshot, or a picture copied alone, attaches. Windows gives files, text and pictures;
//! Linux (X11 and Wayland) gives text and pictures, and copied files only as their paths in
//! text.

use std::path::PathBuf;

use gpui_kit::{ClipboardEntry, ClipboardItem, Image};

/// What a paste is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Pasted {
    /// Files copied in a file manager: attach them.
    Files(Vec<PathBuf>),
    /// A picture: attach it.
    Picture(Image),
    /// Text: the field pastes it.
    Text,
}

impl Pasted {
    /// What the clipboard's `item` is: files first, then a picture, else text.
    #[must_use]
    pub(crate) fn of(item: &ClipboardItem) -> Self {
        let files = item.entries().iter().find_map(|entry| match entry {
            ClipboardEntry::ExternalPaths(paths) if !paths.paths().is_empty() => {
                Some(paths.paths().to_vec())
            }
            _ => None,
        });
        if let Some(files) = files {
            return Self::Files(files);
        }
        let picture = item.entries().iter().find_map(|entry| match entry {
            ClipboardEntry::Image(image) => Some(image.clone()),
            _ => None,
        });
        picture.map_or(Self::Text, Self::Picture)
    }
}

#[cfg(test)]
mod tests {
    use gpui_kit::{ClipboardString, ExternalPaths, ImageFormat};

    use super::*;

    fn picture() -> Image {
        Image::from_bytes(ImageFormat::Png, b"\x89PNG".to_vec())
    }

    #[test]
    fn text_short_or_long_pastes_as_text() {
        assert_eq!(
            Pasted::of(&ClipboardItem::new_string("Hi".into())),
            Pasted::Text
        );
        let long = "a long paste ".repeat(5_000);
        assert_eq!(Pasted::of(&ClipboardItem::new_string(long)), Pasted::Text);
        let empty = ClipboardItem {
            entries: Vec::new(),
        };
        assert_eq!(Pasted::of(&empty), Pasted::Text);
    }

    #[test]
    fn a_picture_attaches() {
        let item = ClipboardItem::new_image(&picture());
        assert_eq!(Pasted::of(&item), Pasted::Picture(picture()));
    }

    #[test]
    fn copied_files_attach_before_their_text_and_any_picture() {
        let paths = vec![PathBuf::from("/files/a.pdf"), "/files/b.png".into()];
        let item = ClipboardItem {
            entries: vec![
                ClipboardEntry::String(ClipboardString::new("a.pdf b.png".into())),
                ClipboardEntry::Image(picture()),
                ClipboardEntry::ExternalPaths(ExternalPaths(paths.clone().into())),
            ],
        };
        assert_eq!(Pasted::of(&item), Pasted::Files(paths));
    }

    #[test]
    fn no_paths_is_no_files() {
        let item = ClipboardItem {
            entries: vec![
                ClipboardEntry::ExternalPaths(ExternalPaths::default()),
                ClipboardEntry::Image(picture()),
            ],
        };
        assert_eq!(Pasted::of(&item), Pasted::Picture(picture()));
    }
}
