//! Outward steps: the commands that reach outside the job (a push, a merge, a pull request, a
//! release, a deploy, a service restart). A worker's or an ultra run's lead's outward step goes
//! through only when the owner's words cover it ([`named`]: their request names it, or their
//! OK was given for it) or every repository it touches is one the installation lets the
//! assistant always merge; otherwise the server asks the owner itself.
//!
//! A command line's steps are read from its verbs ([`steps`], over [`super::shell`]), never
//! from a heredoc's body or a quoted string. `bash -c`, `sh <<EOF`, `eval`, `ssh`, `sudo`,
//! `env`, `xargs`, `su -c`, `tmux`, `trap`, `npm exec`, `uv run` and the like are read through
//! to the command or the code they run; a wrapper is known by its name, and by its path only
//! in the system's own folders. As a backstop, a command whose own words hold a push or a `gh`
//! write is held whatever runs it.
//!
//! The reading fails closed: what it can't be sure of is held like an outward step
//! ([`unsure`]). That is a line it can't take apart, more than [`WRAPPERS`] words before a
//! command, a command whose name is a variable or a substitution, a script file or a shell
//! reading a pipe, an interpreter's inline code or script, a command `xargs` or `find -exec`
//! runs whose words decide what it does, git with a setting that can change what it runs or a
//! command that isn't one of its own, a `gh` alias or extension, and an HTTP write (a method
//! other than GET or HEAD, or a body) to another machine. So are a shell that `xargs` or
//! `find -exec` runs, a word holding `xargs -I`'s replacement, keys typed into `tmux` or
//! `screen`, and `parallel`. It reads a line, not what the line runs: a build script, a test
//! runner or a git hook is code it never sees.

use std::sync::LazyLock;

use regex::Regex;

use super::shell::{self, ShellError};

mod wrap;

/// What a step held back tells the session that ran it.
pub const WAITS: &str = "Outward actions (push, merge, PR, release, deploy) wait for the \
owner's OK, and their request for this task doesn't name them: the server asks the owner for \
you now (a card on the task). Don't ask them yourself. Have it ready, carry on with the rest, \
and run it again when you're told they OK'd it (a sub-agent reports it to the lead). A command \
the server can't read through (a script file, inline code, a command in a variable, an HTTP \
write) is held the same way: run its steps as plain commands where you can.";

/// A regex compiled once; `None` only if its pattern were wrong (a test checks every one).
pub(crate) type Pattern = LazyLock<Option<Regex>>;

macro_rules! pattern {
    ($name:ident, $re:expr) => {
        static $name: Pattern = LazyLock::new(|| Regex::new($re).ok());
    };
}

/// Whether `re` matches `text`.
pub(crate) fn is(re: &Pattern, text: &str) -> bool {
    re.as_ref().is_some_and(|re| re.is_match(text))
}

pattern!(
    NAMES,
    r"(?i)\b(push(es|ed|ing)?|merg(e|es|ed|ing)|deploy(s|ed|ing|ment)?|ship(s|ped|ping)?\s+(it|them|this|that)|publish(es|ed|ing)?|release\s+(it|them|this|that)|(put|make|take|push|get)\s+(it|them|this|that|everything|all\s+of\s+it)?\s*(live|to\s+prod(uction)?|on\s+prod(uction)?|in\s+prod(uction)?)|go(es)?\s+live|open\s+(a\s+|the\s+)?prs?)\b"
);
// The words that make a sentence no OK: a no, a wait, a condition. Read over
// [`super::go::words`], so "don’t" with a curly apostrophe is "dont".
pattern!(
    NOT,
    r"(?:^|\s)(?:no|not|dont|do not|never|without|hold|wait|until|before|after|skip|stop|cancel|later|if|whether|unless|once|when|maybe|لا|مش|ما|بلاش|استنى|بعدين|لما|اذا)(?:\s|$)"
);
// A sentence that asks: it ends on a question mark, or starts like a question.
pattern!(
    ASKS,
    r"^(?:did|didnt|have|has|is|isnt|are|arent|can|could|should|shall|would|will|does|do you|why|how|what|where|when|who|which|هل|ليش|شو|كيف|وين|متى)(?:\s|$)"
);
pattern!(SENTENCES, r"[^.!?؟;\n]+[.!?؟;\n]*");

/// The owner's words that name an outward step ("merge it", "deploy", "put it live",
/// "publish"), or none. Each sentence is read on its own, its text normalised first
/// ([`super::go::words`]: case, accents, apostrophes and punctuation): a question names none,
/// and nor does a sentence with a no, a wait or a condition in it ("don’t merge it yet",
/// "hold off on merging", "merge it if the tests pass"). In doubt it's none: a missed OK costs
/// one question, a false one pushes or merges without the owner.
#[must_use]
pub fn named(text: &str) -> Option<String> {
    let names = NAMES.as_ref()?;
    let sentences = SENTENCES.as_ref()?;
    for sentence in sentences.find_iter(text) {
        let sentence = sentence.as_str();
        if sentence.contains('?') || sentence.contains('؟') {
            continue;
        }
        let words = super::go::words(sentence);
        if words.is_empty() || is(&ASKS, &words) || is(&NOT, &words) {
            continue;
        }
        if let Some(found) = names.find(&words) {
            return Some(found.as_str().to_owned());
        }
    }
    None
}

/// A repository's `owner/name` from its address (`git@github.com:o/n.git`, a URL, `o/n`).
#[must_use]
pub fn repo_name(text: &str) -> Option<String> {
    pattern!(
        REPO,
        r"(?:github\.com[:/])?([\w.-]+)/([\w.-]+?)(?:\.git)?/?$"
    );
    let caps = REPO.as_ref()?.captures(text.trim())?;
    Some(format!("{}/{}", &caps[1], &caps[2]))
}

/// One outward step of a command line: what it is ("git push", "gh pr merge"), its words,
/// the folder it runs in, and the repositories it touches (none: they can't be told).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Step {
    /// What it is, in words.
    pub what: String,
    /// The command's words.
    pub argv: Vec<String>,
    /// Where it runs, when that can be told.
    pub dir: Option<String>,
    /// The repositories it touches, when they can be told.
    pub repos: Option<Vec<String>>,
}

/// What reading a command line needs from outside: the user's home, and a folder's git
/// remotes (`None`: not a repository, or they can't be read).
pub trait Remotes {
    /// The user's home.
    fn home(&self) -> &str;
    /// The remotes of the repository at `dir`, as `owner/name` where they are GitHub's.
    fn remotes(&self, dir: &str) -> Option<Vec<String>>;
}

/// A reader that knows no repository: for a line whose repositories don't matter.
#[derive(Clone, Debug)]
pub struct NoRemotes(pub String);

impl Remotes for NoRemotes {
    fn home(&self) -> &str {
        &self.0
    }

    fn remotes(&self, _dir: &str) -> Option<Vec<String>> {
        None
    }
}

/// The most words read before a command as what runs it (`sudo`, `env`, `nice`, `timeout`,
/// `xargs`...), the command's own word included; a line with more is held.
pub const WRAPPERS: usize = 9;

