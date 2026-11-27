//! This device as the server's device list names it.

use crate::brand::APP_NAME;

/// The name a pairing gives this device: `Emberhut · <host>`.
#[must_use]
pub(crate) fn name() -> String {
    format!("{APP_NAME} · {}", host_name())
}

/// This machine's name: the Mac's Computer Name (a Mac has no `/etc/hostname`), else the host
/// name, else the platform's.
fn host_name() -> String {
    #[cfg(target_os = "macos")]
    if let Some(name) = mac_computer_name() {
        return name;
    }
    std::env::var("COMPUTERNAME")
        .ok()
        .or_else(|| std::fs::read_to_string("/etc/hostname").ok())
        .map(|name| name.trim().to_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| platform_name().to_owned())
}

#[cfg(target_os = "macos")]
fn mac_computer_name() -> Option<String> {
    let out = std::process::Command::new("/usr/sbin/scutil")
        .args(["--get", "ComputerName"])
        .output()
        .inspect_err(|error| log::debug!("scutil didn't run: {error}"))
        .ok()?;
    let name = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    (out.status.success() && !name.is_empty()).then_some(name)
}

fn platform_name() -> &'static str {
    match std::env::consts::OS {
        "macos" => "Mac",
        "windows" => "Windows",
        _ => "Linux",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_device_is_named_after_the_app_and_the_host() {
        let host = host_name();
        assert_eq!(name(), format!("Emberhut · {host}"));
        assert_ne!(host, "");
        assert_eq!(
            host.trim(),
            host,
            "no stray whitespace or newline from the host file"
        );
    }

    #[test]
    fn a_platform_without_a_host_name_is_named_for_its_kind() {
        let expected = match std::env::consts::OS {
            "macos" => "Mac",
            "windows" => "Windows",
            _ => "Linux",
        };
        assert_eq!(platform_name(), expected);
    }
}
