//! How Party words its numbers and states: tokens (`23.4k`), dollars (`$0.12`), a party's
//! status and its meter (`Turn 5 of 12 · 23.4k of 150k tokens · $0.12`).

use emberhut_api::party::{Party, PartyStatus};

/// How a status is coloured.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Tone {
    /// Talking: the success colour.
    Live,
    /// Paused: the warning colour.
    Waiting,
    /// Done or stopped: secondary.
    Still,
}

/// A count of tokens: `950`, `23.4k`, `1.5M`.
#[must_use]
pub(crate) fn tokens(count: u64) -> String {
    // Display only: a count past 2^52 tokens is not a party's.
    #[expect(
        clippy::cast_precision_loss,
        reason = "a token count shown to one decimal"
    )]
    let value = count as f64;
    if count >= 1_000_000 {
        format!("{:.1}M", value / 1_000_000.0)
    } else if count >= 1000 {
        format!("{:.1}k", value / 1000.0)
    } else {
        count.to_string()
    }
}

/// US dollars: `$0.12`, and `<$0.01` for a cost above nothing that rounds to it.
#[must_use]
pub(crate) fn cost(usd: f64) -> String {
    if usd > 0.0 && usd < 0.01 {
        "<$0.01".to_owned()
    } else {
        format!("${usd:.2}")
    }
}

/// A status in a word: Talking, Paused, Done, Stopped (a newer one capitalised).
#[must_use]
pub(crate) fn status_word(status: &PartyStatus) -> String {
    match status {
        PartyStatus::Running => "Talking".to_owned(),
        PartyStatus::Paused => "Paused".to_owned(),
        PartyStatus::Done => "Done".to_owned(),
        PartyStatus::Stopped => "Stopped".to_owned(),
        other => capitalised(other.as_str()),
    }
}

/// A status's colour.
#[must_use]
pub(crate) fn tone(status: &PartyStatus) -> Tone {
    match status {
        PartyStatus::Running => Tone::Live,
        PartyStatus::Paused => Tone::Waiting,
        _ => Tone::Still,
    }
}

/// The cast's names: `Nora, Victor, Sam`.
#[must_use]
pub(crate) fn cast(party: &Party) -> String {
    party
        .personas
        .iter()
        .map(|persona| persona.name.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

/// Where a party is against its caps: `Turn 5 of 12 · 23.4k of 150k tokens · $0.12`.
#[must_use]
pub(crate) fn meter(party: &Party) -> String {
    format!(
        "Turn {} of {} · {} of {} tokens · {}",
        party.turns,
        party.max_turns,
        tokens(party.tokens),
        tokens(party.max_tokens),
        cost(party.cost)
    )
}

/// A role's first sentence, 40 characters at most (`…` after a cut), for a menu.
#[must_use]
pub(crate) fn short_role(role: &str) -> String {
    let first = role.split('.').next().unwrap_or(role);
    if first.chars().count() > 40 {
        format!("{}…", first.chars().take(40).collect::<String>())
    } else {
        first.to_owned()
    }
}

/// A name's first letter, for its round badge.
#[must_use]
pub(crate) fn initial(name: &str) -> String {
    name.chars().next().map_or_else(String::new, String::from)
}

/// `word` with its first letter in capitals.
fn capitalised(word: &str) -> String {
    let mut letters = word.chars();
    letters.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(letters).collect()
    })
}

/// A model's name when the server's list lacks it: `claude-opus-5-5` reads `Opus-5-5`.
#[must_use]
pub(crate) fn model_fallback(id: &str) -> String {
    let bare = id.strip_prefix("claude-").unwrap_or(id);
    bare.split('-')
        .map(capitalised)
        .collect::<Vec<_>>()
        .join("-")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_read_short() {
        assert_eq!(tokens(950), "950");
        assert_eq!(tokens(23_400), "23.4k");
        assert_eq!(tokens(150_000), "150.0k");
        assert_eq!(tokens(1_500_000), "1.5M");
    }

    #[test]
    fn a_tiny_cost_isnt_nothing() {
        assert_eq!(cost(0.0), "$0.00");
        assert_eq!(cost(0.004), "<$0.01");
        assert_eq!(cost(0.1234), "$0.12");
    }

    #[test]
    fn each_status_has_its_word_and_tone() {
        assert_eq!(status_word(&PartyStatus::Running), "Talking");
        assert_eq!(status_word(&PartyStatus::from("resting")), "Resting");
        assert_eq!(tone(&PartyStatus::Running), Tone::Live);
        assert_eq!(tone(&PartyStatus::Paused), Tone::Waiting);
        assert_eq!(tone(&PartyStatus::Stopped), Tone::Still);
    }

    #[test]
    fn a_role_is_cut_to_its_first_sentence() {
        assert_eq!(short_role("A skeptic. You poke holes."), "A skeptic");
        let long = "A".repeat(50);
        assert_eq!(short_role(&long), format!("{}…", "A".repeat(40)));
        assert_eq!(initial("Émile"), "É");
        assert_eq!(initial(""), "");
    }

    #[test]
    fn a_model_without_a_name_reads_from_its_id() {
        assert_eq!(model_fallback("claude-opus-5-5"), "Opus-5-5");
        assert_eq!(model_fallback("local"), "Local");
    }
}
