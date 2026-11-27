//! A shell command line read as bash would run it, cut into its simple commands: their words
//! with the quotes taken off, the folder each runs in (`cd` and `pushd` followed, a subshell's
//! `cd` kept inside it), and the text a heredoc or a here-string feeds them.
//!
//! The outward rule ([`super::outward`]) reads verbs from these, never from a heredoc's body
//! or a quoted string: a script that only mentions `git push` in a string is not a push.
//!
//! It is not a whole shell: a `case` pattern, arithmetic (`(( ))`) and the like are an
//! [`ShellError`], and the outward rule then holds the line as one it can't read.

/// One simple command, as it runs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Run {
    /// Its words, quotes taken off.
    pub argv: Vec<String>,
    /// For each word: `false` when it holds an expansion that can't be known here (`$VAR`,
    /// `$(...)`), `true` when its text is what runs.
    pub fixed: Vec<bool>,
    /// What heredocs and here-strings feed it, in order.
    pub stdin: Vec<String>,
    /// The folder it runs in; none when it can't be told (a `cd` to a variable, `popd`, a
    /// command on another machine).
    pub dir: Option<String>,
}

/// A command line this reader can't take apart.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct ShellError(pub String);

impl ShellError {
    fn new(why: &str) -> Self {
        Self(why.to_owned())
    }
}

/// Where a list of commands runs: its folder, and whether it is on another machine.
#[derive(Clone, Debug)]
struct Ctx {
    dir: Option<String>,
    remote: bool,
}

/// A word: its text, and whether all of it is known.
struct Word {
    text: String,
    fixed: bool,
}

/// A heredoc waiting for the end of its line: its delimiter, whether tabs are stripped
/// (`<<-`), whether it is quoted (no expansion in its body), the command it feeds, where it was.
struct Pending {
    delim: String,
    strip: bool,
    quoted: bool,
    run: usize,
    ctx: Ctx,
}

/// The redirection operators of two characters or more, in the order they are tried.
const OPS: [&str; 10] = [
    "<<<", "<<-", "<<", "<>", ">>", ">&", "<&", ">|", "&>>", "&>",
];

/// The characters that end a word.
const META: &[char] = &[' ', '\t', '\n', ';', '&', '|', '(', ')', '<', '>'];

struct Parser<'h> {
    s: Vec<char>,
    i: usize,
    home: &'h str,
    /// The runs in the order they run, each with the id of the command it was (heredocs find
    /// theirs by it; runs from another text have none).
    runs: Vec<(Option<usize>, Run)>,
    pending: Vec<Pending>,
    next_id: usize,
}

impl<'h> Parser<'h> {
    fn new(text: &str, home: &'h str) -> Self {
        Self {
            s: text.chars().collect(),
            i: 0,
            home,
            runs: Vec::new(),
            pending: Vec::new(),
            next_id: 0,
        }
    }

    fn at(&self, j: usize) -> char {
        self.s.get(j).copied().unwrap_or('\0')
    }

    fn len(&self) -> usize {
        self.s.len()
    }

    fn take_id(&mut self) -> usize {
        self.next_id += 1;
        self.next_id
    }

