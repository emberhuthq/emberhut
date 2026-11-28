//! Facts: what the assistant keeps about the owner, one small markdown file per fact in the
//! context folder, grouped by category (`people/ada-lovelace.md`, `money/rent.md`).
//!
//! A file is a front matter of `key: value` lines between `---` lines, then the fact's text:
//!
//! ```text
//! ---
//! title: Ada Lovelace
//! source: they said, 14 Oct
//! updated: 2026-10-14T09:00:00.000Z
//! now: waiting on the contract
//! nowAt: 2026-10-14T09:00:00.000Z
//! chats: wa:662, slack:Team:Ada
//! important: yes
//! project: Garden
//! ---
//! Works on the engine.
//! ```
//!
//! A fact's id is its path without `.md` (`people/ada-lovelace`). A person's card is a `people`
//! fact with the assistant's running note on them (`now`), the chats that are theirs and whether
//! a message from them wakes the assistant at once (`important`).
//!
//! Every fact may also say where it came from and how far it can be trusted (ADR 0034), each
//! key optional (a file without them reads as before):
//! - `origin`: who put it there ([`origin`]), decided by the server, not the assistant;
//! - `from`: where in the record it was said (`chat #22`, a transcript's `path:line`);
//! - `pending: yes`: written from what someone else sent; it waits for the owner to confirm,
//!   and stays out of the profile until then;
//! - `confidence`, `valid` (`since..until`), `superseded_by`, `sensitive`, `last_used`;
//! - `profile`: `pin` (always in the profile) or `hide` (never).

use std::sync::LazyLock;

use regex::Regex;
use unicode_normalization::UnicodeNormalization as _;

use super::cut;

/// The categories a fact is filed under, in the order they are listed.
pub const CATEGORIES: [&str; 7] = [
    "people",
    "work",
    "money",
    "preferences",
    "health",
    "home",
    "projects",
];

/// The category of people's cards.
pub const PEOPLE: &str = "people";

/// The categories whose facts are in the profile only when the owner pins them.
pub const SENSITIVE: [&str; 2] = ["health", "money"];

/// Who put a fact there: how far its content can be trusted.
pub mod origin {
    /// The owner, in the app.
    pub const OWNER: &str = "owner";
    /// The assistant, in a turn the owner's own message started.
    pub const ASSISTANT: &str = "assistant";
    /// The assistant, in a turn something else started: someone else's message, mail, a
    /// voice note, a scheduled wake. Written pending.
    pub const MESSAGE: &str = "message";
    /// A web page.
    pub const WEB: &str = "web";
    /// Brought in from another assistant's memory.
    pub const IMPORT: &str = "import";
}

/// One fact, as its file holds it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Fact {
    /// `category/slug`.
    pub id: String,
    /// One of [`CATEGORIES`].
    pub category: String,
    /// A few words.
    pub title: String,
    /// One or two plain sentences.
    pub text: String,
    /// Where it came from.
    pub source: String,
    /// When it last changed, as the file says (an ISO instant).
    pub updated: String,
    /// People: what is going on with them now.
    pub now: Option<String>,
    /// People: when `now` was written.
    pub now_at: Option<String>,
    /// People: the chats that are theirs (`wa:662`, `slack:Team:Ada`).
    pub chats: Option<Vec<String>>,
    /// People: a message from them wakes the assistant at once. Only ever `Some(true)`.
    pub important: Option<bool>,
    /// The project it belongs to (its name); none: every room has it.
    pub project: Option<String>,
    /// Who put it there ([`origin`]); none: from before this was kept.
    pub origin: Option<String>,
    /// Where in the record it was said.
    pub from: Option<String>,
    /// How sure it is (`high`, `medium`, `low`).
    pub confidence: Option<String>,
    /// When it is true (`2026-11-25..`, `..2026-12-01`, `2026-11-25..2026-12-01`).
    pub valid: Option<String>,
    /// The fact that took its place.
    pub superseded_by: Option<String>,
    /// It waits for the owner to confirm it.
    pub pending: bool,
    /// `health` or `money`: in the profile only when pinned.
    pub sensitive: Option<String>,
    /// When the assistant last used it (an ISO date or instant).
    pub last_used: Option<String>,
    /// `pin` or `hide`: the owner's say on the profile.
    pub profile: Option<String>,
}

