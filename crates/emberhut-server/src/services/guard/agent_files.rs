//! Which parts of a file tool's call reach files, for an agent's worker: every field of the
//! CLI's file tools that names a path or a glob is checked against the agent's roots, and a
//! call the gate can't be sure of is refused.
//!
//! - A path (`file_path`, `notebook_path`, a search's `path`) must resolve under the roots
//!   ([`scope::within`]); a relative one is read from the call's working folder.
//! - A glob (Glob's `pattern`, Grep's `glob`) is searched from its own fixed start when it is
//!   absolute, and from the call's `path` (or the working folder) when it is relative, so the
//!   folder its fixed part names must be under the roots ([`scope::glob_within`]).
//! - Refused when in doubt: a path or glob field that isn't text, a tool's file field that is
//!   missing, a field the tool isn't known to have whose name says it names a file, and any
//!   other unknown field whose text is an absolute path, starts at a home folder (`~`) or steps
//!   up (`..`).

use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::services::agents::scope;

/// The file tools an agent's worker may be given, each of whose calls the gate checks here.
pub(super) const FILE_TOOLS: [&str; 6] = ["Read", "Glob", "Grep", "Write", "Edit", "NotebookEdit"];

/// How a field of a file tool's input reaches files.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Reach {
    /// It names a file or a folder.
    File,
    /// It is a glob, searched from the call's folder.
    Glob,
    /// It names no file (words, a regular expression, a count, a mode).
    Nothing,
}

/// The fields `tool`'s input is known to have and how each reaches files, and the field it
/// can't do without (none: it searches its working folder).
fn fields(tool: &str) -> (&'static [(&'static str, Reach)], Option<&'static str>) {
    use Reach::{File, Glob, Nothing};
    match tool {
        "Read" => (
            &[
                ("file_path", File),
                ("offset", Nothing),
                ("limit", Nothing),
                ("pages", Nothing),
            ],
            Some("file_path"),
        ),
        "Write" => (
            &[("file_path", File), ("content", Nothing)],
            Some("file_path"),
        ),
        "Edit" => (
            &[
                ("file_path", File),
                ("old_string", Nothing),
                ("new_string", Nothing),
                ("replace_all", Nothing),
            ],
            Some("file_path"),
        ),
        "NotebookEdit" => (
            &[
                ("notebook_path", File),
                ("cell_id", Nothing),
                ("new_source", Nothing),
                ("cell_type", Nothing),
                ("edit_mode", Nothing),
            ],
            Some("notebook_path"),
        ),
        "Glob" => (&[("path", File), ("pattern", Glob)], Some("pattern")),
        "Grep" => (
            &[
                ("path", File),
                ("glob", Glob),
                ("pattern", Nothing),
                ("output_mode", Nothing),
                ("type", Nothing),
                ("-A", Nothing),
                ("-B", Nothing),
                ("-C", Nothing),
                ("-n", Nothing),
                ("-i", Nothing),
                ("context", Nothing),
                ("head_limit", Nothing),
                ("offset", Nothing),
                ("multiline", Nothing),
            ],
            Some("pattern"),
        ),
        _ => (&[], None),
    }
}

/// How an input field `tool` isn't known to have reaches files, by its name: a name that says
/// it names a file or a glob is checked as one; any other is checked only by its text.
fn unknown_reach(name: &str) -> Option<Reach> {
    let name = name.to_ascii_lowercase();
    if name.contains("glob") || name.contains("pattern") {
        Some(Reach::Glob)
    } else if ["path", "file", "dir", "folder", "cwd", "root"]
        .iter()
        .any(|word| name.contains(word))
    {
        Some(Reach::File)
    } else {
        None
    }
}

/// Whether text in a field that names no file still reads as a path elsewhere: absolute, at a
/// home folder, or stepping up.
fn looks_elsewhere(text: &str) -> bool {
    text.starts_with('/')
        || text.starts_with('~')
        || text.split(['/', '\\']).any(|part| part == "..")
}

/// Whether any text in `value` (itself, or in a list or an object it holds) is `found`.
fn any_text(value: &Value, found: fn(&str) -> bool) -> bool {
    match value {
        Value::String(text) => found(text),
        Value::Array(items) => items.iter().any(|item| any_text(item, found)),
        Value::Object(fields) => fields.values().any(|item| any_text(item, found)),
        Value::Null | Value::Bool(_) | Value::Number(_) => false,
    }
}

