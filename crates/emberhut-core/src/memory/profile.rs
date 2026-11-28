//! The profile: the few lines about the owner every session's prompt carries, so the assistant
//! knows them from its first word without reading every fact (ADR 0034).
//!
//! It is made from the facts, one line each with the fact's id, grouped by category and kept
//! within a budget ([`PROFILE_TOKENS`], counted as characters / 4). Pinned facts come first, then
//! the most recently used and changed. Left out: facts waiting for the owner to confirm them,
//! facts no longer true, facts the owner hid, health and money unless pinned, and a project's
//! facts, which go in that project's room's slice instead ([`slice()`], [`SLICE_TOKENS`]).

use super::cut;
use super::facts::{CATEGORIES, Fact, plain};

/// The profile's budget, in tokens.
pub const PROFILE_TOKENS: usize = 1500;

/// A project room's slice's budget, in tokens.
pub const SLICE_TOKENS: usize = 500;

/// The profile's file in the context folder, beside the categories' folders.
pub const FILE: &str = "_profile.md";

/// The longest a fact's text is in its line, in characters.
const LINE_TEXT: usize = 280;

/// About how many tokens `text` is: its characters / 4, rounded up.
#[must_use]
pub fn tokens(text: &str) -> usize {
    text.chars().count().div_ceil(4)
}

/// Whether `fact` may be in a profile or a slice on `day`: confirmed, still true, not hidden,
/// and health or money only when pinned.
#[must_use]
pub fn eligible(fact: &Fact, day: &str) -> bool {
    let pinned = fact.profile.as_deref() == Some("pin");
    !fact.pending
        && fact.current(day)
        && fact.profile.as_deref() != Some("hide")
        && (pinned || !fact.is_sensitive())
}

/// `fact`'s line: `- Title: what it says (now: ...) [id]`.
#[must_use]
pub fn line(fact: &Fact) -> String {
    let text = fact.text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut said = cut(&text, LINE_TEXT).to_owned();
    if said.len() < text.len() {
        said.push('…');
    }
    let now = fact
        .now
        .as_deref()
        .map_or_default(|now| format!(" (now: {})", cut(now, 160)));
    let body = if said.is_empty() {
        String::new()
    } else {
        format!(": {said}")
    };
    format!("- {}{body}{now} [{}]", fact.title, fact.id)
}

/// The order facts are chosen in: pinned first, then by when last used, then by when changed.
fn chosen_first(facts: &mut [&Fact]) {
    facts.sort_by(|a, b| {
        let pinned = |fact: &Fact| fact.profile.as_deref() == Some("pin");
        pinned(b)
            .cmp(&pinned(a))
            .then_with(|| b.last_used.cmp(&a.last_used))
            .then_with(|| b.updated.cmp(&a.updated))
    });
}

/// The lines of `facts` that fit in `budget` tokens, chosen as [`chosen_first`] orders them and
/// shown under their categories' names in [`CATEGORIES`]' order; empty when none fits.
fn within(mut facts: Vec<&Fact>, budget: usize) -> String {
    chosen_first(&mut facts);
    let mut spent = 0;
    let mut kept: Vec<&Fact> = Vec::new();
    for fact in facts {
        let cost = tokens(&line(fact)) + 1;
        if spent + cost > budget {
            continue;
        }
        spent += cost;
        kept.push(fact);
    }
    let mut out = Vec::new();
    for category in CATEGORIES {
        let lines: Vec<String> = kept
            .iter()
            .filter(|fact| fact.category == category)
            .map(|fact| line(fact))
            .collect();
        if !lines.is_empty() {
            out.push(format!("{}:\n{}", heading(category), lines.join("\n")));
        }
    }
    out.join("\n")
}

fn heading(category: &str) -> String {
    let mut chars = category.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(chars).collect()
    })
}

