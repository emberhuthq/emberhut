//! Forgetting: what the owner forgets is taken out of every derived copy (the fact, its pieces
//! in recall and their vectors, the pieces of the record that said it), and stays forgotten
//! (ADR 0034).
//!
//! A forgotten fact leaves a tombstone: the hashes of its distinctive words, never its text, one
//! line per fact in the context folder's [`FILE`]. A fact the assistant tries to keep, or a new
//! piece of the record, that holds enough of a tombstone's words ([`Tombstone::matches`]) is
//! refused. The hashes keep the words from being read at a glance; anyone with the file and a
//! dictionary could still test words against them, so the file stays in the owner's data folder,
//! like the rest of it.

use std::collections::HashSet;

use super::facts::plain;
use super::recall::is_stop_word;

/// The tombstones' file in the context folder.
pub const FILE: &str = ".forgotten";

/// The fewest letters a word has to count.
const SHORTEST: usize = 3;

/// `text` folded for comparing: [`plain`]'s spelling, and the Arabic letters that are written
/// either way as one (`أ`, `إ`, `آ` as `ا`; `ى` as `ي`; `ة` as `ه`).
#[must_use]
pub fn fold(text: &str) -> String {
    plain(text)
        .chars()
        .map(|c| match c {
            'أ' | 'إ' | 'آ' => 'ا',
            'ى' => 'ي',
            'ة' => 'ه',
            other => other,
        })
        .collect()
}

/// The words of `text` that can tell it apart: folded, of three letters or more, not
/// stop words, each once, in the order they come.
#[must_use]
pub fn words(text: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    fold(text)
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| word.chars().count() >= SHORTEST && !is_stop_word(word))
        .filter(|word| seen.insert((*word).to_owned()))
        .map(str::to_owned)
        .collect()
}

/// FNV-1a, 64 bits: the same on every build and platform.
#[must_use]
pub fn hash(word: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in word.as_bytes() {
        h ^= u64::from(*byte);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

/// How rare a word is in the record.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Rarity {
    /// In many pieces: it tells nothing apart.
    Common,
    /// In few pieces.
    Rare,
    /// In hardly any (a name, a medicine): a key to what was said.
    Key,
}

/// The words of a fact (`text`: its title and text) to find it and its tombstone by, and the
/// keys among them: the words `rarity` finds rare or keys, or every word when none is.
#[must_use]
pub fn distinctive(text: &str, rarity: impl Fn(&str) -> Rarity) -> (Vec<String>, Vec<String>) {
    let all: Vec<(String, Rarity)> = words(text)
        .into_iter()
        .map(|word| {
            let rarity = rarity(&word);
            (word, rarity)
        })
        .collect();
    let keys = all
        .iter()
        .filter(|(_, rarity)| *rarity == Rarity::Key)
        .map(|(word, _)| word.clone())
        .collect();
    let rare: Vec<String> = all
        .iter()
        .filter(|(_, rarity)| *rarity >= Rarity::Rare)
        .map(|(word, _)| word.clone())
        .collect();
    if rare.is_empty() {
        (all.into_iter().map(|(word, _)| word).collect(), keys)
    } else {
        (rare, keys)
    }
}

/// What is left of a forgotten fact: when, and its distinctive words' hashes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tombstone {
    /// When it was forgotten (an ISO instant).
    pub at: String,
    /// The hashes of its distinctive words, in order, each once.
    pub marks: Vec<u64>,
    /// The hashes of its keys (among them): words in hardly any piece.
    pub keys: Vec<u64>,
}

fn hashes(words: &[String]) -> Vec<u64> {
    let mut seen = HashSet::new();
    words
        .iter()
        .map(|word| hash(word))
        .filter(|mark| seen.insert(*mark))
        .collect()
}

fn hex(marks: &[u64]) -> String {
    marks
        .iter()
        .map(|mark| format!("{mark:016x}"))
        .collect::<Vec<_>>()
        .join(",")
}

fn unhex(text: &str) -> Option<Vec<u64>> {
    text.split(',')
        .filter(|mark| !mark.trim().is_empty())
        .map(|mark| u64::from_str_radix(mark.trim(), 16).ok())
        .collect()
}

