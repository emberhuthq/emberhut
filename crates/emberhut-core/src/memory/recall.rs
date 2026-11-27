//! Recall: one search over everything the assistant has seen, so what never became a fact can
//! still be found, in other words than the ones used then.
//!
//! What is indexed is cut into pieces ([`pieces`]) of about 1,500 characters, one hit each.
//! Two searches run over them and are fused by rank ([`fuse`]): words (full-text, BM25) and
//! meaning (the nearest embedding vectors), when an embedding model is configured.

use std::collections::HashSet;
use std::sync::LazyLock;

use jiff::Timestamp;

use super::cut;

/// About how many characters a piece holds.
pub const PIECE: usize = 1500;

/// The most hits a search gives by default, and at most.
pub const LIMITS: (usize, usize) = (8, 30);

/// How many of each search's best are fused.
pub const EACH: usize = 50;

/// The kinds of piece, as stored and as the tools name them.
pub mod source {
    /// A turn of an agent session's transcript.
    pub const SESSION: &str = "session";
    /// A transcript's compaction summary.
    pub const SUMMARY: &str = "summary";
    /// A message of the chat with the assistant.
    pub const CHAT: &str = "chat";
    /// A task and its history.
    pub const TASK: &str = "task";
    /// A fact.
    pub const FACT: &str = "fact";
    /// A voice note someone sent, as what they said.
    pub const VOICE: &str = "voice";
    /// A quick note or a diary entry.
    pub const NOTE: &str = "note";
    /// Every kind, as the recall tool takes them.
    pub const ALL: [&str; 7] = [SESSION, SUMMARY, CHAT, TASK, FACT, VOICE, NOTE];
}

/// Something to index: one piece per [`PIECE`] of its text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewDoc {
    /// Its kind ([`source`]).
    pub source: String,
    /// Unique within its source (a transcript's `path:line`, `chat:12`, `people/ada`). Its
    /// pieces after the first are `ref#1`, `ref#2`...
    pub reference: String,
    /// When it was said or last changed, if known.
    pub at: Option<Timestamp>,
    /// What it is, in a few words.
    pub title: String,
    /// What it says.
    pub text: String,
    /// How to open it (`chat #12`, a transcript's `path:line`).
    pub place: String,
}

/// An indexed piece.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Doc {
    /// Its id in the index.
    pub id: i64,
    /// Its kind.
    pub source: String,
    /// Its reference.
    pub reference: String,
    /// When it was said.
    pub at: Option<Timestamp>,
    /// What it is.
    pub title: String,
    /// What it says.
    pub text: String,
    /// How to open it.
    pub place: String,
}

/// Long text in pieces of about [`PIECE`] characters, each cut at a paragraph, a line or a
/// sentence where one is in the second half of the window; the pieces are trimmed and none is
/// empty.
#[must_use]
pub fn pieces(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest: Vec<char> = text.trim().chars().collect();
    while rest.len() > PIECE {
        let window = &rest[..PIECE];
        let cut_at = [
            last_of(window, &['\n', '\n']),
            last_of(window, &['\n']),
            last_of(window, &['.', ' ']),
        ]
        .into_iter()
        .flatten()
        .max()
        .filter(|at| *at >= PIECE / 2)
        .unwrap_or(PIECE);
        out.push(rest[..cut_at].iter().collect::<String>().trim().to_owned());
        rest = rest[cut_at..]
            .iter()
            .collect::<String>()
            .trim()
            .chars()
            .collect();
    }
    if !rest.is_empty() {
        out.push(rest.into_iter().collect());
    }
    out
}

/// Where `needle` last starts in `window`.
fn last_of(window: &[char], needle: &[char]) -> Option<usize> {
    window
        .windows(needle.len())
        .rposition(|candidate| candidate == needle)
}

/// The reference of `reference`'s piece `n` (the first is the reference itself).
#[must_use]
pub fn piece_reference(reference: &str, n: usize) -> String {
    if n == 0 {
        reference.to_owned()
    } else {
        format!("{reference}#{n}")
    }
}

/// The piece number of `candidate` when it is a later piece of `reference` (`ref#3`: 3).
#[must_use]
pub fn piece_number(reference: &str, candidate: &str) -> Option<usize> {
    candidate
        .strip_prefix(reference)?
        .strip_prefix('#')?
        .parse()
        .ok()
}

/// The reference a piece's reference belongs to (`ref#3`: `ref`).
#[must_use]
pub fn base_reference(reference: &str) -> &str {
    reference.split('#').next().unwrap_or(reference)
}

/// Words too common to mean anything in a search, in English and Arabic: any of them alone
/// would match half the index.
static STOP: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    "a an the and or but if of to in on at by for with from about as is are was were be been being do does did \
     have has had i you he she it we they me him her us them my your his its our their this that these those what which who whom \
     when where why how not no yes so than then there here can could will would should may might just also very too any some all \
     في من على إلى الى عن مع هذا هذه ذلك التي الذي هو هي انا أنا انت أنت كان ما لا لم لن قد و او أو ثم بس شو اللي"
        .split(' ')
        .collect()
});