pattern!(SHELLS, r"^(bash|sh|zsh|dash|ksh|ash|fish)$");
pattern!(
    INTERPRETERS,
    r"^(python[\d.]*|pypy[\d.]*|node|nodejs|deno|tsx|ts-node|ruby|perl|php|lua|osascript|pwsh|Rscript)$"
);
pattern!(DEPLOYISH, r"(?i)^deploy([._-]|$)");
pattern!(SCRIPT, r"(?i)deploy|release|publish");
pattern!(SOURCE_FILE, r"(?i)\.(m?[jt]sx?|c[jt]s)$");
pattern!(CS_FILE, r"(?i)\.cs$");
pattern!(
    RESTARTS,
    r"^(restart|try-restart|reload-or-restart|try-reload-or-restart|condrestart|force-reload|reload|start|stop|kill)$"
);
pattern!(URLS, r"(?:https?://|git@)github\.com[:/][\w.-]+/[\w.-]+");
pattern!(
    API_ARGS,
    r"^(-X|--method|-f|-F|--field|--raw-field|--input|-H|--header|-q|--jq|-t|--template|--cache|--hostname|-p|--preview)$"
);
pattern!(ASSIGN, r"^[A-Za-z_]\w*=");
pattern!(
    LEADS,
    r"^(then|do|else|elif|if|while|until|!|\{|time|nohup|command|builtin|chronic|unbuffer|caffeinate)$"
);
// The environment that changes what git runs: its settings and where its commands are.
pattern!(GIT_ENV, r"^GIT_(CONFIG\w*|EXEC_PATH)=");
// Git's own commands that keep to this machine (a push is read on its own). Any other word is
// an alias, a plumbing command that reaches out (`send-pack`) or a `git-<name>` on the path.
pattern!(
    GIT_LOCAL,
    r"^(add|am|annotate|apply|archive|bisect|blame|branch|bundle|cat-file|check-attr|check-ignore|check-mailmap|check-ref-format|checkout|checkout-index|cherry|cherry-pick|clean|clone|commit|commit-graph|commit-tree|config|count-objects|describe|diff|diff-files|diff-index|diff-tree|fetch|for-each-ref|format-patch|fsck|gc|grep|hash-object|help|init|log|ls-files|ls-remote|ls-tree|maintenance|merge|merge-base|merge-file|merge-tree|mktag|mktree|mv|name-rev|notes|pack-refs|prune|pull|range-diff|read-tree|rebase|reflog|remote|repack|replace|rerere|reset|restore|rev-list|rev-parse|revert|rm|shortlog|show|show-branch|show-ref|sparse-checkout|stash|status|stripspace|submodule|subtree|switch|symbolic-ref|tag|update-index|update-ref|var|verify-commit|verify-tag|version|whatchanged|worktree|write-tree)$"
);
// The `git -c` settings that change neither what a command runs nor where it reaches.
pattern!(
    GIT_SAFE_CONFIG,
    r"(?i)^(user\.(name|email)|color\.[\w.-]+|advice\.[\w-]+|core\.quotepath|commit\.gpgsign|tag\.gpgsign|init\.defaultbranch|safe\.directory|pull\.(rebase|ff)|merge\.ff|log\.[\w-]+|status\.[\w-]+|gc\.auto)(=|$)"
);
// `gh`'s own commands; any other is an alias or an extension, which can run anything.
pattern!(
    GH_KNOWN,
    r"^(alias|api|attestation|auth|browse|cache|codespace|completion|config|extension|ext|gist|gpg-key|help|issue|label|org|pr|project|release|repo|ruleset|run|search|secret|ssh-key|status|variable|workflow)$"
);
// The commands whose words decide whether they reach outside: run by `xargs` or `find -exec`,
// with words read from their input, they are held.
pattern!(
    WORDY,
    r"^(git|gh|npm|pnpm|yarn|bun|npx|bunx|pnpx|make|vercel|netlify|fly|flyctl|railway|firebase|wrangler|gcloud|kubectl|helm|terraform|tofu|systemctl|launchctl|service|curl|wget|http|https|xh|xhs|dotnet|docker|scp|rsync)$"
);

/// What a step it can't be sure of is: held like an outward step, its repositories unknown.
pub mod unsure {
    /// A line the reader can't take apart, or wrappers deeper than [`super::WRAPPERS`].
    pub const UNREAD: &str = "a command line it can't read";
    /// A command whose name is a variable or a substitution.
    pub const VERB: &str = "a command in a variable";
    /// A script file, or a shell reading its input from a pipe or a file.
    pub const SCRIPT: &str = "a script";
    /// An interpreter running code it doesn't read (`-c`, `-e`, its input, a file).
    pub const CODE: &str = "inline code or a script";
    /// A command `xargs` or `find -exec` runs, with words from its input.
    pub const PIPED: &str = "a command xargs or find runs";
    /// Git with settings or a command that can change what it runs or where it reaches.
    pub const GIT: &str = "git with a setting or a command it doesn't know";
    /// A command git runs (`submodule foreach`, `bisect run`, `rebase --exec`).
    pub const GIT_RUNS: &str = "a command git runs";
    /// A `gh` alias or extension.
    pub const GH: &str = "a gh command it doesn't know";
    /// An HTTP write (a method other than GET or HEAD, or a body) to another machine.
    pub const HTTP: &str = "an HTTP write";
}

