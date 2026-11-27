//! The rules the helper keeps whatever the server asks (the server keeps them too): the apps it
//! never uses, the one it only looks at, Chrome and the browsers built on Chromium, and never a
//! password.

use emberhut_core::browser::{is_secret_label, mac_app_is_chromium};

use crate::fail::Fail;

/// The helper's own bundle id.
pub const OWN_BUNDLE: &str = "com.emberhut.hands";

/// The role (or subrole) of a password field.
pub const SECURE_ROLE: &str = "AXSecureTextField";

/// Emberhut's own desktop app: where the owner answers the assistant's Go ahead cards.
pub const DESKTOP_BUNDLE: &str = "com.emberhut.desktop";

/// Apps never used: other terminals, settings, passwords and keys, scripting, system tools that
/// can undo the rules, screen sharing, the login window, Emberhut's own app (the assistant would
/// answer its own Go ahead cards there) and this helper itself.
const NEVER: &[&str] = &[
    "com.googlecode.iterm2",
    "dev.warp.Warp-Stable",
    "com.mitchellh.ghostty",
    "net.kovidgoyal.kitty",
    "io.alacritty",
    "com.github.wez.wezterm",
    "co.zeit.hyper",
    "org.tabby",
    "com.apple.systempreferences",
    "com.apple.Settings",
    "com.apple.keychainaccess",
    "com.apple.Passwords",
    "com.1password.1password",
    "com.agilebits.onepassword7",
    "com.bitwarden.desktop",
    "org.keepassxc.keepassxc",
    "com.dashlane.dashlanephonefinal",
    "com.dashlane.Dashlane",
    "in.sinew.Enpass-Desktop",
    "com.lastpass.LastPass",
    "com.apple.ScriptEditor2",
    "com.apple.Automator",
    "com.apple.shortcuts",
    "com.apple.ActivityMonitor",
    "com.apple.DiskUtility",
    "com.apple.MigrateAssistant",
    "com.apple.ScreenSharing",
    "com.apple.loginwindow",
    "com.apple.SecurityAgent",
    // The system's consent and permission prompts, and its controls.
    "com.apple.CoreServicesUIAgent",
    "com.apple.coreservices.uiagent",
    "com.apple.UserNotificationCenter",
    "com.apple.accessibility.universalAccessAuthWarn",
    "com.apple.LocalAuthentication.UIAgent",
    "com.apple.controlcenter",
    "com.apple.systemuiserver",
    DESKTOP_BUNDLE,
    OWN_BUNDLE,
];

/// Apps only looked at: never pressed, typed into or their menus picked.
const LOOK_ONLY: &[&str] = &["com.apple.Terminal"];

/// The roles a text is typed into, whose label may say it wants a secret.
const TEXT_ENTRY: &[&str] = &["AXTextField", "AXTextArea", "AXComboBox", "AXSearchField"];

/// What the agent is told for Chrome or a browser built on Chromium.
pub const CHROME: &str = "Chrome goes through the browser extension: use the browser tools with \
                          browser \"chrome:<profile>\" (chrome_pair lists them).";

/// What the agent is told for a password field.
pub const PASSWORD: &str = "That's a password field: the assistant never types, sets or reads \
                            those. Hand it to the owner: they type it themselves.";

/// What a request does to an app.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Use {
    /// Reads it: a snapshot, a description, which app a name means, opening it.
    Look,
    /// Acts in it: presses, sets, keys, menus.
    Act,
}

/// Whether the app with `bundle` is one the helper never uses.
#[must_use]
pub fn never_used(bundle: &str) -> bool {
    NEVER.iter().any(|id| id.eq_ignore_ascii_case(bundle))
}

/// Whether the app named `name` with `bundle` is Chrome or built on Chromium.
#[must_use]
pub fn is_chromium(name: &str, bundle: &str) -> bool {
    mac_app_is_chromium(name, bundle)
}

/// Why the app named `name` with `bundle` can't be used as `wanted`, or `None` when it can.
#[must_use]
pub fn refusal(name: &str, bundle: &str, wanted: Use) -> Option<Fail> {
    if is_chromium(name, bundle) {
        return Some(Fail::never(CHROME));
    }
    if never_used(bundle) {
        return Some(Fail::never(format!(
            "Emberhut Hands never uses {name} on this Mac."
        )));
    }
    if wanted == Use::Act && LOOK_ONLY.iter().any(|id| id.eq_ignore_ascii_case(bundle)) {
        return Some(Fail::never(format!(
            "{name} is only looked at: the assistant never presses, types or picks menus there."
        )));
    }
    None
}

/// Whether an element with `role`, `subrole` and `label` is a password field, or a text field
/// whose label says it wants a password or a code: never typed into, set or read.
#[must_use]
pub fn is_secret(role: &str, subrole: &str, label: &str) -> bool {
    role == SECURE_ROLE
        || subrole == SECURE_ROLE
        || (TEXT_ENTRY.contains(&role) && is_secret_label(label))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chrome_and_its_kin_are_refused_by_bundle_or_name() {
        for (name, bundle) in [
            ("Google Chrome", "com.google.Chrome"),
            ("Chrome Beta", "com.google.Chrome.beta"),
            ("Work", "com.google.Chrome.app.abcdef"),
            ("Brave Browser", "com.brave.Browser"),
            ("Microsoft Edge", "com.microsoft.edgemac"),
            ("Arc", "company.thebrowser.Browser"),
            ("Chromium", ""),
        ] {
            let refused = refusal(name, bundle, Use::Look).unwrap();
            assert_eq!(refused.status, 403);
            assert_eq!(refused.text, CHROME, "{name}");
        }
        assert!(refusal("Safari", "com.apple.Safari", Use::Act).is_none());
    }

    #[test]
    fn the_never_list_and_the_helper_itself_are_refused() {
        let refused = refusal("Keychain Access", "com.apple.keychainaccess", Use::Look).unwrap();
        assert_eq!(
            refused.text,
            "Emberhut Hands never uses Keychain Access on this Mac."
        );
        assert!(refusal("Emberhut Hands", "com.emberhut.hands", Use::Look).is_some());
        assert!(refusal("Emberhut", "com.emberhut.desktop", Use::Look).is_some());
        assert!(never_used("org.keepassxc.keepassxc"));
        assert!(never_used("com.github.wez.wezterm"));
        assert!(never_used("com.apple.UserNotificationCenter"));
        assert!(never_used(
            "com.apple.accessibility.universalAccessAuthWarn"
        ));
        assert!(never_used("COM.APPLE.SETTINGS"));
        assert!(!never_used("com.apple.calculator"));
    }

    #[test]
    fn terminal_is_only_looked_at() {
        assert!(refusal("Terminal", "com.apple.Terminal", Use::Look).is_none());
        let refused = refusal("Terminal", "com.apple.Terminal", Use::Act).unwrap();
        assert!(refused.text.starts_with("Terminal is only looked at"));
    }

    #[test]
    fn password_fields_are_secret_by_role_subrole_or_label() {
        assert!(is_secret("AXSecureTextField", "", ""));
        assert!(is_secret("AXTextField", "AXSecureTextField", "Name"));
        assert!(is_secret("AXTextField", "", "Verification code"));
        assert!(is_secret("AXTextField", "", "Password"));
        assert!(!is_secret("AXTextField", "", "Name"));
        // A button or a text that mentions a password is not a field.
        assert!(!is_secret("AXButton", "", "Forgot password?"));
        assert!(!is_secret("AXStaticText", "", "Password"));
    }
}