/// The words of `query` to search for: its runs of letters, digits and `_`, without the stop
/// words (unless that leaves none).
#[must_use]
pub fn query_words(query: &str) -> Vec<String> {
    let found: Vec<String> = query
        .split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .filter(|word| !word.is_empty())
        .map(str::to_owned)
        .collect();
    let kept: Vec<String> = found
        .iter()
        .filter(|word| !STOP.contains(word.to_lowercase().as_str()))
        .cloned()
        .collect();
    if kept.is_empty() { found } else { kept }
}

/// The full-text query for `words`: any of them, each as a quoted string (the words hold only
/// letters, digits and `_`, so nothing in them is read as syntax).
#[must_use]
pub fn match_query(words: &[String]) -> String {
    words
        .iter()
        .map(|word| format!("\"{word}\""))
        .collect::<Vec<_>>()
        .join(" OR ")
}

/// Reciprocal rank fusion of ranked lists of piece ids: each list gives its `n`th (from 0)
/// `1 / (60 + n)`; the best total first, ties in the order first seen. A piece both searches
/// find comes first.
#[must_use]
pub fn fuse(lists: &[Vec<i64>]) -> Vec<i64> {
    let mut scores: Vec<(i64, f64)> = Vec::new();
    for list in lists {
        for (rank, id) in list.iter().enumerate() {
            #[expect(
                clippy::cast_precision_loss,
                reason = "a rank is at most a few hundred"
            )]
            let gain = 1.0 / (60.0 + rank as f64);
            match scores.iter_mut().find(|(known, _)| known == id) {
                Some((_, score)) => *score += gain,
                None => scores.push((*id, gain)),
            }
        }
    }
    scores.sort_by(|a, b| b.1.total_cmp(&a.1));
    scores.into_iter().map(|(id, _)| id).collect()
}

/// `vector` scaled to length 1 (a zero or broken one is left as it is).
#[must_use]
pub fn normalised(mut vector: Vec<f32>) -> Vec<f32> {
    let length = vector
        .iter()
        .map(|x| f64::from(*x) * f64::from(*x))
        .sum::<f64>()
        .sqrt();
    if length > 0.0 && length.is_finite() {
        for x in &mut vector {
            #[expect(
                clippy::cast_possible_truncation,
                reason = "a unit vector's parts are within f32's range"
            )]
            let scaled = (f64::from(*x) / length) as f32;
            *x = scaled;
        }
    }
    vector
}

/// How close two normalised vectors are: their dot product (missing parts count as nothing).
#[must_use]
pub fn closeness(a: &[f32], b: &[f32]) -> f64 {
    a.iter()
        .zip(b)
        .map(|(x, y)| f64::from(*x) * f64::from(*y))
        .sum()
}

/// What the meaning search embeds for a piece.
#[must_use]
pub fn embed_text(title: &str, text: &str) -> String {
    format!("title: {title} | text: {text}")
}

/// What the meaning search embeds for a query.
#[must_use]
pub fn embed_query(query: &str) -> String {
    format!("task: search result | query: {query}")
}

/// A search's filters.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Filter {
    /// Only this kind (`session` takes `summary` too).
    pub source: Option<String>,
    /// Only pieces at or after this.
    pub since: Option<Timestamp>,
    /// Only pieces at or before this.
    pub until: Option<Timestamp>,
    /// Leave the owner's notes out (a room that isn't the owner's).
    pub without_notes: bool,
}

impl Filter {
    /// Whether `doc` passes. A piece with no time is never in a range.
    #[must_use]
    pub fn keeps(&self, doc: &Doc) -> bool {
        let source = self.source.as_deref().is_none_or(|wanted| {
            doc.source == wanted || (wanted == source::SESSION && doc.source == source::SUMMARY)
        });
        let since = self
            .since
            .is_none_or(|since| doc.at.is_some_and(|at| at >= since));
        let until = self
            .until
            .is_none_or(|until| doc.at.is_some_and(|at| at <= until));
        let notes = !(self.without_notes && doc.source == source::NOTE);
        source && since && until && notes
    }
}

/// A date or an instant given to a search (`2026-11-24`, `2026-11-24T10:00:00Z`): a day is
/// its start (UTC), as text sorts it.
#[must_use]
pub fn read_bound(text: &str) -> Option<Timestamp> {
    let text = text.trim();
    text.parse::<Timestamp>().ok().or_else(|| {
        text.parse::<jiff::civil::Date>()
            .ok()
            .and_then(|date| date.to_zoned_datetime(jiff::tz::TimeZone::UTC).ok())
            .map(|zoned| zoned.timestamp())
    })
}

/// What the same words said twice look like (the assistant's answer in its session and in the
/// chat): the first 300 characters, white space as one space.
#[must_use]
pub fn sameness(text: &str) -> String {
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    cut(&collapsed, 300).to_owned()
}

/// A search's answer.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Found {
    /// The hits, best first.
    pub hits: Vec<Doc>,
    /// The meaning search ran (an embedding model answered).
    pub meaning: bool,
    /// Pieces in the index.
    pub indexed: u64,
    /// Pieces with a vector.
    pub embedded: usize,
}

