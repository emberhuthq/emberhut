//! The words of commands that run another command or shell code (`xargs`, `su`, `script`,
//! `tmux`, `npm exec`, `uv run`...), and the backstop over a command's own words.

use super::{api_writes, base, gh_writes, matches, positional};

/// What `xargs -I`, `-J`, `-i` or `--replace` puts its input's words in place of.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Replace {
    /// No replacement: the input's words go after the command.
    None,
    /// Each word holding this text gets the input's words.
    With(String),
    /// A replacement it can't know (a variable).
    Unknown,
}

/// Skips `xargs`'s flags from `k` (GNU's and the BSDs'), and says what it replaces.
pub(super) fn xargs(argv: &[String], fixed: &[bool], k: &mut usize) -> Replace {
    let known = |j: usize| fixed.get(j).copied().unwrap_or(false);
    let mut replace = Replace::None;
    while *k < argv.len() && argv[*k].starts_with('-') && argv[*k] != "-" {
        let flag = argv[*k].as_str();
        *k += 1;
        if flag == "--" {
            break;
        }
        if let Some(long) = flag.strip_prefix("--") {
            if long == "replace" {
                replace = Replace::With("{}".to_owned());
            } else if let Some(with) = long.strip_prefix("replace=") {
                replace = Replace::With(with.to_owned());
            } else if matches(
                "^(arg-file|delimiter|max-args|max-procs|max-chars|process-slot-var)$",
                Some(long),
            ) {
                *k += 1;
            }
            continue;
        }
        let letters: Vec<char> = flag.chars().skip(1).collect();
        for (n, letter) in letters.iter().enumerate() {
            let tail: String = letters[n + 1..].iter().collect();
            match letter {
                'I' | 'J' => {
                    replace = if tail.is_empty() {
                        let value = argv.get(*k).filter(|_| known(*k));
                        *k += 1;
                        value.map_or(Replace::Unknown, |w| Replace::With(w.clone()))
                    } else {
                        Replace::With(tail)
                    };
                    break;
                }
                'i' => {
                    replace = Replace::With(if tail.is_empty() {
                        "{}".to_owned()
                    } else {
                        tail
                    });
                    break;
                }
                'a' | 'd' | 'E' | 'L' | 'n' | 'P' | 's' | 'R' | 'S' => {
                    if tail.is_empty() {
                        *k += 1;
                    }
                    break;
                }
                // Their value, if any, is the rest of the word.
                'e' | 'l' => break,
                _ => {}
            }
        }
    }
    replace
}

/// What a command that hands over to a shell or a command runs next.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Then {
    /// Shell code: its text, and whether all of it is known.
    Code {
        /// The code.
        text: String,
        /// Whether every part of it is known here.
        known: bool,
    },
    /// A command, its words from this one.
    Command(usize),
    /// A shell reading its input.
    Shell,
    /// Words given to a shell that aren't code it reads (a script's name, its arguments).
    Unread,
}

/// Shell code given as `flag`'s value: the rest of the word (`-ccode`, `--command=code`), or
/// the next one.
fn code_after(argv: &[String], fixed: &[bool], j: usize, tail: &str) -> Then {
    let known = |j: usize| fixed.get(j).copied().unwrap_or(false);
    if tail.is_empty() {
        Then::Code {
            text: argv.get(j + 1).cloned().unwrap_or_default(),
            known: argv.get(j + 1).is_none() || known(j + 1),
        }
    } else {
        Then::Code {
            text: tail.to_owned(),
            known: known(j),
        }
    }
}

