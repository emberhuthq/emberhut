//! Python's inline code (`python3 -c '…'`, `python3 - <<'EOF'`) that only reads and prints.
//!
//! The code is read as Python tokens: its strings (f-strings' fields read as code), comments
//! and the rest. It reads only when every module it imports is one of [`MODULES`] (`os` and
//! `pathlib` only for what reads), every `open` it calls opens to read, and none of its names
//! runs, writes, deletes, reaches the network or reaches a name it can't see
//! ([`ANYWHERE`], [`IN_CODE`], a dunder). What it can't read that way it doesn't pass: the
//! outward rule then holds it like any inline code.
//!
//! It reads the code, not what the code imports: a module the folder's files shadow is code it
//! never sees, like a build script.

/// The modules whose import keeps the code to reading and printing.
const MODULES: &[&str] = &[
    "argparse",
    "base64",
    "binascii",
    "bisect",
    "calendar",
    "collections",
    "copy",
    "csv",
    "dataclasses",
    "datetime",
    "decimal",
    "difflib",
    "enum",
    "fnmatch",
    "fractions",
    "functools",
    "glob",
    "hashlib",
    "heapq",
    "html",
    "ipaddress",
    "itertools",
    "json",
    "locale",
    "math",
    "os",
    "pathlib",
    "pprint",
    "random",
    "re",
    "secrets",
    "shlex",
    "statistics",
    "string",
    "struct",
    "sys",
    "textwrap",
    "time",
    "typing",
    "unicodedata",
    "uuid",
    "zoneinfo",
];

/// What of `os` reads (`os.path` and the rest).
const OS_READS: &[&str] = &[
    "path",
    "environ",
    "getenv",
    "getcwd",
    "listdir",
    "scandir",
    "walk",
    "stat",
    "lstat",
    "sep",
    "linesep",
    "pathsep",
    "curdir",
    "pardir",
    "devnull",
    "name",
    "cpu_count",
    "getpid",
    "fspath",
    "fsdecode",
    "fsencode",
    "get_terminal_size",
    "access",
    "readlink",
    "R_OK",
    "F_OK",
    "X_OK",
    "W_OK",
    "SEEK_SET",
    "SEEK_END",
];

/// The names that run code from a string or reach what can't be seen: refused anywhere in the
/// code, its strings too (where a reading gone wrong would hide them).
const ANYWHERE: &[&str] = &[
    "exec",
    "execfile",
    "eval",
    "compile",
    "getattr",
    "setattr",
    "delattr",
    "breakpoint",
    "builtins",
    "importlib",
    "subprocess",
    "system",
    "popen",
    "modules",
];

/// The names that write, move or delete files, or reach what can't be seen: refused in the
/// code (outside its strings). `os`'s are read by [`OS_READS`] and the modules by
/// [`MODULES`].
const IN_CODE: &[&str] = &[
    "globals",
    "locals",
    "vars",
    "unlink",
    "rmdir",
    "mkdir",
    "rename",
    "chmod",
    "chown",
    "touch",
    "symlink_to",
    "hardlink_to",
    "write_text",
    "write_bytes",
];

/// One piece of the code: code text, or a string's content.
#[derive(Debug)]
enum Piece {
    Code(String),
    Str { prefix: String, text: String },
}

/// The code cut into code and strings; none when it can't be (a string not closed, an
/// f-string field it can't follow).
fn pieces(code: &str) -> Option<Vec<Piece>> {
    let s: Vec<char> = code.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    code_until(&s, &mut i, false, &mut out)?;
    Some(out)
}

/// Whether a string starts at `i`: its prefix's length and its quote (`'`, `"`, `'''`, `"""`).
fn string_at(s: &[char], i: usize) -> Option<(usize, String)> {
    let mut j = i;
    while j < s.len() && j - i < 2 && matches!(s[j], 'r' | 'R' | 'b' | 'B' | 'u' | 'U' | 'f' | 'F')
    {
        j += 1;
    }
    let quote = *s.get(j)?;
    if quote != '\'' && quote != '"' {
        return None;
    }
    // A prefix only counts when nothing of a name comes before it.
    if j > i && i > 0 && (s[i - 1].is_ascii_alphanumeric() || s[i - 1] == '_') {
        return None;
    }
    let triple = s.get(j + 1) == Some(&quote) && s.get(j + 2) == Some(&quote);
    let q: String = if triple {
        [quote; 3].iter().collect()
    } else {
        quote.to_string()
    };
    Some((j - i, q))
}