/// The last part of a path.
fn base(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// Whether `pattern` (compiled now) matches `text`.
fn matches(pattern: &str, text: Option<&str>) -> bool {
    text.is_some_and(|text| Regex::new(pattern).is_ok_and(|re| re.is_match(text)))
}

/// Whether `path` is a program in one of the system's own folders, read by its name.
fn system_program(path: &str) -> bool {
    [
        "/bin/",
        "/sbin/",
        "/usr/bin/",
        "/usr/sbin/",
        "/usr/local/bin/",
        "/opt/homebrew/bin/",
    ]
    .iter()
    .any(|dir| {
        path.strip_prefix(dir)
            .is_some_and(|name| !name.contains('/'))
    })
}

/// A step it can't be sure of ([`unsure`]): held, its repositories unknown.
fn held(what: &str, argv: &[String], dir: Option<String>) -> Vec<Step> {
    vec![Step {
        what: what.to_owned(),
        argv: argv.to_vec(),
        dir,
        repos: None,
    }]
}

/// The outward steps of `command` run in `cwd`. `remote`: it runs on another machine.
///
/// # Errors
///
/// [`ShellError`] when the line can't be taken apart.
pub fn steps(
    command: &str,
    cwd: Option<&str>,
    remote: bool,
    remotes: &dyn Remotes,
) -> Result<Vec<Step>, ShellError> {
    steps_at(command, cwd, remote, 0, remotes)
}

fn steps_at(
    command: &str,
    cwd: Option<&str>,
    remote: bool,
    depth: usize,
    remotes: &dyn Remotes,
) -> Result<Vec<Step>, ShellError> {
    let mut steps = Vec::new();
    for run in shell::parse(command, cwd, remote, remotes.home())? {
        let found = steps_of(&run, remote, depth, remotes, false)?;
        // The backstop: a push or a `gh` write in the command's own words is held whatever
        // runs it, even where the reading above found no step.
        match wrap::named_step(&run.argv, &run.fixed) {
            Some(what) if found.is_empty() => {
                steps.extend(held(&what, &run.argv, run.dir.clone()));
            }
            _ => steps.extend(found),
        }
    }
    Ok(steps)
}

/// The steps of one simple command. `piped`: `xargs` or `find -exec` runs it, adding words
/// that can't be known here.
#[expect(
    clippy::too_many_lines,
    reason = "one table of the verbs that reach outside"
)]
fn steps_of(
    run: &shell::Run,
    remote: bool,
    depth: usize,
    remotes: &dyn Remotes,
    piped: bool,
) -> Result<Vec<Step>, ShellError> {
    if depth > 8 {
        return Err(ShellError("too deep".to_owned()));
    }
    let argv = &run.argv;
    // `xargs -I` makes the words holding its replacement unknown.
    let mut fixed = run.fixed.clone();
    let mut dir = run.dir.clone();
    let stdin = &run.stdin;
    let mut piped = piped;
    // How many words were read up to the first `xargs`: a wrapper after it may run its input.
    let mut xargs_at = None;
    let mut git_env = false;
    // A wrapper that runs a shell when it is given no command (`su`, `sudo -s`, `chroot`).
    let mut shell_if_none = false;
    let nested =
        |script: &str, d: Option<&str>, far: bool| steps_at(script, d, far, depth + 1, remotes);
    // Shell code a command runs (`sh -c`, `su -c`, `trap`), its words joined: read through, or
    // held when xargs or find adds words to it or a part of it can't be known here.
    let code = |words: &[String], known: &[bool], d: Option<String>, piped: bool| {
        if piped {
            return Ok(held(unsure::PIPED, words, d));
        }
        if known.len() < words.len() || known.iter().any(|f| !f) {
            return Ok(held(unsure::CODE, words, d));
        }
        nested(&words.join(" "), d.as_deref(), remote)
    };
    // What a shell reading its input runs: a heredoc read through, a pipe or a file held.
    let input = |d: Option<String>, piped: bool| -> Result<Vec<Step>, ShellError> {
        if stdin.is_empty() || piped {
            return Ok(held(unsure::SCRIPT, argv, d));
        }
        let mut all = Vec::new();
        for text in stdin {
            all.extend(nested(text, d.as_deref(), remote)?);
        }
        Ok(all)
    };
    let arg = |j: usize| argv.get(j).map(String::as_str);
    let rest = |k: usize| argv.get(k..).unwrap_or_default().join(" ");
    let mut k = 0;
    // Skips -x and --x, and the value of one that takes it.
    let flags = |k: &mut usize, with_arg: &str| {
        while *k < argv.len() && argv[*k].starts_with('-') && argv[*k] != "-" {
            if argv[*k] == "--" {
                *k += 1;
                break;
            }
            let flag = &argv[*k];
            *k += 1;
            if matches(with_arg, Some(flag)) && !flag.contains('=') {
                *k += 1;
            }
        }
    };
    let mut words = 0;
    while k < argv.len() {
        let w = argv[k].as_str();
        // A leading word or a wrapper is read by its name, and by its path only in the
        // system's own folders: anywhere else it is a script that may only share the name.
        let b = if system_program(w) { base(w) } else { w };
        if is(&ASSIGN, w) {
            git_env |= is(&GIT_ENV, w);
            k += 1;
            continue;
        }
        words += 1;
        if words > WRAPPERS {
            return Err(ShellError(format!(
                "more than {WRAPPERS} words before a command"
            )));
        }
        if !fixed.get(k).copied().unwrap_or(false) {
            // Its name can't be known: the verb's check holds it.
            break;
        }
        if is(&LEADS, b) {
            if b == "command" && matches!(arg(k + 1), Some("-v" | "-V")) {
                // `command -v git` looks a command up; it runs nothing.
                return Ok(Vec::new());
            }
            k += 1;
            match b {
                "time" => flags(&mut k, "^-[fo]$|^--(format|output)$"),
                "command" => flags(&mut k, "^$"),
                "caffeinate" => flags(&mut k, "^-[tw]$"),
                _ => {}
            }
            continue;
        }
        match b {
            "sudo" | "doas" => {
                k += 1;
                let from = k;
                flags(
                    &mut k,
                    "^-[ugCDhpRTU]$|^--(user|group|chdir|host|prompt|role|type|other-user|close-from)$",
                );
                let given = argv.get(from..k.min(argv.len())).unwrap_or_default();
                if given.iter().any(|f| wrap::runs_a_shell(f)) {
                    // `-s`, `-i`: a shell runs the command, or reads its input.
                    if k < argv.len() {
                        let known = fixed.get(k..).unwrap_or_default();
                        return code(&argv[k..], known, dir, piped);
                    }
                    shell_if_none = true;
                }
                continue;
            }
            "env" => {
                k += 1;
                while k < argv.len() && (argv[k].starts_with('-') || is(&ASSIGN, &argv[k])) {
                    let flag = argv[k].as_str();
                    git_env |= is(&GIT_ENV, flag);
                    k += 1;
                    if flag == "-C" || flag == "--chdir" {
                        dir = if k < argv.len() && fixed[k] {
                            shell::place(&argv[k], dir.as_deref(), remotes.home())
                        } else {
                            None
                        };
                        k += 1;
                    } else if let Some(to) = flag.strip_prefix("--chdir=") {
                        dir = shell::place(to, dir.as_deref(), remotes.home());
                    } else if flag == "-S" || flag == "--split-string" {
                        if piped {
                            return Ok(held(unsure::PIPED, argv, dir));
                        }
                        return nested(&rest(k), dir.as_deref(), remote);
                    } else if let Some(value) = wrap::env_split(flag) {
                        // `-S'…'`, `-iS …`, `--split-string=…`: the split string is shell code.
                        let glued = !value.is_empty();
                        let mut words: Vec<String> =
                            glued.then(|| value.to_owned()).into_iter().collect();
                        let mut known: Vec<bool> =
                            glued.then(|| fixed[k - 1]).into_iter().collect();
                        words.extend(argv.get(k..).unwrap_or_default().iter().cloned());
                        known.extend(fixed.get(k..).unwrap_or_default());
                        return code(&words, &known, dir, piped);
                    } else if flag == "-u" || flag == "--unset" {
                        k += 1;
                    } else if flag == "--" {
                        break;
                    }
                }
                continue;
            }
            "exec" => {
                k += 1;
                flags(&mut k, "^-a$");
                continue;
            }
            "nice" => {
                k += 1;
                flags(&mut k, "^-n$|^--adjustment$");
                continue;
            }
            "ionice" => {
                k += 1;
                flags(&mut k, "^-[cnp]$|^--(class|classdata|pid)$");
                continue;
            }
            "stdbuf" => {
                k += 1;
                flags(&mut k, "^-[ioe]$");
                continue;
            }
            "timeout" => {
                k += 1;
                flags(&mut k, "^-[sk]$|^--(signal|kill-after)$");
                k += 1;
                continue;
            }
            "flock" => {
                k += 1;
                if let Some(c) = argv[k.min(argv.len())..]
                    .iter()
                    .position(|a| a == "-c" || a == "--command")
                {
                    if piped {
                        return Ok(held(unsure::PIPED, argv, dir));
                    }
                    return nested(arg(k + c + 1).unwrap_or(""), dir.as_deref(), remote);
                }
                flags(&mut k, "^-[wEn]$|^--(timeout|conflict-exit-code)$");
                k += 1;
                continue;
            }
            "xargs" => {
                k += 1;
                match wrap::xargs(argv, &fixed, &mut k) {
                    wrap::Replace::None => {}
                    wrap::Replace::With(with) => {
                        for (word, known) in argv.iter().zip(fixed.iter_mut()).skip(k) {
                            *known &= !word.contains(with.as_str());
                        }
                    }
                    wrap::Replace::Unknown => return Ok(held(unsure::PIPED, argv, dir)),
                }
                piped = true;
                xargs_at.get_or_insert(words);
                continue;
            }
            "setsid" | "firejail" => {
                k += 1;
                flags(&mut k, "^$");
                shell_if_none |= b == "firejail";
                continue;
            }
            "taskset" => {
                // Its flags, then the CPU mask.
                k += 1;
                flags(&mut k, "^$");
                k += 1;
                continue;
            }
            "chrt" => {
                // Its flags, then the priority.
                k += 1;
                flags(&mut k, "^-[TPD]$|^--sched-(runtime|period|deadline)$");
                k += 1;
                continue;
            }
            "strace" => {
                k += 1;
                flags(
                    &mut k,
                    "^-[abeEIoOpPsSuXU]$|^--(trace|trace-path|signal|status|output|attach|user|env|string-limit|columns|summary-sort-by|overhead|interruptible|decode-fds|quiet|silence|absolute-timestamps|syscall-limit)$",
                );
                continue;
            }
            "ltrace" => {
                k += 1;
                flags(
                    &mut k,
                    "^-[aADeFlnopsuwx]$|^--(align|output|library|indent)$",
                );
                continue;
            }
            "unshare" => {
                k += 1;
                flags(
                    &mut k,
                    "^-[SGRw]$|^--(setuid|setgid|root|wd|map-user|map-group|map-users|map-groups|propagation|setgroups)$",
                );
                shell_if_none = true;
                continue;
            }
            "chroot" => {
                // Its flags, then the new root.
                k += 1;
                flags(&mut k, "^--(userspec|groups)$");
                k += 1;
                shell_if_none = true;
                continue;
            }
            "systemd-run" => {
                k += 1;
                flags(
                    &mut k,
                    "^-[upHMEC]$|^--(unit|property|host|machine|description|slice|setenv|uid|gid|nice|working-directory|on-active|on-boot|on-startup|on-unit-active|on-unit-inactive|on-calendar|timer-property|path-property|socket-property|service-type|capsule)$",
                );
                shell_if_none = true;
                continue;
            }
            // Its words are shell code, or commands from its input.
            "parallel" => return Ok(held(unsure::PIPED, argv, dir)),
            "su" | "runuser" | "script" => {
                if piped {
                    return Ok(held(unsure::PIPED, argv, dir));
                }
                let then = if b == "script" {
                    wrap::script(argv, &fixed, k)
                } else {
                    wrap::su(argv, &fixed, k, b == "runuser")
                };
                match then {
                    wrap::Then::Code { text, known } => {
                        return code(std::slice::from_ref(&text), &[known], dir, piped);
                    }
                    wrap::Then::Command(j) => {
                        k = j;
                        continue;
                    }
                    wrap::Then::Shell => return input(dir, piped),
                    wrap::Then::Unread => return Ok(held(unsure::SCRIPT, argv, dir)),
                }
            }
            "tmux" | "screen" => {
                // Each word may be a command it runs: read as shell code.
                if piped {
                    return Ok(held(unsure::PIPED, argv, dir));
                }
                let words = argv.get(k + 1..).unwrap_or_default();
                let known = fixed.get(k + 1..).unwrap_or_default();
                if known.iter().any(|f| !f) || words.iter().any(|w| wrap::multiplexer_unread(b, w))
                {
                    return Ok(held(unsure::CODE, argv, dir));
                }
                let mut all = Vec::new();
                for word in words {
                    all.extend(nested(word, dir.as_deref(), remote)?);
                }
                return Ok(all);
            }
            "trap" => {
                let mut j = k + 1;
                if matches!(arg(j), Some("-p" | "-P" | "-l")) {
                    return Ok(Vec::new());
                }
                if arg(j) == Some("--") {
                    j += 1;
                }
                if matches!(arg(j), None | Some("-")) {
                    return Ok(Vec::new());
                }
                let known = fixed.get(j..=j).unwrap_or_default();
                return code(&argv[j..=j], known, dir, piped);
            }
            "bun" | "yarn" if arg(k + 1) == Some("exec") => {
                // Their `exec` runs its words as a shell script.
                let j = k + 2;
                return code(
                    argv.get(j..).unwrap_or_default(),
                    fixed.get(j..).unwrap_or_default(),
                    dir,
                    piped,
                );
            }
            "npm" | "pnpm" | "bun" if matches!(arg(k + 1), Some("exec" | "x")) => {
                k += 2;
                if let Some((text, known)) = wrap::runner(argv, &fixed, &mut k) {
                    return code(std::slice::from_ref(&text), &[known], dir, piped);
                }
                continue;
            }
            "uv" | "uvx" | "poetry" | "pipx"
                if b == "uvx"
                    || arg(k + 1) == Some("run")
                    || (b == "uv" && arg(k + 1) == Some("tool") && arg(k + 2) == Some("run")) =>
            {
                k += match (b, arg(k + 1)) {
                    ("uvx", _) => 1,
                    (_, Some("tool")) => 3,
                    _ => 2,
                };
                if wrap::python_runner(argv, &mut k) {
                    return Ok(held(unsure::CODE, argv, dir));
                }
                continue;
            }
            "watch" => {
                k += 1;
                flags(&mut k, "^-[nq]$|^--(interval|equexit)$");
                if piped {
                    return Ok(held(unsure::PIPED, argv, dir));
                }
                return nested(&rest(k), dir.as_deref(), remote);
            }
            "npx" | "bunx" | "pnpx" => {
                k += 1;
                if let Some((text, known)) = wrap::runner(argv, &fixed, &mut k) {
                    return code(std::slice::from_ref(&text), &[known], dir, piped);
                }
                continue;
            }
            "pnpm" | "yarn" if arg(k + 1) == Some("dlx") => {
                k += 2;
                flags(&mut k, "^-p$|^--package$");
                continue;
            }
            "eval" => {
                if piped {
                    return Ok(held(unsure::PIPED, argv, dir));
                }
                return nested(&rest(k + 1), dir.as_deref(), remote);
            }
            "source" | "." => {
                return Ok(if k + 1 < argv.len() {
                    held(unsure::SCRIPT, argv, dir)
                } else {
                    Vec::new()
                });
            }
            "tailscale" if arg(k + 1) == Some("ssh") => {
                k += 1;
                continue;
            }
            "ssh" | "mosh" => {
                if piped {
                    return Ok(held(unsure::PIPED, argv, dir));
                }
                k += 1;
                flags(&mut k, "^-[bcDEeFIiJLlmOopQRSWw]$");
                // The host.
                k += 1;
                if k < argv.len() {
                    return nested(&rest(k), None, true);
                }
                let mut all = Vec::new();
                for text in stdin {
                    all.extend(nested(text, None, true)?);
                }
                return Ok(all);
            }
            _ => {}
        }
        if is(&SHELLS, b) {
            if piped {
                // `xargs sh -c`, `find -exec sh -c`: its code or its words come from the input.
                return Ok(held(unsure::PIPED, argv, dir));
            }
            k += 1;
            let mut from_input = false;
            while k < argv.len() && argv[k].starts_with('-') && argv[k] != "-" && argv[k] != "--" {
                if matches!(argv[k].as_str(), "--version" | "--help") {
                    return Ok(Vec::new());
                }
                let lower = argv[k].to_lowercase();
                if !argv[k].starts_with("--") {
                    if matches("^-[a-z]*c[a-z]*$", Some(&lower)) {
                        let j = k + 1;
                        return code(
                            argv.get(j..=j).unwrap_or_default(),
                            fixed.get(j..=j).unwrap_or_default(),
                            dir,
                            piped,
                        );
                    }
                    from_input |= argv[k][1..].contains('s');
                }
                if matches("^-[oO]$|^--(rcfile|init-file)$", Some(&argv[k])) {
                    k += 1;
                }
                k += 1;
            }
            if arg(k) == Some("--") {
                k += 1;
            }
            if k >= argv.len() || argv[k] == "-" || from_input {
                // Its input is what runs: a heredoc is read through, a pipe or a file isn't.
                return input(dir, piped);
            }
            return Ok(held(unsure::SCRIPT, argv, dir));
        }
        break;
    }
    let a = argv.get(k..).unwrap_or_default();
    let fa = fixed.get(k..).unwrap_or_default();
    let Some(first) = a.first() else {
        return if shell_if_none {
            input(dir, piped)
        } else if xargs_at.is_some_and(|at| words > at) {
            // `xargs env`, `xargs sudo`: the wrapper runs the command xargs adds from its input.
            Ok(held(unsure::PIPED, argv, dir))
        } else {
            Ok(Vec::new())
        };
    };
    if !fa.first().copied().unwrap_or(false) {
        return Ok(held(unsure::VERB, a, dir));
    }
    let v = base(first);
    let at = |j: usize| a.get(j).map(String::as_str);
    let p0 = |from: usize, with_arg: &str| positional(a, from, with_arg).into_iter().next();
    let one = |what: String, dir: Option<String>| {
        Ok(vec![make_step(&what, a, dir, None, false, remotes)])
    };
    if piped && (is(&WORDY, v) || is(&INTERPRETERS, v) || first.contains('/')) {
        return Ok(held(unsure::PIPED, a, dir));
    }
    if first.contains('/') && !system_program(first) {
        return Ok(held(unsure::SCRIPT, a, dir));
    }
    if is(&INTERPRETERS, v) {
        let asks_only = a.len() > 1
            && a[1..].iter().all(|t| {
                matches!(
                    t.as_str(),
                    "--version" | "-V" | "-v" | "--help" | "-h" | "version"
                )
            });
        return Ok(if asks_only {
            Vec::new()
        } else {
            held(unsure::CODE, a, dir)
        });
    }

    if v == "git" {
        return Ok(git_steps(a, fa, dir, git_env, remotes));
    }

    if v == "gh" {
        let p = positional(a, 1, "^(-R|--repo|--hostname)$");
        let (s1, s2) = (p.first().map(String::as_str), p.get(1).map(String::as_str));
        if s1.is_some_and(|s| !is(&GH_KNOWN, s)) {
            return Ok(held(unsure::GH, a, dir));
        }
        if s1 == Some("api") {
            let at_api = a.iter().position(|w| w == "api").unwrap_or(0);
            let after = &a[at_api + 1..];
            let endpoint = positional(after, 0, API_ARGS_SOURCE).into_iter().next();
            if api_writes(after, endpoint.as_deref()) {
                let named = gh_repos(a, Some(endpoint.as_deref().unwrap_or("")));
                return Ok(vec![make_step("gh api", a, dir, named, true, remotes)]);
            }
            return Ok(Vec::new());
        }
        if gh_writes(s1, s2) {
            let what = format!("gh {} {}", s1.unwrap_or(""), s2.unwrap_or(""));
            return Ok(vec![make_step(
                &what,
                a,
                dir,
                gh_repos(a, None),
                true,
                remotes,
            )]);
        }
        return Ok(Vec::new());
    }

    if matches!(v, "curl" | "wget" | "http" | "https" | "xh" | "xhs") {
        return Ok(if http::writes(v, a, fa) {
            held(unsure::HTTP, a, dir)
        } else {
            Vec::new()
        });
    }

    if v == "find" {
        return find_steps(a, fa, dir.as_deref(), remote, depth, remotes);
    }

    if matches!(v, "npm" | "pnpm" | "yarn" | "bun") {
        let p = positional(a, 1, "^(--prefix|-C|--dir|--filter|-w|--workspace|--cwd)$");
        let (q0, q1) = (p.first().map(String::as_str), p.get(1).map(String::as_str));
        if v == "bun"
            && (a
                .iter()
                .any(|t| matches!(t.as_str(), "-e" | "--eval" | "-p" | "--print"))
                || q0.is_some_and(|q| is(&SOURCE_FILE, q))
                || (q0 == Some("run") && q1.is_some_and(|q| is(&SOURCE_FILE, q))))
        {
            return Ok(held(unsure::CODE, a, dir));
        }
        if q0 == Some("publish") || (v == "yarn" && q0 == Some("npm") && q1 == Some("publish")) {
            return one(format!("{v} publish"), dir);
        }
        if matches!(q0, Some("run" | "run-script")) && q1.is_some_and(|s| is(&SCRIPT, s)) {
            return one(format!("{v} run {}", q1.unwrap_or("")), dir);
        }
        if v == "yarn"
            && let Some(q0) = q0
            && !matches(
                "^(add|install|remove|run|npm|dlx|why|info|list|workspace|workspaces|test|build)$",
                Some(q0),
            )
            && is(&SCRIPT, q0)
        {
            return one(format!("yarn {q0}"), dir);
        }
        return Ok(Vec::new());
    }
    let pos_all = |from: usize| positional(a, from, "^$");
    let joined = a.join(" ");
    let found = match v {
        "make" => positional(a, 1, "^-[CfIjlo]$")
            .iter()
            .any(|t| is(&SCRIPT, t))
            .then(|| "make deploy".to_owned()),
        "vercel" => (a
            .iter()
            .any(|t| matches("^--prod(uction)?$|^--target=production$", Some(t)))
            || p0(1, "^$").as_deref() == Some("promote")
            || joined.contains("--target production"))
        .then(|| "vercel --prod".to_owned()),
        "netlify" => (p0(1, "^$").as_deref() == Some("deploy") && a.iter().any(|t| t == "--prod"))
            .then(|| "netlify deploy --prod".to_owned()),
        "fly" | "flyctl" => (p0(1, "^(-a|--app|-c|--config)$").as_deref() == Some("deploy"))
            .then(|| "fly deploy".to_owned()),
        "railway" => {
            matches("^(up|redeploy)$", p0(1, "^$").as_deref()).then(|| "railway up".to_owned())
        }
        "firebase" => {
            (p0(1, "^$").as_deref() == Some("deploy")).then(|| "firebase deploy".to_owned())
        }
        "wrangler" => pos_all(1)
            .iter()
            .any(|t| t == "deploy" || t == "publish")
            .then(|| "wrangler deploy".to_owned()),
        "gcloud" => pos_all(1)
            .iter()
            .any(|t| t == "deploy")
            .then(|| "gcloud deploy".to_owned()),
        "kubectl" => matches(
            "^(apply|rollout|replace|set)$",
            p0(1, "^(-n|--namespace|--context|-f)$").as_deref(),
        )
        .then(|| "kubectl apply".to_owned()),
        "helm" => matches(
            "^(install|upgrade|rollback|uninstall)$",
            p0(1, "^$").as_deref(),
        )
        .then(|| "helm upgrade".to_owned()),
        "terraform" | "tofu" => matches("^(apply|destroy)$", p0(1, "^$").as_deref())
            .then(|| "terraform apply".to_owned()),
        "systemctl" => p0(
            1,
            "^(-H|--host|-M|--machine|-t|--type|-p|--property|-s|--signal|--kill-whom)$",
        )
        .filter(|p| {
            is(&RESTARTS, p)
                || (matches("^(enable|disable|mask)$", Some(p)) && a.iter().any(|t| t == "--now"))
        })
        .map(|p| format!("systemctl {p}")),
        "launchctl" => at(1)
            .filter(|w| {
                matches(
                    "^(kickstart|bootout|bootstrap|load|unload|stop|start|kill)$",
                    Some(w),
                )
            })
            .map(|w| format!("launchctl {w}")),
        "service" => at(2)
            .filter(|w| matches("^(restart|start|stop|reload|force-reload)$", Some(w)))
            .map(|w| format!("service {w}")),
        "dotnet"
            if a[1..]
                .iter()
                .any(|t| is(&CS_FILE, t) || is(&DEPLOYISH, base(t))) =>
        {
            Some("a deploy script".to_owned())
        }
        _ if is(&DEPLOYISH, v) => Some("a deploy script".to_owned()),
        _ => None,
    };
    match found {
        Some(what) => one(what, dir),
        None => Ok(Vec::new()),
    }
}

