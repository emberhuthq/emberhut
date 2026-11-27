//! Which files on this machine may go into a page (`browser_upload`) and where a page's file may
//! be saved (`browser_download`): the owner's home (when there is one) or `/tmp`, symlinks
//! resolved; never a hidden
//! folder of the home (keys, credentials, browser profiles); in the server's own data folder
//! only what it made or was given; never the secrets.

use std::io::Read as _;
use std::path::{Component, Path, PathBuf};

use emberhut_core::browser::{UPLOAD_MAX, holds_key, key_like_name, size_text, upload_type};

/// In the server's data folder, only these may go into a page or take a download.
const DATA_OK: [&str; 6] = [
    "work",
    "files",
    "uploads",
    "browser/uploads",
    "shots",
    "research",
];

/// Where files may come from and go to.
#[derive(Clone, Debug)]
pub struct Places {
    /// The owner's home (`~`); none: only `/tmp` and the data folder's own folders.
    pub home: Option<PathBuf>,
    /// The server's data folder.
    pub data_dir: PathBuf,
    /// The server's secrets.
    pub secrets_dir: PathBuf,
}

/// A file that may go into a page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Uploadable {
    /// Its path, symlinks resolved.
    pub path: PathBuf,
    /// Its name.
    pub name: String,
    /// Its type.
    pub mime: &'static str,
    /// Its size in bytes.
    pub size: u64,
}

fn real(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

impl Places {
    /// `given` with a leading `~` as the home.
    fn expand(&self, given: &str) -> PathBuf {
        let given = given.trim();
        match &self.home {
            Some(home) if given == "~" => home.clone(),
            Some(home) if given.starts_with("~/") => home.join(&given[2..]),
            _ => PathBuf::from(given),
        }
    }

    /// Whether `path` (resolved) may be read from or written to.
    #[must_use]
    pub fn allowed(&self, path: &Path) -> bool {
        let data = real(&self.data_dir);
        let secrets = real(&self.secrets_dir);
        if path.starts_with(&secrets) {
            return false;
        }
        let home = self.home.as_deref().map(real);
        let in_data = path.starts_with(&data);
        let in_home = home.as_ref().is_some_and(|home| path.starts_with(home));
        if in_data && (!in_home || home.as_ref().is_some_and(|home| data.starts_with(home))) {
            return DATA_OK.iter().any(|ok| path.starts_with(data.join(ok)));
        }
        if let Some(rest) = home.as_ref().and_then(|home| path.strip_prefix(home).ok()) {
            return !rest.components().any(|c| match c {
                Component::Normal(part) => part.to_string_lossy().starts_with('.'),
                _ => false,
            });
        }
        // `/tmp` by its real path too (`/private/tmp` on macOS), as `path` is resolved.
        path.starts_with("/tmp") || path.starts_with(real(Path::new("/tmp")))
    }

    /// The file at `given` when it may go into a page; else why not, for the agent.
    ///
    /// # Errors
    ///
    /// What the agent is told.
    pub fn uploadable(&self, given: &str) -> Result<Uploadable, String> {
        let raw = self.expand(given);
        if !raw.is_absolute() {
            return Err(format!(
                "\"{given}\" isn't a path on this machine: give the full path (/… or ~/…)."
            ));
        }
        let shown = raw.display().to_string();
        let path = std::fs::canonicalize(&raw)
            .map_err(|_| format!("No file at {shown} on this machine."))?;
        let name = path
            .file_name()
            .map_or_default(|n| n.to_string_lossy().into_owned());
        let meta =
            std::fs::metadata(&path).map_err(|_| format!("No file at {shown} on this machine."))?;
        if !meta.is_file() {
            return Err(format!("{shown} isn't a file."));
        }
        if !self.allowed(&path) {
            let folder = path.parent().map_or_default(|p| p.display().to_string());
            return Err(format!(
                "{name} is never uploaded from there ({folder}): the server's secrets and logins, \
                 hidden folders (keys, credentials, browser profiles) and system folders stay on \
                 this machine. If the owner wants it uploaded, they do it themselves."
            ));
        }
        if key_like_name(&name) {
            return Err(format!(
                "{name} looks like a key, a password or credentials file by its name: never \
                 uploaded. If it's really something else, ask the owner to upload it themselves."
            ));
        }
        let ext = name
            .rsplit_once('.')
            .map_or_default(|(_, e)| e.to_lowercase());
        let Some(mime) = upload_type(&ext) else {
            let ext = if ext.is_empty() {
                "(no extension)".to_owned()
            } else {
                ext
            };
            return Err(format!(
                "{name}: only images, PDFs, documents (Word, Excel, PowerPoint, text, CSV) and \
                 zips can be uploaded, not .{ext} files."
            ));
        };
        let size = meta.len();
        if size > UPLOAD_MAX {
            return Err(format!(
                "{name} is {}: uploads stop at 50 MB.",
                size_text(size)
            ));
        }
        if size == 0 {
            return Err(format!("{name} is empty."));
        }
        let mut head = Vec::new();
        std::fs::File::open(&path)
            .and_then(|f| f.take(1_000_000).read_to_end(&mut head))
            .map_err(|_| format!("No file at {shown} on this machine."))?;
        let text: String = head.iter().map(|b| char::from(*b)).collect();
        if holds_key(&text, mime) {
            return Err(format!(
                "{name} holds a private key or an API token: never uploaded."
            ));
        }
        Ok(Uploadable {
            path,
            name,
            mime,
            size,
        })
    }

    /// Where a download named `name` goes for `given` (a folder, ending in `/` or existing, or a
    /// file path): made if missing, never over a file that's there (`name (2).pdf`).
    ///
    /// # Errors
    ///
    /// What the agent is told.
    pub fn dest_for(&self, given: &str, name: &str) -> Result<PathBuf, String> {
        let raw_text = given.trim();
        let raw = self.expand(raw_text);
        if !raw.is_absolute() {
            return Err(format!(
                "\"{given}\" isn't a path on this machine: give the full path (/… or ~/…)."
            ));
        }
        if raw.components().any(|c| c == Component::ParentDir) {
            return Err("No \"..\" in dest: give the folder itself.".to_owned());
        }
        let folder = raw_text.ends_with('/') || raw.is_dir();
        let (dir, file) = if folder {
            (raw.clone(), name.to_owned())
        } else {
            (
                raw.parent()
                    .map_or_else(|| PathBuf::from("/"), Path::to_path_buf),
                raw.file_name()
                    .map_or_else(|| name.to_owned(), |n| n.to_string_lossy().into_owned()),
            )
        };
        let mut have = dir.clone();
        let mut rest = Vec::new();
        while !have.exists() {
            let Some(last) = have.file_name().map(std::ffi::OsStr::to_os_string) else {
                break;
            };
            rest.push(last);
            if !have.pop() {
                break;
            }
        }
        let mut dir = real(&have);
        for part in rest.into_iter().rev() {
            dir.push(part);
        }
        if !self.allowed(&dir.join(&file)) {
            return Err(format!(
                "Nothing is saved into {}: only the home folder and /tmp, never a hidden folder, \
                 the server's secrets or its own data. Pick a folder in the home, or the task's \
                 folder.",
                dir.display()
            ));
        }
        std::fs::create_dir_all(&dir)
            .map_err(|e| format!("Couldn't make {}: {e}", dir.display()))?;
        let (stem, ext) = match file.rfind('.') {
            Some(at) if at > 0 => (file[..at].to_owned(), file[at..].to_owned()),
            _ => (file.clone(), String::new()),
        };
        let mut chosen = dir.join(&file);
        let mut n = 2;
        while chosen.exists() {
            chosen = dir.join(format!("{stem} ({n}){ext}"));
            n += 1;
        }
        Ok(chosen)
    }
}

/// A download's `.part` file at `part`, new and for the server's user only. One left there (by
/// an earlier download that stopped, or a link planted in its place) is removed first, never
/// followed.
///
/// # Errors
///
/// The file system's error.
pub(crate) async fn create_part(part: &Path) -> std::io::Result<tokio::fs::File> {
    match create_new(part).await {
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            tokio::fs::remove_file(part).await?;
            create_new(part).await
        }
        other => other,
    }
}

