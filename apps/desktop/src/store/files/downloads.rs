//! Saving a kept file where the user keeps theirs: into Downloads under its own name, or
//! `name (1).pdf`, `name (2).pdf`… when that is taken, as the system's own downloads do.

use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};

/// The user's Downloads folder, its links resolved; `None` when the system names none.
#[must_use]
pub(crate) fn downloads_dir() -> Option<PathBuf> {
    let dir = directories::UserDirs::new()?.download_dir()?.to_path_buf();
    Some(std::fs::canonicalize(&dir).unwrap_or(dir))
}

/// The `n`th name for `name` in a folder: the name itself first, then `stem (n).ext`.
#[must_use]
fn numbered(name: &str, n: u32) -> String {
    if n == 0 {
        return name.to_owned();
    }
    // A leading dot starts a hidden name, not an extension.
    match name.rfind('.').filter(|&dot| dot > 0) {
        Some(dot) => format!("{} ({n}){}", &name[..dot], &name[dot..]),
        None => format!("{name} ({n})"),
    }
}

/// Copies the file at `from` into `dir` as `name`, or the first numbered name free there; gives
/// where it went. A name taken meanwhile (another app saving the same) moves on to the next.
///
/// # Errors
///
/// When `from` can't be read or nothing can be written in `dir`.
pub(crate) fn save_copy(from: &Path, dir: &Path, name: &str) -> io::Result<PathBuf> {
    let mut source = File::open(from)?;
    for n in 0..10_000 {
        let to = dir.join(numbered(name, n));
        match File::options().write(true).create_new(true).open(&to) {
            Ok(mut file) => {
                let copied = io::copy(&mut source, &mut file).and_then(|_| file.sync_all());
                if let Err(error) = copied {
                    drop(file);
                    if let Err(removed) = std::fs::remove_file(&to) {
                        log::warn!("couldn't remove {}: {removed}", to.display());
                    }
                    return Err(error);
                }
                return Ok(to);
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "every numbered name is taken",
    ))
}

/// Copies the file at `from` to `to`, replacing what is there (the user picked it in a save
/// panel, which asked before replacing).
///
/// # Errors
///
/// When `from` can't be read or `to` can't be written.
pub(crate) fn save_to(from: &Path, to: &Path) -> io::Result<()> {
    let mut source = File::open(from)?;
    let mut file = File::create(to)?;
    io::copy(&mut source, &mut file)?;
    file.sync_all()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_taken_name_is_numbered_before_its_extension() {
        assert_eq!(numbered("review-pack.pdf", 0), "review-pack.pdf");
        assert_eq!(numbered("review-pack.pdf", 1), "review-pack (1).pdf");
        assert_eq!(numbered("archive.tar.gz", 2), "archive.tar (2).gz");
        assert_eq!(numbered("Makefile", 3), "Makefile (3)");
        assert_eq!(numbered(".env", 1), ".env (1)");
    }

    #[test]
    fn a_copy_never_overwrites_and_keeps_the_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let from = dir.path().join("kept.pdf");
        std::fs::write(&from, b"the file").unwrap();
        let out = dir.path().join("out");
        std::fs::create_dir(&out).unwrap();
        std::fs::write(out.join("a.pdf"), b"theirs").unwrap();
        let first = save_copy(&from, &out, "a.pdf").unwrap();
        assert_eq!(first, out.join("a (1).pdf"));
        assert_eq!(std::fs::read(&first).unwrap(), b"the file");
        assert_eq!(std::fs::read(out.join("a.pdf")).unwrap(), b"theirs");
        let second = save_copy(&from, &out, "a.pdf").unwrap();
        assert_eq!(second, out.join("a (2).pdf"));
        assert!(save_copy(&dir.path().join("gone"), &out, "a.pdf").is_err());
    }

    #[test]
    fn save_to_replaces_what_the_user_chose_to_replace() {
        let dir = tempfile::tempdir().unwrap();
        let from = dir.path().join("kept.txt");
        std::fs::write(&from, b"new").unwrap();
        let to = dir.path().join("mine.txt");
        std::fs::write(&to, b"old and longer").unwrap();
        save_to(&from, &to).unwrap();
        assert_eq!(std::fs::read(&to).unwrap(), b"new");
    }
}
