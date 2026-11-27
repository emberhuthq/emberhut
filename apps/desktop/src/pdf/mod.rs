//! A PDF's pages as pictures, for the viewer: how many pages and how big, and one page drawn
//! at a width. GPUI draws no PDF, so the platform does: `PDFKit` on macOS, Poppler's command line
//! tools (`pdfinfo`, `pdftoppm`) elsewhere, where they are installed (they come with most Linux
//! desktops). Everything here blocks: call it off the UI thread.

#[cfg(target_os = "macos")]
mod pdfkit;
#[cfg(not(target_os = "macos"))]
mod poppler;

#[cfg(target_os = "macos")]
use self::pdfkit as platform;
#[cfg(not(target_os = "macos"))]
use self::poppler as platform;

use std::path::Path;

use gpui_kit::ImageFormat;

use crate::ui::viewer::pages::PageSize;

/// Why a PDF couldn't be read or drawn.
#[derive(Debug, thiserror::Error)]
pub(crate) enum Error {
    /// Nothing here draws PDFs (Poppler's tools aren't installed). `PDFKit` is always there
    /// on macOS, so only the other platforms have this case.
    #[cfg(not(target_os = "macos"))]
    #[error("PDFs show here once Poppler's tools (pdftoppm) are installed")]
    NoRenderer,
    /// The file isn't a PDF that can be read.
    #[error("couldn't read the PDF: {0}")]
    Unreadable(String),
    /// A page couldn't be drawn.
    #[error("couldn't draw page {page}: {why}")]
    Page {
        /// The page, from 1.
        page: usize,
        /// Why.
        why: String,
    },
}

/// A page drawn: its encoded picture.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Drawn {
    /// The picture's bytes.
    pub(crate) bytes: Vec<u8>,
    /// How they are encoded.
    pub(crate) format: ImageFormat,
}

/// The size of each page of the PDF at `path`, in points.
///
/// # Errors
///
/// When nothing draws PDFs here, or the file can't be read.
pub(crate) fn pages(path: &Path) -> Result<Vec<PageSize>, Error> {
    platform::pages(path)
}

/// Page `index` (from 0) of the PDF at `path`, drawn `width` pixels wide.
///
/// # Errors
///
/// When nothing draws PDFs here, or the page can't be drawn.
pub(crate) fn draw(path: &Path, index: usize, width: u32) -> Result<Drawn, Error> {
    platform::draw(path, index, width)
}
