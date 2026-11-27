//! Finding tasks: by key (the key prefix and a number, or the bare number) or by words of
//! their title and description.

use super::{Owner, ProjectId, Status, TaskId, wire};

/// What a search asks for.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TaskSearch {
    /// A task's number, when the query is a key or a bare number: it comes first.
    pub key: Option<TaskId>,
    /// The query's words: each one in the title or the description.
    pub words: Vec<String>,
    /// Only this project's (`Some(None)`: only tasks with no project).
    pub project: Option<Option<ProjectId>>,
    /// Only this area's.
    pub area: Option<i64>,
    /// Only these statuses.
    pub statuses: Vec<Status>,
    /// Only this owner's.
    pub owner: Option<Owner>,
    /// Only done and canceled ones.
    pub closed: bool,
    /// Only canceled ones.
    pub archived: bool,
    /// At most this many.
    pub limit: u64,
}

/// The most words of a query a search reads: each adds terms to the query, and a long one
/// would reach the database's limit on an expression's depth.
pub const MAX_WORDS: usize = 16;

/// The default number of tasks a search gives.
pub const DEFAULT_LIMIT: u64 = 50;
/// The most a search gives.
pub const MAX_LIMIT: u64 = 200;

impl TaskSearch {
    /// A search for `q`: a key or a bare number finds that task first; its words (the first
    /// [`MAX_WORDS`]) find tasks with every one of them in the title or the description.
    #[must_use]
    pub fn new(q: &str) -> Self {
        let q = q.trim();
        Self {
            key: key_of(q),
            words: q
                .split_whitespace()
                .take(MAX_WORDS)
                .map(str::to_owned)
                .collect(),
            limit: DEFAULT_LIMIT,
            ..Self::default()
        }
    }

    /// The same search giving at most `limit` (a missing or zero limit is the default; at most
    /// [`MAX_LIMIT`]).
    #[must_use]
    pub fn limited(mut self, limit: Option<u64>) -> Self {
        self.limit = limit
            .filter(|limit| *limit > 0)
            .unwrap_or(DEFAULT_LIMIT)
            .min(MAX_LIMIT);
        self
    }

    /// A list with no query: it has no findings (a search finds them too).
    #[must_use]
    pub fn is_list(&self) -> bool {
        self.words.is_empty()
    }
}

/// The task number `q` names as a key: the key prefix (in any case, its dash optional) and the
/// number, or the bare number.
#[must_use]
pub fn key_of(q: &str) -> Option<TaskId> {
    let q = q.trim();
    let prefix = wire::KEY_PREFIX.trim_end_matches('-');
    let rest = match q.get(..prefix.len()) {
        Some(head) if head.eq_ignore_ascii_case(prefix) => {
            let rest = &q[prefix.len()..];
            rest.strip_prefix('-').unwrap_or(rest).trim_start()
        }
        _ => q,
    };
    (!rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit()))
        .then(|| rest.parse::<i64>().ok())
        .flatten()
        .map(TaskId::new)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_or_a_number_names_a_task() {
        let lower = wire::KEY_PREFIX.trim_end_matches('-').to_lowercase();
        for q in [
            format!("{}212", wire::KEY_PREFIX),
            format!("{lower}212"),
            format!(" {lower}- 212 "),
            "212".to_owned(),
        ] {
            assert_eq!(key_of(&q), Some(TaskId::new(212)), "{q}");
        }
        for q in [wire::KEY_PREFIX, "report 212", "2.5", ""] {
            assert_eq!(key_of(q), None, "{q}");
        }
        let search = TaskSearch::new(" name  the report ").limited(Some(500));
        assert_eq!(search.words, ["name", "the", "report"]);
        let long = vec!["word"; MAX_WORDS * 4].join(" ");
        assert_eq!(TaskSearch::new(&long).words.len(), MAX_WORDS);
        assert_eq!(search.limit, MAX_LIMIT);
        assert_eq!(TaskSearch::new("").limited(Some(0)).limit, DEFAULT_LIMIT);
    }
}