/// The profile of `facts` on `day`, within [`PROFILE_TOKENS`]: the facts every room shares
/// (none of a project's unless pinned).
#[must_use]
pub fn profile(facts: &[Fact], day: &str) -> String {
    let shared: Vec<&Fact> = facts
        .iter()
        .filter(|fact| eligible(fact, day))
        .filter(|fact| fact.project.is_none() || fact.profile.as_deref() == Some("pin"))
        .collect();
    within(shared, PROFILE_TOKENS)
}

/// The slice of `facts` that belong to `project` (its name, any spelling), within
/// [`SLICE_TOKENS`].
#[must_use]
pub fn slice(facts: &[Fact], project: &str, day: &str) -> String {
    let wanted = plain(project);
    let own: Vec<&Fact> = facts
        .iter()
        .filter(|fact| eligible(fact, day))
        .filter(|fact| {
            fact.project
                .as_deref()
                .is_some_and(|name| plain(name) == wanted)
        })
        .collect();
    within(own, SLICE_TOKENS)
}

/// The profile's file: a heading, a line on what it is, then the profile.
#[must_use]
pub fn file(profile: &str, made: &str) -> String {
    format!(
        "# What the assistant always knows\n\nMade {made} from the facts, within {PROFILE_TOKENS} \
         tokens. Pin a fact to keep it here, hide it to leave it out.\n\n{profile}\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fact(id: &str, text: &str, updated: &str) -> Fact {
        let (category, _) = id.split_once('/').unwrap();
        Fact {
            id: id.to_owned(),
            category: category.to_owned(),
            title: id.rsplit('/').next().unwrap().to_owned(),
            text: text.to_owned(),
            updated: updated.to_owned(),
            ..Fact::default()
        }
    }

    #[test]
    fn the_profile_leaves_out_what_is_not_the_owners_truth_and_keeps_its_budget() {
        let mut pending = fact("preferences/invoices", "Send invoices to me", "2026-11-03");
        pending.pending = true;
        let mut hidden = fact("people/old-friend", "Met at school", "2026-11-04");
        hidden.profile = Some("hide".to_owned());
        let mut ended = fact("work/contract", "On contract until October", "2026-10-01");
        ended.valid = Some("2026-01-01..2026-10-31".to_owned());
        let health = fact("health/iron", "Takes iron tablets", "2026-11-05");
        let mut pinned_health = fact("health/allergy", "Allergic to penicillin", "2026-01-01");
        pinned_health.profile = Some("pin".to_owned());
        let mut project = fact("projects/shelf", "Oak, 120 cm", "2026-11-06");
        project.project = Some("Oak shelf".to_owned());
        let facts = vec![
            fact("people/sami", "Her brother, a dentist", "2026-11-01"),
            pending,
            hidden,
            ended,
            health,
            pinned_health,
            project,
        ];
        let shown = profile(&facts, "2026-11-28");
        assert_eq!(
            shown,
            "People:\n- sami: Her brother, a dentist [people/sami]\nHealth:\n- allergy: Allergic to penicillin [health/allergy]"
        );
        assert_eq!(
            slice(&facts, "oak SHELF", "2026-11-28"),
            "Projects:\n- shelf: Oak, 120 cm [projects/shelf]"
        );

        let many: Vec<Fact> = (0..400)
            .map(|n| {
                fact(
                    &format!("work/item-{n:03}"),
                    &"word ".repeat(20),
                    &format!("2026-11-{:02}", n % 28 + 1),
                )
            })
            .collect();
        let shown = profile(&many, "2026-11-28");
        assert!(tokens(&shown) <= PROFILE_TOKENS + 10, "{}", tokens(&shown));
        assert!(
            shown.contains("item-027"),
            "the most recently changed are chosen"
        );
        assert!(
            !shown.contains("item-028"),
            "the least recently changed are left out"
        );
    }

    #[test]
    fn a_long_fact_is_cut_in_its_line() {
        let long = fact("home/boiler", &"serviced ".repeat(100), "2026-11-01");
        let shown = line(&long);
        assert!(shown.ends_with("… [home/boiler]"));
        assert!(shown.chars().count() < 340);
    }
}
