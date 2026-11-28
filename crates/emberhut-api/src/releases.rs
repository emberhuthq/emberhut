//! The release feed: which builds of an app are published, newest first, with the payloads a
//! desktop app installs (`GET /api/releases/v{format}/{platform}`, [`crate::paths::release_feed`]).
//!
//! One small JSON document per platform and format. A reader ignores fields it doesn't know, as
//! for the rest of the contract, and reads any [`ReleaseFeed::format`] up to its own
//! ([`FORMAT`]); a change that breaks readers is a new format at a new path. The feed itself is
//! not signed: an app trusts a payload only once it has checked the payload's own signature,
//! which binds it to the entry's version, build, `sha256` and notes.
//!
//! ```
//! use emberhut_api::releases::{Platform, ReleaseFeed};
//!
//! let feed: ReleaseFeed = serde_json::from_str(r#"{
//!     "format": 1, "product": "emberhut-mobile", "platform": "ios",
//!     "published": "2026-12-08T10:00:00.000Z",
//!     "releases": [{"version": "0.3.0", "build": 15, "date": "2026-12-08", "notes": "Faster."}]
//! }"#).unwrap();
//! assert_eq!(feed.platform, Platform::Ios);
//! assert!(feed.releases[0].assets.is_empty(), "a phone's feed lists no payloads");
//! ```

use std::fmt;

use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use crate::open_enum::open_enum;
use crate::time::{Day, Timestamp};

/// The newest feed format this contract describes. A reader asks for it first and, on a `404`,
/// for each lower one it reads.
pub const FORMAT: u32 = 1;

open_enum! {
    /// Which app a feed is for.
    pub enum Platform {
        /// The Mac app.
        Macos = "macos",
        /// The Windows app.
        Windows = "windows",
        /// The iPhone app (from the App Store's beta testing; its feed only says which build is newest).
        Ios = "ios",
        /// The Android app (on Play's internal track; as for iOS).
        Android = "android",
    }
}

open_enum! {
    /// The processor a payload is built for, as the release names it.
    pub enum Arch {
        /// Apple silicon (`aarch64`).
        Arm64 = "arm64",
        /// Intel.
        X86_64 = "x86_64",
    }
}

/// A build number: one counter for the whole product, raised by one every release, so a higher
/// number is always a later build.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Build(u64);

impl Build {
    /// Wraps a build number.
    #[must_use]
    pub const fn new(build: u64) -> Self {
        Self(build)
    }

    /// The number.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl From<u64> for Build {
    fn from(build: u64) -> Self {
        Self(build)
    }
}

impl fmt::Display for Build {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

/// The feed of one platform, in one format.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ReleaseFeed {
    /// The format this document is written in ([`FORMAT`] or lower for this contract).
    pub format: u32,
    /// The product it publishes, e.g. `emberhut-desktop`.
    pub product: String,
    /// The platform it is for.
    pub platform: Platform,
    /// When it was written: how old a feed is shows a server that stopped publishing.
    pub published: Timestamp,
    /// The last few releases, newest first.
    #[serde(default)]
    pub releases: Vec<Release>,
}

impl ReleaseFeed {
    /// The newest release listed.
    #[must_use]
    pub fn newest(&self) -> Option<&Release> {
        self.releases.iter().max_by_key(|release| release.build)
    }
}

/// One release: its version and build, its notes, and its payloads.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Release {
    /// The product's version, plain `MAJOR.MINOR.PATCH` (`0.3.0`).
    pub version: String,
    /// Its build number.
    pub build: Build,
    /// The day it was released.
    pub date: Day,
    /// Its notes for the person who uses the app (the changelog's section), as written. A
    /// payload's signature covers their exact bytes.
    #[serde(default)]
    pub notes: String,
    /// The oldest system it runs on (`13.0` for macOS), when the platform has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minimum_os: Option<String>,
    /// The payloads, one per processor; none on a phone's feed, where the store installs.
    #[serde(default)]
    pub assets: Vec<Asset>,
}

