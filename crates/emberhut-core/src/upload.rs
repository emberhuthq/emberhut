//! Files attached in the chat: what a stored file is called on disk, what kind it is, the type it
//! is served with, and which bytes of it a `Range` request asks for.
//!
//! The kind and the content type come from the name's extension; the stored name is 8 random hex
//! digits, `-`, and the name made safe; a request for a file reads one byte range at most
//! (`bytes=a-b`, `bytes=a-` or `bytes=-n`), and anything else gets the whole file.

use jiff::Timestamp;

use crate::ids::FileId;
use crate::random::{RandomError, RandomSource, bytes};

/// What kind of file it is, from its name's extension.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FileKind {
    /// A picture (`png jpg jpeg gif heic webp`).
    Image,
    /// A video (`mov mp4 m4v`).
    Video,
    /// Anything else.
    File,
}

impl FileKind {
    /// The kind of a file called `name`.
    ///
    /// ```
    /// use emberhut_core::upload::FileKind;
    ///
    /// assert_eq!(FileKind::of("Photo.JPG"), FileKind::Image);
    /// assert_eq!(FileKind::of("clip.mov"), FileKind::Video);
    /// assert_eq!(FileKind::of("notes.pdf"), FileKind::File);
    /// ```
    #[must_use]
    pub fn of(name: &str) -> Self {
        match content_type(name) {
            Some(kind) if kind.starts_with("image/") => Self::Image,
            Some(kind) if kind.starts_with("video") => Self::Video,
            _ => Self::File,
        }
    }

    /// The kind as the contract and the database write it: `image`, `video` or `file`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Image => "image",
            Self::Video => "video",
            Self::File => "file",
        }
    }
}

/// The content type of an HTML file: a report, served sandboxed ([`REPORT_POLICY`]).
pub const HTML: &str = "text/html; charset=utf-8";

/// The content type of a file whose extension isn't known.
pub const UNKNOWN_TYPE: &str = "application/octet-stream";

/// The `Content-Security-Policy` an HTML file is served with: an origin of its own (no cookies or
/// storage of the server's), its inline styles and scripts, pictures, and no requests, forms,
/// frames or framing. A page that could reach the server from the person's device would be let in
/// as them.
pub const REPORT_POLICY: &str = "sandbox allow-scripts allow-popups allow-popups-to-escape-sandbox; \
    default-src 'none'; img-src https: data: blob:; media-src data: blob:; \
    style-src 'unsafe-inline'; script-src 'unsafe-inline'; font-src data:; connect-src 'none'; \
    form-action 'none'; base-uri 'none'; frame-src 'none'; frame-ancestors 'none'";

/// The content type of a file called `name`, when its extension is one the server knows.
///
/// The extension is what follows the last `.`, in any case; a name without a `.` is read as all
/// extension.
#[must_use]
pub fn content_type(name: &str) -> Option<&'static str> {
    let extension = name.rsplit('.').next().unwrap_or(name).to_lowercase();
    Some(match extension.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "heic" => "image/heic",
        "webp" => "image/webp",
        "mov" => "video/quicktime",
        "mp4" => "video/mp4",
        "m4v" => "video/x-m4v",
        "pdf" => "application/pdf",
        "txt" => "text/plain",
        "csv" => "text/csv",
        "html" | "htm" => HTML,
        _ => return None,
    })
}

/// The most characters of a name a stored name keeps (its end, where the extension is).
const SAFE_NAME_LENGTH: usize = 120;

/// The stored name of a file with nothing usable in its name.
const UNNAMED: &str = "file";

