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

/// Long text in pieces of about [`PIECE`] characters. Each is cut, in this order of preference,
/// at a paragraph break, a line break, the end of a sentence or a space, whichever comes first
/// in that order in the second half of the window; a sentence keeps its full stop. The pieces
/// are trimmed and none is empty.
#[must_use]
pub fn pieces(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest: Vec<char> = text.trim().chars().collect();
    while rest.len() > PIECE {
        let cut_at = cut_point(&rest[..PIECE]);
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

/// Where to cut `window` (just after the boundary): the latest paragraph break in its second
/// half, else the latest line break there, else the latest sentence end, else the latest
/// space, else the whole window.
fn cut_point(window: &[char]) -> usize {
    let half = window.len() / 2;
    let after_last = |is_boundary: &dyn Fn(usize) -> bool| {
        (half..window.len())
            .rev()
            .find(|at| is_boundary(*at))
            .map(|at| at + 1)
    };
    let paragraph = |at: usize| at > 0 && window[at] == '\n' && window[at - 1] == '\n';
    let line = |at: usize| window[at] == '\n';
    let sentence = |at: usize| {
        matches!(window[at], '.' | '!' | '?' | '؟' | '。')
            && window.get(at + 1).is_none_or(|next| next.is_whitespace())
    };
    let space = |at: usize| window[at].is_whitespace();
    after_last(&paragraph)
        .or_else(|| after_last(&line))
        .or_else(|| after_last(&sentence))
        .or_else(|| after_last(&space))
        .unwrap_or(window.len())
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

/// How close two normalised vectors are: their dot product. Vectors of different lengths come
/// from different models and are not compared: they are as far apart as can be (`-1`).
#[must_use]
pub fn closeness(a: &[f32], b: &[f32]) -> f64 {
    if a.len() != b.len() {
        return -1.0;
    }
    a.iter()
        .zip(b)
        .map(|(x, y)| f64::from(*x) * f64::from(*y))
        .sum()
}

/// Every piece's vector in one block, each held as `i8`s with a scale of its own (a quarter of
/// the room `f32`s take), for scoring a query against all of them. A snapshot: a change makes a
/// new one ([`Vectors::with`], [`Vectors::retaining`]), so a search scores one that nothing
/// changes under it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Vectors {
    dimension: usize,
    ids: Vec<i64>,
    scales: Vec<f32>,
    parts: Vec<i8>,
}

impl Vectors {
    /// These vectors (all of one length; one of another length is left out).
    #[must_use]
    pub fn from_pairs(pairs: impl IntoIterator<Item = (i64, Vec<f32>)>) -> Self {
        let mut vectors = Self::default();
        for (id, vector) in pairs {
            vectors.push(id, &vector);
        }
        vectors
    }

    fn push(&mut self, id: i64, vector: &[f32]) {
        if vector.is_empty() {
            return;
        }
        if self.ids.is_empty() {
            self.dimension = vector.len();
        }
        if vector.len() != self.dimension {
            return;
        }
        let top = vector.iter().fold(0.0_f32, |top, x| top.max(x.abs()));
        let scale = if top > 0.0 && top.is_finite() {
            top / 127.0
        } else {
            1.0
        };
        self.ids.push(id);
        self.scales.push(scale);
        self.parts.extend(vector.iter().map(|x| {
            #[expect(
                clippy::cast_possible_truncation,
                reason = "rounded and clamped to i8's range"
            )]
            let part = (x / scale).round().clamp(-127.0, 127.0) as i8;
            part
        }));
    }

    /// How many vectors it holds.
    #[must_use]
    pub fn len(&self) -> usize {
        self.ids.len()
    }

    /// Whether it holds none.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    /// Their length (0 when there are none).
    #[must_use]
    pub fn dimension(&self) -> usize {
        self.dimension
    }

    /// A copy with `added` in it, each in place of the vector its piece had.
    #[must_use]
    pub fn with(&self, added: &[(i64, Vec<f32>)]) -> Self {
        let replaced: HashSet<i64> = added.iter().map(|(id, _)| *id).collect();
        let mut next = self.retaining(|id| !replaced.contains(&id));
        for (id, vector) in added {
            next.push(*id, vector);
        }
        next
    }

    /// A copy with only the vectors whose piece `keep` says to keep.
    #[must_use]
    pub fn retaining(&self, keep: impl Fn(i64) -> bool) -> Self {
        let mut next = Self {
            dimension: self.dimension,
            ..Self::default()
        };
        for (n, id) in self.ids.iter().enumerate() {
            if keep(*id) {
                next.ids.push(*id);
                next.scales.push(self.scales[n]);
                next.parts
                    .extend_from_slice(&self.parts[n * self.dimension..(n + 1) * self.dimension]);
            }
        }
        next
    }

    /// Every piece's closeness to `query` (normalised), unordered; none for a query of another
    /// length (another model's).
    #[must_use]
    pub fn scores(&self, query: &[f32]) -> Vec<(i64, f32)> {
        if query.len() != self.dimension || self.dimension == 0 {
            return Vec::new();
        }
        self.ids
            .iter()
            .enumerate()
            .map(|(n, id)| {
                let parts = &self.parts[n * self.dimension..(n + 1) * self.dimension];
                let dot: f32 = parts
                    .iter()
                    .zip(query)
                    .map(|(part, q)| f32::from(*part) * q)
                    .sum();
                (*id, dot * self.scales[n])
            })
            .collect()
    }

    /// The `n` pieces closest to `query`, closest first.
    #[must_use]
    pub fn nearest(&self, query: &[f32], n: usize) -> Vec<(i64, f32)> {
        let mut scores = self.scores(query);
        let closer = |a: &(i64, f32), b: &(i64, f32)| b.1.total_cmp(&a.1);
        if n < scores.len() {
            scores.select_nth_unstable_by(n, closer);
            scores.truncate(n);
        }
        scores.sort_by(closer);
        scores
    }
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
    fn a_sentence_keeps_its_full_stop_and_a_paragraph_wins() {
        // Sentences only: cut after the last full stop, and the next piece starts with a word.
        let sentences = "One sentence here. ".repeat(100);
        let cut = pieces(&sentences);
        assert!(
            cut[0].ends_with("here."),
            "{:?}",
            &cut[0][cut[0].len() - 10..]
        );
        assert!(cut[1].starts_with("One"), "{:?}", &cut[1][..10]);
        // A paragraph break in the second half wins over a later sentence end.
        let text = format!("{}\n\n{}", "a".repeat(900), "Later sentence. ".repeat(60));
        let cut = pieces(&text);
        assert_eq!(cut[0], "a".repeat(900));
        assert!(cut[1].starts_with("Later"));
        // An Arabic question mark ends a sentence too.
        let arabic = "هل وصلت الطاولة؟ ".repeat(120);
        assert!(pieces(&arabic)[0].ends_with('؟'));
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
        assert!(
            closeness(&[1.0, 0.0], &[1.0, 0.0, 0.0]) < 0.0,
            "another model's vector is never close"
        );
    }

    #[test]
    fn the_vector_block_scores_like_f32_and_keeps_one_model() {
        let unit = |v: Vec<f32>| normalised(v);
        let block = Vectors::from_pairs([
            (1, unit(vec![1.0, 0.0, 0.0])),
            (2, unit(vec![0.7, 0.7, 0.0])),
            (3, unit(vec![0.0, 0.0, 1.0])),
            (4, unit(vec![1.0, 0.0])), // another model's: left out
        ]);
        assert_eq!((block.len(), block.dimension()), (3, 3));
        let query = unit(vec![1.0, 0.1, 0.0]);
        let near = block.nearest(&query, 2);
        assert_eq!(
            near.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
            vec![1, 2]
        );
        let exact = closeness(&unit(vec![0.7, 0.7, 0.0]), &query);
        assert!(
            (f64::from(near[1].1) - exact).abs() < 0.01,
            "int8 is close to f32"
        );
        assert!(
            block.nearest(&[1.0, 0.0], 2).is_empty(),
            "a query of another length"
        );

        // Piece 3 changed to say what the query asks; piece 5 is new.
        let changed = block.with(&[(3, query.clone()), (5, unit(vec![0.0, 1.0, 0.0]))]);
        assert_eq!(changed.len(), 4);
        assert_eq!(changed.nearest(&query, 1)[0].0, 3);
        assert_eq!(changed.nearest(&unit(vec![0.0, 1.0, 0.0]), 1)[0].0, 5);
        let fewer = changed.retaining(|id| id != 3);
        assert_eq!(fewer.nearest(&query, 1)[0].0, 1);
        assert_eq!(block.len(), 3, "a snapshot is not changed by its copies");
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
