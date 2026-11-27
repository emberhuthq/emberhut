//! This device's name for the assistant's browser, kept across launches in `browser-device`
//! in the app's directory: the browser lets one device drive while the person's others watch,
//! so each must say which it is, the same every time.

use std::path::Path;

use emberhut_api::ids::BrowserDeviceId;

use crate::config::AppFiles;

/// The file's name in the app's directory.
const FILE: &str = "browser-device";

/// How a new id starts.
const PREFIX: &str = "zd-";

/// The longest id the server keeps whole.
const MAX_LEN: usize = 64;

/// This device's id: the kept one, else a new one, kept for next time (without `files`, as in
/// the tests, a new one each run).
#[must_use]
pub(crate) fn load(files: Option<&AppFiles>) -> BrowserDeviceId {
    let Some(files) = files else {
        return made();
    };
    let path = files.dir().join(FILE);
    if let Some(kept) = read(&path) {
        return kept;
    }
    let id = made();
    let written =
        std::fs::create_dir_all(files.dir()).and_then(|()| std::fs::write(&path, id.as_str()));
    if let Err(error) = written {
        log::warn!("couldn't keep this device's browser id: {error}");
    }
    id
}

/// The id kept at `path`, if one is there and is usable.
fn read(path: &Path) -> Option<BrowserDeviceId> {
    let text = std::fs::read_to_string(path).ok()?;
    usable(text.trim()).then(|| BrowserDeviceId::new(text.trim()))
}

/// Whether `id` can stand for this device: not empty (empty means no device), not over what
/// the server keeps, and plain characters only.
fn usable(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_LEN
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// A new id: [`PREFIX`] and 16 random bytes in hex; from the clock when the system has no
/// randomness (it only has to differ from the person's other devices).
fn made() -> BrowserDeviceId {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut bytes = [0_u8; 16];
    if let Err(error) = getrandom::getrandom(&mut bytes) {
        log::warn!("no randomness for this device's browser id ({error}): using the clock");
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |since| since.as_nanos());
        bytes = nanos.to_le_bytes();
    }
    let id = bytes.iter().fold(String::from(PREFIX), |mut id, byte| {
        id.push(char::from(HEX[usize::from(byte >> 4)]));
        id.push(char::from(HEX[usize::from(byte & 0xf)]));
        id
    });
    BrowserDeviceId::new(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_id_is_made_once_and_kept() {
        let dir = tempfile::tempdir().unwrap();
        let files = AppFiles::in_dir(dir.path());
        let first = load(Some(&files));
        assert!(first.as_str().starts_with(PREFIX));
        assert!(usable(first.as_str()));
        assert_eq!(load(Some(&files)), first, "the same next launch");
        assert_ne!(load(None), first, "without files, a new one");
    }

    #[test]
    fn a_kept_id_that_isnt_usable_is_made_again() {
        let dir = tempfile::tempdir().unwrap();
        let files = AppFiles::in_dir(dir.path());
        std::fs::write(dir.path().join(FILE), "  \n").unwrap();
        let id = load(Some(&files));
        assert!(usable(id.as_str()));
        assert_eq!(
            std::fs::read_to_string(dir.path().join(FILE)).unwrap(),
            id.as_str()
        );
        for bad in ["", "a b", "é", &"x".repeat(65)] {
            assert!(!usable(bad), "{bad:?}");
        }
        assert!(usable("zd-0a1b_C"));
    }
}