/// `name` as a stored file's name keeps it: each run of characters other than ASCII letters and
/// digits, `_`, `.`, `-` and spaces becomes one `_`, and only the last 120 characters are kept;
/// `file` when nothing is left.
///
/// ```
/// use emberhut_core::upload::safe_name;
///
/// assert_eq!(safe_name("Week 3 (final).pdf"), "Week 3 _final_.pdf");
/// assert_eq!(safe_name("../../etc/passwd"), ".._.._etc_passwd");
/// assert_eq!(safe_name(""), "file");
/// ```
#[must_use]
pub fn safe_name(name: &str) -> String {
    let mut safe = String::with_capacity(name.len());
    let mut in_run = false;
    for c in name.chars() {
        if c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-' | ' ') {
            safe.push(c);
            in_run = false;
        } else if !in_run {
            safe.push('_');
            in_run = true;
        }
    }
    // Every character left is ASCII, so the byte index is a character index.
    let keep = safe.len().saturating_sub(SAFE_NAME_LENGTH);
    let kept = &safe[keep..];
    if kept.is_empty() {
        UNNAMED.to_owned()
    } else {
        kept.to_owned()
    }
}

/// A new stored name for a file called `name`: 8 random lower-case hex digits, `-`, and
/// [`safe_name`] of it.
///
/// # Errors
///
/// [`RandomError`] when `random` has no bytes.
pub fn stored_name(random: &dyn RandomSource, name: &str) -> Result<String, RandomError> {
    let prefix: [u8; 4] = bytes(random)?;
    let mut stored = String::with_capacity(9 + name.len());
    for byte in prefix {
        stored.push(hex_digit(byte >> 4));
        stored.push(hex_digit(byte & 0x0F));
    }
    stored.push('-');
    stored.push_str(&safe_name(name));
    Ok(stored)
}

/// Whether `name` (decoded from a request's path) may name a stored file: not empty, and no
/// `..`, path separator or NUL in it, so it can only name a file directly in the uploads folder.
///
/// ```
/// use emberhut_core::upload::is_stored_name;
///
/// assert!(is_stored_name("1a2b3c4d-photo.png"));
/// assert!(!is_stored_name("../emberhut.db"));
/// assert!(!is_stored_name("sub/photo.png"));
/// ```
#[must_use]
pub fn is_stored_name(name: &str) -> bool {
    !name.is_empty() && !name.contains("..") && !name.contains(['/', '\\', '\0'])
}

/// Whether `name` may name a file brought from a runner's machine: a folder of its own,
/// `<machine>-<8 lower-case hex digits>`, then `/` and a name [`is_stored_name`] takes. Nothing
/// else with a separator is ever served. A machine's name is letters, digits, `-`, `_` and `.`,
/// as a runner's name may be (the folder always ends in its hex digits, so it is never `.` or
/// `..`).
///
/// ```
/// use emberhut_core::upload::is_pulled_name;
///
/// assert!(is_pulled_name("build-1a2b3c4d/report.pdf"));
/// assert!(is_pulled_name("build.lan-1a2b3c4d/report.pdf"));
/// assert!(!is_pulled_name("build-1a2b3c4d/../emberhut.db"));
/// assert!(!is_pulled_name("secrets/key"));
/// assert!(!is_pulled_name("1a2b3c4d-photo.png"));
/// ```
#[must_use]
pub fn is_pulled_name(name: &str) -> bool {
    let Some((folder, file)) = name.split_once('/') else {
        return false;
    };
    let Some((machine, hex)) = folder.rsplit_once('-') else {
        return false;
    };
    !machine.is_empty()
        && machine
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        && hex.len() == 8
        && hex
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        && is_stored_name(file)
}

/// The address a stored file is served at: `/uploads/` and the stored name escaped as
/// JavaScript's `encodeURIComponent` escapes it (a space is `%20`).
///
/// ```
/// use emberhut_core::upload::url_of;
///
/// assert_eq!(url_of("1a2b3c4d-Week 3.png"), "/uploads/1a2b3c4d-Week%203.png");
/// ```
#[must_use]
pub fn url_of(stored_name: &str) -> String {
    let mut url = String::with_capacity(9 + stored_name.len());
    url.push_str("/uploads/");
    for byte in stored_name.bytes() {
        if byte.is_ascii_alphanumeric()
            || matches!(
                byte,
                b'-' | b'_' | b'.' | b'!' | b'~' | b'*' | b'\'' | b'(' | b')'
            )
        {
            url.push(char::from(byte));
        } else {
            url.push('%');
            url.push(hex_digit(byte >> 4).to_ascii_uppercase());
            url.push(hex_digit(byte & 0x0F).to_ascii_uppercase());
        }
    }
    url
}

