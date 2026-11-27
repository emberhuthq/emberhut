//! Assembling `Emberhut.app`: its directory tree, `Info.plist`, binary and icon.

use std::fs;

use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::Context as _;

use crate::bundle_macos::app::{EXECUTABLE, ICON_NAME, Layout, VERSION};
use crate::bundle_macos::build::Built;
use crate::bundle_macos::plist;
use crate::bundle_macos::tool;
use crate::cmd;
use crate::files::remove_dir_if_present;
use crate::say;

/// The icon's point sizes; an `.icns` holds each at 1x and 2x.
const ICON_SIZES: [u32; 5] = [16, 32, 128, 256, 512];

/// The file names and pixel sizes of an iconset's images, as `iconutil` expects them.
#[must_use]
pub(crate) fn iconset_images() -> Vec<(String, u32)> {
    ICON_SIZES
        .iter()
        .flat_map(|&size| {
            [
                (format!("icon_{size}x{size}.png"), size),
                (format!("icon_{size}x{size}@2x.png"), size * 2),
            ]
        })
        .collect()
}

/// Builds a fresh `Emberhut.app` at the layout's bundle path around `built`: the app's binary,
/// and the hut's server beside it in `Contents/MacOS` when it was built.
///
/// # Errors
///
/// Fails when a file can't be written or `sips`/`iconutil` fail.
pub(crate) fn assemble(layout: &Layout, built: &Built) -> anyhow::Result<()> {
    let bundle = layout.app_bundle();
    say(&format!("Assembling {}", bundle.display()));
    remove_dir_if_present(&bundle)?;
    let contents = bundle.join("Contents");
    let macos = contents.join("MacOS");
    let resources = contents.join("Resources");
    for dir in [&macos, &resources] {
        fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    let executable = macos.join(EXECUTABLE);
    copy(&built.app, &executable)?;
    if let Some(server) = &built.server {
        copy(server, &layout.bundled_server())?;
    }
    let info = contents.join("Info.plist");
    fs::write(&info, plist::info_plist(VERSION))
        .with_context(|| format!("writing {}", info.display()))?;
    make_icon(layout, &resources.join(format!("{ICON_NAME}.icns")))
}

/// Copies the binary `from` to `to`.
fn copy(from: &Path, to: &Path) -> anyhow::Result<()> {
    fs::copy(from, to)
        .map(drop)
        .with_context(|| format!("copying {} to {}", from.display(), to.display()))
}

/// Renders the 1024 px source icon into every iconset size and packs them into `icns`.
fn make_icon(layout: &Layout, icns: &Path) -> anyhow::Result<()> {
    let source = layout.icon_source();
    let iconset = layout.iconset();
    remove_dir_if_present(&iconset)?;
    fs::create_dir_all(&iconset).with_context(|| format!("creating {}", iconset.display()))?;
    for (name, pixels) in iconset_images() {
        let pixels = pixels.to_string();
        cmd::run(
            Command::new(tool::SIPS)
                .args(["-z", &pixels, &pixels])
                .arg(&source)
                .arg("--out")
                .arg(iconset.join(name))
                .stdout(Stdio::null()),
        )?;
    }
    cmd::run(
        Command::new(tool::ICONUTIL)
            .args(["-c", "icns"])
            .arg(&iconset)
            .arg("-o")
            .arg(icns),
    )?;
    remove_dir_if_present(&iconset)
}

#[cfg(test)]
mod tests {
    use super::iconset_images;

    #[test]
    fn the_iconset_has_every_size_at_1x_and_2x() {
        let images = iconset_images();
        assert_eq!(images.len(), 10);
        assert_eq!(images[0], ("icon_16x16.png".to_owned(), 16));
        assert_eq!(images[1], ("icon_16x16@2x.png".to_owned(), 32));
        assert_eq!(images[9], ("icon_512x512@2x.png".to_owned(), 1024));
    }
}
