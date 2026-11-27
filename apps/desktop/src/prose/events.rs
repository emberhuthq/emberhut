//! A run of the server's event lines (what the assistant noticed: check-ins, mail, work done)
//! summed up for the one quiet line that stands for it, and listed when it is opened. The server
//! writes each as a `- [kind] text` line.

/// The most lines an opened run lists; the rest are left out.
pub(crate) const MOST_LINES: usize = 100;

/// The most characters of an event line kept (it shows in two lines at most).
pub(crate) const MOST_LINE_CHARS: usize = 400;

/// One event line: `- [checkin] Morning check-in`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct EventLine {
    /// Its kind as the server names it (`checkin`, `new_mail`).
    pub(crate) kind: String,
    /// What happened.
    pub(crate) text: String,
}

impl EventLine {
    /// Whether it is a check-in (drawn with an alarm clock).
    #[must_use]
    pub(crate) fn is_check_in(&self) -> bool {
        self.kind == "checkin" || self.kind == "check_in"
    }
}

/// The event lines in the texts of a run, in order, up to [`MOST_LINES`], each cut at
/// [`MOST_LINE_CHARS`]. Lines that aren't `- [kind] text` are left out.
#[must_use]
pub(crate) fn lines<'a>(texts: impl IntoIterator<Item = &'a str>) -> Vec<EventLine> {
    texts
        .into_iter()
        .flat_map(str::lines)
        .filter_map(line)
        .take(MOST_LINES)
        .collect()
}

/// `- [kind] text` read, the kind being letters, digits and `_`.
fn line(raw: &str) -> Option<EventLine> {
    let rest = raw.trim_end().strip_prefix("- [")?;
    let (kind, text) = rest.split_once("] ")?;
    (!kind.is_empty() && kind.chars().all(|c| c.is_alphanumeric() || c == '_')).then(|| EventLine {
        kind: cut(kind),
        text: cut(text),
    })
}

/// `text`'s first [`MOST_LINE_CHARS`] characters.
#[must_use]
pub(crate) fn cut(text: &str) -> String {
    text.chars().take(MOST_LINE_CHARS).collect()
}

/// A kind as words: `checkin` reads `check-in`, `new_mail` reads `new mail`.
#[must_use]
pub(crate) fn kind_name(kind: &str) -> String {
    if kind == "checkin" || kind == "check_in" {
        "check-in".to_owned()
    } else {
        kind.replace('_', " ")
    }
}

/// What the run's line says after "noticed": each kind once with its count, in the order they
/// first come (`1 check-in · 2 new mails`); `None` when no line is an event line (the line then
/// shows the run's text as it is).
#[must_use]
pub(crate) fn summary(lines: &[EventLine]) -> Option<String> {
    let mut kinds: Vec<(String, usize)> = Vec::new();
    for line in lines {
        let name = kind_name(&line.kind);
        match kinds.iter_mut().find(|(kind, _)| *kind == name) {
            Some((_, count)) => *count += 1,
            None => kinds.push((name, 1)),
        }
    }
    (!kinds.is_empty()).then(|| {
        kinds
            .iter()
            .map(|(kind, count)| counted(kind, *count))
            .collect::<Vec<_>>()
            .join(" · ")
    })
}

/// `count` of `kind`: `1 check-in`, `2 new mails`, `3 replies`, `2 news`.
fn counted(kind: &str, count: usize) -> String {
    if count == 1 || kind.ends_with('s') {
        format!("{count} {kind}")
    } else if let Some(stem) = kind.strip_suffix('y') {
        format!("{count} {stem}ies")
    } else {
        format!("{count} {kind}s")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_run_is_summed_up_by_kind() {
        let found = lines([
            "- [checkin] Morning\n- [new_mail] From the bank",
            "- [new_mail] From the school\nnot an event line",
            "- [reply] One\n- [reply] Two\n- [news] A\n- [news] B",
        ]);
        assert_eq!(found.len(), 7);
        assert!(found[0].is_check_in());
        assert_eq!(found[1].text, "From the bank");
        assert_eq!(
            summary(&found).as_deref(),
            Some("1 check-in · 2 new mails · 2 replies · 2 news")
        );
    }

    #[test]
    fn a_run_with_no_event_lines_has_no_summary() {
        let found = lines(["something happened", "- [] empty kind", "- [a b] spaced"]);
        assert_eq!(found, []);
        assert_eq!(summary(&found), None);
    }

    #[test]
    fn an_opened_run_lists_at_most_so_many_lines() {
        let long = "- [work] x\n".repeat(MOST_LINES + 5);
        assert_eq!(lines([long.as_str()]).len(), MOST_LINES);
        let wide = format!("- [work] {}", "x".repeat(MOST_LINE_CHARS * 3));
        assert_eq!(
            lines([wide.as_str()])[0].text.chars().count(),
            MOST_LINE_CHARS
        );
    }

    #[test]
    fn kinds_read_as_words() {
        assert_eq!(kind_name("checkin"), "check-in");
        assert_eq!(kind_name("check_in"), "check-in");
        assert_eq!(kind_name("new_mail"), "new mail");
    }
}