/// The lower-case hex digit for a nibble (`0..=15`).
fn hex_digit(nibble: u8) -> char {
    char::from(b"0123456789abcdef"[usize::from(nibble & 0x0F)])
}

/// Which bytes of a file of a given size a request's `Range` header asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ByteRange {
    /// No range, or one the server doesn't read (several ranges, another unit): the whole file.
    Whole,
    /// Bytes `start..=end` (206 Partial Content).
    Part {
        /// The first byte.
        start: u64,
        /// The last byte, included.
        end: u64,
    },
    /// A range that starts at or past the end (416 Range Not Satisfiable).
    Unsatisfiable,
}

impl ByteRange {
    /// The range `header` asks for in a file of `size` bytes: `bytes=a-b` (`b` capped at the
    /// last byte), `bytes=a-` (from `a` to the end) or `bytes=-n` (the last `n`).
    ///
    /// ```
    /// use emberhut_core::upload::ByteRange;
    ///
    /// assert_eq!(ByteRange::of(Some("bytes=0-1"), 10), ByteRange::Part { start: 0, end: 1 });
    /// assert_eq!(ByteRange::of(Some("bytes=-3"), 10), ByteRange::Part { start: 7, end: 9 });
    /// assert_eq!(ByteRange::of(Some("bytes=10-"), 10), ByteRange::Unsatisfiable);
    /// assert_eq!(ByteRange::of(None, 10), ByteRange::Whole);
    /// ```
    #[must_use]
    pub fn of(header: Option<&str>, size: u64) -> Self {
        let Some((from, to)) = header
            .and_then(|header| header.strip_prefix("bytes="))
            .and_then(|range| range.split_once('-'))
            .filter(|(from, to)| all_digits(from) && all_digits(to))
        else {
            return Self::Whole;
        };
        let (start, end) = match (number(from), number(to)) {
            (None, None) => return Self::Whole,
            (Some(start), to) => (start, to),
            (None, Some(last)) => (size.saturating_sub(last), None),
        };
        if start >= size {
            return Self::Unsatisfiable;
        }
        let end = end.map_or(size - 1, |end| end.min(size - 1));
        if start > end {
            return Self::Unsatisfiable;
        }
        Self::Part { start, end }
    }
}

/// Whether `text` is ASCII digits only (or empty).
fn all_digits(text: &str) -> bool {
    text.bytes().all(|byte| byte.is_ascii_digit())
}

/// The number `digits` writes, or `None` when there are none; a number too big for 64 bits is
/// read as the biggest, which is past the end of any file.
fn number(digits: &str) -> Option<u64> {
    (!digits.is_empty()).then(|| digits.parse().unwrap_or(u64::MAX))
}

/// A file stored for the chat.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredFile {
    /// Its id.
    pub id: FileId,
    /// Its name in the uploads folder (see [`stored_name`]).
    pub stored_name: String,
    /// Its name as given, which the chat shows.
    pub name: String,
    /// Its kind, from the name.
    pub kind: FileKind,
    /// Its size in bytes.
    pub size: u64,
    /// When it was stored.
    pub created_at: Timestamp,
}

