//! The helper's `Info.plist`, its entitlements and its launch agent's plist.

use super::app::{BUNDLE_ID, DISPLAY_NAME, EXECUTABLE, ICON_NAME, LABEL};
use crate::bundle_macos::plist::{Value, document};

/// The `Info.plist` of `Emberhut Hands.app` at `version`: its own name and icon in System Settings'
/// privacy lists, no Dock icon, and why it asks for the microphone.
#[must_use]
pub(crate) fn info_plist(version: &str) -> String {
    document(&[
        ("CFBundleName", Value::String(DISPLAY_NAME)),
        ("CFBundleDisplayName", Value::String(DISPLAY_NAME)),
        ("CFBundleIdentifier", Value::String(BUNDLE_ID)),
        ("CFBundleExecutable", Value::String(EXECUTABLE)),
        ("CFBundleIconFile", Value::String(ICON_NAME)),
        ("CFBundlePackageType", Value::String("APPL")),
        ("CFBundleShortVersionString", Value::String(version)),
        ("CFBundleVersion", Value::String(version)),
        ("CFBundleInfoDictionaryVersion", Value::String("6.0")),
        // ScreenCaptureKit's microphone capture.
        ("LSMinimumSystemVersion", Value::String("15.0")),
        // An accessory: no Dock icon, never in front.
        ("LSUIElement", Value::True),
        // The server may be reached over plain http on a local or private network (ADR 0012),
        // which App Transport Security blocks by default. NSAllowsLocalNetworking lets `.local`
        // names and IP addresses through; the helper itself takes plain http only to loopback,
        // private and `.local` addresses (`emberhut_hands::network`), never NSAllowsArbitraryLoads.
        (
            "NSAppTransportSecurity",
            Value::Dict(vec![("NSAllowsLocalNetworking", Value::True)]),
        ),
        (
            "NSMicrophoneUsageDescription",
            Value::String(
                "Emberhut Hands records your side of a call when you ask, as its own track, to \
                 transcribe it on your server.",
            ),
        ),
    ])
}

/// The entitlements under the hardened runtime: the microphone, for the owner's track of a
/// recorded call.
#[must_use]
pub(crate) fn entitlements() -> String {
    document(&[("com.apple.security.device.audio-input", Value::True)])
}

/// The launch agent: the helper started at login and kept running, logging to `log`, with
/// `environment` (name, value) set.
#[must_use]
pub(crate) fn launch_agent(program: &str, log: &str, environment: &[(&str, &str)]) -> String {
    let mut entries = vec![
        ("Label", Value::String(LABEL)),
        (
            "ProgramArguments",
            Value::Array(vec![Value::String(program)]),
        ),
        ("RunAtLoad", Value::True),
        ("KeepAlive", Value::True),
        // Only in the owner's graphical session: the helper needs the window server.
        ("LimitLoadToSessionType", Value::String("Aqua")),
        ("ProcessType", Value::String("Interactive")),
        ("StandardOutPath", Value::String(log)),
        ("StandardErrorPath", Value::String(log)),
    ];
    if !environment.is_empty() {
        entries.push((
            "EnvironmentVariables",
            Value::Dict(
                environment
                    .iter()
                    .map(|(name, value)| (*name, Value::String(value)))
                    .collect(),
            ),
        ));
    }
    document(&entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_helper_is_its_own_bundle_without_a_dock_icon() {
        let xml = info_plist("0.1.0");
        assert!(xml.contains("<key>CFBundleIdentifier</key><string>com.emberhut.hands</string>"));
        assert!(xml.contains("<key>CFBundleName</key><string>Emberhut Hands</string>"));
        assert!(xml.contains("<key>CFBundleExecutable</key><string>emberhut-hands</string>"));
        assert!(xml.contains("<key>CFBundleIconFile</key><string>AppIcon</string>"));
        assert!(xml.contains("<key>LSUIElement</key><true/>"));
        assert!(xml.contains("<key>NSMicrophoneUsageDescription</key>"));
        assert!(xml.contains("<key>NSAllowsLocalNetworking</key><true/>"));
        assert!(!xml.contains("NSAllowsArbitraryLoads"));
    }

    #[test]
    fn the_entitlements_ask_for_the_microphone_only() {
        let xml = entitlements();
        assert!(xml.contains("<key>com.apple.security.device.audio-input</key><true/>"));
        assert_eq!(xml.matches("<key>").count(), 1);
    }

    #[test]
    fn the_launch_agent_runs_the_helper_with_its_settings() {
        let xml = launch_agent(
            "/Users/example/Applications/Emberhut Hands.app/Contents/MacOS/emberhut-hands",
            "/Users/example/Library/Logs/Emberhut Hands.log",
            &[("EMBERHUT_HANDS_LISTEN", "10.0.0.5")],
        );
        assert!(xml.contains("<key>Label</key><string>com.emberhut.hands</string>"));
        assert!(xml.contains(
            "<string>/Users/example/Applications/Emberhut Hands.app/Contents/MacOS/emberhut-hands</string>"
        ));
        assert!(xml.contains("<key>KeepAlive</key><true/>"));
        assert!(xml.contains("<key>EMBERHUT_HANDS_LISTEN</key><string>10.0.0.5</string>"));
        assert!(!launch_agent("/p", "/l", &[]).contains("EnvironmentVariables"));
    }
}
