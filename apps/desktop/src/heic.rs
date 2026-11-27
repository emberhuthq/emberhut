//! Pictures GPUI's decoders don't read, HEIC from an iPhone above all, turned by the platform
//! into one they do. On macOS that is `ImageIO`, through `NSImage`, which reads every format the
//! system knows; elsewhere nothing here reads them (that needs libheif, a C library), so such a
//! picture shows as a file.

/// Whether this platform turns HEIC pictures into ones GPUI reads.
pub(crate) const CONVERTS: bool = cfg!(target_os = "macos");

/// `bytes`, a picture in a format the system reads, as TIFF; `None` when it can't be read (and
/// always off macOS). It decodes the whole picture: call it off the UI thread.
#[cfg(target_os = "macos")]
#[must_use]
pub(crate) fn to_tiff(bytes: &[u8]) -> Option<Vec<u8>> {
    use objc2::AnyThread as _;
    use objc2_app_kit::NSImage;
    use objc2_foundation::NSData;

    let data = NSData::with_bytes(bytes);
    let picture = NSImage::initWithData(NSImage::alloc(), &data)?;
    picture.TIFFRepresentation().map(|tiff| tiff.to_vec())
}

/// `bytes` as TIFF: never here (see the module).
#[cfg(not(target_os = "macos"))]
#[must_use]
pub(crate) fn to_tiff(_bytes: &[u8]) -> Option<Vec<u8>> {
    None
}

/// Whether the file called `name` is a HEIC or HEIF picture.
#[must_use]
pub(crate) fn is_heic(name: &str) -> bool {
    std::path::Path::new(name)
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("heic") || ext.eq_ignore_ascii_case("heif"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heic_is_known_by_its_extension() {
        assert!(is_heic("IMG_0001.HEIC"));
        assert!(is_heic("a.heif"));
        assert!(!is_heic("a.jpg"));
    }
}
