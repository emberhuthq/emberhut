//! Tools and their input in words, for transcripts and the live head (`clip`, `doing`,
//! `tool_summary`, `diff_of`), and sizes in words (`mb`).

use serde_json::Value;

/// `s` cut to `n` characters, with an ellipsis when it was longer.
#[must_use]
pub fn clip(s: &str, n: usize) -> String {
    match s.char_indices().nth(n) {
        Some((at, _)) => format!("{}…", &s[..at]),
        None => s.to_owned(),
    }
}

/// A string field of a tool's input, when it is there and not empty.
fn field<'a>(input: &'a Value, key: &str) -> Option<&'a str> {
    input
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
}

/// The last part of a path, whichever separator it uses.
fn base(path: Option<&str>) -> &str {
    path.unwrap_or_default()
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or_default()
}

/// What a tool call is doing, in a few words ("Running: cargo test").
#[must_use]
pub fn doing(name: &str, input: &Value) -> String {
    let f = |k: &str| field(input, k);
    match name {
        "Bash" => format!(
            "Running: {}",
            clip(f("description").or_else(|| f("command")).unwrap_or(""), 70)
        ),
        "Read" => format!("Reading {}", base(f("file_path"))),
        "Edit" | "MultiEdit" => format!("Editing {}", base(f("file_path"))),
        "Write" => format!("Writing {}", base(f("file_path"))),
        "Grep" | "Glob" => format!("Searching {}", clip(f("pattern").unwrap_or(""), 50)),
        "WebFetch" => format!("Reading {}", clip(f("url").unwrap_or(""), 60)),
        "WebSearch" => format!("Searching the web: {}", clip(f("query").unwrap_or(""), 50)),
        "Agent" | "Task" => format!("Helper agent: {}", clip(f("description").unwrap_or(""), 60)),
        "TodoWrite" | "TaskCreate" | "TaskUpdate" => "Updating its to-do list".to_owned(),
        _ => format!("Using {name}"),
    }
}

/// The one line that says what a tool call acts on: its command, file, pattern, address or query.
#[must_use]
pub fn tool_summary(name: &str, input: &Value) -> String {
    let f = |k: &str| field(input, k);
    let what = if name == "Bash" {
        f("command")
    } else {
        f("file_path")
            .or_else(|| f("pattern"))
            .or_else(|| f("url"))
            .or_else(|| f("query"))
            .or_else(|| f("description"))
    };
    clip(what.unwrap_or(""), 300)
}

/// The lines of a text field (none: one empty line).
fn lines(v: Option<&Value>) -> Vec<&str> {
    v.and_then(Value::as_str)
        .unwrap_or("")
        .split('\n')
        .collect()
}

