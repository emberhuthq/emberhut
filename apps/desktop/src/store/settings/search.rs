//! Settings' search over [`ENTRIES`]: case-insensitive, words in any order, each found in a
//! setting's title, line, place or other names. Title matches rank first, then the other
//! names, then the rest; everyday settings before technical ones, then the list's order. With
//! nothing found, the closest few: those that match some of the words, or the start of one.
//! It only finds: opening a result is the view's, and nothing here changes a setting.

use std::cmp::Reverse;
use std::ops::Range;

use super::entries::{ENTRIES, Entry, Kind};
use super::now::Now;

/// How many of the closest settings show when nothing matches.
const CLOSEST: usize = 3;

/// The shortest start of a word that still counts as a near match.
const NEAR: usize = 3;

/// A setting found, with what to mark in its title and line.
#[derive(Clone, Debug)]
pub(crate) struct Hit {
    /// The setting.
    pub(crate) entry: &'static Entry,
    /// Its line now.
    pub(crate) line: String,
    /// The parts of its title that matched, as byte ranges.
    pub(crate) title_marks: Vec<Range<usize>>,
    /// The parts of its line that matched.
    pub(crate) line_marks: Vec<Range<usize>>,
}

/// What a search found.
#[derive(Clone, Debug, Default)]
pub(crate) struct Found {
    /// The settings that match every word, best first.
    pub(crate) hits: Vec<Hit>,
    /// With no hits, the closest few.
    pub(crate) closest: Vec<Hit>,
}

impl Found {
    /// The rows shown: the hits, or else the closest.
    #[must_use]
    pub(crate) fn rows(&self) -> &[Hit] {
        if self.hits.is_empty() {
            &self.closest
        } else {
            &self.hits
        }
    }
}

/// The query's words, lowercased.
fn words(query: &str) -> Vec<String> {
    query.split_whitespace().map(str::to_lowercase).collect()
}

/// Where `word` starts in `text` (lowercased), if it does: at a word's start first.
fn find(text: &str, word: &str) -> Option<(usize, bool)> {
    let lower = text.to_lowercase();
    // Marks are byte ranges of `text`: only where lowercasing kept every byte in place.
    if lower.len() != text.len() {
        return lower.contains(word).then_some((usize::MAX, false));
    }
    let mut first = None;
    for (at, _) in lower.match_indices(word) {
        let starts = at == 0 || !lower[..at].ends_with(char::is_alphanumeric);
        if starts {
            return Some((at, true));
        }
        first.get_or_insert((at, false));
    }
    first
}

/// How well `word` matches `entry`: 0 for not at all.
fn score(entry: &Entry, line: &str, path: &str, word: &str) -> u32 {
    let title = match find(entry.title, word) {
        Some((_, true)) => 12,
        Some((_, false)) => 9,
        None => 0,
    };
    let other = entry
        .synonyms
        .iter()
        .filter_map(|synonym| find(synonym, word))
        .map(|(_, start)| if start { 6 } else { 4 })
        .max()
        .unwrap_or(0);
    let rest = [line, path]
        .iter()
        .filter_map(|text| find(text, word))
        .map(|(_, start)| if start { 2 } else { 1 })
        .max()
        .unwrap_or(0);
    title.max(other).max(rest)
}

/// The byte ranges of `words` in `text`, sorted, for marking.
fn marks(text: &str, words: &[String]) -> Vec<Range<usize>> {
    let mut marks: Vec<Range<usize>> = words
        .iter()
        .filter_map(|word| match find(text, word) {
            Some((at, _)) if at != usize::MAX => Some(at..at + word.len()),
            _ => None,
        })
        .collect();
    marks.sort_by_key(|range| range.start);
    marks
}

/// The hit for `entry` under `words`.
fn hit(entry: &'static Entry, line: String, words: &[String]) -> Hit {
    Hit {
        title_marks: marks(entry.title, words),
        line_marks: marks(&line, words),
        line,
        entry,
    }
}

