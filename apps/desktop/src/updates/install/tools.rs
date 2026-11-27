//! Running the system's tools (`hdiutil`, `ditto`, `codesign`, `spctl`, `plutil`) by their
//! absolute paths, so nothing on `PATH` stands in for them, and saying how one failed.

use std::ffi::OsStr;
use std::process::Command;

/// A tool that failed.
#[derive(Debug, thiserror::Error)]
pub(crate) enum ToolError {
    /// It couldn't be started.
    #[error("couldn't run {tool}")]
    Start {
        /// The tool.
        tool: &'static str,
        /// Why.
        #[source]
        error: std::io::Error,
    },
    /// It ran and said no.
    #[error("{tool} failed: {said}")]
    Failed {
        /// The tool.
        tool: &'static str,
        /// The last thing it wrote on stderr (or its exit status).
        said: String,
    },
}

/// What a tool wrote: stdout and stderr, as text.
#[derive(Debug)]
pub(crate) struct Said {
    /// Its standard output.
    pub(crate) out: String,
    /// Its standard error.
    pub(crate) err: String,
}

/// Runs `tool` with `args` and waits for it.
///
/// # Errors
///
/// It couldn't start, or it exited with a failure.
pub(crate) fn run<I, S>(tool: &'static str, args: I) -> Result<Said, ToolError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let output = Command::new(tool)
        .args(args)
        .output()
        .map_err(|error| ToolError::Start { tool, error })?;
    let said = Said {
        out: String::from_utf8_lossy(&output.stdout).into_owned(),
        err: String::from_utf8_lossy(&output.stderr).into_owned(),
    };
    if output.status.success() {
        Ok(said)
    } else {
        let last = said
            .err
            .lines()
            .rev()
            .find(|line| !line.trim().is_empty())
            .map_or_else(|| output.status.to_string(), |line| line.trim().to_owned());
        Err(ToolError::Failed { tool, said: last })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tool_that_fails_says_its_last_line() {
        let said = run("/bin/sh", ["-c", "echo out; echo first >&2; echo last >&2"]).unwrap();
        assert_eq!(said.out, "out\n");
        let error = run("/bin/sh", ["-c", "echo why >&2; exit 3"]).unwrap_err();
        assert_eq!(error.to_string(), "/bin/sh failed: why");
        let silent = run("/bin/sh", ["-c", "exit 4"]).unwrap_err();
        assert!(silent.to_string().contains('4'), "{silent}");
        assert!(matches!(
            run("/nonexistent/tool", ["x"]),
            Err(ToolError::Start { .. })
        ));
    }
}