impl Tombstone {
    /// The tombstone of a fact whose distinctive words are `words` and keys `keys` (any of
    /// `words`), forgotten `at`.
    #[must_use]
    pub fn new(at: &str, words: &[String], keys: &[String]) -> Self {
        Self {
            at: at.to_owned(),
            marks: hashes(words),
            keys: hashes(keys),
        }
    }

    /// Its line in [`FILE`]: the instant, the hashes in hex, comma-separated, then `;` and the
    /// keys' hashes.
    #[must_use]
    pub fn line(&self) -> String {
        format!("{} {};{}", self.at, hex(&self.marks), hex(&self.keys))
    }

    /// A line of [`FILE`] as a tombstone; none for one that isn't.
    #[must_use]
    pub fn read(line: &str) -> Option<Self> {
        let (at, rest) = line.trim().split_once(' ')?;
        let (marks, keys) = rest.split_once(';').unwrap_or((rest, ""));
        let marks = unhex(marks).filter(|marks| !marks.is_empty())?;
        Some(Self {
            at: at.to_owned(),
            marks,
            keys: unhex(keys)?,
        })
    }

    /// Whether `text` says what was forgotten. It holds two of the words (all, when there are
    /// fewer), and one of them is a key or it holds a third: two common-enough words alone
    /// ("Tuesday evenings") don't make it the same thing.
    #[must_use]
    pub fn matches(&self, text: &str) -> bool {
        let held: HashSet<u64> = words(text).iter().map(|word| hash(word)).collect();
        let found = self.marks.iter().filter(|mark| held.contains(mark)).count();
        let key = self.keys.iter().any(|mark| held.contains(mark));
        let enough = self.marks.len().min(2);
        !self.marks.is_empty()
            && found >= enough
            && (self.keys.is_empty() || key || found >= 3 || found == self.marks.len())
    }
}

/// Every tombstone in [`FILE`]'s `text`.
#[must_use]
pub fn read_all(text: &str) -> Vec<Tombstone> {
    text.lines().filter_map(Tombstone::read).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tombstone_keeps_hashes_and_finds_the_same_thing_said_again() {
        let fact = "Blood test: ferritin was low (9); taking iron tablets for three months";
        let rarity = |word: &str| match word {
            "ferritin" => Rarity::Key,
            "iron" | "tablets" => Rarity::Rare,
            _ => Rarity::Common,
        };
        let (kept, keys) = distinctive(fact, rarity);
        assert_eq!(kept, ["ferritin", "iron", "tablets"]);
        assert_eq!(keys, ["ferritin"]);
        let tomb = Tombstone::new("2026-11-28T10:00:00.000Z", &kept, &keys);
        let line = tomb.line();
        assert!(!line.contains("ferritin"), "the words are not kept");
        assert_eq!(Tombstone::read(&line), Some(tomb.clone()));
        assert_eq!(
            read_all(&format!("{line}\nnot a tombstone\n")),
            std::slice::from_ref(&tomb)
        );

        assert!(tomb.matches("The doctor put me on IRON tablets, my ferritin is 9"));
        assert!(tomb.matches("ferritin and iron"), "a key and another word");
        assert!(
            !tomb.matches("iron tablets for the plants"),
            "two rare words without a key"
        );
        assert!(
            !tomb.matches("Pick up the iron for the shirts"),
            "one word is not enough"
        );
        assert!(!tomb.matches("Lunch with Sami"));
    }

    #[test]
    fn arabic_spellings_fold_to_one() {
        assert_eq!(fold("أحمد"), fold("احمد"));
        assert_eq!(fold("مدرسة"), "مدرسه");
        let tomb = Tombstone::new("t", &words("حساسية البنسلين"), &[]);
        assert!(tomb.matches("عندها حساسيه من البنسلين"));
    }

    #[test]
    fn with_no_rare_word_every_word_counts() {
        assert_eq!(
            distinctive("the red car", |_| Rarity::Common).0,
            ["red", "car"]
        );
        let one = Tombstone::new("t", &["penicillin".to_owned()], &[]);
        assert!(one.matches("allergic to penicillin"));
    }
}
