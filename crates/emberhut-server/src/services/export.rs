//! The memory export (`GET /api/memory/export`, ADR 0034): everything the assistant keeps about
//! the owner, as one zip they can read and take elsewhere: the context folder as it is (one
//! markdown file per fact, the profile, the tombstones of what was forgotten) under `context/`,
//! and `memory.json` (the quick notes and diary, the corrections, the standing rules and the
//! rules on trial).

use std::fs;
use std::io::{self, Cursor, Write as _};
use std::path::Path;

use serde_json::Value;
use zip::ZipWriter;
use zip::write::SimpleFileOptions;

/// The zip of the context folder `folder` and `memory`, as bytes.
///
/// # Errors
///
/// When a file can't be read or the zip written.
pub fn zip(folder: &Path, memory: &Value) -> io::Result<Vec<u8>> {
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let mut files = Vec::new();
    walk(folder, folder, &mut files)?;
    files.sort();
    for relative in files {
        let name = format!("context/{}", relative.replace('\\', "/"));
        zip.start_file(name, options).map_err(io::Error::other)?;
        zip.write_all(&fs::read(folder.join(&relative))?)?;
    }
    zip.start_file("memory.json", options)
        .map_err(io::Error::other)?;
    zip.write_all(
        serde_json::to_string_pretty(memory)
            .map_err(io::Error::other)?
            .as_bytes(),
    )?;
    Ok(zip.finish().map_err(io::Error::other)?.into_inner())
}

/// Every file under `dir` (in `root`), by its path from `root`; half-written temporary files
/// left out. A folder that isn't there has none.
fn walk(root: &Path, dir: &Path, found: &mut Vec<String>) -> io::Result<()> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        let kind = entry.file_type()?;
        if kind.is_dir() {
            walk(root, &path, found)?;
        } else if kind.is_file()
            && !entry.file_name().to_string_lossy().ends_with(".tmp")
            && let Ok(relative) = path.strip_prefix(root)
        {
            found.push(relative.to_string_lossy().into_owned());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::Read as _;

    use super::*;

    #[test]
    fn the_zip_holds_the_folder_and_the_rest() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("people")).unwrap();
        fs::write(
            dir.path().join("people/sami.md"),
            "---\ntitle: Sami\n---\nA dentist.\n",
        )
        .unwrap();
        fs::write(dir.path().join("people/.sami.md.tmp"), "half").unwrap();
        fs::write(dir.path().join("_profile.md"), "# profile\n").unwrap();
        let bytes = zip(dir.path(), &serde_json::json!({ "notes": [] })).unwrap();
        let mut read = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
        let mut names: Vec<String> = read
            .file_names()
            .map(|name| name.unwrap().into_owned())
            .collect();
        names.sort();
        assert_eq!(
            names,
            [
                "context/_profile.md",
                "context/people/sami.md",
                "memory.json"
            ]
        );
        let mut text = String::new();
        read.by_name("context/people/sami.md")
            .unwrap()
            .read_to_string(&mut text)
            .unwrap();
        assert!(text.ends_with("A dentist.\n"));
    }
}