/// Code from `i`: up to the end, or (`field`) to the `}` closing an f-string's field.
fn code_until(s: &[char], i: &mut usize, field: bool, out: &mut Vec<Piece>) -> Option<()> {
    let mut text = String::new();
    let mut depth = 0usize;
    while *i < s.len() {
        let c = s[*i];
        if c == '#' {
            if field {
                return None;
            }
            while *i < s.len() && s[*i] != '\n' {
                *i += 1;
            }
            continue;
        }
        if let Some((plen, quote)) = string_at(s, *i) {
            out.push(Piece::Code(std::mem::take(&mut text)));
            let prefix: String = s[*i..*i + plen].iter().collect();
            *i += plen + quote.chars().count();
            string_until(s, i, &prefix, &quote, out)?;
            continue;
        }
        if field {
            match c {
                '(' | '[' | '{' => depth += 1,
                ')' | ']' => depth = depth.checked_sub(1)?,
                '}' if depth > 0 => depth -= 1,
                '}' => {
                    *i += 1;
                    out.push(Piece::Code(text));
                    return Some(());
                }
                // A format spec (`:>10`, `:{w}`): read as a string's text, its fields as code.
                ':' if depth == 0 => {
                    *i += 1;
                    out.push(Piece::Code(std::mem::take(&mut text)));
                    return spec_until(s, i, out);
                }
                _ => {}
            }
        }
        text.push(c);
        *i += 1;
    }
    if field {
        return None;
    }
    out.push(Piece::Code(text));
    Some(())
}

/// An f-string field's format spec from `i`, to the `}` that closes the field.
fn spec_until(s: &[char], i: &mut usize, out: &mut Vec<Piece>) -> Option<()> {
    while *i < s.len() {
        match s[*i] {
            '}' => {
                *i += 1;
                return Some(());
            }
            '{' => {
                *i += 1;
                code_until(s, i, true, out)?;
            }
            '\'' | '"' | '\\' | '\n' => return None,
            _ => *i += 1,
        }
    }
    None
}

/// A string's content from `i` (just past its opening quote) to its closing `quote`.
fn string_until(
    s: &[char],
    i: &mut usize,
    prefix: &str,
    quote: &str,
    out: &mut Vec<Piece>,
) -> Option<()> {
    let close: Vec<char> = quote.chars().collect();
    let fields = prefix.contains(['f', 'F']);
    let mut text = String::new();
    while *i < s.len() {
        let ch = s[*i];
        if ch == '\\' {
            // An escaped character, kept; a raw string too can't end on an escaped quote.
            text.push(ch);
            if let Some(&next) = s.get(*i + 1) {
                text.push(next);
            }
            *i += 2;
            continue;
        }
        if s[*i..].starts_with(&close) {
            *i += close.len();
            out.push(Piece::Str {
                prefix: prefix.to_owned(),
                text,
            });
            return Some(());
        }
        if close.len() == 1 && ch == '\n' {
            return None;
        }
        if fields && ch == '{' {
            if s.get(*i + 1) == Some(&'{') {
                text.push_str("{{");
                *i += 2;
                continue;
            }
            *i += 1;
            out.push(Piece::Str {
                prefix: prefix.to_owned(),
                text: std::mem::take(&mut text),
            });
            code_until(s, i, true, out)?;
            continue;
        }
        text.push(ch);
        *i += 1;
    }
    None
}

/// The names (identifiers) of `text`, with where each starts.
fn names(text: &str) -> Vec<(usize, &str)> {
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if c.is_ascii_alphabetic() || c == b'_' {
            let start = i;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            out.push((start, &text[start..i]));
        } else if c.is_ascii_digit() {
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
        } else {
            i += 1;
        }
    }
    out
}

/// Whether a name is one the code may not use anywhere: [`ANYWHERE`], or a dunder (they reach
/// classes, globals and the builtins).
fn forbidden(name: &str) -> bool {
    if name.len() > 4 && name.starts_with("__") && name.ends_with("__") {
        return !matches!(name, "__name__" | "__main__" | "__file__" | "__doc__");
    }
    ANYWHERE.contains(&name)
}