/// What `su` or `runuser` (its word at `k`) runs: `-c` code, `runuser -u`'s command, or a
/// shell reading its input. Any other words after the user go to that shell, which runs them.
pub(super) fn su(argv: &[String], fixed: &[bool], k: usize, runuser: bool) -> Then {
    let mut user_mode = false;
    let mut user = false;
    let mut j = k + 1;
    while j < argv.len() {
        let t = argv[j].as_str();
        if t == "--" {
            return if user_mode {
                Then::Command(j + 1)
            } else if j + 1 < argv.len() {
                Then::Unread
            } else {
                Then::Shell
            };
        }
        if let Some(long) = t.strip_prefix("--") {
            if let Some(code) = long
                .strip_prefix("command=")
                .or_else(|| long.strip_prefix("session-command="))
            {
                return code_after(argv, fixed, j, code);
            }
            if long == "command" || long == "session-command" {
                return code_after(argv, fixed, j, "");
            }
            if runuser && (long == "user" || long.starts_with("user=")) {
                user_mode = true;
            }
            if matches(
                "^(group|supp-group|shell|whitelist-environment|user)$",
                Some(long),
            ) {
                j += 1;
            }
            j += 1;
            continue;
        }
        if t != "-" && t.starts_with('-') {
            let letters: Vec<char> = t.chars().skip(1).collect();
            for (n, letter) in letters.iter().enumerate() {
                let tail: String = letters[n + 1..].iter().collect();
                match letter {
                    'c' => return code_after(argv, fixed, j, &tail),
                    'u' if runuser => {
                        user_mode = true;
                        if tail.is_empty() {
                            j += 1;
                        }
                        break;
                    }
                    'g' | 'G' | 's' | 'w' => {
                        if tail.is_empty() {
                            j += 1;
                        }
                        break;
                    }
                    _ => {}
                }
            }
            j += 1;
            continue;
        }
        if t == "-" {
            j += 1;
            continue;
        }
        if user_mode {
            return Then::Command(j);
        }
        if user {
            return Then::Unread;
        }
        user = true;
        j += 1;
    }
    Then::Shell
}

/// What `script` (its word at `k`) runs: `-c` code (util-linux), the command after its file
/// (the BSDs'), or a shell reading its input.
pub(super) fn script(argv: &[String], fixed: &[bool], k: usize) -> Then {
    let mut j = k + 1;
    while j < argv.len() && argv[j].starts_with('-') && argv[j] != "-" {
        let t = argv[j].as_str();
        if t == "--" {
            j += 1;
            break;
        }
        if let Some(long) = t.strip_prefix("--") {
            if let Some(code) = long.strip_prefix("command=") {
                return code_after(argv, fixed, j, code);
            }
            if long == "command" {
                return code_after(argv, fixed, j, "");
            }
            if matches(
                "^(echo|log-in|log-out|log-io|log-timing|logging-format|output-limit)$",
                Some(long),
            ) {
                j += 1;
            }
            j += 1;
            continue;
        }
        let letters: Vec<char> = t.chars().skip(1).collect();
        for (n, letter) in letters.iter().enumerate() {
            let tail: String = letters[n + 1..].iter().collect();
            match letter {
                'c' => return code_after(argv, fixed, j, &tail),
                'E' | 'I' | 'O' | 'B' | 'T' | 'm' | 'F' => {
                    if tail.is_empty() {
                        j += 1;
                    }
                    break;
                }
                _ => {}
            }
        }
        j += 1;
    }
    // The file it records to, then (on the BSDs) the command.
    if j + 1 < argv.len() {
        Then::Command(j + 1)
    } else {
        Then::Shell
    }
}

/// Whether `word`, one of `tmux`'s or `screen`'s, makes it run what isn't read here: keys typed
/// into a shell, a file of its own commands, its own commands that run others, a setting, a
/// format's `#(…)`.
pub(super) fn multiplexer_unread(program: &str, word: &str) -> bool {
    let tmux = "^(send-keys|send|send-prefix|pipe-pane|pipep|source-file|source|if-shell|if|\
        bind-key|bind|confirm-before|confirm|command-prompt|display-menu|menu|set-option|set|\
        set-window-option|setw|set-hook|set-environment|setenv|load-buffer|loadb|paste-buffer|\
        pasteb|-f)$";
    let screen = "^(-X|-c|stuff|eval|exec|source|bindkey|bind|setenv)$";
    word.contains("#(") || matches(if program == "tmux" { tmux } else { screen }, Some(word))
}

