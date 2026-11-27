//! PDFs through Poppler's command line tools: `pdfinfo` for the pages' sizes, `pdftoppm` for a
//! page as a PNG. Looked for on the `PATH`.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use gpui_kit::ImageFormat;

use super::{Drawn, Error};
use crate::ui::viewer::pages::PageSize;

/// The tool `name`, if it is on the `PATH`.
fn tool(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|folder| folder.join(name))
        .find(|candidate| candidate.is_file())
}

/// The pages' sizes, from `pdfinfo` asked for every page.
pub(super) fn pages(path: &Path) -> Result<Vec<PageSize>, Error> {
    let pdfinfo = tool("pdfinfo").ok_or(Error::NoRenderer)?;
    let out = Command::new(pdfinfo)
        .args(["-f", "1", "-l", "100000"])
        .arg(path)
        .stdin(Stdio::null())
        .output()
        .map_err(|error| Error::Unreadable(error.to_string()))?;
    if !out.status.success() {
        return Err(Error::Unreadable(
            String::from_utf8_lossy(&out.stderr).trim().to_owned(),
        ));
    }
    let sizes = page_sizes(&String::from_utf8_lossy(&out.stdout));
    if sizes.is_empty() {
        return Err(Error::Unreadable("it has no pages".to_owned()));
    }
    Ok(sizes)
}

/// The sizes in `pdfinfo`'s lines: `Page    3 size: 612 x 792 pts (letter)`, turned by the
/// page's `Page    3 rot:  90` when it is on its side.
fn page_sizes(info: &str) -> Vec<PageSize> {
    let mut sizes: Vec<PageSize> = Vec::new();
    for line in info.lines() {
        let Some(rest) = line.strip_prefix("Page") else {
            continue;
        };
        if let Some((_, size)) = rest.split_once("size:") {
            let mut numbers = size
                .split_whitespace()
                .filter_map(|word| word.parse::<f32>().ok());
            if let (Some(width), Some(height)) = (numbers.next(), numbers.next()) {
                sizes.push(PageSize { width, height });
            }
        } else if let Some((_, rot)) = rest.split_once("rot:")
            && matches!(rot.trim().parse::<i32>(), Ok(90 | 270))
            && let Some(last) = sizes.last_mut()
        {
            *last = PageSize {
                width: last.height,
                height: last.width,
            };
        }
    }
    sizes
}

/// Page `index` as a PNG `width` pixels wide, which `pdftoppm` writes to its output when it is
/// given no file to write.
pub(super) fn draw(path: &Path, index: usize, width: u32) -> Result<Drawn, Error> {
    let pdftoppm = tool("pdftoppm").ok_or(Error::NoRenderer)?;
    let page = (index + 1).to_string();
    let failed = |why: String| Error::Page {
        page: index + 1,
        why,
    };
    let out = Command::new(pdftoppm)
        .args(["-png", "-singlefile", "-f", &page, "-l", &page])
        .args(["-scale-to-x", &width.to_string(), "-scale-to-y", "-1"])
        .arg(path)
        .stdin(Stdio::null())
        .output()
        .map_err(|error| failed(error.to_string()))?;
    if !out.status.success() || out.stdout.is_empty() {
        return Err(failed(
            String::from_utf8_lossy(&out.stderr).trim().to_owned(),
        ));
    }
    Ok(Drawn {
        bytes: out.stdout,
        format: ImageFormat::Png,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pdfinfos_lines_read_as_page_sizes() {
        let info = "Title: x\nPages: 3\nPage    1 size: 612 x 792 pts (letter)\nPage    1 rot:  0\n\
                    Page    2 size: 595.276 x 841.89 pts (A4)\nPage    2 rot:  90\n\
                    Page    3 size: 100 x 50 pts\nFile size: 1 bytes\n";
        let sizes = page_sizes(info);
        assert_eq!(sizes.len(), 3);
        assert_eq!(
            sizes[0],
            PageSize {
                width: 612.,
                height: 792.
            }
        );
        assert_eq!(
            sizes[1],
            PageSize {
                width: 841.89,
                height: 595.276
            },
            "a page on its side is turned"
        );
        assert_eq!(
            sizes[2],
            PageSize {
                width: 100.,
                height: 50.
            }
        );
        assert_eq!(page_sizes("Pages: 0\n"), Vec::new());
    }
}
