//! Running external tools, with the command line in every error.

use std::process::{Command, ExitStatus, Stdio};

use anyhow::{Context as _, bail};

/// A command's exit status and what it wrote, decoded lossily.
#[derive(Debug)]
pub(crate) struct Captured {
    /// How the command exited.
    pub(crate) status: ExitStatus,
    /// What it wrote to stdout.
    pub(crate) stdout: String,
    /// What it wrote to stderr.
    pub(crate) stderr: String,
}

impl Captured {
    /// Its stdout followed by its stderr.
    #[must_use]
    pub(crate) fn combined(&self) -> String {
        format!("{}{}", self.stdout, self.stderr)
    }
}

/// Runs `command` with the terminal as its stdio and waits for it.
///
/// # Errors
///
/// Fails, naming the command line, when it can't start or exits unsuccessfully.
pub(crate) fn run(command: &mut Command) -> anyhow::Result<()> {
    let status = command
        .status()
        .with_context(|| format!("could not start `{}`", describe(command)))?;
    if !status.success() {
        bail!("`{}` failed ({status})", describe(command));
    }
    Ok(())
}

/// Runs `command`, capturing its output; the caller judges the exit status.
///
/// # Errors
///
/// Fails, naming the command line, when it can't start.
pub(crate) fn capture(command: &mut Command) -> anyhow::Result<Captured> {
    let output = command
        .output()
        .with_context(|| format!("could not start `{}`", describe(command)))?;
    Ok(Captured {
        status: output.status,
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    })
}

/// Runs `command` with stderr on the terminal and returns what it wrote to stdout.
///
/// # Errors
///
/// Fails, naming the command line, when it can't start or exits unsuccessfully.
pub(crate) fn stdout(command: &mut Command) -> anyhow::Result<String> {
    let output = command
        .stdin(Stdio::null())
        .stderr(Stdio::inherit())
        .output()
        .with_context(|| format!("could not start `{}`", describe(command)))?;
    if !output.status.success() {
        bail!("`{}` failed ({})", describe(command), output.status);
    }
    String::from_utf8(output.stdout)
        .with_context(|| format!("`{}` wrote output that isn't UTF-8", describe(command)))
}

/// `command` as a shell-like line, for messages; arguments with spaces or quotes are quoted.
#[must_use]
pub(crate) fn describe(command: &Command) -> String {
    std::iter::once(command.get_program())
        .chain(command.get_args())
        .map(|part| quote(&part.to_string_lossy()))
        .collect::<Vec<_>>()
        .join(" ")
}

/// `word` as a shell would need it written.
fn quote(word: &str) -> String {
    let plain = !word.is_empty()
        && word
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_./=:,+@%".contains(c));
    if plain {
        word.to_owned()
    } else {
        format!("'{}'", word.replace('\'', r"'\''"))
    }
}

#[cfg(test)]
mod tests {
    use std::process::Command;

    use super::{capture, describe, run, stdout};

    #[test]
    fn describe_quotes_only_what_needs_it() {
        let mut command = Command::new("/usr/bin/codesign");
        command.args([
            "--sign",
            "Developer ID Application: Example",
            "it's",
            "",
            "a/b.app",
        ]);
        assert_eq!(
            describe(&command),
            r"/usr/bin/codesign --sign 'Developer ID Application: Example' 'it'\''s' '' a/b.app"
        );
    }

    #[test]
    fn a_missing_program_names_the_command() {
        let err = run(Command::new("/nonexistent/tool").arg("--flag")).unwrap_err();
        assert!(
            err.to_string().contains("`/nonexistent/tool --flag`"),
            "{err}"
        );
    }

    #[test]
    fn a_failing_command_names_the_command_and_its_status() {
        let err = run(&mut Command::new("false")).unwrap_err();
        let message = err.to_string();
        assert!(message.contains("`false` failed"), "{message}");
        assert!(message.contains("exit status: 1"), "{message}");
    }

    #[test]
    fn capture_keeps_stdout_and_stderr() {
        let captured =
            capture(Command::new("sh").args(["-c", "echo out; echo err >&2; exit 3"])).unwrap();
        assert_eq!(captured.status.code(), Some(3));
        assert_eq!(captured.stdout, "out\n");
        assert_eq!(captured.stderr, "err\n");
        assert_eq!(captured.combined(), "out\nerr\n");
    }

    #[test]
    fn stdout_returns_stdout_and_checks_the_status() {
        let text = stdout(Command::new("sh").args(["-c", "echo out"])).unwrap();
        assert_eq!(text, "out\n");
        let err = stdout(Command::new("sh").args(["-c", "exit 2"])).unwrap_err();
        assert!(err.to_string().contains("exit status: 2"), "{err}");
    }
}
