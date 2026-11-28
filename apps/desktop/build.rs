//! On Windows, the program's icon and version information, as Explorer, the taskbar and the
//! installed programs list show them: a resource script written here and compiled into the
//! executable. Nothing is done for other systems.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// The icon, made from `assets/AppIcon-1024.png` (see the README, "Building on Windows").
const ICON: &str = "assets/AppIcon.ico";

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed={ICON}");
    println!("cargo:rerun-if-env-changed=EMBERHUT_VERSION");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return Ok(());
    }
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR")?);
    let out = PathBuf::from(std::env::var("OUT_DIR")?);
    let version = match std::env::var("EMBERHUT_VERSION") {
        Ok(version) if !version.is_empty() => version,
        _ => std::env::var("CARGO_PKG_VERSION")?,
    };
    let script = out.join("emberhut-desktop.rc");
    std::fs::write(&script, resource_script(&manifest.join(ICON), &version)?)?;
    embed_resource::compile(&script, embed_resource::NONE)
        .manifest_optional()
        .map_err(|error| format!("couldn't compile the Windows resources: {error}"))?;
    Ok(())
}

/// The resource script: the icon at `icon`, and the version information of `version`
/// (`X.Y.Z` or `X.Y.Z-rc.N`; its numeric fields are X, Y and Z).
fn resource_script(icon: &Path, version: &str) -> Result<String, Box<dyn std::error::Error>> {
    let base = version.split(['-', '+']).next().unwrap_or(version);
    let mut numbers = base.split('.').map(str::parse::<u16>);
    let mut next = || numbers.next().unwrap_or(Ok(0));
    let (major, minor, patch) = (next()?, next()?, next()?);
    let numeric = format!("{major},{minor},{patch},0");
    let icon = icon.display().to_string().replace('\\', "\\\\");
    let mut rc = String::new();
    writeln!(rc, "1 ICON \"{icon}\"")?;
    writeln!(rc, "1 VERSIONINFO")?;
    writeln!(rc, "FILEVERSION {numeric}")?;
    writeln!(rc, "PRODUCTVERSION {numeric}")?;
    // No flags; VOS_NT_WINDOWS32; VFT_APP.
    writeln!(rc, "FILEFLAGSMASK 0x3F")?;
    writeln!(rc, "FILEFLAGS 0x0")?;
    writeln!(rc, "FILEOS 0x40004")?;
    writeln!(rc, "FILETYPE 0x1")?;
    writeln!(rc, "FILESUBTYPE 0x0")?;
    writeln!(rc, "BEGIN")?;
    writeln!(rc, "  BLOCK \"StringFileInfo\"")?;
    writeln!(rc, "  BEGIN")?;
    // US English, Unicode.
    writeln!(rc, "    BLOCK \"040904B0\"")?;
    writeln!(rc, "    BEGIN")?;
    for (key, value) in [
        ("CompanyName", "Emberhut"),
        ("FileDescription", "Emberhut"),
        ("FileVersion", version),
        ("InternalName", "emberhut-desktop"),
        ("LegalCopyright", "MIT OR Apache-2.0"),
        ("OriginalFilename", "emberhut-desktop.exe"),
        ("ProductName", "Emberhut"),
        ("ProductVersion", version),
    ] {
        writeln!(rc, "      VALUE \"{key}\", \"{value}\"")?;
    }
    writeln!(rc, "    END")?;
    writeln!(rc, "  END")?;
    writeln!(rc, "  BLOCK \"VarFileInfo\"")?;
    writeln!(rc, "  BEGIN")?;
    writeln!(rc, "    VALUE \"Translation\", 0x409, 1200")?;
    writeln!(rc, "  END")?;
    writeln!(rc, "END")?;
    Ok(rc)
}