/// One edit as the lines that changed: the common head and tail dropped, `- ` before, `+ ` after.
fn one_edit(old: Option<&Value>, new: Option<&Value>) -> Vec<String> {
    let (x, y) = (lines(old), lines(new));
    let head = x.iter().zip(&y).take_while(|(a, b)| a == b).count();
    let tail = x[head..]
        .iter()
        .rev()
        .zip(y[head..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    let gone = x[head..x.len() - tail].iter().map(|t| format!("- {t}"));
    let came = y[head..y.len() - tail].iter().map(|t| format!("+ {t}"));
    gone.chain(came).collect()
}

/// An edit as the lines that changed, `- ` before and `+ ` after (a new file: its first 40 lines);
/// `None` for other tools.
#[must_use]
pub fn diff_of(name: &str, input: &Value) -> Option<String> {
    if input.is_null() {
        return None;
    }
    let out: Vec<String> = match name {
        "Edit" => one_edit(input.get("old_string"), input.get("new_string")),
        "MultiEdit" => {
            let edits = input.get("edits")?.as_array()?;
            let mut out = Vec::new();
            for (k, e) in edits.iter().enumerate() {
                if k > 0 {
                    out.push("…".to_owned());
                }
                out.extend(one_edit(e.get("old_string"), e.get("new_string")));
            }
            out
        }
        "Write" => lines(input.get("content"))
            .into_iter()
            .take(40)
            .map(|t| format!("+ {t}"))
            .collect(),
        _ => return None,
    };
    (!out.is_empty()).then(|| clip(&out.join("\n"), 4000))
}

/// A size in words: tenths of a MB from 1 MiB up, whole KB (1 at least) below.
#[must_use]
pub fn mb(bytes: u64) -> String {
    const MIB: u128 = 1024 * 1024;
    let n = u128::from(bytes);
    if n >= MIB {
        let tenths = (n * 10 + MIB / 2) / MIB;
        format!("{}.{} MB", tenths / 10, tenths % 10)
    } else {
        format!("{} KB", ((n + 512) / 1024).max(1))
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn clip_cuts_by_characters_with_an_ellipsis() {
        assert_eq!(clip("abcdef", 3), "abc…");
        assert_eq!(clip("abc", 3), "abc");
        assert_eq!(clip("", 0), "");
        assert_eq!(clip("ééééé", 2), "éé…");
    }

    #[test]
    fn doing_says_what_a_tool_does_in_a_few_words() {
        assert_eq!(
            doing(
                "Bash",
                &json!({"command": "cargo test", "description": "Run the tests"})
            ),
            "Running: Run the tests"
        );
        assert_eq!(
            doing("Bash", &json!({"command": "cargo test"})),
            "Running: cargo test"
        );
        assert_eq!(
            doing("Read", &json!({"file_path": "/w/src/main.rs"})),
            "Reading main.rs"
        );
        assert_eq!(
            doing("MultiEdit", &json!({"file_path": "C:\\w\\a.rs"})),
            "Editing a.rs"
        );
        assert_eq!(doing("Write", &json!({})), "Writing ");
        assert_eq!(
            doing("Grep", &json!({"pattern": "fn main"})),
            "Searching fn main"
        );
        assert_eq!(
            doing("WebSearch", &json!({"query": "tokio"})),
            "Searching the web: tokio"
        );
        assert_eq!(
            doing("Task", &json!({"description": "Look around"})),
            "Helper agent: Look around"
        );
        assert_eq!(doing("TodoWrite", &Value::Null), "Updating its to-do list");
        assert_eq!(doing("Mystery", &Value::Null), "Using Mystery");
        assert_eq!(
            doing("Bash", &json!({"command": "x".repeat(80)})),
            format!("Running: {}…", "x".repeat(70))
        );
    }

    #[test]
    fn the_summary_is_the_command_or_what_the_tool_acts_on() {
        assert_eq!(
            tool_summary("Bash", &json!({"command": "ls", "description": "List"})),
            "ls"
        );
        assert_eq!(
            tool_summary("Read", &json!({"file_path": "/w/a.rs", "pattern": "x"})),
            "/w/a.rs"
        );
        assert_eq!(
            tool_summary("WebFetch", &json!({"url": "https://example.com"})),
            "https://example.com"
        );
        assert_eq!(tool_summary("Other", &json!({"file_path": ""})), "");
    }

    #[test]
    fn an_edit_shows_only_the_lines_that_changed() {
        assert_eq!(
            diff_of(
                "Edit",
                &json!({"old_string": "a\nold\nz", "new_string": "a\nnew\nz"})
            )
            .as_deref(),
            Some("- old\n+ new")
        );
        assert_eq!(
            diff_of("Edit", &json!({"old_string": "old", "new_string": "new"})).as_deref(),
            Some("- old\n+ new")
        );
        assert_eq!(
            diff_of("Edit", &json!({"old_string": "same", "new_string": "same"})),
            None
        );
        assert_eq!(
            diff_of(
                "MultiEdit",
                &json!({"edits": [{"old_string": "a", "new_string": "b"}, {"old_string": "c", "new_string": "d"}]})
            )
            .as_deref(),
            Some("- a\n+ b\n…\n- c\n+ d")
        );
        let long = (1..=50).fold(String::new(), |s, n| s + &n.to_string() + "\n");
        let w = diff_of("Write", &json!({"content": long})).unwrap();
        assert_eq!(w.lines().count(), 40);
        assert!(w.starts_with("+ 1\n+ 2"));
        assert_eq!(diff_of("Read", &json!({"file_path": "/w/a"})), None);
        assert_eq!(diff_of("Edit", &Value::Null), None);
        assert_eq!(diff_of("MultiEdit", &json!({})), None);
    }

    #[test]
    fn sizes_read_in_kb_or_tenths_of_a_mb() {
        assert_eq!(mb(0), "1 KB");
        assert_eq!(mb(1536), "2 KB");
        assert_eq!(mb(1024 * 1024), "1.0 MB");
        assert_eq!(mb(200 * 1024 * 1024), "200.0 MB");
        assert_eq!(mb(1024 * 1024 + 300 * 1024), "1.3 MB");
        assert!(mb(u64::MAX).ends_with(" MB"));
    }
}
