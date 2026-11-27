//! What a task produced, on disk: which files and links a task may show, the ones a line of
//! its history names, and their thumbnails. A file is shown only when its resolved path (links
//! followed, `.` and `..` gone) is under the server user's home or a temporary folder, outside
//! the private folders (the server's secrets, the machine's keys and configuration), and of a
//! kind the app opens; it is served only by its artifact's id, never by path, and resolved
//! again each time it is served. A link with a key in it stays out.

use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use futures_util::future::BoxFuture;
use regex::Regex;

use crate::services::private::forms;

/// What a file is, by its extension: what the app opens it as, and its content type.
const KINDS: &[(&str, &str, &str)] = &[
    ("png", "image", "image/png"),
    ("jpg", "image", "image/jpeg"),
    ("jpeg", "image", "image/jpeg"),
    ("gif", "image", "image/gif"),
    ("webp", "image", "image/webp"),
    ("heic", "image", "image/heic"),
    ("mp4", "video", "video/mp4"),
    ("mov", "video", "video/quicktime"),
    ("m4v", "video", "video/x-m4v"),
    ("webm", "video", "video/webm"),
    ("pdf", "pdf", "application/pdf"),
    ("md", "md", "text/markdown; charset=utf-8"),
    ("html", "html", "text/html; charset=utf-8"),
    ("htm", "html", "text/html; charset=utf-8"),
    (
        "pptx",
        "doc",
        "application/vnd.openxmlformats-officedocument.presentationml.presentation",
    ),
    ("ppt", "doc", "application/vnd.ms-powerpoint"),
    (
        "docx",
        "doc",
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
    ),
    ("doc", "doc", "application/msword"),
    ("pages", "doc", "application/vnd.apple.pages"),
    (
        "xlsx",
        "doc",
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
    ),
    ("xls", "doc", "application/vnd.ms-excel"),
    ("numbers", "doc", "application/vnd.apple.numbers"),
];

fn extension(path: &str) -> String {
    path.rsplit('.').next().unwrap_or_default().to_lowercase()
}

/// The kind the app opens `path` as, when it opens it at all.
#[must_use]
pub fn kind_of(path: &str) -> Option<&'static str> {
    let ext = extension(path);
    KINDS
        .iter()
        .find(|(e, _, _)| *e == ext)
        .map(|(_, kind, _)| *kind)
}

/// The content type `path` is served with.
#[must_use]
pub fn content_type(path: &str) -> &'static str {
    let ext = extension(path);
    KINDS
        .iter()
        .find(|(e, _, _)| *e == ext)
        .map_or("application/octet-stream", |(_, _, t)| *t)
}

/// A pattern of this module's: none if it didn't compile (its tests say so), and then nothing
/// is found by it (and every link counts as a secret one).
fn rx(pattern: &str) -> Option<Regex> {
    Regex::new(pattern).ok()
}

static SECRET_LINK: LazyLock<Option<Regex>> =
    LazyLock::new(|| rx(r"(?i)[?&#](t|token|key|code|secret|sig|signature|access_token)="));