impl Release {
    /// The payload built for `arch`.
    #[must_use]
    pub fn asset(&self, arch: &Arch) -> Option<&Asset> {
        self.assets.iter().find(|asset| &asset.arch == arch)
    }
}

/// A payload: the disk image an app installs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Asset {
    /// The processor it is built for.
    pub arch: Arch,
    /// Where it is: a path on the server that serves the feed
    /// (`/api/releases/files/Emberhut_0.3.0_15_macos_arm64.dmg`), fetched with the device's session.
    pub url: String,
    /// Its exact size in bytes.
    pub size: u64,
    /// The SHA-256 of its bytes, 64 lowercase hex digits.
    pub sha256: String,
    /// Its update signature: a minisign signature file, as text, whose trusted comment names
    /// the platform, arch, version, build, this `sha256` and the notes' SHA-256.
    pub signature: String,
}

/// The trusted comment a payload's update signature must carry: the line the signature also
/// covers, which binds the payload to its feed entry.
///
/// It reads exactly
/// `<product> <platform> <arch> <version> build <build> sha256:<hex> notes:<hex>`: `sha256` is
/// the payload's SHA-256 and the last field the SHA-256 of `notes`' UTF-8 bytes as the feed
/// carries them (no trimming, no normalising), both in lowercase hex. A feed can then neither
/// relabel an old payload as a new release nor change the notes the app shows. The signer
/// (`cargo xtask release feed`) and the app's verifier both call this, so they agree byte for
/// byte.
///
/// ```
/// use emberhut_api::releases::{Arch, Build, Platform, update_comment};
///
/// let comment = update_comment(
///     "emberhut-desktop", &Platform::Windows, &Arch::X86_64, "0.1.0-rc.1", Build::new(10001),
///     &[0xab; 32], "",
/// );
/// assert!(comment.starts_with("emberhut-desktop windows x86_64 0.1.0-rc.1 build 10001 sha256:abab"));
/// // The SHA-256 of no bytes at all.
/// assert!(comment.ends_with(
///     " notes:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
/// ));
/// ```
#[must_use]
pub fn update_comment(
    product: &str,
    platform: &Platform,
    arch: &Arch,
    version: &str,
    build: Build,
    sha256: &[u8; 32],
    notes: &str,
) -> String {
    format!(
        "{product} {platform} {arch} {version} build {build} sha256:{payload} notes:{notes}",
        payload = hex(sha256),
        notes = hex(&Sha256::digest(notes.as_bytes()).into()),
    )
}

/// `bytes` as lowercase hex, two digits a byte.
fn hex(bytes: &[u8; 32]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut text = String::with_capacity(64);
    for byte in bytes {
        text.push(char::from(DIGITS[usize::from(byte >> 4)]));
        text.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    text
}

#[cfg(test)]
mod tests {
    use super::{Arch, Build, Platform, update_comment};

    #[test]
    fn the_comment_names_the_platform_and_hashes_the_notes_as_they_are() {
        let comment = |platform: &Platform, notes: &str| {
            update_comment(
                "emberhut-desktop",
                platform,
                &Arch::Arm64,
                "0.3.0",
                Build::new(15),
                &[0x0f; 32],
                notes,
            )
        };
        let mac = comment(&Platform::Macos, "- Faster.\n");
        assert!(
            mac.starts_with(&format!(
                "emberhut-desktop macos arm64 0.3.0 build 15 sha256:{} notes:",
                "0f".repeat(32)
            )),
            "{mac}"
        );
        assert_eq!(
            mac.len(),
            mac.find(" notes:").unwrap() + " notes:".len() + 64
        );
        assert!(comment(&Platform::Windows, "- Faster.\n").contains(" windows arm64 "));
        assert_ne!(
            comment(&Platform::Macos, "- Faster."),
            mac,
            "a newline is part of the notes"
        );
    }

    #[test]
    fn windows_is_a_known_platform() {
        assert_eq!(Platform::from("windows"), Platform::Windows);
        assert_eq!(Platform::Windows.as_str(), "windows");
        assert!(Platform::Windows.is_known());
    }
}