/// Whether `gh <s1> <s2>` writes: a pull request merged, opened or reviewed, an issue opened
/// or commented on, a release changed, a repository deleted, a workflow run.
fn gh_writes(s1: Option<&str>, s2: Option<&str>) -> bool {
    (s1 == Some("pr") && matches("^(merge|create|comment|review)$", s2))
        || (s1 == Some("issue") && matches("^(comment|create)$", s2))
        || (s1 == Some("release") && matches("^(create|upload|edit|delete|delete-asset)$", s2))
        || (s1 == Some("repo") && s2 == Some("delete"))
        || (s1 == Some("workflow") && s2 == Some("run"))
}

/// The steps of a git command `a` (its words' `fixed` in `fa`), run in `dir`; `config`: the
/// environment sets git's settings.
fn git_steps(
    a: &[String],
    fa: &[bool],
    dir: Option<String>,
    config: bool,
    remotes: &dyn Remotes,
) -> Vec<Step> {
    let at = |j: usize| a.get(j).map(String::as_str);
    let known = |j: usize| fa.get(j).copied().unwrap_or(false);
    let mut config = config;
    let mut asks = false;
    let mut j = 1;
    let mut d = dir;
    while j < a.len() && a[j].starts_with('-') {
        let o = a[j].as_str();
        if o == "-C" || o == "--git-dir" || o == "--work-tree" {
            d = if known(j + 1) {
                shell::place(&a[j + 1], d.as_deref(), remotes.home())
            } else {
                None
            };
            j += 1;
        } else if o == "-c" {
            if !(known(j + 1) && at(j + 1).is_some_and(|s| is(&GIT_SAFE_CONFIG, s))) {
                config = true;
            }
            j += 1;
        } else if o == "--config-env" {
            config = true;
            j += 1;
        } else if o.starts_with("--config-env=") || o.starts_with("--exec-path=") {
            config = true;
        } else if o == "--namespace" || o == "--super-prefix" {
            j += 1;
        } else if matches("^--(git-dir|work-tree)=", Some(o)) {
            let to = o.split_once('=').map_or("", |(_, to)| to);
            d = shell::place(to, d.as_deref(), remotes.home());
        } else if matches!(
            o,
            "--version"
                | "--help"
                | "-h"
                | "--exec-path"
                | "--html-path"
                | "--man-path"
                | "--info-path"
        ) {
            asks = true;
        }
        j += 1;
    }
    let Some(sub) = at(j) else {
        return if asks {
            Vec::new()
        } else {
            held(unsure::GIT, a, d)
        };
    };
    if sub == "push" || (sub == "subtree" && at(j + 1) == Some("push")) || !known(j) {
        return vec![make_step("git push", a, d, None, false, remotes)];
    }
    if config || !is(&GIT_LOCAL, sub) {
        return held(unsure::GIT, a, d);
    }
    let after = a.get(j + 1..).unwrap_or_default();
    let runs = (sub == "submodule" && after.iter().any(|t| t == "foreach"))
        || (sub == "bisect" && after.first().is_some_and(|t| t == "run"))
        || (sub == "rebase"
            && after
                .iter()
                .any(|t| t == "-x" || t == "--exec" || t.starts_with("--exec=")));
    if runs {
        return held(unsure::GIT_RUNS, a, d);
    }
    Vec::new()
}

