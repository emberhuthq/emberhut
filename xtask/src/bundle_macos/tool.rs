//! The macOS tools `bundle-macos` runs, by absolute path so `PATH` can't swap them.

/// Signs and verifies code signatures.
pub(crate) const CODESIGN: &str = "/usr/bin/codesign";
/// Copies bundles and makes zips that keep their metadata.
pub(crate) const DITTO: &str = "/usr/bin/ditto";
/// Turns an iconset into an `.icns`.
pub(crate) const ICONUTIL: &str = "/usr/bin/iconutil";
/// Runs a command at lower priority.
pub(crate) const NICE: &str = "/usr/bin/nice";
/// Resizes images.
pub(crate) const SIPS: &str = "/usr/bin/sips";
/// Runs Xcode's tools (`notarytool`, `stapler`).
pub(crate) const XCRUN: &str = "/usr/bin/xcrun";
/// Reads values out of property lists, XML or binary.
pub(crate) const PLUTIL: &str = "/usr/bin/plutil";
/// Makes, attaches and detaches disk images.
pub(crate) const HDIUTIL: &str = "/usr/bin/hdiutil";
/// Joins per-architecture binaries into a universal one.
pub(crate) const LIPO: &str = "/usr/bin/lipo";
/// Asks Gatekeeper whether it would open a file.
pub(crate) const SPCTL: &str = "/usr/sbin/spctl";
/// Says which session this runs in (`managername`: `Aqua` for a logged-in desktop).
pub(crate) const LAUNCHCTL: &str = "/bin/launchctl";
/// Packs the standalone server's archive.
pub(crate) const TAR: &str = "/usr/bin/tar";
