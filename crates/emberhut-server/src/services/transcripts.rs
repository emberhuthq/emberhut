//! What an agent session's transcript says, for recall: the JSON lines a coding agent writes
//! (one object per line: `{ type, message: { content }, timestamp, ... }`), read for what the
//! user typed, the agent's answers and the compaction summaries. Prompts a program wrote
//! (server events, briefs, notifications), tool output and the server's own reminders are left
//! out: the chat and the tasks are indexed from the server's own records, and the rest is long,
//! repetitive and crowds out what was said.

use std::sync::LazyLock;

use regex::Regex;
use serde_json::Value;

/// Who said a transcript line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// The user typed it.
    User,
    /// The agent answered it.
    Assistant,
    /// A compaction summary.
    Summary,
}

impl Role {
    /// The word for it in a hit's title.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Assistant => "assistant",
            Self::Summary => "summary",
        }
    }
}

/// Whether a raw line may say something, looked for in its bytes so that a line of tool output
/// (most of a transcript) is never parsed.
#[must_use]
pub fn may_say(line: &[u8]) -> bool {
    let has = |needle: &[u8]| line.windows(needle.len()).any(|window| window == needle);
    !has(br#""tool_use_id""#) && (has(br#""type":"user""#) || has(br#""type":"assistant""#))
}

/// Patterns cut out of what was said: the server's reminders, the memory it puts back, the
/// instructions after it, the new notes riding along, and command echoes.
static CUT: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    [
        r"<system-reminder>[\s\S]*?</system-reminder>",
        r"\A\(Your server puts your memory back[\s\S]*?\n---\n\n",
        r"\nHow you work:\n[\s\S]*\z",
        r"\n\n\(New quick notes since you last looked[\s\S]*\z",
        r"<local-command-stdout>[\s\S]*?</local-command-stdout>",
        r"<command-name>[\s\S]*?</command-name>",
        r"<command-message>[\s\S]*?</command-message>",
        r"<command-args>[\s\S]*?</command-args>",
    ]
    .iter()
    .filter_map(|pattern| Regex::new(pattern).ok())
    .collect()
});

/// A prompt that starts like a program's (`<event ...>`, `[from ...]`).
static PROGRAMS: LazyLock<Option<Regex>> = LazyLock::new(|| Regex::new(r"^\s*[<\[]").ok());

/// JavaScript's truthiness, as the agent's fields are read: missing, `null`, `false`, `0` and
/// `""` are false.
fn truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(flag)) => *flag,
        Some(Value::Number(number)) => number.as_f64().is_some_and(|n| n != 0.0),
        Some(Value::String(text)) => !text.is_empty(),
        Some(_) => true,
    }
}

/// What the transcript line `line` says, if anything it says is worth indexing.
#[must_use]
pub fn said(line: &Value) -> Option<(Role, String)> {
    let kind = line.get("type").and_then(Value::as_str)?;
    if kind != "user" && kind != "assistant" {
        return None;
    }
    let content = line
        .get("message")
        .and_then(|message| message.get("content"));
    let text = match content {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(blocks)) => blocks
            .iter()
            .filter(|block| block.get("type").and_then(Value::as_str) == Some("text"))
            .map(|block| match block.get("text") {
                Some(Value::String(text)) => text.clone(),
                Some(other) => other.to_string(),
                None => String::new(),
            })
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    };
    if truthy(line.get("isCompactSummary")) {
        return Some((Role::Summary, text));
    }
    if truthy(line.get("isMeta")) {
        return None;
    }
    let is = |key: &str, value: &str| line.get(key).and_then(Value::as_str) == Some(value);
    let typed = is("turnOrigin", "human")
        || is("promptSource", "typed")
        || (!truthy(line.get("turnOrigin"))
            && !truthy(line.get("promptSource"))
            && !truthy(line.get("isSidechain"))
            && !PROGRAMS
                .as_ref()
                .is_some_and(|programs| programs.is_match(&text)));
    if kind == "user" && !typed {
        return None;
    }
    let mut clean = text;
    for cut in CUT.iter() {
        clean = cut.replace_all(&clean, "").into_owned();
    }
    let clean = clean.trim();
    let role = if kind == "user" {
        Role::User
    } else {
        Role::Assistant
    };
    (clean.chars().count() > 2).then(|| (role, clean.to_owned()))
}

/// A transcript folder's name (`-srv-projects-garden`) as a place to tell sessions apart
/// (`/srv/projects/garden`); a home folder's prefix (`-home-ada-`) reads as `~/`.
#[must_use]
pub fn place(folder: &str) -> String {
    let rest = folder
        .strip_prefix("-home-")
        .and_then(|rest| rest.find('-').map(|at| format!("~{}", &rest[at..])))
        .unwrap_or_else(|| folder.to_owned());
    rest.replace('-', "/")
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn typed_words_answers_and_summaries_are_read() {
        let typed = json!({ "type": "user", "message": { "content": "Paint the door blue" } });
        assert_eq!(
            said(&typed),
            Some((Role::User, "Paint the door blue".to_owned()))
        );
        let answer = json!({ "type": "assistant", "message": { "content": [
            { "type": "text", "text": "Done, it's blue." },
            { "type": "tool_use", "name": "x" },
        ] } });
        assert_eq!(
            said(&answer),
            Some((Role::Assistant, "Done, it's blue.".to_owned()))
        );
        let summary = json!({ "type": "user", "isCompactSummary": true, "message": { "content": "We painted." } });
        assert_eq!(
            said(&summary),
            Some((Role::Summary, "We painted.".to_owned()))
        );
    }

    #[test]
    fn programs_meta_and_reminders_are_left_out() {
        let event = json!({ "type": "user", "message": { "content": "<event>tick</event>" } });
        assert_eq!(said(&event), None);
        let relayed = json!({ "type": "user", "turnOrigin": "human", "message": { "content": "[from the app] hi there" } });
        assert_eq!(
            said(&relayed),
            Some((Role::User, "[from the app] hi there".to_owned()))
        );
        let meta = json!({ "type": "user", "isMeta": true, "message": { "content": "Caveat" } });
        assert_eq!(said(&meta), None);
        let side =
            json!({ "type": "user", "isSidechain": true, "message": { "content": "Brief" } });
        assert_eq!(said(&side), None);
        let reminded = json!({ "type": "assistant", "message": { "content":
            "Yes<system-reminder>secret</system-reminder> indeed\n\n(New quick notes since you last looked, x)" } });
        assert_eq!(
            said(&reminded),
            Some((Role::Assistant, "Yes indeed".to_owned()))
        );
        let short = json!({ "type": "assistant", "message": { "content": "ok" } });
        assert_eq!(said(&short), None);
        assert_eq!(said(&json!({ "type": "system" })), None);
    }

    #[test]
    fn only_lines_that_may_say_something_are_parsed() {
        assert!(may_say(br#"{"type":"user","message":{}}"#));
        assert!(!may_say(br#"{"type":"user","tool_use_id":"x"}"#));
        assert!(!may_say(br#"{"type":"progress"}"#));
    }

    #[test]
    fn folders_read_as_places() {
        assert_eq!(place("-home-ada-garden"), "~/garden");
        assert_eq!(place("-srv-projects"), "/srv/projects");
    }
}