/// The steps of the commands `find -exec`, `-execdir`, `-ok` and `-okdir` run (each found
/// file's name, `{}`, can't be known).
fn find_steps(
    a: &[String],
    fa: &[bool],
    dir: Option<&str>,
    remote: bool,
    depth: usize,
    remotes: &dyn Remotes,
) -> Result<Vec<Step>, ShellError> {
    let mut all = Vec::new();
    let mut j = 1;
    while j < a.len() {
        if !matches!(a[j].as_str(), "-exec" | "-execdir" | "-ok" | "-okdir") {
            j += 1;
            continue;
        }
        let from = j + 1;
        let to = (from..a.len())
            .find(|&e| a[e] == ";" || a[e] == "+")
            .unwrap_or(a.len());
        let words = a.get(from..to).unwrap_or_default();
        let inner = shell::Run {
            argv: words.to_vec(),
            fixed: words
                .iter()
                .zip(fa.get(from..to).unwrap_or_default())
                .map(|(word, fixed)| *fixed && !word.contains("{}"))
                .collect(),
            stdin: Vec::new(),
            dir: dir.map(str::to_owned),
        };
        all.extend(steps_of(&inner, remote, depth + 1, remotes, true)?);
        j = to + 1;
    }
    Ok(all)
}

/// HTTP clients' calls: whether one writes to another machine.
mod http {
    /// What one of a client's flags is.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Flag {
        /// The method, its value.
        Method,
        /// A body, its value.
        Body,
        /// A body, no value (a form's fields follow).
        Form,
        /// The body goes in the address (`curl -G`).
        Get,
        /// The address, its value.
        Url,
        /// Its words or where it connects come from elsewhere (a config file, a proxy).
        Unsure,
        /// Any other flag with a value.
        Value,
        /// A flag without a value.
        Plain,
    }

    impl Flag {
        const fn takes_value(self) -> bool {
            !matches!(self, Self::Form | Self::Get | Self::Plain)
        }
    }

    fn curl_short(c: char) -> Flag {
        match c {
            'X' => Flag::Method,
            'd' | 'F' | 'T' => Flag::Body,
            'G' => Flag::Get,
            'K' | 'x' => Flag::Unsure,
            'H' | 'o' | 'u' | 'A' | 'e' | 'b' | 'c' | 'w' | 'm' | 'r' | 'E' | 'C' | 'D' | 'U'
            | 't' | 'Q' => Flag::Value,
            _ => Flag::Plain,
        }
    }

    fn curl_long(name: &str) -> Flag {
        match name {
            "request" => Flag::Method,
            "data" | "data-raw" | "data-binary" | "data-urlencode" | "data-ascii" | "json"
            | "form" | "form-string" | "upload-file" => Flag::Body,
            "get" => Flag::Get,
            "url" => Flag::Url,
            "config" | "resolve" | "connect-to" | "proxy" | "preproxy" | "socks4" | "socks5"
            | "socks5-hostname" | "variable" => Flag::Unsure,
            "header" | "output" | "user" | "user-agent" | "referer" | "cookie" | "cookie-jar"
            | "write-out" | "max-time" | "connect-timeout" | "retry" | "retry-delay"
            | "retry-max-time" | "cacert" | "capath" | "cert" | "key" | "limit-rate"
            | "max-redirs" | "range" | "oauth2-bearer" | "dump-header" | "trace"
            | "trace-ascii" | "stderr" | "url-query" | "proxy-user" | "quote" => Flag::Value,
            _ => Flag::Plain,
        }
    }

    fn wget_short(c: char) -> Flag {
        match c {
            'e' | 'i' => Flag::Unsure,
            'O' | 'o' | 'a' | 'P' | 'U' | 't' | 'T' | 'w' | 'l' | 'Q' => Flag::Value,
            _ => Flag::Plain,
        }
    }

    fn wget_long(name: &str) -> Flag {
        match name {
            "method" => Flag::Method,
            "post-data" | "post-file" | "body-data" | "body-file" => Flag::Body,
            "execute" | "input-file" | "config" => Flag::Unsure,
            "output-document" | "output-file" | "append-output" | "directory-prefix"
            | "user-agent" | "header" | "user" | "password" | "http-user" | "http-password"
            | "tries" | "timeout" | "wait" | "level" | "quota" | "referer" | "load-cookies"
            | "save-cookies" => Flag::Value,
            _ => Flag::Plain,
        }
    }

    fn httpie_short(c: char) -> Flag {
        match c {
            'f' => Flag::Form,
            'a' | 'A' | 'o' | 'p' => Flag::Value,
            _ => Flag::Plain,
        }
    }

    fn httpie_long(name: &str) -> Flag {
        match name {
            "form" | "multipart" => Flag::Form,
            "raw" => Flag::Body,
            "proxy" => Flag::Unsure,
            "auth" | "auth-type" | "output" | "session" | "session-read-only" | "verify"
            | "cert" | "cert-key" | "timeout" | "print" | "pretty" | "style" | "boundary" => {
                Flag::Value
            }
            _ => Flag::Plain,
        }
    }

    /// A call's words as its client reads them.
    #[derive(Debug, Default)]
    struct Call {
        method: Option<String>,
        body: bool,
        get: bool,
        /// The addresses given with a flag.
        urls: Vec<String>,
        /// The words that aren't flags (the addresses, httpie's method and items).
        words: Vec<String>,
    }

    impl Call {
        /// Takes `flag`, whose value is `value`; `false`: the call can't be told.
        fn take(&mut self, flag: Flag, value: Option<String>) -> bool {
            match flag {
                Flag::Method => self.method = value,
                Flag::Body | Flag::Form => self.body = true,
                Flag::Get => self.get = true,
                Flag::Url => self.urls.extend(value),
                Flag::Unsure => return false,
                Flag::Value | Flag::Plain => {}
            }
            true
        }
    }

    /// `a` read with a client's flags; none when it can't be told.
    fn read(a: &[String], short: fn(char) -> Flag, long: fn(&str) -> Flag) -> Option<Call> {
        let mut call = Call::default();
        let mut j = 1;
        while j < a.len() {
            let t = a[j].as_str();
            if t == "--" {
                call.words.extend(a[j + 1..].iter().cloned());
                break;
            }
            if let Some(name) = t.strip_prefix("--") {
                let (name, glued) = name
                    .split_once('=')
                    .map_or((name, None), |(name, value)| (name, Some(value.to_owned())));
                let flag = long(name);
                let value = if flag.takes_value() && glued.is_none() {
                    j += 1;
                    a.get(j).cloned()
                } else {
                    glued
                };
                if !call.take(flag, value) {
                    return None;
                }
            } else if t.len() > 1 && t.starts_with('-') {
                for (at, c) in t.char_indices().skip(1) {
                    let flag = short(c);
                    if !flag.takes_value() {
                        if !call.take(flag, None) {
                            return None;
                        }
                        continue;
                    }
                    let rest = &t[at + c.len_utf8()..];
                    let value = if rest.is_empty() {
                        j += 1;
                        a.get(j).cloned()
                    } else {
                        Some(rest.to_owned())
                    };
                    if !call.take(flag, value) {
                        return None;
                    }
                    break;
                }
            } else {
                call.words.push(t.to_owned());
            }
            j += 1;
        }
        Some(call)
    }

    /// Whether `url`'s host is this machine.
    fn local(url: &str) -> bool {
        let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
        let place = rest.split(['/', '?', '#']).next().unwrap_or("");
        let place = place.rsplit_once('@').map_or(place, |(_, place)| place);
        let host = match place.strip_prefix('[') {
            Some(v6) => v6.split(']').next().unwrap_or(""),
            None => place.split(':').next().unwrap_or(""),
        };
        let host = host.to_lowercase();
        matches!(host.as_str(), "localhost" | "127.0.0.1" | "::1" | "0.0.0.0")
            || host.ends_with(".localhost")
    }

    /// Whether a method writes.
    fn writing(method: &str) -> bool {
        !matches!(method.to_uppercase().as_str(), "GET" | "HEAD" | "OPTIONS")
    }

    /// Whether `v` (curl, wget, httpie's `http`/`https`, xh) with words `a` (`fa`: each known)
    /// writes (a method other than GET or HEAD, or a body) to anything but this machine, or
    /// to a place it can't tell.
    pub(super) fn writes(v: &str, a: &[String], fa: &[bool]) -> bool {
        let unknown = fa.iter().any(|fixed| !fixed);
        let httpie = !matches!(v, "curl" | "wget");
        let call = match v {
            "curl" => read(a, curl_short, curl_long),
            "wget" => read(a, wget_short, wget_long),
            _ => read(a, httpie_short, httpie_long),
        };
        let Some(mut call) = call else {
            return true;
        };
        if httpie {
            // [METHOD] URL [ITEM...]; `:port/path` is this machine.
            let mut words = call.words.iter();
            if let Some(word) = call.words.first()
                && word.chars().all(|c| c.is_ascii_uppercase())
            {
                call.method = Some(word.clone());
                words.next();
            }
            let url = words.next().map(|url| {
                if url.starts_with(':') {
                    format!("localhost{url}")
                } else {
                    url.clone()
                }
            });
            call.body |= words.any(|item| {
                item.contains(":=")
                    || item.contains('@')
                    || (item.contains('=') && !item.contains("=="))
            });
            call.urls.extend(url);
        } else {
            call.urls.append(&mut call.words);
        }
        let writes = call
            .method
            .as_deref()
            .map_or(call.body && !call.get, writing);
        writes && (unknown || call.urls.is_empty() || call.urls.iter().any(|url| !local(url)))
    }
}
/// [`API_ARGS`]'s pattern, for the reading of `gh api`'s words.
const API_ARGS_SOURCE: &str = r"^(-X|--method|-f|-F|--field|--raw-field|--input|-H|--header|-q|--jq|-t|--template|--cache|--hostname|-p|--preview)$";

