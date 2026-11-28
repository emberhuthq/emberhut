//! What this computer is, as far as choosing a payload goes: its system (macOS or Windows), its
//! processor and its system's version.

use std::cmp::Ordering;
use std::fmt;

use emberhut_api::releases::{Arch, Platform};

/// A system version, `13.0` or `14.5.1`: up to three numbers, the missing ones 0.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct OsVersion([u32; 3]);

impl OsVersion {
    /// `text` read as a version; `None` unless it is one to three whole numbers joined by dots.
    #[must_use]
    pub(crate) fn parse(text: &str) -> Option<Self> {
        let mut parts = [0; 3];
        for (at, part) in text.trim().split('.').enumerate() {
            *parts.get_mut(at)? = part.parse().ok()?;
        }
        Some(Self(parts))
    }
}

impl Ord for OsVersion {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0.cmp(&other.0)
    }
}

impl PartialOrd for OsVersion {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl fmt::Display for OsVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let [major, minor, patch] = self.0;
        if patch == 0 {
            write!(f, "{major}.{minor}")
        } else {
            write!(f, "{major}.{minor}.{patch}")
        }
    }
}

/// This computer, for choosing a payload.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ThisMachine {
    /// Its system, as feeds name it.
    pub(crate) platform: Platform,
    /// Its processor, as releases name it.
    pub(crate) arch: Arch,
    /// Its system's version.
    pub(crate) os: OsVersion,
}

/// The system's name, as people know it (`macOS`, `Windows`).
#[must_use]
pub(crate) fn system_name(platform: &Platform) -> &str {
    match platform {
        Platform::Macos => "macOS",
        Platform::Windows => "Windows",
        other => other.as_str(),
    }
}

/// The processor this copy was built for, as releases name it.
#[must_use]
pub(crate) fn arch() -> Arch {
    arch_named(std::env::consts::ARCH)
}

/// Rust's name of a processor (`aarch64`) as releases name it (`arm64`).
fn arch_named(rust: &str) -> Arch {
    match rust {
        "aarch64" => Arch::Arm64,
        other => Arch::from(other),
    }
}

/// The system's version: macOS's (`sw_vers -productVersion`, `14.5`) or Windows's (from
/// `ver`, `10.0.22631`); `None` elsewhere or when it can't be read.
#[must_use]
pub(crate) fn os_version() -> Option<OsVersion> {
    if cfg!(target_os = "macos") {
        let output = std::process::Command::new("/usr/bin/sw_vers")
            .arg("-productVersion")
            .output()
            .inspect_err(|error| log::warn!("can't ask for the macOS version: {error}"))
            .ok()?;
        return OsVersion::parse(&String::from_utf8_lossy(&output.stdout));
    }
    if cfg!(windows) {
        return windows_version();
    }
    None
}

/// Windows's version, from `cmd /c ver` (run without a window).
#[cfg(windows)]
fn windows_version() -> Option<OsVersion> {
    use std::os::windows::process::CommandExt as _;
    /// `CREATE_NO_WINDOW`: no console flashes up.
    const NO_WINDOW: u32 = 0x0800_0000;
    let system = std::env::var_os("SystemRoot")?;
    let output =
        std::process::Command::new(std::path::Path::new(&system).join("System32\\cmd.exe"))
            .args(["/d", "/c", "ver"])
            .creation_flags(NO_WINDOW)
            .output()
            .inspect_err(|error| log::warn!("can't ask for the Windows version: {error}"))
            .ok()?;
    windows_version_in(&String::from_utf8_lossy(&output.stdout))
}

#[cfg(not(windows))]
fn windows_version() -> Option<OsVersion> {
    None
}

/// The version in `ver`'s line, `Microsoft Windows [Version 10.0.22631.4460]`: its first three
/// numbers.
#[cfg_attr(
    not(any(windows, test)),
    expect(dead_code, reason = "read on Windows only")
)]
fn windows_version_in(said: &str) -> Option<OsVersion> {
    let start = said.find("Version ")? + "Version ".len();
    let numbers = said[start..].split(']').next()?.trim();
    let first_three: Vec<&str> = numbers.split('.').take(3).collect();
    OsVersion::parse(&first_three.join("."))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_read_and_order_by_their_numbers() {
        let v = |text| OsVersion::parse(text).unwrap();
        assert_eq!(v("13"), v("13.0"));
        assert_eq!(v("13.0"), v("13.0.0"));
        assert!(v("13.10") > v("13.9"));
        assert!(v("14.5.1") > v("14.5"));
        assert!(v("12.7.6") < v("13.0"));
        assert_eq!(v(" 15.1\n").to_string(), "15.1");
        assert_eq!(v("14.5.1").to_string(), "14.5.1");
        for bad in ["", "13.a", "1.2.3.4", "13..0", "-1"] {
            assert_eq!(OsVersion::parse(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn windows_says_its_version_in_brackets() {
        let said = "\r\nMicrosoft Windows [Version 10.0.22631.4460]\r\n";
        assert_eq!(windows_version_in(said), OsVersion::parse("10.0.22631"));
        assert!(windows_version_in(said).unwrap() >= OsVersion::parse("10.0.19041").unwrap());
        assert_eq!(windows_version_in("no version here"), None);
    }

    #[test]
    fn processors_go_by_the_releases_names() {
        assert_eq!(arch_named("aarch64"), Arch::Arm64);
        assert_eq!(arch_named("x86_64"), Arch::X86_64);
        assert_eq!(arch_named("riscv64"), Arch::Other("riscv64".into()));
    }
}
