//! The swap: two folders exchanged in one atomic step, so there is never a moment without a
//! `Emberhut.app`. On macOS this is `renameatx_np` with `RENAME_SWAP` (rustix's `EXCHANGE` maps to
//! it there), on Linux `renameat2` with `RENAME_EXCHANGE`.

use std::io;
use std::path::Path;

use rustix::fs::{CWD, RenameFlags, renameat_with};

/// Exchanges what `a` and `b` name, atomically; both must exist on the same volume.
///
/// # Errors
///
/// The system refused (one is missing, they are on different volumes, the file system can't
/// exchange).
pub(crate) fn exchange(a: &Path, b: &Path) -> io::Result<()> {
    renameat_with(CWD, a, CWD, b, RenameFlags::EXCHANGE).map_err(io::Error::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_folders_trade_places() {
        let dir = tempfile::tempdir().unwrap();
        let (app, new) = (
            dir.path().join("Emberhut.app"),
            dir.path().join(".Emberhut.app.new"),
        );
        std::fs::create_dir(&app).unwrap();
        std::fs::write(app.join("build"), "12").unwrap();
        std::fs::create_dir(&new).unwrap();
        std::fs::write(new.join("build"), "15").unwrap();
        match exchange(&app, &new) {
            Ok(()) => {}
            // A file system that can't exchange (some overlays): nothing to test here.
            Err(error) if error.raw_os_error() == Some(rustix::io::Errno::INVAL.raw_os_error()) => {
                return;
            }
            Err(error) => panic!("{error}"),
        }
        assert_eq!(std::fs::read_to_string(app.join("build")).unwrap(), "15");
        assert_eq!(std::fs::read_to_string(new.join("build")).unwrap(), "12");
        assert!(exchange(&app, &dir.path().join("missing")).is_err());
    }
}