/// The words of `a` from `from` that aren't flags (nor the value of a flag `with_arg` names);
/// everything after `--`.
fn positional(a: &[String], from: usize, with_arg: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut j = from;
    while j < a.len() {
        if a[j] == "--" {
            out.extend(a[j + 1..].iter().cloned());
            break;
        }
        if a[j].starts_with('-') && a[j] != "-" {
            if matches(with_arg, Some(&a[j])) && !a[j].contains('=') {
                j += 1;
            }
            j += 1;
            continue;
        }
        out.push(a[j].clone());
        j += 1;
    }
    out
}

/// A `gh api` call (its words after `api`) that writes: a method other than GET, or (gh's own
/// default) fields or an input, so a POST; `graphql` only for a mutation (or a query it can't
/// read: a file, an input).
fn api_writes(a: &[String], endpoint: Option<&str>) -> bool {
    let mut method: Option<String> = None;
    let mut body = false;
    let mut mutation = false;
    let mut j = 0;
    while j < a.len() {
        let t = a[j].as_str();
        if let Some(rest) = t.strip_prefix("-X").or_else(|| {
            t.strip_prefix("--method")
                .map(|r| r.strip_prefix('=').unwrap_or(r))
        }) {
            let value = if rest.is_empty() {
                j += 1;
                a.get(j).cloned().unwrap_or_default()
            } else {
                rest.to_owned()
            };
            method = Some(value.to_uppercase());
            j += 1;
            continue;
        }
        let field = ["-f", "-F", "--field", "--raw-field"]
            .iter()
            .find_map(|flag| {
                t.strip_prefix(flag).and_then(|rest| {
                    if rest.is_empty() {
                        Some(None)
                    } else {
                        rest.strip_prefix('=').map(|v| Some(v.to_owned()))
                    }
                })
            });
        if let Some(value) = field {
            body = true;
            let value = value.unwrap_or_else(|| {
                j += 1;
                a.get(j).cloned().unwrap_or_default()
            });
            if matches(r"\bmutation\b|^query=@", Some(&value.to_lowercase())) {
                mutation = true;
            }
            j += 1;
            continue;
        }
        if t == "--input" || t.starts_with("--input=") {
            body = true;
            mutation = true;
            if t == "--input" {
                j += 1;
            }
            j += 1;
            continue;
        }
        if is(&API_ARGS, t) {
            j += 1;
        }
        j += 1;
    }
    if endpoint == Some("graphql") {
        return mutation;
    }
    if let Some(method) = method {
        return !matches!(method.as_str(), "GET" | "HEAD");
    }
    body
}

