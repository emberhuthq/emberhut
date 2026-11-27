//! What this Mac is, as far as choosing a payload goes: its processor and its macOS version.

use std::cmp::Ordering;
use std::fmt;

use emberhut_api::releases::Arch;

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

/// This machine, for choosing a payload.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ThisMac {
    /// Its processor, as releases name it.
    pub(crate) arch: Arch,
    /// Its macOS version.
    pub(crate) os: OsVersion,
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

/// This Mac's macOS version (`sw_vers -productVersion`); `None` off macOS or when it can't be
/// read.
#[must_use]
pub(crate) fn os_version() -> Option<OsVersion> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    let output = std::process::Command::new("/usr/bin/sw_vers")
        .arg("-productVersion")
        .output()
        .inspect_err(|error| log::warn!("can't ask for the macOS version: {error}"))
        .ok()?;
    OsVersion::parse(&String::from_utf8_lossy(&output.stdout))
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
    fn processors_go_by_the_releases_names() {
        assert_eq!(arch_named("aarch64"), Arch::Arm64);
        assert_eq!(arch_named("x86_64"), Arch::X86_64);
        assert_eq!(arch_named("riscv64"), Arch::Other("riscv64".into()));
    }
}
