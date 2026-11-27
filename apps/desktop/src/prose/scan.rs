//! Finding the next mark in a text when the places asked from only move forward, as a scan
//! of a message's text asks them: each search starts where the last one ended, so a text full
//! of marks with nothing to close them (`[[[[`) is read once, never once for each mark.

/// The next place where `needle` is, at or after a place.
#[derive(Clone, Debug)]
pub(crate) struct Next<'a> {
    text: &'a str,
    needle: Needle,
    /// The last answer: where it was asked from, and what was found.
    last: Option<(usize, Option<usize>)>,
}

/// What [`Next`] looks for.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Needle {
    /// These characters, in a row.
    Text(&'static str),
    /// Any white space.
    Space,
}

impl<'a> Next<'a> {
    /// Looks for `needle` in `text`.
    #[must_use]
    pub(crate) fn new(text: &'a str, needle: Needle) -> Self {
        Self {
            text,
            needle,
            last: None,
        }
    }

    /// Where `needle` next is at or after `from` (a character boundary of the text). Asked from
    /// places that only move forward, every byte of the text is searched at most once.
    pub(crate) fn at_or_after(&mut self, from: usize) -> Option<usize> {
        if let Some((asked, found)) = self.last {
            match found {
                Some(at) if at >= from && asked <= from => return Some(at),
                None if asked <= from => return None,
                _ => {}
            }
        }
        let rest = self.text.get(from..)?;
        let found = match self.needle {
            Needle::Text(needle) => rest.find(needle),
            Needle::Space => rest.find(char::is_whitespace),
        }
        .map(|at| from + at);
        self.last = Some((from, found));
        found
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_next_mark_is_found_from_anywhere() {
        let mut next = Next::new("a](b](c", Needle::Text("]("));
        assert_eq!(next.at_or_after(0), Some(1));
        assert_eq!(next.at_or_after(1), Some(1));
        assert_eq!(next.at_or_after(2), Some(4));
        assert_eq!(next.at_or_after(5), None);
        assert_eq!(next.at_or_after(9), None, "past the end");
        assert_eq!(next.at_or_after(0), Some(1), "asked again from the start");
        let mut space = Next::new("ab c\td", Needle::Space);
        assert_eq!(space.at_or_after(0), Some(2));
        assert_eq!(space.at_or_after(3), Some(4));
        assert_eq!(space.at_or_after(5), None);
    }
}