static LINK: LazyLock<Option<Regex>> = LazyLock::new(|| rx(r#"https://[^\s"'`<>()\[\]]+"#));
/// What follows a folder in a path a line names.
const PATH_TAIL: &str = r#"/[^\s"'`<>()\[\]]+"#;
static NAME: LazyLock<Option<Regex>> = LazyLock::new(|| {
    rx(
        r"(?i)\b\w[\w.-]*\.(?:png|jpe?g|gif|webp|heic|mp4|mov|m4v|webm|pdf|md|html?|pptx?|docx?|pages|xlsx?|numbers)\b",
    )
});

/// A path or a link as a sentence ends it, without the punctuation after it.
#[must_use]
pub fn clean(reference: &str) -> &str {
    reference
        .trim()
        .trim_end_matches(['.', ',', ';', ':', '!', '?', '\'', '"', '`', '*'])
}

/// Makes a small JPEG of a picture or a video.
pub trait Thumbnailer: std::fmt::Debug + Send + Sync {
    /// Writes a thumbnail of `source` (a video's from its first second) to `dest`.
    fn make<'a>(
        &'a self,
        source: &'a Path,
        dest: &'a Path,
        video: bool,
    ) -> BoxFuture<'a, Result<(), String>>;
}

/// Thumbnails with ffmpeg: one frame, at most 640 pixels wide.
#[derive(Clone, Debug)]
pub struct Ffmpeg {
    program: PathBuf,
}

impl Ffmpeg {
    /// ffmpeg at `program` (a name on the `PATH`, or a path).
    #[must_use]
    pub fn new(program: PathBuf) -> Self {
        Self { program }
    }

    async fn run(&self, source: &Path, dest: &Path, seek: bool) -> Result<(), String> {
        let mut command = tokio::process::Command::new(&self.program);
        command.args(["-y", "-loglevel", "error"]);
        if seek {
            command.args(["-ss", "1"]);
        }
        command
            .arg("-i")
            .arg(source)
            .args(["-frames:v", "1", "-vf", "scale='min(640,iw)':-2"])
            .arg(dest)
            .kill_on_drop(true);
        let status = tokio::time::timeout(Duration::from_secs(30), command.status())
            .await
            .map_err(|_| "ffmpeg took too long".to_owned())?
            .map_err(|error| error.to_string())?;
        if status.success() {
            Ok(())
        } else {
            Err(format!("ffmpeg ended with {status}"))
        }
    }
}

impl Thumbnailer for Ffmpeg {
    fn make<'a>(
        &'a self,
        source: &'a Path,
        dest: &'a Path,
        video: bool,
    ) -> BoxFuture<'a, Result<(), String>> {
        Box::pin(async move {
            if video && self.run(source, dest, true).await.is_ok() {
                return Ok(());
            }
            self.run(source, dest, false).await
        })
    }
}

/// The temporary folders a task's files may be in, besides the home.
const TEMPORARY: [&str; 2] = ["/tmp", "/var/tmp"];

/// The home's folders a task's files are never taken from.
const PRIVATE_IN_HOME: [&str; 4] = [".ssh", ".gnupg", ".aws", ".config"];

/// Where a task's files may come from, and where their thumbnails go.
#[derive(Clone, Debug)]
pub struct ArtifactFiles {
    home: Option<PathBuf>,
    /// The folders a file may be under, as given and resolved.
    roots: Vec<PathBuf>,
    /// A path under one of them, or under `~`, as a line names it.
    paths: Option<Regex>,
    /// The folders a file is never under, resolved (and as given, for one not made yet).
    private: Vec<PathBuf>,
    thumbs: PathBuf,
    thumbnailer: Arc<dyn Thumbnailer>,
}

impl ArtifactFiles {
    /// Files under `home` (and the temporary folders), never under `secrets_dir` or the home's
    /// private folders; thumbnails kept in `thumbs`, made by `thumbnailer`. The folders are
    /// resolved once, here, as the server starts.
    #[must_use]
    pub fn new(
        home: Option<PathBuf>,
        secrets_dir: &Path,
        thumbs: PathBuf,
        thumbnailer: Arc<dyn Thumbnailer>,
    ) -> Self {
        let mut roots: Vec<PathBuf> = TEMPORARY
            .iter()
            .flat_map(|dir| forms(Path::new(dir)))
            .collect();
        let mut private = forms(secrets_dir);
        if let Some(home) = &home {
            roots.extend(forms(home));
            for dir in PRIVATE_IN_HOME {
                private.extend(forms(&home.join(dir)));
            }
        }
        let named: Vec<String> = std::iter::once("~".to_owned())
            .chain(
                roots
                    .iter()
                    .map(|root| regex::escape(root.to_string_lossy().trim_end_matches('/'))),
            )
            .collect();
        let paths = rx(&format!("(?:{}){PATH_TAIL}", named.join("|")));
        Self {
            home,
            roots,
            paths,
            private,
            thumbs,
            thumbnailer,
        }
    }

    /// `reference` as a full path: `~/` is the home.
    #[must_use]
    pub fn full(&self, reference: &str) -> String {
        match (reference.strip_prefix("~/"), &self.home) {
            (Some(rest), Some(home)) => home.join(rest).display().to_string(),
            _ => reference.to_owned(),
        }
    }