/// The first part of `tool`'s call with `input` (run in `cwd`) that may reach a file outside
/// `roots`, as the worker is told it; none when every part stays in them.
pub(super) fn first_outside(
    tool: &str,
    input: &Value,
    cwd: &Path,
    roots: &[PathBuf],
) -> Option<String> {
    let (known, needed) = fields(tool);
    let Some(input) = input.as_object() else {
        return Some(format!("{tool}'s input"));
    };
    if let Some(needed) = needed
        && !input.contains_key(needed)
    {
        return Some(format!("{tool} with no {needed}"));
    }
    let base = match input.get("path") {
        None | Some(Value::Null) => Path::new("."),
        Some(Value::String(path)) => Path::new(path.as_str()),
        Some(_) => return Some(format!("{tool}'s path")),
    };
    for (name, value) in input {
        let reach = known
            .iter()
            .find(|(field, _)| field == name)
            .map(|(_, reach)| *reach)
            .or_else(|| unknown_reach(name));
        let outside = match (reach, value) {
            (Some(Reach::Nothing), _) | (None, Value::Null) => false,
            (None, other) => any_text(other, looks_elsewhere),
            (Some(_), Value::Null) if Some(name.as_str()) != needed => false,
            (Some(Reach::File), Value::String(path)) => !scope::within(Path::new(path), cwd, roots),
            (Some(Reach::Glob), Value::String(glob)) => !scope::glob_within(glob, base, cwd, roots),
            (Some(_), _) => true,
        };
        if outside {
            return Some(match value {
                Value::String(text) => text.clone(),
                _ => format!("{tool}'s {name}"),
            });
        }
    }
    let searched = known.iter().any(|(field, _)| *field == "path");
    (searched && !scope::within(base, cwd, roots)).then(|| base.display().to_string())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    /// A root with a file in it, the folder above it, and the roots.
    fn setup() -> (tempfile::TempDir, PathBuf, Vec<PathBuf>) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("agents").join("client-a");
        std::fs::create_dir_all(root.join("notes")).unwrap();
        std::fs::write(root.join("brief.md"), b"x").unwrap();
        let roots = vec![root.clone()];
        (dir, root, roots)
    }

    #[test]
    fn every_file_field_of_every_file_tool_is_checked() {
        let (dir, root, roots) = setup();
        let mine = root.join("brief.md").display().to_string();
        let theirs = dir.path().display().to_string();
        let through = [
            ("Read", json!({ "file_path": mine, "offset": 2 })),
            (
                "Write",
                json!({ "file_path": "draft.md", "content": "/etc/passwd" }),
            ),
            (
                "Edit",
                json!({ "file_path": "brief.md", "old_string": "..", "new_string": "/x" }),
            ),
            ("Glob", json!({ "pattern": "**/*.md" })),
            ("Glob", json!({ "pattern": "*.md", "path": "notes" })),
            (
                "Grep",
                json!({ "pattern": "fee", "glob": "*.md", "path": "notes" }),
            ),
            (
                "Grep",
                json!({ "pattern": "/etc/passwd|\\.\\./", "-i": true }),
            ),
        ];
        for (tool, input) in through {
            assert_eq!(
                first_outside(tool, &input, &root, &roots),
                None,
                "{tool} {input}"
            );
        }
        // Its own folder named absolutely: through on Unix; held on Windows, whose paths hold
        // backslashes, which a glob never may.
        let absolute = json!({ "pattern": format!("{}/**", root.display()) });
        assert_eq!(
            first_outside("Glob", &absolute, &root, &roots).is_none(),
            cfg!(unix),
            "{absolute}"
        );
        let held = [
            (
                "Read",
                json!({ "file_path": format!("{theirs}/owner.txt") }),
            ),
            ("Read", json!({ "offset": 1 })),
            ("Read", json!({ "file_path": 7 })),
            (
                "Read",
                json!({ "file_path": mine, "other_file": "/etc/passwd" }),
            ),
            (
                "Read",
                json!({ "file_path": mine, "extra": "../../owner.txt" }),
            ),
            (
                "Read",
                json!({ "file_path": mine, "extra": { "at": ["/etc/passwd"] } }),
            ),
            ("Edit", json!({ "file_path": "../../owner.txt" })),
            ("NotebookEdit", json!({ "notebook_path": "/etc/n.ipynb" })),
            ("Glob", json!({ "pattern": "/etc/*" })),
            ("Glob", json!({ "pattern": "../../**" })),
            ("Glob", json!({ "pattern": format!("{theirs}/**/*.pdf") })),
            ("Glob", json!({ "pattern": "*.md", "path": theirs })),
            ("Glob", json!({ "pattern": "*.md", "path": ["notes"] })),
            ("Glob", json!({})),
            ("Grep", json!({ "pattern": "fee", "path": "/" })),
            ("Grep", json!({ "pattern": "fee", "glob": "/etc/*" })),
            ("Grep", json!({ "pattern": "fee", "glob": "../*" })),
            ("Grep", json!({ "pattern": "fee", "glob": 3 })),
            (
                "Grep",
                json!({ "pattern": "fee", "search_root": "x/../.." }),
            ),
        ];
        for (tool, input) in held {
            assert!(
                first_outside(tool, &input, &root, &roots).is_some(),
                "{tool} {input}"
            );
        }
        assert!(
            first_outside("Grep", &json!({ "pattern": "fee" }), Path::new("/"), &roots).is_some(),
            "searching a working folder outside the roots"
        );
        assert!(first_outside("Read", &json!("brief.md"), &root, &roots).is_some());
    }
}