/// `env`'s split string in a spelling other than a bare `-S` or `--split-string`: `Some` with
/// the part `flag` holds (`-S'…'`, `-iS…`, `--split-string=…`, `--split=…`), empty when it is
/// all in the next words (`-iS`, `--split`), `None` for another flag.
pub(super) fn env_split(flag: &str) -> Option<&str> {
    if let Some(long) = flag.strip_prefix("--") {
        let (name, value) = long.split_once('=').unwrap_or((long, ""));
        return (!name.is_empty() && "split-string".starts_with(name)).then_some(value);
    }
    let letters = flag.strip_prefix('-')?;
    for (i, letter) in letters.char_indices() {
        match letter {
            'S' => return Some(&letters[i + 1..]),
            // A flag whose value is the rest of the word.
            'u' | 'C' | 'P' | 'a' => return None,
            _ => {}
        }
    }
    None
}

/// Skips an `npx`-like runner's flags from `k`. `Some`: `-c`, `--call` or `--shell-mode`
/// gives it shell code (its text and whether all of it is known), the rest of its words.
pub(super) fn runner(argv: &[String], fixed: &[bool], k: &mut usize) -> Option<(String, bool)> {
    while *k < argv.len() && argv[*k].starts_with('-') && argv[*k] != "-" {
        let flag = argv[*k].as_str();
        *k += 1;
        if flag == "--" {
            break;
        }
        let attached = flag.strip_prefix("--call=");
        if attached.is_some() || matches!(flag, "-c" | "--call" | "--shell-mode") {
            let from = if attached.is_some() { *k - 1 } else { *k };
            let mut words: Vec<&str> = argv
                .get(from..)
                .unwrap_or_default()
                .iter()
                .map(String::as_str)
                .collect();
            if let (Some(code), Some(first)) = (attached, words.first_mut()) {
                *first = code;
            }
            let known = fixed
                .get(from..argv.len())
                .is_some_and(|known| known.iter().all(|f| *f));
            return Some((words.join(" "), known));
        }
        if matches(
            "^(-p|--package|-w|--workspace|--filter|-C|--dir|--prefix)$",
            Some(flag),
        ) {
            *k += 1;
        }
    }
    None
}

/// Skips a Python runner's flags from `k` (`uv run`, `uvx`, `poetry run`, `pipx run`).
/// `true`: it runs Python code rather than a command (`-m`, `--script`, a `.py` file, a URL).
pub(super) fn python_runner(argv: &[String], k: &mut usize) -> bool {
    while *k < argv.len() && argv[*k].starts_with('-') && argv[*k] != "-" {
        let flag = argv[*k].as_str();
        *k += 1;
        if flag == "--" {
            break;
        }
        if matches("^(-m|--module|-s|--script|--gui-script)$", Some(flag)) {
            return true;
        }
        if matches(
            "^(--with|--with-editable|--with-requirements|-p|--python|--project|--directory|-C|\
             --config-setting|-P|--package|--extra|--group|--only-group|--no-group|--env-file|\
             --index|--default-index|-i|--index-url|--extra-index-url|-f|--find-links|--from|\
             --spec|--pip-args|--cache-dir|--config-file|--exclude-newer|--backend)$",
            Some(flag),
        ) {
            *k += 1;
        }
    }
    argv.get(*k)
        .is_some_and(|w| matches(r"(?i)\.pyw?$|://", Some(w)))
}

/// The backstop over a command's own words (never what a heredoc feeds it): the first word
/// whose name is `git`, `gh` or a package manager followed by an outward step of its own (a
/// push, a `gh` write, a publish), whatever runs it and whether or not the reader read it
/// through.
pub(super) fn named_step(argv: &[String], fixed: &[bool]) -> Option<String> {
    for (i, word) in argv.iter().enumerate() {
        if !fixed.get(i).copied().unwrap_or(false) {
            continue;
        }
        let a = &argv[i..];
        let fa = fixed.get(i..).unwrap_or_default();
        let v = base(word);
        let found = match v {
            "git" => git_pushes(a, fa).then(|| "git push".to_owned()),
            "gh" => gh_step(a),
            "npm" | "pnpm" | "yarn" | "bun" => {
                let p = positional(a, 1, "^(--prefix|-C|--dir|--filter|-w|--workspace|--cwd)$");
                let (q0, q1) = (p.first().map(String::as_str), p.get(1).map(String::as_str));
                (q0 == Some("publish") || (q0 == Some("npm") && q1 == Some("publish")))
                    .then(|| format!("{v} publish"))
            }
            _ => None,
        };
        if found.is_some() {
            return found;
        }
    }
    None
}