/// The code text, its strings each one placeholder: what the checks of calls and imports read.
fn flat(pieces: &[Piece]) -> String {
    let mut out = String::new();
    for piece in pieces {
        match piece {
            Piece::Code(text) => out.push_str(text),
            Piece::Str { prefix, text } => {
                let mode = text.chars().all(|c| matches!(c, 'r' | 'b' | 't' | 'U'));
                // A plain string's text stays when it could be a file mode, so `open` can be
                // read; every other string is `S`.
                if mode && !text.is_empty() && !prefix.contains(['f', 'F']) {
                    out.push('\'');
                    out.push_str(text);
                    out.push('\'');
                } else {
                    out.push('S');
                }
            }
        }
    }
    out
}

/// Whether every import of the code (`flat`) is of [`MODULES`], `os` and `pathlib` with no
/// other name.
fn imports_read(flat: &str) -> bool {
    for line in flat.split(['\n', ';']) {
        let line = line.trim();
        let words: Vec<&str> = line
            .split(|c: char| c.is_whitespace() || c == ',' || c == '(' || c == ')')
            .filter(|w| !w.is_empty())
            .collect();
        let Some(at) = words.iter().position(|w| *w == "import") else {
            continue;
        };
        // An import over more than one line (`from os import (`, a backslash at its end) isn't
        // read.
        if line.contains('(') || line.ends_with('\\') {
            return false;
        }
        if let Some(from) = words.iter().position(|w| *w == "from")
            && from < at
        {
            let module = words.get(from + 1).copied().unwrap_or("");
            let root = module.split('.').next().unwrap_or("");
            if !MODULES.contains(&root) || module.starts_with('.') {
                return false;
            }
            let taken = &words[at + 1..];
            if taken.contains(&"as") || taken.contains(&"*") && (root == "os" || root == "pathlib")
            {
                return false;
            }
            if module == "os" && taken.iter().any(|n| !OS_READS.contains(n)) {
                return false;
            }
            continue;
        }
        let taken = &words[at + 1..];
        let mut j = 0;
        while j < taken.len() {
            let root = taken[j].split('.').next().unwrap_or("");
            if !MODULES.contains(&root) {
                return false;
            }
            if taken.get(j + 1) == Some(&"as") {
                if root == "os" || root == "pathlib" {
                    return false;
                }
                j += 2;
            }
            j += 1;
        }
    }
    true
}

/// The text from just past the `(` at `open` (`flat[from..]`) to its `)`, or none.
fn call_args(flat: &str, from: usize) -> Option<&str> {
    let mut depth = 0usize;
    for (n, c) in flat[from..].char_indices() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' if depth > 0 => depth -= 1,
            ')' => return Some(&flat[from..from + n]),
            _ => {}
        }
    }
    None
}

/// Whether an `open`'s arguments open to read: its mode, the `position`th argument or
/// `mode=`, left out or a plain string of `r`, `b`, `t`, `U`; no `*args`, `**kwargs` or
/// `opener`.
fn opens_to_read(args: &str, position: usize) -> bool {
    if args.contains('*') || args.contains("opener") {
        return false;
    }
    let mut parts = Vec::new();
    let mut depth = 0usize;
    let mut start = 0;
    for (n, c) in args.char_indices() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                parts.push(args[start..n].trim());
                start = n + 1;
            }
            _ => {}
        }
    }
    parts.push(args[start..].trim());
    let plain_mode = |v: &str| {
        v.len() >= 3
            && v.starts_with('\'')
            && v.ends_with('\'')
            && v[1..v.len() - 1]
                .chars()
                .all(|c| matches!(c, 'r' | 'b' | 't' | 'U'))
    };
    let mut positional = 0;
    for part in parts.iter().filter(|p| !p.is_empty()) {
        if let Some((key, value)) = part.split_once('=')
            && !key.trim().is_empty()
            && key
                .trim()
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_')
            && !value.starts_with('=')
        {
            if key.trim() == "mode" && !plain_mode(value.trim()) {
                return false;
            }
            continue;
        }
        if positional == position && !plain_mode(part) {
            return false;
        }
        positional += 1;
    }
    true
}