/// `found` as the recall tools answer it.
#[must_use]
pub fn render(found: &Found) -> String {
    let head = if found.meaning {
        ""
    } else {
        "(Meaning search is off: no embedding model answered, so this matched words only.)\n\n"
    };
    if found.hits.is_empty() {
        return format!("{head}Nothing found in {} indexed pieces.", found.indexed);
    }
    let hits: Vec<String> = found
        .hits
        .iter()
        .map(|hit| {
            let at = hit.at.map_or_else(String::new, |at| {
                cut(&super::iso(at), 16).replacen('T', " ", 1)
            });
            format!(
                "[{at}] {} · {}\n  {}\n{}",
                hit.source,
                hit.title,
                hit.place,
                cut(&hit.text, 900)
            )
        })
        .collect();
    format!("{head}{}", hits.join("\n\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_text_is_cut_at_a_paragraph_or_a_sentence() {
        assert_eq!(pieces("  short  "), vec!["short".to_owned()]);
        assert_eq!(pieces("   "), [] as [String; 0]);
        let paragraph = format!("{}\n\n{}", "a".repeat(1000), "b".repeat(1000));
        let cut = pieces(&paragraph);
        assert_eq!(cut, vec!["a".repeat(1000), "b".repeat(1000)]);
        let solid = "c".repeat(3100);
        let cut = pieces(&solid);
        assert_eq!(
            cut.iter().map(String::len).collect::<Vec<_>>(),
            vec![1500, 1500, 100]
        );
    }

    #[test]
    fn pieces_have_references_of_their_own() {
        assert_eq!(piece_reference("chat:1", 0), "chat:1");
        assert_eq!(piece_reference("chat:1", 2), "chat:1#2");
        assert_eq!(piece_number("chat:1", "chat:1#2"), Some(2));
        assert_eq!(piece_number("chat:1", "chat:12#2"), None);
        assert_eq!(base_reference("people/ada#3"), "people/ada");
    }

    #[test]
    fn stop_words_go_unless_nothing_is_left() {
        assert_eq!(
            query_words("what did we decide about the roof?"),
            vec!["decide", "roof"]
        );
        assert_eq!(query_words("what is this"), vec!["what", "is", "this"]);
        assert_eq!(query_words("شو قال عن السقف"), vec!["قال", "السقف"]);
        assert_eq!(
            match_query(&query_words("roof tiles")),
            "\"roof\" OR \"tiles\""
        );
    }

    #[test]
    fn what_both_searches_find_comes_first() {
        assert_eq!(fuse(&[vec![1, 2, 3], vec![3, 4]]), vec![3, 1, 2, 4]);
        assert_eq!(fuse(&[vec![5, 6]]), vec![5, 6]);
        assert_eq!(fuse(&[]), [] as [i64; 0]);
    }

    #[test]
    fn vectors_are_compared_as_unit_vectors() {
        let a = normalised(vec![3.0, 4.0]);
        assert!((closeness(&a, &a) - 1.0).abs() < 1e-6);
        assert_eq!(normalised(vec![0.0, 0.0]), vec![0.0, 0.0]);
        assert!(closeness(&normalised(vec![1.0, 0.0]), &normalised(vec![0.0, 1.0])).abs() < 1e-9);
    }

    fn doc(source: &str, at: Option<&str>) -> Doc {
        Doc {
            id: 1,
            source: source.to_owned(),
            reference: "r".to_owned(),
            at: at.map(|at| at.parse().unwrap()),
            title: "t".to_owned(),
            text: "x".to_owned(),
            place: "p".to_owned(),
        }
    }

    #[test]
    fn filters_keep_what_they_say() {
        let session = Filter {
            source: Some("session".to_owned()),
            ..Filter::default()
        };
        assert!(session.keeps(&doc("summary", None)));
        assert!(!session.keeps(&doc("chat", None)));
        let since = Filter {
            since: read_bound("2026-11-24"),
            ..Filter::default()
        };
        assert!(since.keeps(&doc("chat", Some("2026-11-24T00:00:00Z"))));
        assert!(!since.keeps(&doc("chat", Some("2026-11-23T23:59:59Z"))));
        assert!(!since.keeps(&doc("chat", None)));
        let private = Filter {
            without_notes: true,
            ..Filter::default()
        };
        assert!(!private.keeps(&doc("note", None)));
        assert_eq!(read_bound("soon"), None);
    }

    #[test]
    fn hits_read_with_their_time_and_place() {
        let mut hit = doc("chat", Some("2026-11-24T10:05:00Z"));
        hit.title = "User to assistant".to_owned();
        let found = Found {
            hits: vec![hit],
            meaning: true,
            indexed: 3,
            embedded: 3,
        };
        assert_eq!(
            render(&found),
            "[2026-11-24 10:05] chat · User to assistant\n  p\nx"
        );
        let none = Found {
            indexed: 3,
            ..Found::default()
        };
        assert!(render(&none).starts_with("(Meaning search is off"));
        assert!(render(&none).ends_with("Nothing found in 3 indexed pieces."));
        assert_eq!(sameness("a  b\n c"), "a b c");
    }
}
