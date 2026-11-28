//! The feed's notes: the merged pull requests GitHub lists for a release, as plain
//! `- title (#N)` lines without author handles or links, which the app shows as they are.

/// The most the feed's notes hold, in bytes; whole lines past it are left out.
pub(crate) const MAX_NOTES: usize = 4_000;

/// The plain notes of `body`, the Markdown GitHub's `releases/generate-notes` returns: one
/// `- title (#N)` line per pull request under "What's Changed", category headings, new
/// contributors and the comparison link left out, at most [`MAX_NOTES`] bytes.
#[must_use]
pub(crate) fn feed_notes(body: &str) -> String {
    let mut lines = Vec::new();
    for line in body.lines() {
        let line = line.trim();
        if line.starts_with("## ") && !line.eq_ignore_ascii_case("## What's Changed") {
            // "New Contributors" and anything after the change list.
            if !lines.is_empty() {
                break;
            }
            continue;
        }
        if let Some(entry) = pull_request(line) {
            lines.push(entry);
        }
    }
    cap(&lines, MAX_NOTES)
}

/// `* title by @login in https://github.com/owner/repo/pull/N` as `- title (#N)`.
fn pull_request(line: &str) -> Option<String> {
    let item = line
        .strip_prefix("* ")
        .or_else(|| line.strip_prefix("- "))?;
    let (title, rest) = item.rsplit_once(" by @")?;
    let (_login, url) = rest.split_once(" in ")?;
    let (_, number) = url.trim().rsplit_once("/pull/")?;
    let plain = !number.is_empty() && number.bytes().all(|b| b.is_ascii_digit());
    let title = title.trim();
    (plain && !title.is_empty()).then(|| format!("- {title} (#{number})"))
}

/// `lines`, each ending in a newline, as many whole ones as fit in `max` bytes.
fn cap(lines: &[String], max: usize) -> String {
    let mut notes = String::new();
    for line in lines {
        if notes.len() + line.len() + 1 > max {
            break;
        }
        notes.push_str(line);
        notes.push('\n');
    }
    notes
}

#[cfg(test)]
mod tests {
    use super::{MAX_NOTES, feed_notes};

    const BODY: &str = "<!-- Release notes generated using configuration in .github/release.yml at main -->\n\
        \n\
        ## What's Changed\n\
        ### Features\n\
        * desktop: rooms open faster by @someone in https://github.com/emberhuthq/emberhut/pull/12\n\
        * server: a title that names by @mention by @other in https://github.com/emberhuthq/emberhut/pull/15\n\
        ### Fixes\n\
        * ci: the cache key includes the lock file by @someone in https://github.com/emberhuthq/emberhut/pull/9\n\
        * not a pull request line\n\
        \n\
        ## New Contributors\n\
        * @other made their first contribution in https://github.com/emberhuthq/emberhut/pull/15\n\
        \n\
        **Full Changelog**: https://github.com/emberhuthq/emberhut/compare/v0.1.0-rc.1...v0.1.0-rc.2\n";

    #[test]
    fn each_pull_request_becomes_a_plain_line() {
        assert_eq!(
            feed_notes(BODY),
            "- desktop: rooms open faster (#12)\n\
             - server: a title that names by @mention (#15)\n\
             - ci: the cache key includes the lock file (#9)\n"
        );
    }

    #[test]
    fn no_changes_is_empty_notes() {
        assert_eq!(feed_notes(""), "");
        assert_eq!(
            feed_notes("**Full Changelog**: https://github.com/o/r/commits/v0.1.0\n"),
            ""
        );
    }

    #[test]
    fn long_notes_keep_whole_lines_within_the_cap() {
        let line =
            "* area: change number N with some words by @a in https://github.com/o/r/pull/N\n";
        let body: Vec<String> = (1..=400)
            .map(|n| line.replace('N', &n.to_string()))
            .collect();
        let body = body.concat();
        let notes = feed_notes(&body);
        assert!(notes.len() <= MAX_NOTES, "{}", notes.len());
        assert!(notes.len() > MAX_NOTES - 100);
        assert!(notes.starts_with("- area: change number 1 with some words (#1)\n"));
        assert!(notes.ends_with(")\n"));
    }
}