    /// The commands up to `end` (the `)` of a subshell or `$(...)`), or the end of the text
    /// (`'\0'`).
    fn list(&mut self, ctx: &mut Ctx, end: char) -> Result<(), ShellError> {
        let mut words: Vec<Word> = Vec::new();
        let mut run = Run {
            dir: ctx.dir.clone(),
            ..Run::default()
        };
        let mut id = self.take_id();
        loop {
            while self.i < self.len()
                && (self.s[self.i] == ' '
                    || self.s[self.i] == '\t'
                    || (self.s[self.i] == '\\' && self.at(self.i + 1) == '\n'))
            {
                self.i += if self.s[self.i] == '\\' { 2 } else { 1 };
            }
            if self.i >= self.len() {
                if end != '\0' {
                    return Err(ShellError::new("unclosed ("));
                }
                self.finish(&mut words, &mut run, &mut id, ctx);
                return Ok(());
            }
            let c = self.s[self.i];
            if end != '\0' && c == end {
                self.finish(&mut words, &mut run, &mut id, ctx);
                self.i += 1;
                return Ok(());
            }
            if c == '\n' {
                self.finish(&mut words, &mut run, &mut id, ctx);
                self.i += 1;
                self.heredocs()?;
                continue;
            }
            if c == '#' {
                while self.i < self.len() && self.s[self.i] != '\n' {
                    self.i += 1;
                }
                continue;
            }
            if c == ')' {
                return Err(ShellError::new("unbalanced )"));
            }
            if let Some((length, op)) = self.redirection() {
                self.i += length;
                self.blanks();
                match op {
                    "<<" | "<<-" => {
                        let start = self.i;
                        let word = self.word_at(ctx)?;
                        let quoted = self.s[start..self.i]
                            .iter()
                            .any(|c| matches!(c, '\'' | '"' | '\\'));
                        self.pending.push(Pending {
                            delim: word.text,
                            strip: op == "<<-",
                            quoted,
                            run: id,
                            ctx: ctx.clone(),
                        });
                    }
                    "<<<" => {
                        let word = self.word_at(ctx)?;
                        run.stdin.push(word.text);
                    }
                    // A file to read or write: not a command.
                    _ => {
                        self.word_at(ctx)?;
                    }
                }
                continue;
            }
            if matches!(c, ';' | '&' | '|') {
                self.finish(&mut words, &mut run, &mut id, ctx);
                while self.i < self.len() && matches!(self.s[self.i], ';' | '&' | '|') {
                    self.i += 1;
                }
                continue;
            }
            if c == '(' {
                self.i += 1;
                if !words.is_empty() {
                    // A function: name() { ... }
                    self.blanks();
                    if self.at(self.i) != ')' {
                        return Err(ShellError::new("( inside a command"));
                    }
                    self.i += 1;
                    self.finish(&mut words, &mut run, &mut id, ctx);
                    continue;
                }
                if self.at(self.i) == '(' {
                    return Err(ShellError::new("arithmetic"));
                }
                // A subshell: its cd stays in it.
                let mut inner = ctx.clone();
                self.list(&mut inner, ')')?;
                continue;
            }
            let word = self.word_at(ctx)?;
            words.push(word);
        }
    }

    /// Ends the command being read: kept when it has words, its `cd` followed.
    fn finish(&mut self, words: &mut Vec<Word>, run: &mut Run, id: &mut usize, ctx: &mut Ctx) {
        if !words.is_empty() {
            let mut done = std::mem::take(run);
            done.argv = words.iter().map(|w| w.text.clone()).collect();
            done.fixed = words.iter().map(|w| w.fixed).collect();
            done.dir.clone_from(&ctx.dir);
            cd_of(&done, ctx, self.home);
            self.runs.push((Some(*id), done));
        }
        words.clear();
        *run = Run {
            dir: ctx.dir.clone(),
            ..Run::default()
        };
        *id = self.take_id();
    }

    /// The redirection operator at the cursor, with the file descriptor before it: its
    /// length and the operator. As bash reads `2>`, `<<`, `&>>`, `>|`...; `<(` and `>(` are
    /// process substitutions, read as words.
    fn redirection(&self) -> Option<(usize, &'static str)> {
        // The next six characters at most.
        let window: Vec<char> = self.s[self.i..self.len().min(self.i + 6)].to_vec();
        let digits = window.iter().take_while(|c| c.is_ascii_digit()).count();
        let rest = &window[digits..];
        for op in OPS {
            let chars: Vec<char> = op.chars().collect();
            if rest.starts_with(&chars) {
                return Some((digits + chars.len(), op));
            }
        }
        match rest.first() {
            Some('<') if rest.get(1) != Some(&'(') => Some((digits + 1, "<")),
            Some('>') if rest.get(1) != Some(&'(') => Some((digits + 1, ">")),
            _ => None,
        }
    }