/// Whether Python's `code` only reads and prints.
#[must_use]
pub(super) fn reads_only(code: &str) -> bool {
    let Some(pieces) = pieces(code) else {
        return false;
    };
    // A name outside the ASCII letters can stand for another (Python reads names NFKC).
    let code_ascii = pieces.iter().all(|p| match p {
        Piece::Code(text) => text.is_ascii(),
        Piece::Str { .. } => true,
    });
    if !code_ascii {
        return false;
    }
    if names(code).iter().any(|(_, name)| forbidden(name)) {
        return false;
    }
    let flat = flat(&pieces);
    if names(&flat).iter().any(|(_, name)| IN_CODE.contains(name)) || !imports_read(&flat) {
        return false;
    }
    let uses_pathlib = names(&flat)
        .iter()
        .any(|(_, n)| *n == "pathlib" || *n == "Path");
    let found = names(&flat);
    for (at, name) in &found {
        let before = flat[..*at].trim_end();
        let after = flat[at + name.len()..].trim_start();
        let is_attr = before.ends_with('.');
        match *name {
            "open" => {
                let Some(rest) = after.strip_prefix('(') else {
                    return false;
                };
                let from = flat.len() - rest.len();
                let Some(args) = call_args(&flat, from) else {
                    return false;
                };
                if !opens_to_read(args, usize::from(!is_attr)) {
                    return false;
                }
            }
            "os" if !is_attr => {
                let line = flat[..*at]
                    .rfind(['\n', ';'])
                    .map_or(&flat[..*at], |start| &flat[start + 1..*at])
                    .trim_start();
                let in_import = line.starts_with("import ") || line.starts_with("from ");
                let attr = after.strip_prefix('.').map(|rest| {
                    rest.trim_start()
                        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                        .next()
                        .unwrap_or("")
                });
                match attr {
                    Some(attr) if OS_READS.contains(&attr) => {}
                    None if in_import => {}
                    _ => return false,
                }
            }
            // `Path.replace` moves a file.
            "replace" if uses_pathlib => return false,
            _ => {}
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::reads_only;

    #[test]
    fn code_that_reads_and_prints() {
        for code in [
            "import json,sys;print(json.load(sys.stdin)['x'])",
            "import re,html\nt=open('a.html').read()\nprint(html.unescape(re.sub(r'<[^>]+>',' ',t))[:600])",
            "import json,sys\nd=json.load(open(sys.argv[1]))\nfor c in d['c']:\n  print(f\"{c['id']:>5} {c['at'][:16]}\")",
            "from pathlib import Path\nprint(Path('x').read_text().count('\\n'))",
            "import os\nfor root, dirs, files in os.walk('.'):\n  print(root, len(files))",
            "with open('a', 'rb') as f: print(len(f.read()))",
            "print(open('a', mode='r', encoding='utf-8').read())",
            "import sys; sys.stdout.write('x'.replace('x', 'y'))",
            "print(f\"{'a'!r:>{10}}\")",
            "x = 'executive'.upper(); print(x)",
        ] {
            assert!(reads_only(code), "{code}");
        }
    }

    #[test]
    fn code_that_could_do_anything_else() {
        for code in [
            "import subprocess; subprocess.run(['git','push'])",
            "import os; os.system('git push')",
            "import os as o; o.listdir('.')",
            "o = __import__('o'+'s')",
            "open('a','w').write('x')",
            "open('a', mode='a')",
            "m = 'w'; open('a', m)",
            "open('a', *m)",
            "f = open; f('a', 'w')",
            "import urllib.request",
            "from os import system",
            "from pathlib import Path\nPath('a').write_text('x')",
            "from pathlib import Path\nPath('a').replace('b')",
            "from pathlib import Path\nPath('a').open('w')",
            "import os\nos.replace('a', 'b')",
            "import os\nx = os\nx.listdir('.')",
            "import requests; requests.post('https://example.com')",
            "getattr(__builtins__, 'ev' + 'al')('1')",
            "print(f\"{eval('1')}\")",
            "import shutil; shutil.rmtree('x')",
            "import sqlite3; sqlite3.connect('db').execute('delete from t')",
            "\u{ff45}val('1')",
            "print('unclosed",
            "x = 'exec time'",
            "().__class__.__base__.__subclasses__()",
        ] {
            assert!(!reads_only(code), "{code}");
        }
    }
}