/// The repositories a `gh` command names (`-R`, `--repo`, a github.com address, an api path
/// `repos/owner/name`); empty: none, so its folder's; `None`: one it can't tell.
fn gh_repos(a: &[String], endpoint: Option<&str>) -> Option<Vec<String>> {
    let mut out = Vec::new();
    let mut j = 1;
    while j < a.len() {
        let t = a[j].as_str();
        let given = if t == "-R" || t == "--repo" {
            j += 1;
            Some(a.get(j).cloned().unwrap_or_default())
        } else {
            t.strip_prefix("-R=")
                .or_else(|| t.strip_prefix("--repo="))
                .map(str::to_owned)
        };
        if let Some(given) = given {
            out.push(repo_name(&given)?);
        }
        j += 1;
    }
    if let Some(urls) = URLS.as_ref() {
        for t in a {
            for found in urls.find_iter(t) {
                if let Some(name) = repo_name(found.as_str()) {
                    out.push(name);
                }
            }
        }
    }
    if let Some(endpoint) = endpoint {
        pattern!(REPOS_PATH, r"^/?repos/([\w.-]+/[\w.-]+)");
        match REPOS_PATH.as_ref().and_then(|re| re.captures(endpoint)) {
            Some(caps) => out.push(caps[1].to_owned()),
            // A call with no repository (user, graphql): no always-OK repository.
            None if out.is_empty() => return None,
            None => {}
        }
    }
    Some(out)
}

