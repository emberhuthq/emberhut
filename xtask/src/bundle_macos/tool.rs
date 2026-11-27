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