    fn blanks(&mut self) {
        while self.i < self.len() && matches!(self.s[self.i], ' ' | '\t') {
            self.i += 1;
        }
    }

    /// The bodies of the heredocs started on the line just read: fed to their command, never
    /// run themselves (unless that command is a shell). An unquoted one's `$(...)` still runs.
    fn heredocs(&mut self) -> Result<(), ShellError> {
        for heredoc in std::mem::take(&mut self.pending) {
            let mut lines = Vec::new();
            while self.i < self.len() {
                let nl = (self.i..self.len())
                    .find(|&j| self.s[j] == '\n')
                    .unwrap_or(self.len());
                let line: String = self.s[self.i..nl].iter().collect();
                self.i = (nl + 1).min(self.len());
                let line = if heredoc.strip {
                    line.trim_start_matches('\t').to_owned()
                } else {
                    line
                };
                if line == heredoc.delim {
                    break;
                }
                lines.push(line);
            }
            let body = lines.join("\n");
            if let Some((_, run)) = self
                .runs
                .iter_mut()
                .find(|(id, _)| *id == Some(heredoc.run))
            {
                run.stdin.push(body.clone());
            }
            if !heredoc.quoted {
                let mut inner = Parser::new(&body, self.home);
                inner.quoted(&heredoc.ctx, '\0')?;
                self.runs
                    .extend(inner.runs.into_iter().map(|(_, run)| (None, run)));
            }
        }
        Ok(())
    }

    /// One word: its text with quotes off, and whether it is all known.
    fn word_at(&mut self, ctx: &Ctx) -> Result<Word, ShellError> {
        let mut text = String::new();
        let mut fixed = true;
        while self.i < self.len() {
            let c = self.s[self.i];
            if matches!(c, '<' | '>') && self.at(self.i + 1) == '(' {
                // <(...) and >(...): they run.
                self.i += 2;
                let mut inner = ctx.clone();
                self.list(&mut inner, ')')?;
                text.push_str("<(…)");
                fixed = false;
                continue;
            }
            if META.contains(&c) {
                break;
            }
            if c == '\\' {
                if self.at(self.i + 1) != '\n' && self.i + 1 < self.len() {
                    text.push(self.s[self.i + 1]);
                }
                self.i += 2;
                continue;
            }
            if c == '\'' {
                let Some(e) = (self.i + 1..self.len()).find(|&j| self.s[j] == '\'') else {
                    return Err(ShellError::new("unclosed '"));
                };
                text.extend(&self.s[self.i + 1..e]);
                self.i = e + 1;
                continue;
            }
            if c == '$' && self.at(self.i + 1) == '\'' {
                self.i += 2;
                text.push_str(&self.ansi()?);
                continue;
            }
            if c == '"' || (c == '$' && self.at(self.i + 1) == '"') {
                self.i += if c == '$' { 2 } else { 1 };
                let word = self.quoted(ctx, '"')?;
                text.push_str(&word.text);
                fixed &= word.fixed;
                continue;
            }
            if matches!(c, '$' | '`') {
                let word = self.dollar(ctx)?;
                text.push_str(&word.text);
                fixed &= word.fixed;
                continue;
            }
            text.push(c);
            self.i += 1;
        }
        Ok(Word { text, fixed })
    }

    /// A double-quoted string from just past its quote (`term` `"`), or a heredoc's body to
    /// its end (`term` `'\0'`).
    fn quoted(&mut self, ctx: &Ctx, term: char) -> Result<Word, ShellError> {
        let mut text = String::new();
        let mut fixed = true;
        loop {
            if self.i >= self.len() {
                if term != '\0' {
                    return Err(ShellError::new("unclosed \""));
                }
                return Ok(Word { text, fixed });
            }
            let c = self.s[self.i];
            if term != '\0' && c == term {
                self.i += 1;
                return Ok(Word { text, fixed });
            }
            let next = self.at(self.i + 1);
            if c == '\\'
                && term != '\0'
                && self.i + 1 < self.len()
                && matches!(next, '$' | '`' | '"' | '\\' | '\n')
            {
                if next != '\n' {
                    text.push(next);
                }
                self.i += 2;
                continue;
            }
            if c == '\\'
                && term == '\0'
                && self.i + 1 < self.len()
                && matches!(next, '$' | '`' | '\\')
            {
                text.push(next);
                self.i += 2;
                continue;
            }
            if matches!(c, '$' | '`') {
                let word = self.dollar(ctx)?;
                text.push_str(&word.text);
                fixed &= word.fixed;
                continue;
            }
            text.push(c);
            self.i += 1;
        }
    }