/// A new file at `path`, for the server's user only; one already there is an error.
async fn create_new(path: &Path) -> std::io::Result<tokio::fs::File> {
    let mut options = tokio::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    options.open(path).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn places(home: Option<&str>) -> Places {
        Places {
            home: home.map(PathBuf::from),
            data_dir: PathBuf::from("/srv/emberhut"),
            secrets_dir: PathBuf::from("/srv/emberhut/secrets"),
        }
    }

    #[test]
    fn with_no_home_only_tmp_and_the_data_folder_s_own_folders_are_allowed() {
        let none = places(None);
        assert!(none.allowed(Path::new("/tmp/report.pdf")));
        assert!(none.allowed(Path::new("/srv/emberhut/uploads/a.png")));
        assert!(!none.allowed(Path::new("/srv/emberhut/emberhut.db")));
        assert!(!none.allowed(Path::new("/srv/emberhut/secrets/key")));
        assert!(!none.allowed(Path::new("/etc/passwd")));
        assert!(!none.allowed(Path::new("/root/notes.txt")));
        assert_eq!(none.expand("~/a.pdf"), PathBuf::from("~/a.pdf"));
        assert!(none.uploadable("~/a.pdf").is_err());

        let home = places(Some("/home/someone"));
        assert!(home.allowed(Path::new("/home/someone/Documents/a.pdf")));
        assert!(!home.allowed(Path::new("/home/someone/.ssh/id")));
        assert!(!home.allowed(Path::new("/etc/passwd")));
        assert_eq!(home.expand("~/a.pdf"), PathBuf::from("/home/someone/a.pdf"));
    }
}
