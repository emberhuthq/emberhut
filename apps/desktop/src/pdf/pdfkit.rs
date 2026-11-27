//! PDFs through `PDFKit`: the document read from its file, each page's crop box (turned when the
//! page is on its side), and a page drawn as a thumbnail of the asked width, handed over as
//! TIFF.

use std::path::Path;

use gpui_kit::ImageFormat;
use objc2::AnyThread as _;
use objc2::rc::Retained;
use objc2_foundation::{NSSize, NSURL};
use objc2_pdf_kit::{PDFDisplayBox, PDFDocument, PDFPage};

use super::{Drawn, Error};
use crate::ui::viewer::pages::PageSize;

/// The document at `path`.
#[expect(unsafe_code, reason = "PDFKit's calls are unsafe in objc2")]
fn open(path: &Path) -> Result<Retained<PDFDocument>, Error> {
    let url = NSURL::from_file_path(path)
        .ok_or_else(|| Error::Unreadable("its path isn't a file URL".to_owned()))?;
    // SAFETY: `initWithURL:` takes any file URL and gives nil for one that isn't a PDF.
    unsafe { PDFDocument::initWithURL(PDFDocument::alloc(), &url) }
        .ok_or_else(|| Error::Unreadable("PDFKit can't open it".to_owned()))
}

/// Page `index` of `document`.
#[expect(unsafe_code, reason = "PDFKit's calls are unsafe in objc2")]
fn page(document: &PDFDocument, index: usize) -> Result<Retained<PDFPage>, Error> {
    // SAFETY: an index past the last page gives nil, which is handled.
    unsafe { document.pageAtIndex(index) }.ok_or_else(|| Error::Page {
        page: index + 1,
        why: "it isn't there".to_owned(),
    })
}

/// `page`'s size as it shows: its crop box, turned when it is on its side.
#[expect(
    clippy::cast_possible_truncation,
    reason = "a page's points fit an f32 many times over"
)]
#[expect(unsafe_code, reason = "PDFKit's calls are unsafe in objc2")]
fn size_of(page: &PDFPage) -> PageSize {
    // SAFETY: plain getters on a page of an open document.
    let (bounds, rotation) =
        unsafe { (page.boundsForBox(PDFDisplayBox::CropBox), page.rotation()) };
    let (width, height) = (bounds.size.width as f32, bounds.size.height as f32);
    if rotation.rem_euclid(180) == 90 {
        PageSize {
            width: height,
            height: width,
        }
    } else {
        PageSize { width, height }
    }
}

/// Each page's size.
#[expect(unsafe_code, reason = "PDFKit's calls are unsafe in objc2")]
pub(super) fn pages(path: &Path) -> Result<Vec<PageSize>, Error> {
    let document = open(path)?;
    // SAFETY: a plain getter.
    let count = unsafe { document.pageCount() };
    if count == 0 {
        return Err(Error::Unreadable("it has no pages".to_owned()));
    }
    (0..count)
        .map(|index| page(&document, index).map(|page| size_of(&page)))
        .collect()
}

/// Page `index`, `width` pixels wide.
#[expect(unsafe_code, reason = "PDFKit's calls are unsafe in objc2")]
pub(super) fn draw(path: &Path, index: usize, width: u32) -> Result<Drawn, Error> {
    let document = open(path)?;
    let page = page(&document, index)?;
    let size = size_of(&page);
    let width = f64::from(width);
    let height = width * f64::from(size.height) / f64::from(size.width.max(1.));
    // SAFETY: PDFKit draws the page into a new picture of that size.
    let picture =
        unsafe { page.thumbnailOfSize_forBox(NSSize::new(width, height), PDFDisplayBox::CropBox) };
    let tiff = picture.TIFFRepresentation().ok_or_else(|| Error::Page {
        page: index + 1,
        why: "PDFKit drew nothing".to_owned(),
    })?;
    Ok(Drawn {
        bytes: tiff.to_vec(),
        format: ImageFormat::Tiff,
    })
}