/// A file to note as stored; its id and time are given when it is noted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewFile {
    /// Its name in the uploads folder.
    pub stored_name: String,
    /// Its name as given.
    pub name: String,
    /// Its kind.
    pub kind: FileKind,
    /// Its size in bytes.
    pub size: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug)]
    struct Fixed(u8);

    impl RandomSource for Fixed {
        fn fill(&self, bytes: &mut [u8]) -> Result<(), RandomError> {
            bytes.fill(self.0);
            Ok(())
        }
    }

    #[test]
    fn kinds_and_types_come_from_the_extension() {
        assert_eq!(FileKind::of("a.heic"), FileKind::Image);
        assert_eq!(FileKind::of("a.webp"), FileKind::Image);
        assert_eq!(FileKind::of("a.M4V"), FileKind::Video);
        assert_eq!(FileKind::of("report.html"), FileKind::File);
        assert_eq!(FileKind::of("archive.tar.gz"), FileKind::File);
        assert_eq!(FileKind::of(""), FileKind::File);
        // No dot: the whole name is read as the extension.
        assert_eq!(FileKind::of("png"), FileKind::Image);
        assert_eq!(content_type("a.htm"), Some(HTML));
        assert_eq!(content_type("a.csv"), Some("text/csv"));
        assert_eq!(content_type("a.bin"), None);
        assert_eq!(FileKind::Video.as_str(), "video");
    }

    #[test]
    fn a_safe_name_folds_runs_and_keeps_the_end() {
        assert_eq!(safe_name("photo.png"), "photo.png");
        assert_eq!(safe_name("صورة.png"), "_.png");
        assert_eq!(
            safe_name("a_é"),
            "a__",
            "a run after an underscore is still one more"
        );
        assert_eq!(safe_name("a&&&b"), "a_b");
        let long = format!("{}.pdf", "x".repeat(200));
        let safe = safe_name(&long);
        assert_eq!(safe.len(), 120);
        assert!(safe.ends_with("x.pdf"));
        assert_eq!(safe_name("ééé"), "_");
    }

    #[test]
    fn a_stored_name_is_eight_hex_digits_then_the_safe_name() {
        assert_eq!(
            stored_name(&Fixed(0xAB), "Week 3/4.png").unwrap(),
            "abababab-Week 3_4.png"
        );
        assert_eq!(stored_name(&Fixed(0), "").unwrap(), "00000000-file");
    }

    #[test]
    fn only_names_in_the_folder_itself_are_served() {
        for bad in ["", "..", "a..b", "a/b", "a\\b", "a\0b"] {
            assert!(!is_stored_name(bad), "{bad:?}");
        }
        assert!(is_stored_name("1a2b3c4d-Week 3.png"));
    }

    #[test]
    fn urls_escape_like_encode_uri_component() {
        assert_eq!(url_of("a b+c%é"), "/uploads/a%20b%2Bc%25%C3%A9");
        assert_eq!(url_of("x!~*'()"), "/uploads/x!~*'()");
    }

    #[test]
    fn byte_ranges_are_read() {
        let part = |start, end| ByteRange::Part { start, end };
        assert_eq!(ByteRange::of(Some("bytes=2-5"), 10), part(2, 5));
        assert_eq!(ByteRange::of(Some("bytes=2-50"), 10), part(2, 9));
        assert_eq!(ByteRange::of(Some("bytes=4-"), 10), part(4, 9));
        assert_eq!(ByteRange::of(Some("bytes=-30"), 10), part(0, 9));
        assert_eq!(
            ByteRange::of(Some("bytes=-0"), 10),
            ByteRange::Unsatisfiable
        );
        assert_eq!(
            ByteRange::of(Some("bytes=5-2"), 10),
            ByteRange::Unsatisfiable
        );
        assert_eq!(
            ByteRange::of(Some("bytes=99999999999999999999-"), 10),
            ByteRange::Unsatisfiable
        );
        assert_eq!(ByteRange::of(Some("bytes=0-"), 0), ByteRange::Unsatisfiable);
        for whole in [
            "bytes=-",
            "bytes=0-1,4-5",
            "items=0-1",
            "bytes= 0-1",
            "bytes=a-b",
        ] {
            assert_eq!(ByteRange::of(Some(whole), 10), ByteRange::Whole, "{whole}");
        }
    }
}