impl Fact {
    /// Whether it was still true on `day` (`2026-11-28`): no `valid` end before then, and
    /// nothing took its place.
    #[must_use]
    pub fn current(&self, day: &str) -> bool {
        let ended = self
            .valid
            .as_deref()
            .and_then(|valid| valid.split_once(".."))
            .map(|(_, until)| until.trim())
            .is_some_and(|until| !until.is_empty() && until < day);
        !ended && self.superseded_by.is_none()
    }

    /// Whether it is health or money, by its category or its `sensitive` key.
    #[must_use]
    pub fn is_sensitive(&self) -> bool {
        self.sensitive.is_some() || SENSITIVE.contains(&self.category.as_str())
    }
}

/// `raw`, the file `file` (`ada-lovelace.md`) in `category`'s folder, as a fact.
#[must_use]
pub fn parse(category: &str, file: &str, raw: &str) -> Fact {
    let (meta, text) = split_front_matter(raw).unwrap_or(("", raw));
    let mut fields: Vec<(String, String)> = Vec::new();
    for line in meta.split('\n') {
        let (key, value) = match line.find(':') {
            Some(at) => (&line[..at], &line[at + 1..]),
            // A line without a colon: all but its last character is the key.
            None => (
                line.char_indices()
                    .last()
                    .map_or("", |(last, _)| &line[..last]),
                line,
            ),
        };
        let key = key.trim().to_owned();
        fields.retain(|(known, _)| *known != key);
        fields.push((key, value.trim().to_owned()));
    }
    let meta = |key: &str| {
        fields
            .iter()
            .find(|(known, _)| known == key)
            .map(|(_, value)| value.clone())
    };
    let now = meta("now").filter(|now| !now.is_empty());
    let given = |key: &str| meta(key).filter(|value| !value.is_empty());
    Fact {
        id: format!("{category}/{}", file.strip_suffix(".md").unwrap_or(file)),
        category: category.to_owned(),
        title: meta("title").unwrap_or_else(|| file.to_owned()),
        text: text.trim().to_owned(),
        source: meta("source").unwrap_or_default(),
        updated: meta("updated").unwrap_or_default(),
        now_at: now.as_ref().and_then(|_| meta("nowAt")),
        now,
        chats: meta("chats")
            .filter(|chats| !chats.is_empty())
            .map(|chats| split_chats(&chats)),
        important: (meta("important").as_deref() == Some("yes")).then_some(true),
        project: meta("project").filter(|project| !project.is_empty()),
        origin: given("origin"),
        from: given("from"),
        confidence: given("confidence"),
        valid: given("valid"),
        superseded_by: given("superseded_by"),
        pending: meta("pending").as_deref() == Some("yes"),
        sensitive: given("sensitive"),
        last_used: given("last_used"),
        profile: given("profile").filter(|say| matches!(say.as_str(), "pin" | "hide")),
    }
}

/// The front matter and the text after it, when `raw` starts with one.
fn split_front_matter(raw: &str) -> Option<(&str, &str)> {
    let rest = raw.strip_prefix("---\n")?;
    let end = rest.find("\n---")?;
    let text = &rest[end + 4..];
    Some((&rest[..end], text.strip_prefix('\n').unwrap_or(text)))
}

/// A `chats:` line's chat ids. A group chat's id may hold commas of its own
/// (`slack:Team:Ada, Grace`): only a comma before a chat id starts the next one.
fn split_chats(chats: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut start = 0;
    for (at, _) in chats.match_indices(',') {
        let next = chats[at + 1..].trim_start();
        if ["slack:", "wa:", "im:"]
            .iter()
            .any(|kind| next.starts_with(kind))
        {
            found.push(&chats[start..at]);
            start = at + 1;
        }
    }
    found.push(&chats[start..]);
    found
        .into_iter()
        .map(str::trim)
        .filter(|chat| !chat.is_empty())
        .map(str::to_owned)
        .collect()
}

