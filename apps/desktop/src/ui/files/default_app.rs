//! The app this device opens a kind of file with, by name, for "Open in Preview": asked of Launch
//! Services on macOS; elsewhere not known here ("Open in Default App").

/// The name of the app that opens files called like `name`, if the platform says.
#[cfg(target_os = "macos")]
#[must_use]
pub(super) fn app_for(name: &str) -> Option<String> {
    use objc2_app_kit::NSWorkspace;
    use objc2_foundation::NSString;
    use objc2_uniform_type_identifiers::UTType;

    let extension = std::path::Path::new(name).extension()?.to_str()?;
    let kind = UTType::typeWithFilenameExtension(&NSString::from_str(extension))?;
    let app = NSWorkspace::sharedWorkspace().URLForApplicationToOpenContentType(&kind)?;
    let file = app.lastPathComponent()?.to_string();
    let shown = file.strip_suffix(".app").unwrap_or(&file).to_owned();
    (!shown.is_empty()).then_some(shown)
}

/// The name of the app that opens files called like `name`: not known here.
#[cfg(not(target_os = "macos"))]
#[must_use]
pub(super) fn app_for(_name: &str) -> Option<String> {
    None
}

/// What Open in its app says for a file called `name`.
#[must_use]
pub(super) fn open_in(name: &str) -> String {
    format!(
        "Open in {}",
        app_for(name).unwrap_or_else(|| "Default App".to_owned())
    )
}
