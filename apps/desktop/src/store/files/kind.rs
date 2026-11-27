//! What kind of file a file is, from its name: it decides how the viewer shows it and which of
//! its uses are offered (only pictures and text are copied, only pictures, PDFs and notes zoom).
//! Apart from its kind, a file may run as a program when the system opens it (a script, an app,
//! an installer, a link file: [`runs_as_program`]); such a file is never handed to the system
//! to open, only shown in its folder.

use std::path::Path;

/// How the viewer shows a file.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Kind {
    /// A picture, to zoom.
    Picture,
    /// A video, to play.
    Video,
    /// A PDF, its pages one after another.
    Pdf,
    /// An HTML report, for the report panel.
    Report,
    /// Text: a note in markdown, or plain text.
    Note,
    /// Anything else: a deck, a document, a sheet, an archive.
    Document,
}

/// Picture extensions.
const PICTURES: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "webp", "heic", "heif", "bmp", "tif", "tiff", "ico", "svg",
];

/// Video extensions.
const VIDEOS: &[&str] = &["mov", "mp4", "m4v", "avi", "mkv", "webm"];

/// Text extensions: notes, data and code, read as text.
const TEXTS: &[&str] = &[
    "md", "markdown", "txt", "text", "log", "csv", "tsv", "json", "xml", "yaml", "yml", "toml",
    "ini", "conf", "cfg", "sh", "py", "rs", "swift", "js", "ts", "css", "c", "h", "sql",
];

/// Markdown extensions, drawn formatted.
const MARKDOWN: &[&str] = &["md", "markdown"];

/// The types of file the system runs, installs or follows when it opens one, on any platform.
const PROGRAMS: &[&str] = &[
    // macOS
    "app",
    "command",
    "tool",
    "terminal",
    "pkg",
    "mpkg",
    "dmg",
    "workflow",
    "action",
    "scpt",
    "scptd",
    "applescript",
    "osax",
    "prefpane",
    "kext",
    "plugin",
    "mobileconfig",
    "webloc",
    "inetloc",
    "fileloc",
    // Linux and other Unixes
    "sh",
    "bash",
    "zsh",
    "csh",
    "tcsh",
    "ksh",
    "fish",
    "desktop",
    "appimage",
    "run",
    "bin",
    "deb",
    "rpm",
    "snap",
    "flatpak",
    "flatpakref",
    // Windows
    "exe",
    "msi",
    "msix",
    "msp",
    "bat",
    "cmd",
    "com",
    "scr",
    "pif",
    "cpl",
    "msc",
    "ps1",
    "psm1",
    "vbs",
    "vbe",
    "js",
    "jse",
    "wsf",
    "wsh",
    "hta",
    "lnk",
    "reg",
    "inf",
    "url",
    "appx",
    "appref-ms",
    "application",
    "gadget",
    "jar",
    // Scripts a system may run on a double click
    "py",
    "pyw",
    "pl",
    "rb",
    "php",
    "lua",
    "tcl",
];

/// Whether the file at `path` (a name, or a server address's path) could run as a program
/// when the system opens it: by its extension on any platform, and on macOS also by the type
/// the system gives that extension ([`system_runs`]).
#[must_use]
pub(crate) fn runs_as_program(path: &str) -> bool {
    let path = path.split(['?', '#']).next().unwrap_or(path);
    let extension = extension(path);
    !extension.is_empty() && (PROGRAMS.contains(&extension.as_str()) || system_runs(&extension))
}

/// The system's types that run, install or lead somewhere when opened: a file whose type
/// conforms to one of them is a program.
#[cfg(target_os = "macos")]
const RUNNING_TYPES: &[&str] = &[
    "public.executable",
    "public.script",
    "public.shell-script",
    "com.apple.application",
    "com.apple.application-bundle",
    "com.apple.terminal.session",
    "com.apple.internet-location",
    "com.apple.web-internet-location",
    "com.apple.file-internet-location",
    "com.apple.installer-package-archive",
    "com.apple.automator-workflow",
];