/// `fact`'s file, written at `now` (an ISO instant): its `updated`, and its `nowAt` when it
/// has none.
#[must_use]
pub fn render(fact: &Fact, now: &str) -> String {
    let mut extra: Vec<String> = Vec::new();
    if let Some(running) = fact.now.as_deref().filter(|running| !running.is_empty()) {
        extra.push(format!(
            "now: {}\nnowAt: {}\n",
            line(running),
            fact.now_at.as_deref().unwrap_or(now)
        ));
    }
    if let Some(chats) = fact.chats.as_ref().filter(|chats| !chats.is_empty()) {
        extra.push(format!("chats: {}\n", chats.join(", ")));
    }
    if fact.important == Some(true) {
        extra.push("important: yes\n".to_owned());
    }
    if let Some(project) = fact
        .project
        .as_deref()
        .filter(|project| !project.is_empty())
    {
        extra.push(format!("project: {}\n", line(project)));
    }
    for (key, value) in [
        ("origin", &fact.origin),
        ("from", &fact.from),
        ("confidence", &fact.confidence),
        ("valid", &fact.valid),
        ("superseded_by", &fact.superseded_by),
        ("sensitive", &fact.sensitive),
        ("last_used", &fact.last_used),
        ("profile", &fact.profile),
    ] {
        if let Some(value) = value.as_deref().filter(|value| !value.trim().is_empty()) {
            extra.push(format!("{key}: {}\n", line(value)));
        }
    }
    if fact.pending {
        extra.push("pending: yes\n".to_owned());
    }
    format!(
        "---\ntitle: {}\nsource: {}\nupdated: {now}\n{}---\n{}\n",
        line(&fact.title),
        line(&fact.source),
        extra.concat(),
        fact.text.trim()
    )
}

/// `text` on one line.
fn line(text: &str) -> String {
    text.replace('\n', " ").trim().to_owned()
}