/// Whether git's words `a` (`fa`: which are known) push: `push`, `subtree push`, or a
/// subcommand that can't be known here.
fn git_pushes(a: &[String], fa: &[bool]) -> bool {
    let mut j = 1;
    while j < a.len() && a[j].starts_with('-') {
        if matches(
            "^(-C|-c|--git-dir|--work-tree|--namespace|--super-prefix|--config-env)$",
            Some(&a[j]),
        ) {
            j += 1;
        }
        j += 1;
    }
    let Some(sub) = a.get(j) else {
        return false;
    };
    !fa.get(j).copied().unwrap_or(false)
        || sub == "push"
        || (sub == "subtree" && a.get(j + 1).is_some_and(|w| w == "push"))
}

/// What `gh`'s words `a` write, if they do: a `gh api` write, or a command [`gh_writes`] names.
fn gh_step(a: &[String]) -> Option<String> {
    let p = positional(a, 1, "^(-R|--repo|--hostname)$");
    let (s1, s2) = (p.first().map(String::as_str), p.get(1).map(String::as_str));
    if s1 == Some("api") {
        let at_api = a.iter().position(|w| w == "api").unwrap_or(0);
        let after = a.get(at_api + 1..).unwrap_or_default();
        let endpoint = positional(after, 0, super::API_ARGS_SOURCE)
            .into_iter()
            .next();
        return api_writes(after, endpoint.as_deref()).then(|| "gh api".to_owned());
    }
    gh_writes(s1, s2).then(|| format!("gh {} {}", s1.unwrap_or(""), s2.unwrap_or("")))
}

/// Whether `word` (a flag of `sudo` or `doas`) makes it run a shell (`-s`, `-i`).
pub(super) fn runs_a_shell(word: &str) -> bool {
    matches!(word, "--shell" | "--login")
        || (word.len() > 1
            && word.starts_with('-')
            && !word.starts_with("--")
            && word[1..].chars().any(|c| c == 's' || c == 'i'))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(line: &str) -> Vec<String> {
        line.split(' ').map(str::to_owned).collect()
    }

    #[test]
    fn xargs_says_what_it_replaces() {
        let read = |line: &str| {
            let argv = words(line);
            let mut k = 1;
            (xargs(&argv, &vec![true; argv.len()], &mut k), k)
        };
        assert_eq!(read("xargs -I {} sh"), (Replace::With("{}".to_owned()), 3));
        assert_eq!(read("xargs -I% sh"), (Replace::With("%".to_owned()), 2));
        assert_eq!(read("xargs -0I@ sh"), (Replace::With("@".to_owned()), 2));
        assert_eq!(read("xargs -i sh"), (Replace::With("{}".to_owned()), 2));
        assert_eq!(
            read("xargs --replace=X sh"),
            (Replace::With("X".to_owned()), 2)
        );
        assert_eq!(read("xargs -n 1 -P 4 git"), (Replace::None, 5));
        assert_eq!(read("xargs -l git"), (Replace::None, 2));
    }

    #[test]
    fn the_backstop_finds_a_push_or_a_write_in_any_words() {
        let found = |line: &str| {
            let argv = words(line);
            named_step(&argv, &vec![true; argv.len()])
        };
        assert_eq!(
            found("wrapper -x git -C . push").as_deref(),
            Some("git push")
        );
        assert_eq!(
            found("x /usr/bin/gh pr merge 5").as_deref(),
            Some("gh pr merge")
        );
        assert_eq!(found("x npm publish").as_deref(), Some("npm publish"));
        assert_eq!(found("git log --grep push"), None);
        assert_eq!(found("grep -rn git src"), None);
        assert_eq!(found("gh pr view 5"), None);
    }
}