    /// `$(...)`, `` `...` ``, `$((...))`, `${...}` or `$NAME` at the cursor. `$HOME` is known;
    /// the rest aren't.
    #[expect(
        clippy::too_many_lines,
        reason = "one case per form of expansion, as bash reads them"
    )]
    fn dollar(&mut self, ctx: &Ctx) -> Result<Word, ShellError> {
        if self.s[self.i] == '`' {
            let mut j = self.i + 1;
            let mut body = String::new();
            while j < self.len() && self.s[j] != '`' {
                if self.s[j] == '\\'
                    && j + 1 < self.len()
                    && matches!(self.s[j + 1], '`' | '$' | '\\')
                {
                    j += 1;
                }
                body.push(self.s[j]);
                j += 1;
            }
            if j >= self.len() {
                return Err(ShellError::new("unclosed `"));
            }
            self.i = j + 1;
            let mut inner = Parser::new(&body, self.home);
            let mut inner_ctx = ctx.clone();
            inner.list(&mut inner_ctx, '\0')?;
            self.runs
                .extend(inner.runs.into_iter().map(|(_, run)| (None, run)));
            return Ok(Word {
                text: "`…`".to_owned(),
                fixed: false,
            });
        }
        let n = self.at(self.i + 1);
        if n == '(' && self.at(self.i + 2) == '(' {
            let mut depth = 0_i32;
            let mut j = self.i + 1;
            while j < self.len() {
                if self.s[j] == '(' {
                    depth += 1;
                } else if self.s[j] == ')' {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                j += 1;
            }
            if j >= self.len() {
                return Err(ShellError::new("unclosed $(("));
            }
            self.i = j + 1;
            return Ok(Word {
                text: "$((…))".to_owned(),
                fixed: false,
            });
        }
        if n == '(' {
            self.i += 2;
            let mut inner = ctx.clone();
            self.list(&mut inner, ')')?;
            return Ok(Word {
                text: "$(…)".to_owned(),
                fixed: false,
            });
        }
        if n == '{' {
            let Some(e) = (self.i + 2..self.len()).find(|&j| self.s[j] == '}') else {
                return Err(ShellError::new("unclosed ${"));
            };
            let name: String = self.s[self.i + 2..e].iter().collect();
            self.i = e + 1;
            return Ok(if name == "HOME" {
                Word {
                    text: self.home.to_owned(),
                    fixed: true,
                }
            } else {
                Word {
                    text: format!("${{{name}}}"),
                    fixed: false,
                }
            });
        }
        let rest = &self.s[self.i + 1..];
        let length = match rest.first() {
            Some(c) if c.is_ascii_alphabetic() || *c == '_' => rest
                .iter()
                .take_while(|c| c.is_alphanumeric() || **c == '_')
                .count(),
            Some(c) if c.is_ascii_digit() || "@*#?$!-".contains(*c) => 1,
            _ => {
                self.i += 1;
                return Ok(Word {
                    text: "$".to_owned(),
                    fixed: true,
                });
            }
        };
        let name: String = rest[..length].iter().collect();
        self.i += 1 + length;
        Ok(if name == "HOME" {
            Word {
                text: self.home.to_owned(),
                fixed: true,
            }
        } else {
            Word {
                text: format!("${name}"),
                fixed: false,
            }
        })
    }

    /// `$'...'` from just past its quote.
    fn ansi(&mut self) -> Result<String, ShellError> {
        let mut text = String::new();
        while self.i < self.len() && self.s[self.i] != '\'' {
            if self.s[self.i] == '\\' && self.i + 1 < self.len() {
                self.i += 1;
                match self.s[self.i] {
                    'n' => text.push('\n'),
                    't' => text.push('\t'),
                    'r' => text.push('\r'),
                    '\\' => text.push('\\'),
                    '\'' => text.push('\''),
                    '"' => text.push('"'),
                    'e' => text.push('\u{1b}'),
                    'a' => text.push('\u{7}'),
                    'b' => text.push('\u{8}'),
                    other => {
                        text.push('\\');
                        text.push(other);
                    }
                }
            } else {
                text.push(self.s[self.i]);
            }
            self.i += 1;
        }
        if self.i >= self.len() {
            return Err(ShellError::new("unclosed $'"));
        }
        self.i += 1;
        Ok(text)
    }
}