/// What makes a word character in a slug: ASCII letters, digits and `_` (an ASCII `\w`).
fn ascii_word(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// The file name (without `.md`) for a title: lower case, accents off, words joined by `-`, at
/// most 60 characters. A title with no Latin letter (an Arabic name) keeps its own letters and
/// digits instead, so two such names don't fall to one file; one with neither is `fact`.
#[must_use]
pub fn slug(title: &str) -> String {
    let latin: String = title
        .to_lowercase()
        .nfkd()
        .filter(|c| ascii_word(*c) || c.is_whitespace() || *c == '-')
        .collect();
    let latin = cut(&dashes(latin.trim()), 60).to_owned();
    if latin.chars().any(|c| c.is_ascii_lowercase()) {
        return latin;
    }
    let own: String = plain(title)
        .chars()
        .filter(|c| {
            c.is_alphanumeric() || is_mark(*c) || c.is_whitespace() || matches!(c, '_' | '-')
        })
        .collect();
    let own = dashes(own.trim());
    let own = cut(&own, 60).trim_matches('-');
    if own.is_empty() {
        "fact".to_owned()
    } else {
        own.to_owned()
    }
}

/// Runs of white space and `_` as one `-`.
fn dashes(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut run = false;
    for c in text.chars() {
        if c.is_whitespace() || c == '_' {
            if !run {
                out.push('-');
            }
            run = true;
        } else {
            out.push(c);
            run = false;
        }
    }
    out
}

/// A combining mark (the Arabic vowel marks among them).
fn is_mark(c: char) -> bool {
    matches!(c, '\u{0300}'..='\u{036F}' | '\u{0610}'..='\u{061A}' | '\u{064B}'..='\u{065F}' | '\u{0670}' | '\u{06D6}'..='\u{06ED}')
}

/// One spelling of a name, for comparing: NFKC-composed, lower case, without the
/// Arabic vowel marks, the stretching tatweel or invisible format characters, trimmed, with
/// runs of white space as one space. `أ` typed as one character or two, or a stretched `مُـراد`,
/// is the same name.
#[must_use]
pub fn plain(text: &str) -> String {
    let kept: String = text
        .nfkc()
        .flat_map(char::to_lowercase)
        .filter(|c| {
            !matches!(c, '\u{0640}' | '\u{064B}'..='\u{065F}' | '\u{0670}') && !is_format(*c)
        })
        .collect();
    kept.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// An invisible format character (Unicode category Cf): joiners, marks of direction, the BOM.
fn is_format(c: char) -> bool {
    matches!(c,
        '\u{00AD}' | '\u{0600}'..='\u{0605}' | '\u{061C}' | '\u{06DD}' | '\u{070F}'
        | '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{2064}'
        | '\u{2066}'..='\u{206F}' | '\u{FEFF}' | '\u{FFF9}'..='\u{FFFB}')
}

/// The id for a fact titled `title` in `category`: its slug, or `slug-2`, `slug-3`... when a
/// fact other than `mine` (the one being moved, if any) has that id already (`taken`), so a
/// different name never overwrites another fact.
#[must_use]
pub fn free_id(
    category: &str,
    title: &str,
    mine: Option<&str>,
    taken: impl Fn(&str) -> bool,
) -> String {
    let slug = slug(title);
    let mut id = format!("{category}/{slug}");
    let mut n = 2;
    while Some(id.as_str()) != mine && taken(&id) {
        id = format!("{category}/{slug}-{n}");
        n += 1;
    }
    id
}

/// Whether `id` has a fact id's shape (`people/ada-lovelace`): a category (lower-case letters,
/// or an agent's own, `agent-<id>`), `/`, and letters, marks, digits, `_` or `-`. Nothing else
/// names a file in the folder.
#[must_use]
pub fn is_id(id: &str) -> bool {
    let Some((category, name)) = id.split_once('/') else {
        return false;
    };
    is_category(category)
        && !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_alphanumeric() || is_mark(c) || c == '_' || c == '-')
}

/// Whether `category` can name a folder of facts: lower-case letters, or an agent's own
/// category (`agent-<id>`, [`crate::agents::AgentId::facts_category`]).
#[must_use]
pub fn is_category(category: &str) -> bool {
    (!category.is_empty() && category.chars().all(|c| c.is_ascii_lowercase()))
        || crate::agents::AgentId::from_facts_category(category).is_some()
}

/// What separates a person's name from a description of them.
static DESCRIBED: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"\s*[(;,:]|\s+-\s+|\s+—\s+").ok());