    /// The file `reference` (`~/` is the home) is, resolved, when a task may show it: a file
    /// whose resolved path is under a place it may come from (whole folders, not a prefix of
    /// their names), outside the private folders, and of a kind the app opens by that path's
    /// own extension.
    pub async fn resolve(&self, reference: &str) -> Option<PathBuf> {
        let resolved = tokio::fs::canonicalize(self.full(reference)).await.ok()?;
        let allowed = self.roots.iter().any(|root| resolved.starts_with(root))
            && !self.private.iter().any(|dir| resolved.starts_with(dir));
        if !allowed || kind_of(resolved.to_str()?).is_none() {
            return None;
        }
        let meta = tokio::fs::metadata(&resolved).await.ok()?;
        meta.is_file().then_some(resolved)
    }

    /// The kind `reference` is attached as (a file's, or `link`), when it may be attached; a
    /// file as its resolved path.
    pub async fn attachable(&self, reference: &str) -> Option<(String, &'static str)> {
        let reference = clean(reference);
        if reference.starts_with("https://") {
            let secret = SECRET_LINK.as_ref().is_none_or(|re| re.is_match(reference));
            return (!secret).then(|| (reference.to_owned(), "link"));
        }
        let path = self.resolve(reference).await?.to_str()?.to_owned();
        kind_of(&path).map(|kind| (path, kind))
    }

    /// Opens the file `reference` is, when a task may show it, and makes sure the file opened
    /// is the one that was checked: its path resolves to itself again, to the same file.
    pub async fn open(&self, reference: &str) -> Option<(tokio::fs::File, &'static str)> {
        let resolved = self.resolve(reference).await?;
        let file = tokio::fs::File::open(&resolved).await.ok()?;
        let opened = file.metadata().await.ok()?;
        let again = tokio::fs::canonicalize(&resolved).await.ok()?;
        let there = tokio::fs::symlink_metadata(&again).await.ok()?;
        (again == resolved && same_file(&opened, &there))
            .then(|| (file, content_type(resolved.to_str().unwrap_or_default())))
    }

    /// The files and links a line names: absolute paths (`~` too), names next to the folder
    /// they are in, and https links. `files` false: links only (a remote session's paths are
    /// on its own machine).
    pub async fn named_in(&self, text: &str, files: bool) -> Vec<String> {
        let mut found = Vec::new();
        if files {
            let paths: Vec<String> = self
                .paths
                .iter()
                .flat_map(|re| re.find_iter(text))
                .map(|m| m.as_str().to_owned())
                .collect();
            found.extend(paths.iter().cloned());
            let mut dirs = Vec::new();
            for path in &paths {
                let dir =
                    self.full(path.trim_end_matches([
                        '.', ',', ';', ':', '!', '?', '\'', '"', '`', '*', '/',
                    ]));
                if is_dir(Path::new(&dir)).await {
                    dirs.push(dir);
                }
            }
            if !dirs.is_empty() {
                for name in NAME.iter().flat_map(|re| re.find_iter(text)) {
                    for dir in &dirs {
                        found.push(format!("{dir}/{}", name.as_str()));
                    }
                }
            }
        }
        found.extend(
            LINK.iter()
                .flat_map(|re| re.find_iter(text))
                .map(|m| m.as_str().to_owned()),
        );
        found
    }

