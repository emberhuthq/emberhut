//! What never leaves the server: the files and folders no tool sends anywhere, into the chat
//! or to another machine.
//!
//! Each one is kept as given (made absolute) and as its real path, resolved once when the set
//! is made; a file is compared by its own real path, so a link, or a folder reached through
//! one, never takes a private file out.

use std::path::{Path, PathBuf};

/// Files and folders that never leave the server: a file is private when it is one of them or
/// inside one.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PrivatePaths {
    /// Each one, as given and as its real path.
    paths: Vec<PathBuf>,
}

impl PrivatePaths {
    /// The set of `paths` (files, or folders whose every file is private), each resolved to
    /// its real path now; an empty path is skipped.
    #[must_use]
    pub fn new(paths: impl IntoIterator<Item = PathBuf>) -> Self {
        Self {
            paths: paths
                .into_iter()
                .filter(|path| !path.as_os_str().is_empty())
                .flat_map(|path| forms(&path))
                .collect(),
        }
    }

    /// Whether the real path `real` (see [`tokio::fs::canonicalize`]) is private: one of the
    /// set, or inside one.
    #[must_use]
    pub fn holds(&self, real: &Path) -> bool {
        self.paths.iter().any(|path| real.starts_with(path))
    }
}

/// The names the user's private folders have in their home folder: keys and credentials.
pub const HOME_PRIVATE: [&str; 4] = [".ssh", ".gnupg", ".aws", ".config"];

/// The user's private folders in the home folder `home` ([`HOME_PRIVATE`]).
pub fn in_home(home: &Path) -> impl Iterator<Item = PathBuf> + '_ {
    HOME_PRIVATE.iter().map(move |dir| home.join(dir))
}

/// `path` as given (made absolute) and as its real path ([`resolved`]): so a path reached
/// through a link (`/tmp` is `/private/tmp` on macOS) is known by the real path its files have,
/// whether or not it is there yet.
#[must_use]
pub fn forms(path: &Path) -> Vec<PathBuf> {
    let absolute = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    let real = resolved(&absolute);
    let mut forms = vec![absolute];
    if let Some(real) = real
        && !forms.contains(&real)
    {
        forms.push(real);
    }
    forms
}

/// The absolute path `path` resolved: its nearest part that is there, links followed, then
/// the rest of it (the folders and the file not made yet). `None` when no part of it
/// resolves, or the rest isn't plain names.
#[must_use]
pub fn resolved(path: &Path) -> Option<PathBuf> {
    let mut rest = Vec::new();
    let mut at = path;
    loop {
        if let Ok(real) = std::fs::canonicalize(at) {
            return Some(rest.iter().rev().fold(real, |real, name| real.join(name)));
        }
        rest.push(at.file_name()?);
        at = at.parent()?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn a_private_folder_holds_its_files_and_a_link_to_one_is_caught_by_its_real_path() {
        let dir = tempfile::tempdir().unwrap();
        let secrets = dir.path().join("secrets");
        std::fs::create_dir(&secrets).unwrap();
        std::fs::write(secrets.join("key"), "k").unwrap();
        let elsewhere = dir.path().join("elsewhere");
        std::fs::create_dir(&elsewhere).unwrap();
        std::os::unix::fs::symlink(secrets.join("key"), elsewhere.join("link")).unwrap();
        let private = PrivatePaths::new([secrets.clone(), PathBuf::new()]);

        let real = std::fs::canonicalize(elsewhere.join("link")).unwrap();
        assert!(private.holds(&real), "the link's real path is the key's");
        assert!(private.holds(&secrets.join("key")));
        assert!(
            !private.holds(&elsewhere.join("link")),
            "compared by real path only"
        );
        assert!(
            !private.holds(&dir.path().join("secrets-old")),
            "a whole folder name"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_folder_reached_through_a_link_and_not_made_yet_is_known_by_its_real_path() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("real")).unwrap();
        std::os::unix::fs::symlink(dir.path().join("real"), dir.path().join("link")).unwrap();
        let keys = dir.path().join("link/home/.ssh");
        let private = PrivatePaths::new([keys.clone()]);

        std::fs::create_dir_all(&keys).unwrap();
        std::fs::write(keys.join("id"), "k").unwrap();
        let real = std::fs::canonicalize(keys.join("id")).unwrap();
        // The temporary folder may itself be reached through a link (`/var` is `/private/var`
        // on macOS), so the expected place is the real one too.
        let base = std::fs::canonicalize(dir.path()).unwrap();
        assert!(real.starts_with(base.join("real")), "{}", real.display());
        assert!(private.holds(&real), "made after the set, through a link");
        assert_eq!(
            resolved(&dir.path().join("link/a/b")),
            Some(base.join("real/a/b"))
        );
    }

    #[test]
    fn a_file_not_there_yet_is_known_by_its_folders_real_path() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("later.db");
        let real = std::fs::canonicalize(dir.path()).unwrap().join("later.db");
        assert!(PrivatePaths::new([file]).holds(&real));
    }
}