/// A card is named for the person, never described: `Ada Lovelace (engine team; waits on
/// merges)` is `Ada Lovelace`. At most 60 characters.
#[must_use]
pub fn person_name(who: &str) -> String {
    let first = DESCRIBED
        .as_ref()
        .and_then(|described| described.split(who).next())
        .unwrap_or(who);
    cut(first.trim(), 60).to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = "---\ntitle: Ada Lovelace\nsource: they said, 14 Oct\nupdated: 2026-10-14T09:00:00.000Z\nnow: waiting on the contract\nnowAt: 2026-10-14T10:00:00.000Z\nchats: wa:662, slack:Team:Ada, Grace\nimportant: yes\nproject: Garden\n---\nWorks on the engine.\n";

    #[test]
    fn a_file_reads_as_its_fact_and_writes_back_the_same() {
        let fact = parse("people", "ada-lovelace.md", FILE);
        assert_eq!(fact.id, "people/ada-lovelace");
        assert_eq!(fact.title, "Ada Lovelace");
        assert_eq!(fact.text, "Works on the engine.");
        assert_eq!(fact.now.as_deref(), Some("waiting on the contract"));
        assert_eq!(fact.now_at.as_deref(), Some("2026-10-14T10:00:00.000Z"));
        assert_eq!(
            fact.chats,
            Some(vec![
                "wa:662".to_owned(),
                "slack:Team:Ada, Grace".to_owned()
            ])
        );
        assert_eq!(fact.important, Some(true));
        assert_eq!(fact.project.as_deref(), Some("Garden"));
        assert_eq!(render(&fact, "2026-10-14T09:00:00.000Z"), FILE);
    }

    #[test]
    fn provenance_reads_and_writes_back_and_old_files_read_as_before() {
        const KEPT: &str = "---\ntitle: Iron\nsource: said in chat\nupdated: 2026-10-14T09:00:00.000Z\norigin: message\nfrom: chat #22\nvalid: 2026-10-01..2026-10-30\nsensitive: health\nprofile: pin\npending: yes\n---\nTakes iron tablets.\n";
        let fact = parse("health", "iron.md", KEPT);
        assert_eq!(fact.origin.as_deref(), Some(origin::MESSAGE));
        assert_eq!(fact.from.as_deref(), Some("chat #22"));
        assert!(fact.pending && fact.is_sensitive());
        assert_eq!(fact.profile.as_deref(), Some("pin"));
        assert!(fact.current("2026-10-30") && !fact.current("2026-10-31"));
        assert_eq!(render(&fact, "2026-10-14T09:00:00.000Z"), KEPT);

        let old = parse("people", "ada-lovelace.md", FILE);
        assert_eq!(
            (&old.origin, old.pending, &old.profile),
            (&None, false, &None)
        );
        assert!(old.current("2030-01-01") && !old.is_sensitive());
        let odd = parse("home", "x.md", "---\ntitle: X\nprofile: always\n---\nY\n");
        assert_eq!(odd.profile, None, "only pin or hide");
    }

    #[test]
    fn a_file_without_front_matter_is_all_text() {
        let fact = parse("home", "keys.md", "Under the mat.\n");
        assert_eq!(fact.title, "keys.md");
        assert_eq!(fact.text, "Under the mat.");
        assert_eq!(fact.now, None);
        assert_eq!(fact.important, None);
    }

    #[test]
    fn slugs_keep_their_latin_form_or_their_own_letters() {
        assert_eq!(slug("Ada Lovelace"), "ada-lovelace");
        assert_eq!(slug("Café  au_lait!"), "cafe-au-lait");
        assert_eq!(slug("مراد غاز طبيعي"), "مراد-غاز-طبيعي");
        assert_eq!(slug("!!!"), "fact");
        assert_eq!(slug(&"a".repeat(80)).len(), 60);
    }

    #[test]
    fn one_name_in_any_spelling_is_one_name() {
        assert_eq!(plain("مُـراد"), plain("مراد"));
        assert_eq!(plain("  Ada   LOVELACE "), "ada lovelace");
    }

    #[test]
    fn a_taken_id_moves_to_the_next_number() {
        let taken = |id: &str| id == "work/rent" || id == "work/rent-2";
        assert_eq!(free_id("work", "Rent", None, taken), "work/rent-3");
        assert_eq!(
            free_id("work", "Rent", Some("work/rent"), taken),
            "work/rent"
        );
        assert_eq!(free_id("work", "Salary", None, taken), "work/salary");
    }

    #[test]
    fn only_a_facts_shape_is_an_id() {
        assert!(is_id("people/ada-lovelace"));
        assert!(is_id("people/مراد"));
        assert!(!is_id("../etc/passwd"));
        assert!(!is_id("people/../x"));
        assert!(!is_id("people"));
        assert!(is_id("agent-client-a/brand-voice"));
        assert!(!is_id("agent-/x"));
        assert!(!is_id("agent-Client/x"));
        assert!(!is_id("people-2/x"));
    }

    #[test]
    fn a_card_is_named_for_the_person() {
        assert_eq!(person_name("Ada Lovelace (engine; waits)"), "Ada Lovelace");
        assert_eq!(person_name("Grace - the admiral"), "Grace");
        assert_eq!(person_name("Grace"), "Grace");
    }
}
