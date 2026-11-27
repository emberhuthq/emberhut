//! Folders and files only the user can read: what the app keeps from the server (chats, fetched
//! files, settings) is the user's own. On Unix a folder this makes is 0700 and a file 0600;
//! elsewhere the platform's defaults apply (a user's own app data folder).

use std::fs::File;
use std::io;
use std::path::Path;

/// Creates `folder` and the ones above it that are missing, private to the user. One that is
/// there already keeps its mode.
pub(crate) fn folder(folder: &Path) -> io::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    builder.create(folder)
}

/// Creates (or empties) `path`, readable by the user only.
pub(crate) fn file(path: &Path) -> io::Result<File> {
    let mut options = File::options();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options.open(path)
}

/// The mode bits of `path` (Unix; 0 when it can't be read), for the tests.
#[cfg(all(test, unix))]
pub(crate) fn mode(path: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::metadata(path).map_or(0, |metadata| metadata.permissions().mode() & 0o777)
}
