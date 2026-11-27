//! The bundle's `Info.plist`.

use crate::bundle_macos::app::{BUNDLE_ID, DISPLAY_NAME, EXECUTABLE, ICON_NAME, URL_SCHEME};
use crate::bundle_macos::xml::escape;

/// One value in a plist.
pub(crate) enum Value<'a> {
    /// `<string>`.
    String(&'a str),
    /// `<true/>`.
    True,
    /// `<array>`.
    Array(Vec<Value<'a>>),
    /// `<dict>`.
    Dict(Vec<(&'a str, Value<'a>)>),
}

impl Value<'_> {
    /// The value's XML, nested `depth` levels into the top dictionary.
    fn xml(&self, depth: usize) -> String {
        let indent = "  ".repeat(depth);
        match self {
            Value::String(text) => format!("<string>{}</string>", escape(text)),
            Value::True => "<true/>".to_owned(),
            Value::Array(items) => {
                let mut xml = String::from("<array>\n");
                for item in items {
                    xml.push_str(&indent);
                    xml.push_str("  ");
                    xml.push_str(&item.xml(depth + 1));
                    xml.push('\n');
                }
                xml.push_str(&indent);
                xml.push_str("</array>");
                xml
            }
            Value::Dict(entries) => {
                let inner: String = entries
                    .iter()
                    .map(|(key, value)| entry(key, value, depth + 1))
                    .collect();
                format!("<dict>\n{inner}{indent}</dict>")
            }
        }
    }
}

/// One `<key>` and its value, a line of a dictionary `depth` levels in.
fn entry(key: &str, value: &Value<'_>, depth: usize) -> String {
    format!(
        "{}<key>{}</key>{}\n",
        "  ".repeat(depth),
        escape(key),
        value.xml(depth)
    )
}

/// The `Info.plist` XML of Emberhut.app at `version`.
#[must_use]
pub(crate) fn info_plist(version: &str) -> String {
    let entries = [
        ("CFBundleName", Value::String(DISPLAY_NAME)),
        ("CFBundleDisplayName", Value::String(DISPLAY_NAME)),
        ("CFBundleIdentifier", Value::String(BUNDLE_ID)),
        ("CFBundleExecutable", Value::String(EXECUTABLE)),
        ("CFBundleIconFile", Value::String(ICON_NAME)),
        ("CFBundlePackageType", Value::String("APPL")),
        ("CFBundleShortVersionString", Value::String(version)),
        ("CFBundleVersion", Value::String(version)),
        ("CFBundleInfoDictionaryVersion", Value::String("6.0")),
        ("LSMinimumSystemVersion", Value::String("13.0")),
        (
            "LSApplicationCategoryType",
            Value::String("public.app-category.productivity"),
        ),
        // Every file the app writes (a file fetched from the server, a copy saved into
        // Downloads) is quarantined, so Gatekeeper checks whatever would run from one.
        ("LSFileQuarantineEnabled", Value::True),
        ("NSHighResolutionCapable", Value::True),
        ("NSSupportsAutomaticGraphicsSwitching", Value::True),
        ("NSPrincipalClass", Value::String("NSApplication")),
        (
            "NSHumanReadableCopyright",
            Value::String("Copyright © 2026 Symploc, LLC"),
        ),
        // The app's links: its own scheme.
        (
            "CFBundleURLTypes",
            Value::Array(vec![Value::Dict(vec![
                ("CFBundleURLName", Value::String(BUNDLE_ID)),
                (
                    "CFBundleURLSchemes",
                    Value::Array(vec![Value::String(URL_SCHEME)]),
                ),
            ])]),
        ),
    ];
    document(&entries)
}

