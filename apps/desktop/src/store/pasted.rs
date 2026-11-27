//! A pasted picture as the server takes one, as the Apple apps send it: named
//! `pasted-<seconds>.<extension>`, and in a format the server and the assistant read as a
//! picture. PNG, JPEG, GIF and WebP go as they are; anything else the clipboard holds (TIFF,
//! BMP, ICO, PNM) is re-encoded as PNG, as the Apple apps re-encode every paste. An SVG can't
//! be drawn into pixels here, so it goes as itself, a file.

use std::io::Cursor;
use std::sync::Arc;

use gpui_kit::{Image, ImageFormat};

use crate::backend::SharedBytes;

/// A pasted picture, ready to upload.
#[derive(Clone, Debug)]
pub(crate) struct Pasted {
    /// Its name: `pasted-1764000000.png`.
    pub(crate) name: String,
    /// The picture, in the format it is sent in.
    pub(crate) picture: Arc<Image>,
}

impl Pasted {
    /// `picture` pasted at `seconds` (since the Unix epoch), converted when it has to be.
    #[must_use]
    pub(crate) fn new(picture: Image, seconds: u64) -> Self {
        let picture = match picture.format() {
            ImageFormat::Png | ImageFormat::Jpeg | ImageFormat::Gif | ImageFormat::Webp => picture,
            _ => as_png(&picture).unwrap_or_else(|error| {
                log::warn!(
                    "couldn't turn a pasted {} picture into PNG ({error}); it goes as it is",
                    picture.format().extension()
                );
                picture
            }),
        };
        Self {
            name: format!("pasted-{seconds}.{}", picture.format().extension()),
            picture: Arc::new(picture),
        }
    }

    /// Its size in bytes.
    #[must_use]
    pub(crate) fn size(&self) -> u64 {
        u64::try_from(self.picture.bytes().len()).unwrap_or(u64::MAX)
    }

    /// Its bytes for the upload: the picture's own, not a copy.
    #[must_use]
    pub(crate) fn bytes(&self) -> SharedBytes {
        Arc::new(PictureBytes(Arc::clone(&self.picture)))
    }
}

/// A picture's bytes, shared with the picture.
struct PictureBytes(Arc<Image>);

impl AsRef<[u8]> for PictureBytes {
    fn as_ref(&self) -> &[u8] {
        self.0.bytes()
    }
}

/// `picture` re-encoded as PNG.
fn as_png(picture: &Image) -> Result<Image, image::ImageError> {
    let decoded = image::load_from_memory(picture.bytes())?;
    let mut png = Vec::new();
    decoded.write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png)?;
    Ok(Image::from_bytes(ImageFormat::Png, png))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 2×1 picture in `format`.
    fn picture(format: image::ImageFormat) -> Vec<u8> {
        let pixels = image::RgbImage::from_raw(2, 1, vec![255, 0, 0, 0, 0, 255]).unwrap();
        let mut bytes = Vec::new();
        pixels
            .write_to(&mut Cursor::new(&mut bytes), format)
            .unwrap();
        bytes
    }

    #[test]
    fn png_and_jpeg_go_as_they_are() {
        let png = picture(image::ImageFormat::Png);
        let pasted = Pasted::new(
            Image::from_bytes(ImageFormat::Png, png.clone()),
            1_764_000_000,
        );
        assert_eq!(pasted.name, "pasted-1764000000.png");
        assert_eq!(pasted.picture.bytes(), png);
        let pasted = Pasted::new(Image::from_bytes(ImageFormat::Jpeg, vec![0xFF, 0xD8]), 7);
        assert_eq!(pasted.name, "pasted-7.jpg");
    }

    #[test]
    fn tiff_and_bmp_become_png() {
        for (format, theirs) in [
            (ImageFormat::Tiff, image::ImageFormat::Tiff),
            (ImageFormat::Bmp, image::ImageFormat::Bmp),
        ] {
            let pasted = Pasted::new(Image::from_bytes(format, picture(theirs)), 7);
            assert_eq!(pasted.name, "pasted-7.png");
            assert_eq!(pasted.picture.format(), ImageFormat::Png);
            let back = image::load_from_memory(pasted.picture.bytes()).unwrap();
            assert_eq!((back.width(), back.height()), (2, 1));
        }
    }

    #[test]
    fn a_picture_that_doesnt_decode_goes_as_it_is() {
        let pasted = Pasted::new(
            Image::from_bytes(ImageFormat::Tiff, b"not a tiff".to_vec()),
            7,
        );
        assert_eq!(pasted.name, "pasted-7.tiff");
    }

    #[test]
    fn the_upload_reads_the_pictures_own_bytes() {
        let pasted = Pasted::new(Image::from_bytes(ImageFormat::Png, vec![1, 2, 3]), 7);
        assert_eq!((*pasted.bytes()).as_ref(), [1, 2, 3]);
        assert_eq!(pasted.size(), 3);
    }
}