/// Searches every setting shown now for `query`.
#[must_use]
pub(crate) fn search(query: &str, now: &Now) -> Found {
    let words = words(query);
    if words.is_empty() {
        return Found::default();
    }
    let mut ranked = Vec::new();
    let mut near = Vec::new();
    for (order, entry) in ENTRIES.iter().enumerate() {
        if !entry.shown(now) {
            continue;
        }
        let (line, path) = (entry.line(now), entry.path());
        let scores: Vec<u32> = words
            .iter()
            .map(|word| score(entry, &line, &path, word))
            .collect();
        let technical = entry.kind == Kind::Technical;
        if scores.iter().all(|score| *score > 0) {
            let in_title = words.iter().all(|word| find(entry.title, word).is_some());
            let total: u32 = scores.iter().sum();
            ranked.push((
                (Reverse(in_title), Reverse(total), technical, order),
                entry,
                line,
            ));
            continue;
        }
        // Near: some words match, or the start of one does.
        let partial: u32 = words
            .iter()
            .map(|word| {
                let chars: Vec<char> = word.chars().collect();
                (NEAR..=chars.len())
                    .rev()
                    .map(|n| chars[..n].iter().collect::<String>())
                    .map(|start| score(entry, &line, &path, &start))
                    .find(|score| *score > 0)
                    .unwrap_or(0)
            })
            .sum();
        if partial > 0 {
            near.push(((Reverse(partial), technical, order), entry, line));
        }
    }
    ranked.sort_by_key(|(key, ..)| *key);
    let hits: Vec<Hit> = ranked
        .into_iter()
        .map(|(_, entry, line)| hit(entry, line, &words))
        .collect();
    let closest = if hits.is_empty() {
        near.sort_by_key(|(key, ..)| *key);
        near.into_iter()
            .take(CLOSEST)
            .map(|(_, entry, line)| hit(entry, line, &words))
            .collect()
    } else {
        Vec::new()
    };
    Found { hits, closest }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(found: &[Hit]) -> Vec<&'static str> {
        found.iter().map(|hit| hit.entry.id).collect()
    }

    fn first(query: &str) -> &'static str {
        search(query, &Now::default()).hits[0].entry.id
    }

    #[test]
    fn other_names_find_their_settings() {
        for (query, id) in [
            ("MCP", "tools"),
            ("model", "thinks-with"),
            ("pause", "on-duty"),
            ("theme", "appearance-mode"),
            ("dark mode", "appearance-mode"),
            ("pair", "pair"),
            ("QR", "pair"),
            ("forget", "about-you"),
            ("memory", "about-you"),
            ("gmail", "sources"),
            ("version", "updates"),
        ] {
            assert_eq!(first(query), id, "{query}");
        }
    }

    #[test]
    fn a_title_match_ranks_first_and_is_marked() {
        let found = search("dar", &Now::default());
        assert_eq!(ids(&found.hits)[..2], ["appearance-mode", "dark-room"]);
        assert_eq!(found.hits[0].title_marks.first(), Some(&(0..3)));
        assert!(
            ids(&found.hits).contains(&"colour"),
            "found in its line too: {:?}",
            ids(&found.hits)
        );
    }

    #[test]
    fn words_match_in_any_order_and_any_case() {
        assert_eq!(first("ROOMS archived"), "archived-rooms");
        assert_eq!(first("history dark"), "dark-room");
    }

    #[test]
    fn everyday_settings_rank_over_technical_ones() {
        let found = search("reconnect", &Now::default());
        assert_eq!(ids(&found.hits)[..2], ["sources", "tools"]);
        let found = search("server", &Now::default());
        assert_eq!(found.hits[0].entry.id, "tools", "a title match first");
        assert!(ids(&found.hits).contains(&"server"));
    }

    #[test]
    fn nothing_found_offers_the_closest() {
        let found = search("darkk", &Now::default());
        assert!(found.hits.is_empty());
        assert!(!found.closest.is_empty() && found.closest.len() <= CLOSEST);
        assert_eq!(found.rows()[0].entry.id, "appearance-mode");
        assert!(search("zzzz", &Now::default()).rows().is_empty());
    }

    #[test]
    fn the_account_is_found_only_in_builds_with_it() {
        assert!(!ids(&search("account", &Now::default()).hits).contains(&"account"));
        let now = Now {
            has_account: true,
            ..Now::default()
        };
        assert_eq!(search("account", &now).hits[0].entry.id, "account");
    }

    #[test]
    fn live_lines_are_searched_as_they_read_now() {
        let now = Now {
            provider: Some("DeepSeek · key ending 0000".into()),
            ..Now::default()
        };
        let found = search("deepseek", &now);
        assert_eq!(found.hits[0].entry.id, "thinks-with");
        assert_eq!(found.hits[0].line_marks.first(), Some(&(0..8)));
    }
}