/// A path with `.` and `..` resolved without looking at the disk, as an absolute path.
fn normalize(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            other => parts.push(other),
        }
    }
    format!("/{}", parts.join("/"))
}

/// A folder as the shell would take it: `~` is `home`, a relative one is under `dir`; none
/// when it can't be told.
#[must_use]
pub fn place(path: &str, dir: Option<&str>, home: &str) -> Option<String> {
    if path == "~" || path.starts_with("~/") {
        return Some(format!("{home}{}", &path[1..]));
    }
    if path.starts_with('/') {
        return Some(normalize(path));
    }
    dir.map(|dir| normalize(&format!("{dir}/{path}")))
}

/// Whether `word` is one of the words that lead a command without being it.
fn is_lead(word: &str) -> bool {
    matches!(
        word,
        "then" | "do" | "else" | "elif" | "if" | "while" | "until" | "!" | "{" | "time"
    )
}

/// `cd` and `pushd` move the rest of the list; `popd` and `cd -` go where it can't tell.
fn cd_of(run: &Run, ctx: &mut Ctx, home: &str) {
    let mut k = 0;
    while k < run.argv.len() && is_lead(&run.argv[k]) {
        k += 1;
    }
    let verb = run.argv.get(k).map(String::as_str);
    if verb == Some("popd") {
        ctx.dir = None;
        return;
    }
    if !matches!(verb, Some("cd" | "pushd")) {
        return;
    }
    k += 1;
    while k < run.argv.len() && is_cd_flag(&run.argv[k]) {
        k += 1;
    }
    if run.argv.get(k).map(String::as_str) == Some("--") {
        k += 1;
    }
    let to = run.argv.get(k).map(String::as_str);
    if ctx.remote || (to.is_some() && !run.fixed[k]) || to == Some("-") {
        ctx.dir = None;
    } else {
        ctx.dir = match to {
            None => Some(home.to_owned()),
            Some(to) => place(to, ctx.dir.as_deref(), home),
        };
    }
}

/// `-L`, `-P`, `-e`, `-@` and their runs: `cd`'s flags.
fn is_cd_flag(word: &str) -> bool {
    word.len() > 1
        && word.starts_with('-')
        && word[1..]
            .chars()
            .all(|c| matches!(c, 'L' | 'P' | 'e' | '@'))
}

/// The simple commands of `command`, in the order they run, started in `cwd`; `home` is the
/// user's home (`~`, `$HOME`, a bare `cd`). `remote`: it runs on another machine (through
/// `ssh`), so no folder of it can be told here.
///
/// # Errors
///
/// [`ShellError`] on what this reader can't take apart.
pub fn parse(
    command: &str,
    cwd: Option<&str>,
    remote: bool,
    home: &str,
) -> Result<Vec<Run>, ShellError> {
    let mut parser = Parser::new(command, home);
    let mut ctx = Ctx {
        dir: if remote { None } else { cwd.map(str::to_owned) },
        remote,
    };
    parser.list(&mut ctx, '\0')?;
    Ok(parser.runs.into_iter().map(|(_, run)| run).collect())
}
