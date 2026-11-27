//! `emberhut-hands`: the helper's binary. On macOS it serves the server until it is stopped; it is
//! built elsewhere only so the workspace builds everywhere, and says so when run.

#[cfg(target_os = "macos")]
mod serve;

#[cfg(target_os = "macos")]
fn main() -> anyhow::Result<()> {
    serve::run()
}

#[cfg(not(target_os = "macos"))]
fn main() -> anyhow::Result<()> {
    anyhow::bail!(
        "Emberhut Hands runs on macOS only: it uses the Mac's Accessibility and Screen Recording"
    )
}