/// Whether macOS gives files with `extension` a type that runs ([`RUNNING_TYPES`]), as
/// Launch Services would treat them; types this list doesn't name yet are caught too.
#[cfg(target_os = "macos")]
fn system_runs(extension: &str) -> bool {
    use objc2_foundation::NSString;
    use objc2_uniform_type_identifiers::UTType;

    let Some(kind) = UTType::typeWithFilenameExtension(&NSString::from_str(extension)) else {
        return false;
    };
    RUNNING_TYPES.iter().any(|identifier| {
        UTType::typeWithIdentifier(&NSString::from_str(identifier))
            .is_some_and(|running| kind.conformsToType(&running))
    })
}

/// Elsewhere the extension list alone says ([`PROGRAMS`]).
#[cfg(not(target_os = "macos"))]
fn system_runs(_extension: &str) -> bool {
    false
}

/// The lower-case extension of a file name.
fn extension(name: &str) -> String {
    Path::new(name).extension().map_or_else(String::new, |ext| {
        ext.to_string_lossy().to_ascii_lowercase()
    })
}

impl Kind {
    /// The kind of the file called `name`.
    #[must_use]
    pub(crate) fn of(name: &str) -> Self {
        let extension = extension(name);
        let is = |list: &[&str]| list.contains(&extension.as_str());
        if is(PICTURES) {
            Self::Picture
        } else if is(VIDEOS) {
            Self::Video
        } else if extension == "pdf" {
            Self::Pdf
        } else if extension == "html" || extension == "htm" {
            Self::Report
        } else if is(TEXTS) {
            Self::Note
        } else {
            Self::Document
        }
    }

    /// Whether the viewer zooms it (⌘+, ⌘−, ⌘0).
    #[must_use]
    pub(crate) fn zooms(self) -> bool {
        matches!(self, Self::Picture | Self::Pdf | Self::Note)
    }

    /// Whether Copy is offered: a picture goes on the clipboard as a picture, text (a report's
    /// page too) as text.
    #[must_use]
    pub(crate) fn copies(self) -> bool {
        matches!(self, Self::Picture | Self::Note | Self::Report)
    }
}

/// Whether the file called `name` is markdown, drawn formatted rather than as plain text.
#[must_use]
pub(crate) fn is_markdown(name: &str) -> bool {
    MARKDOWN.contains(&extension(name).as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_is_known_by_its_extension_in_any_case() {
        assert_eq!(Kind::of("IMG_0001.HEIC"), Kind::Picture);
        assert_eq!(Kind::of("shot.png"), Kind::Picture);
        assert_eq!(Kind::of("clip.MOV"), Kind::Video);
        assert_eq!(Kind::of("review-pack.pdf"), Kind::Pdf);
        assert_eq!(Kind::of("week.HTML"), Kind::Report);
        assert_eq!(Kind::of("notes.md"), Kind::Note);
        assert_eq!(Kind::of("data.csv"), Kind::Note);
        assert_eq!(Kind::of("deck.pptx"), Kind::Document);
        assert_eq!(Kind::of("Makefile"), Kind::Document);
    }

    #[test]
    fn pictures_pdfs_and_notes_zoom_and_only_pictures_and_text_copy() {
        assert!(Kind::Picture.zooms() && Kind::Pdf.zooms() && Kind::Note.zooms());
        assert!(!Kind::Video.zooms() && !Kind::Document.zooms());
        assert!(Kind::Picture.copies() && Kind::Note.copies() && Kind::Report.copies());
        assert!(!Kind::Pdf.copies() && !Kind::Video.copies() && !Kind::Document.copies());
    }

    #[test]
    fn programs_are_known_by_their_type_in_any_case() {
        for program in [
            "a.command",
            "b.SH",
            "c.app",
            "d.pkg",
            "e.exe",
            "f.desktop",
            "g.ps1",
            "h.terminal",
            "i.fileloc",
            "/uploads/run.BAT?x=1",
        ] {
            assert!(runs_as_program(program), "{program}");
        }
        for file in [
            "a.pdf",
            "b.png",
            "c.txt",
            "d",
            "e.html",
            "notes.md",
            "/api/artifacts/66",
        ] {
            assert!(!runs_as_program(file), "{file}");
        }
    }

    #[test]
    fn only_markdown_is_drawn_formatted() {
        assert!(is_markdown("plan.MD"));
        assert!(!is_markdown("log.txt"));
    }
}