    /// The thumbnail of the picture or video at `source` (a resolved path), made once, as
    /// artifact `id`'s.
    ///
    /// # Errors
    ///
    /// Why it could not be made.
    pub async fn thumbnail(&self, id: i64, source: &Path, video: bool) -> Result<PathBuf, String> {
        let dest = self.thumbs.join(format!("{id}.jpg"));
        if is_file(&dest).await {
            return Ok(dest);
        }
        tokio::fs::create_dir_all(&self.thumbs)
            .await
            .map_err(|error| error.to_string())?;
        // Made under a name of its own and moved into place whole, so a request at the same
        // time never serves a half-written one.
        let making = self.thumbs.join(format!(
            "{id}.{}.making.jpg",
            MAKING.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let made = self.thumbnailer.make(source, &making, video).await;
        if made.is_ok() && is_file(&making).await {
            tokio::fs::rename(&making, &dest)
                .await
                .map_err(|error| error.to_string())?;
            return Ok(dest);
        }
        if let Err(error) = tokio::fs::remove_file(&making).await
            && error.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!(%error, "a thumbnail being made was not removed");
        }
        made.and(Err("no thumbnail was written".to_owned()))
    }
}

/// Tells apart the thumbnails being made at once.
static MAKING: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Whether `path` is a folder, without blocking the runtime.
async fn is_dir(path: &Path) -> bool {
    tokio::fs::metadata(path)
        .await
        .is_ok_and(|meta| meta.is_dir())
}

/// Whether `path` is a file, without blocking the runtime.
async fn is_file(path: &Path) -> bool {
    tokio::fs::metadata(path)
        .await
        .is_ok_and(|meta| meta.is_file())
}

/// Whether two files' metadata are the same file.
#[cfg(unix)]
fn same_file(a: &std::fs::Metadata, b: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt as _;
    a.dev() == b.dev() && a.ino() == b.ino()
}

/// Whether two files' metadata are the same file: without inode numbers, the same kind, size
/// and time.
#[cfg(not(unix))]
fn same_file(a: &std::fs::Metadata, b: &std::fs::Metadata) -> bool {
    a.is_file() == b.is_file() && a.len() == b.len() && a.modified().ok() == b.modified().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_pattern_compiles() {
        assert!(SECRET_LINK.is_some(), "SECRET_LINK");
        assert!(LINK.is_some(), "LINK");
        assert!(NAME.is_some(), "NAME");
    }

    #[derive(Debug)]
    struct NoThumbs;

    impl Thumbnailer for NoThumbs {
        fn make<'a>(
            &'a self,
            _: &'a Path,
            _: &'a Path,
            _: bool,
        ) -> BoxFuture<'a, Result<(), String>> {
            Box::pin(async { Err("none".to_owned()) })
        }
    }

    fn files_in(home: &Path) -> ArtifactFiles {
        ArtifactFiles::new(
            Some(home.to_path_buf()),
            &home.join("secrets"),
            home.join("thumbs"),
            Arc::new(NoThumbs),
        )
    }

    #[tokio::test]
    async fn only_files_in_their_places_and_links_without_keys_are_shown() {
        let home = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(home.path().join("secrets")).unwrap();
        std::fs::create_dir_all(home.path().join("out")).unwrap();
        std::fs::write(home.path().join("out/chart.png"), b"png").unwrap();
        std::fs::write(home.path().join("out/notes.txt"), b"txt").unwrap();
        std::fs::write(home.path().join("secrets/key.png"), b"png").unwrap();
        let files = files_in(home.path());
        let real = std::fs::canonicalize(home.path()).unwrap();
        let chart = format!("{}/out/chart.png", real.display());
        assert_eq!(
            files
                .attachable(&format!("{}/out/chart.png.", home.path().display()))
                .await,
            Some((chart.clone(), "image"))
        );
        assert_eq!(
            files
                .attachable(&format!("{}/out/notes.txt", home.path().display()))
                .await,
            None
        );
        assert_eq!(
            files
                .attachable(&format!("{}/secrets/key.png", home.path().display()))
                .await,
            None
        );
        assert_eq!(files.attachable("/etc/chart.png").await, None);
        assert_eq!(
            files.attachable("https://example.com/pr/3,").await,
            Some(("https://example.com/pr/3".to_owned(), "link"))
        );
        assert_eq!(
            files.attachable("https://example.com/s?token=abc").await,
            None
        );
        let named = files
            .named_in(
                &format!(
                    "Made chart.png in {}/out and https://example.com/demo",
                    home.path().display()
                ),
                true,
            )
            .await;
        assert!(
            named.contains(&format!("{}/out/chart.png", home.path().display())),
            "{named:?}"
        );
        assert!(named.contains(&"https://example.com/demo".to_owned()));
        assert_eq!(
            files.named_in("see /tmp/x.png", false).await,
            Vec::<String>::new()
        );
        assert_eq!(content_type("a.MP4"), "video/mp4");
        assert_eq!(kind_of("deck.pptx"), Some("doc"));
    }

    /// A folder reached through a link, as macOS's temporary folders are (`/tmp` is
    /// `/private/tmp`, `/var/folders` is `/private/var/folders`).
    #[cfg(unix)]
    fn linked_dir() -> (tempfile::TempDir, PathBuf) {
        let real = tempfile::tempdir().unwrap();
        std::fs::create_dir(real.path().join("real")).unwrap();
        std::os::unix::fs::symlink(real.path().join("real"), real.path().join("link")).unwrap();
        let linked = real.path().join("link");
        (real, linked)
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_home_reached_through_a_link_and_made_later_shows_its_files_and_keeps_its_secrets() {
        let (_dir, linked) = linked_dir();
        let home = linked.join("home");
        // Made before the home is: its folders resolve through the link all the same.
        let files = files_in(&home);
        std::fs::create_dir_all(home.join("out")).unwrap();
        std::fs::create_dir_all(home.join(".ssh")).unwrap();
        std::fs::create_dir_all(home.join("secrets")).unwrap();
        std::fs::write(home.join("out/chart.png"), b"png").unwrap();
        std::fs::write(home.join(".ssh/id.png"), b"key").unwrap();
        std::fs::write(home.join("secrets/key.png"), b"key").unwrap();
        let real = std::fs::canonicalize(&home).unwrap();
        assert_ne!(real, home, "the home is reached through a link");

        let chart = format!("{}/out/chart.png", real.display());
        for written in [format!("{}/out/chart.png", home.display()), chart.clone()] {
            assert_eq!(
                files.attachable(&written).await,
                Some((chart.clone(), "image")),
                "{written}"
            );
        }
        for written in [
            home.join(".ssh/id.png"),
            real.join(".ssh/id.png"),
            home.join("secrets/key.png"),
            real.join("secrets/key.png"),
        ] {
            let written = written.display().to_string();
            assert_eq!(files.attachable(&written).await, None, "{written}");
        }
        let named = files
            .named_in(
                &format!(
                    "Made {}/out/chart.png and {}/out",
                    home.display(),
                    real.display()
                ),
                true,
            )
            .await;
        assert!(
            named.contains(&format!("{}/out/chart.png", home.display())),
            "{named:?}"
        );
        assert!(named.contains(&chart), "{named:?}");
    }

    #[tokio::test]
    async fn a_private_file_is_refused_however_its_path_is_written() {
        let home = tempfile::tempdir().unwrap();
        let config = home.path().join(".config");
        std::fs::create_dir_all(&config).unwrap();
        std::fs::create_dir_all(home.path().join("secrets")).unwrap();
        std::fs::create_dir_all(home.path().join("out")).unwrap();
        std::fs::write(config.join("x.md"), b"private").unwrap();
        std::fs::write(home.path().join("secrets/key.md"), b"private").unwrap();
        let files = files_in(home.path());
        let root = home.path().display();
        for written in [
            format!("{root}/./.config/x.md"),
            format!("{root}//.config/x.md"),
            format!("{root}/out/../.config/x.md"),
            format!("{root}/out/../secrets/key.md"),
        ] {
            assert_eq!(files.attachable(&written).await, None, "{written}");
            assert_eq!(files.resolve(&written).await, None, "{written}");
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_link_to_a_private_file_is_refused() {
        let home = tempfile::tempdir().unwrap();
        let config = home.path().join(".config");
        std::fs::create_dir_all(&config).unwrap();
        std::fs::create_dir_all(home.path().join("out")).unwrap();
        std::fs::write(config.join("x.md"), b"private").unwrap();
        let into_private = home.path().join("out/a.md");
        std::os::unix::fs::symlink(config.join("x.md"), &into_private).unwrap();
        let files = files_in(home.path());
        assert_eq!(
            files.attachable(&into_private.display().to_string()).await,
            None
        );
        assert!(
            files
                .open(&into_private.display().to_string())
                .await
                .is_none()
        );
        // A link whose own name looks like a picture, to a file of another kind, is not one.
        std::fs::write(home.path().join("out/notes.txt"), b"text").unwrap();
        let named_png = home.path().join("out/b.png");
        std::os::unix::fs::symlink(home.path().join("out/notes.txt"), &named_png).unwrap();
        assert_eq!(
            files.attachable(&named_png.display().to_string()).await,
            None
        );
    }

    #[tokio::test]
    async fn a_file_is_opened_by_its_resolved_path() {
        let home = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(home.path().join("out")).unwrap();
        std::fs::write(home.path().join("out/chart.png"), b"png").unwrap();
        let files = files_in(home.path());
        let (_, content) = files
            .open(&format!("{}/out/./chart.png", home.path().display()))
            .await
            .unwrap();
        assert_eq!(content, "image/png");
    }
}
