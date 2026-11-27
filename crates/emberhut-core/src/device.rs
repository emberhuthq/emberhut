//! What a device says about itself when it signs in.

/// The longest name kept, in characters.
const NAME_LIMIT: usize = 60;
/// The longest model kept, in characters.
const MODEL_LIMIT: usize = 40;
/// The longest install id kept, in characters.
const INSTALL_LIMIT: usize = 64;
/// The name of a device that gave none.
const UNNAMED: &str = "A device";

/// A device's name, model and install id, trimmed and cut to the lengths the server keeps.
///
/// Sign-in is lenient here on purpose: a device that sends a long or empty name still signs
/// in, with its name cut or [a default](DeviceDetails::name) put in its place.
///
/// ```
/// use emberhut_core::DeviceDetails;
///
/// let device = DeviceDetails::new(Some("  Work laptop "), None, Some("install-1"));
/// assert_eq!(device.name(), "Work laptop");
/// assert_eq!(device.model(), "");
/// assert_eq!(device.install(), Some("install-1"));
/// assert_eq!(DeviceDetails::new(None, None, None).name(), "A device");
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceDetails {
    name: String,
    model: String,
    install: Option<String>,
}

impl DeviceDetails {
    /// The details from what the device sent; any of them may be missing.
    #[must_use]
    pub fn new(name: Option<&str>, model: Option<&str>, install: Option<&str>) -> Self {
        Self {
            name: clipped(name, NAME_LIMIT).unwrap_or_else(|| UNNAMED.to_owned()),
            model: clipped(model, MODEL_LIMIT).unwrap_or_default(),
            install: clipped(install, INSTALL_LIMIT),
        }
    }

    /// The device's name; "A device" when it gave none.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The device's model; empty when it gave none.
    #[must_use]
    pub fn model(&self) -> &str {
        &self.model
    }

    /// The app's id for this install, which keeps one row per install across sign-ins.
    #[must_use]
    pub fn install(&self) -> Option<&str> {
        self.install.as_deref()
    }
}

/// `text` trimmed and cut to `limit` characters; `None` when nothing is left.
fn clipped(text: Option<&str>, limit: usize) -> Option<String> {
    let text = text?.trim();
    (!text.is_empty()).then(|| text.chars().take(limit).collect())
}

/// An install id as the server keeps it (see [`DeviceDetails::install`]).
pub(crate) fn install_id(install: Option<&str>) -> Option<String> {
    clipped(install, INSTALL_LIMIT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_fields_are_cut_by_characters() {
        let long = "é".repeat(100);
        let device = DeviceDetails::new(Some(&long), Some(&long), Some(&long));
        assert_eq!(device.name().chars().count(), NAME_LIMIT);
        assert_eq!(device.model().chars().count(), MODEL_LIMIT);
        assert_eq!(
            device.install().map(|i| i.chars().count()),
            Some(INSTALL_LIMIT)
        );
    }

    #[test]
    fn blank_fields_count_as_missing() {
        let device = DeviceDetails::new(Some("   "), Some(""), Some(" "));
        assert_eq!(device, DeviceDetails::new(None, None, None));
        assert_eq!(device.name(), UNNAMED);
        assert_eq!(device.install(), None);
    }
}
