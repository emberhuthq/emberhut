//! The tokens file: a `NAME=TOKEN` line per machine, the bearer token its coding-thread app
//! takes. It is read at every call, with tokio's file system calls (off the async workers), so a
//! new token works without a restart.

use std::path::Path;

use super::CodingAppError;

/// `machine`'s token in the tokens file at `path`, read now; `None` when the file has none.
pub(super) async fn read(path: &Path, machine: &str) -> Result<Option<String>, CodingAppError> {
    let text =
        tokio::fs::read_to_string(path)
            .await
            .map_err(|error| CodingAppError::Unreachable {
                machine: machine.to_owned(),
                reason: format!("cannot read the tokens file: {error}"),
            })?;
    Ok(token_in(&text, machine))
}

/// `machine`'s token in a tokens file's `text`: the first `NAME=TOKEN` line for it. A line
/// starting with `#` is a comment.
fn token_in(text: &str, machine: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.starts_with('#'))
        .filter_map(|line| line.split_once('='))
        .find(|(name, _)| name.trim() == machine)
        .map(|(_, token)| token.trim().to_owned())
        .filter(|token| !token.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_token_is_found_by_its_machine() {
        let file = "# tokens\nlaptop = abc\n\ndesk=def=g\nempty=\n";
        assert_eq!(token_in(file, "laptop").as_deref(), Some("abc"));
        assert_eq!(token_in(file, "desk").as_deref(), Some("def=g"));
        assert_eq!(token_in(file, "empty"), None);
        assert_eq!(token_in(file, "other"), None);
    }
}
