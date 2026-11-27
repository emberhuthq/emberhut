//! Writing the updater's small files: through a temporary file renamed over the old one, so a
//! reader sees the old file or the new one, never half of one.

use std::io;
use std::path::Path;

use serde::Serialize;

/// Writes `value` as JSON to `path`, atomically.
///
/// # Errors
///
/// The file couldn't be written or renamed into place.
pub(crate) fn write_json(path: &Path, value: &impl Serialize) -> io::Result<()> {
    let bytes = serde_json::to_vec_pretty(value).map_err(io::Error::other)?;
    let temporary = path.with_extension(format!("tmp-{}", std::process::id()));
    std::fs::write(&temporary, bytes)
        .and_then(|()| std::fs::rename(&temporary, path))
        .inspect_err(|_| {
            if let Err(error) = std::fs::remove_file(&temporary) {
                log::debug!("no temporary file to remove: {error}");
            }
        })
}