/// A plist document whose top dictionary holds `entries`.
#[must_use]
pub(crate) fn document(entries: &[(&str, Value<'_>)]) -> String {
    let mut xml = String::from(concat!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
        "<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" ",
        "\"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n",
        "<plist version=\"1.0\">\n<dict>\n",
    ));
    xml.extend(entries.iter().map(|(key, value)| entry(key, value, 1)));
    xml.push_str("</dict>\n</plist>\n");
    xml
}

#[cfg(test)]
mod tests {
    use super::info_plist;

    /// The `<string>` value that follows `<key>{key}</key>` in `xml`.
    fn string_after(xml: &str, key: &str) -> Option<String> {
        let start = xml.find(&format!("<key>{key}</key><string>"))?;
        let rest = &xml[start + key.len() + "<key></key><string>".len()..];
        Some(rest[..rest.find("</string>")?].to_owned())
    }

    #[test]
    fn both_names_are_emberhut() {
        let xml = info_plist("0.1.0");
        assert_eq!(
            string_after(&xml, "CFBundleName").as_deref(),
            Some("Emberhut")
        );
        assert_eq!(
            string_after(&xml, "CFBundleDisplayName").as_deref(),
            Some("Emberhut")
        );
    }

    #[test]
    fn identifier_executable_icon_and_version() {
        let xml = info_plist("1.2.3");
        let value = |key| string_after(&xml, key);
        assert_eq!(
            value("CFBundleIdentifier").as_deref(),
            Some("com.emberhut.desktop")
        );
        assert_eq!(
            value("CFBundleExecutable").as_deref(),
            Some("emberhut-desktop")
        );
        assert_eq!(value("CFBundleIconFile").as_deref(), Some("AppIcon"));
        assert_eq!(value("CFBundlePackageType").as_deref(), Some("APPL"));
        assert_eq!(
            value("CFBundleShortVersionString").as_deref(),
            Some("1.2.3")
        );
        assert_eq!(value("CFBundleVersion").as_deref(), Some("1.2.3"));
        assert_eq!(value("LSMinimumSystemVersion").as_deref(), Some("13.0"));
        assert_eq!(value("NSPrincipalClass").as_deref(), Some("NSApplication"));
        assert!(xml.contains("<key>NSHighResolutionCapable</key><true/>"));
        assert!(xml.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n"));
        assert!(xml.ends_with("</dict>\n</plist>\n"));
    }

    #[test]
    fn the_app_never_asks_for_the_camera_or_the_microphone() {
        // The report web view (docs/adr/0006-report-webview.md) runs on a Wry that grants every
        // page's camera and microphone request; only the missing usage strings keep macOS from
        // handing them over. Declare neither until that web view has a permission handler that
        // refuses them.
        let xml = info_plist("0.1.0");
        for key in ["NSCameraUsageDescription", "NSMicrophoneUsageDescription"] {
            assert!(
                !xml.contains(key),
                "{key} in Info.plist: the report web view needs a permission handler first \
                 (ADR 0006)"
            );
        }
    }

    #[test]
    fn the_files_the_app_writes_are_quarantined() {
        let xml = info_plist("0.1.0");
        assert!(xml.contains("<key>LSFileQuarantineEnabled</key><true/>"));
    }

    #[test]
    fn the_link_scheme_opens_the_app() {
        let xml = info_plist("0.1.0");
        let types = &xml[xml.find("<key>CFBundleURLTypes</key>").unwrap()..];
        assert!(types.starts_with("<key>CFBundleURLTypes</key><array>\n    <dict>\n"));
        assert!(types.contains("<key>CFBundleURLName</key><string>com.emberhut.desktop</string>"));
        assert!(types.contains(
            "<key>CFBundleURLSchemes</key><array>\n        <string>emberhut</string>\n      </array>"
        ));
        assert!(xml.ends_with("  </array>\n</dict>\n</plist>\n"));
    }

    #[test]
    fn values_are_escaped() {
        let xml = info_plist("1.0 <beta> & \"rc\"");
        assert_eq!(
            string_after(&xml, "CFBundleVersion").as_deref(),
            Some("1.0 &lt;beta&gt; &amp; &quot;rc&quot;")
        );
    }
}