fn make_step(
    what: &str,
    argv: &[String],
    dir: Option<String>,
    named: Option<Vec<String>>,
    gh: bool,
    remotes: &dyn Remotes,
) -> Step {
    let here = dir.as_deref().and_then(|d| remotes.remotes(d));
    let step = |repos| Step {
        what: what.to_owned(),
        argv: argv.to_vec(),
        dir,
        repos,
    };
    if gh && named.is_none() {
        return step(None);
    }
    if let Some(named) = named.filter(|n| !n.is_empty()) {
        return step(Some(named));
    }
    let mut urls = Vec::new();
    if let Some(re) = URLS.as_ref() {
        for t in argv {
            for found in re.find_iter(t) {
                if let Some(name) = repo_name(found.as_str()) {
                    urls.push(name);
                }
            }
        }
    }
    step(here.map(|here| {
        let mut all = here;
        for url in urls {
            if !all.contains(&url) {
                all.push(url);
            }
        }
        all.dedup();
        all
    }))
}

/// The outward steps of `command`, or none when the line can't be taken apart.
#[must_use]
pub fn steps_or_none(command: &str, cwd: Option<&str>, remotes: &dyn Remotes) -> Option<Vec<Step>> {
    steps(command, cwd, false, remotes).ok()
}

/// Whether `command` has an outward step, or one it can't be sure of ([`unsure`]; a line it
/// can't take apart is one), whoever's repository it is in.
#[must_use]
pub fn is_outward(command: &str, home: &str) -> bool {
    steps_or_none(command, None, &NoRemotes(home.to_owned())).is_none_or(|steps| !steps.is_empty())
}

/// Whether every outward step of `command` (run in `cwd`) is in a repository `always` lets
/// through. A line that can't be taken apart, or a step whose repositories can't be told,
/// never is.
#[must_use]
pub fn always_ok(
    command: &str,
    cwd: &str,
    remotes: &dyn Remotes,
    always: &dyn Fn(&str) -> bool,
) -> bool {
    steps_or_none(command, Some(cwd), remotes).is_some_and(|steps| {
        !steps.is_empty()
            && steps.iter().all(|step| {
                step.repos
                    .as_ref()
                    .is_some_and(|repos| !repos.is_empty() && repos.iter().all(|r| always(r)))
            })
    })
}

/// Every pattern here compiles.
#[must_use]
pub fn patterns_compile() -> bool {
    [
        &NAMES,
        &NOT,
        &ASKS,
        &SENTENCES,
        &SHELLS,
        &INTERPRETERS,
        &DEPLOYISH,
        &SCRIPT,
        &SOURCE_FILE,
        &CS_FILE,
        &RESTARTS,
        &URLS,
        &API_ARGS,
        &ASSIGN,
        &LEADS,
        &GIT_ENV,
        &GIT_LOCAL,
        &GIT_SAFE_CONFIG,
        &GH_KNOWN,
        &WORDY,
    ]
    .iter()
    .all(|re| re.is_some())
}
