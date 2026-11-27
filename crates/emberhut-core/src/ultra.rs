//! Ultra runs: one task's run splits a big job across many sub-agents working in parallel, a
//! verify agent per part. The task's session is the lead: it posts a plan first (parts with
//! sizes, the sub-agents, the tokens, the accounts), a big plan waits for the owner's OK, and a
//! gate before each new sub-agent holds the plan, the parallel cap and the spending cap.
//!
//! The rules without I/O: the sizes' estimate, the caps, a sub-agent's kind and part from its
//! name, the parts done, a token count in words, a line for a phone, and the lead's brief.

use std::sync::LazyLock;

use regex::Regex;

/// A part's rough size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Size {
    /// About 80k tokens.
    S,
    /// About 200k tokens.
    M,
    /// About 500k tokens.
    L,
}

impl Size {
    /// The size `text` names (`S`, `M`, `L`).
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "S" => Some(Self::S),
            "M" => Some(Self::M),
            "L" => Some(Self::L),
            _ => None,
        }
    }

    /// Its letter.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::S => "S",
            Self::M => "M",
            Self::L => "L",
        }
    }

    /// The tokens a part of this size takes, roughly (its sub-agents' counts).
    #[must_use]
    pub const fn tokens(self) -> f64 {
        match self {
            Self::S => 80_000.0,
            Self::M => 200_000.0,
            Self::L => 500_000.0,
        }
    }
}

/// One part of a plan.
#[derive(Clone, Debug, PartialEq)]
pub struct Part {
    /// Its number.
    pub n: f64,
    /// A short name.
    pub name: String,
    /// Its size.
    pub size: Size,
    /// `false`: it needs no verify agent.
    pub verify: bool,
}

/// The caps an ultra run is held to.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Limits {
    /// Sub-agents at a time.
    pub parallel: u32,
    /// The spending cap: sub-agents' tokens for a run.
    pub budget: f64,
    /// A plan over this many tokens waits for the owner's OK.
    pub ask_tokens: f64,
    /// A plan of more sub-agents than this waits for the owner's OK.
    pub ask_agents: u32,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            parallel: 4,
            budget: 3_000_000.0,
            ask_tokens: 1_500_000.0,
            ask_agents: 12,
        }
    }
}

/// What a plan comes to: sub-agents (the parts and their verify agents) and tokens.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Estimate {
    /// Sub-agents.
    pub agents: u32,
    /// Tokens.
    pub tokens: f64,
}

/// The sizes' estimate for `parts`: a verified part counts one and a half.
#[must_use]
pub fn estimate(parts: &[Part]) -> Estimate {
    let verify = parts.iter().filter(|p| p.verify).count();
    Estimate {
        agents: u32::try_from(parts.len() + verify).unwrap_or(u32::MAX),
        tokens: parts
            .iter()
            .map(|p| p.size.tokens() * if p.verify { 1.5 } else { 1.0 })
            .sum(),
    }
}

/// Whether a plan of `agents` sub-agents and `tokens` is big: over the caps that ask.
#[must_use]
pub fn big(agents: f64, tokens: f64, limits: &Limits) -> bool {
    tokens > limits.ask_tokens || agents > f64::from(limits.ask_agents)
}

/// A token count in words: `1.5M`, `200k`, `800`.
#[must_use]
pub fn tokens(n: f64) -> String {
    if n >= 1_000_000.0 {
        let fixed = to_fixed_1(n / 1_000_000.0);
        let fixed = fixed.strip_suffix(".0").unwrap_or(&fixed).to_owned();
        format!("{fixed}M")
    } else if n >= 1000.0 {
        format!("{}k", js_number((n / 1000.0).round()))
    } else {
        js_number(n)
    }
}

/// A number as JavaScript writes it: whole numbers without a point.
fn js_number(n: f64) -> String {
    if n.fract() == 0.0 && n.abs() < 1e21 {
        format!("{n:.0}")
    } else {
        format!("{n}")
    }
}

/// `x.toFixed(1)`: from the number's exact value, a tie going up (1.25 is "1.3").
fn to_fixed_1(x: f64) -> String {
    let exact = format!("{x:.40}");
    let Some((whole, frac)) = exact.split_once('.') else {
        return exact;
    };
    let mut digits = frac.chars();
    let tenth = digits.next().and_then(|c| c.to_digit(10)).unwrap_or(0);
    let rest: String = digits.collect();
    let up = rest.starts_with(['5', '6', '7', '8', '9']);
    let whole: u64 = whole.parse().unwrap_or(0);
    let mut tenths = whole * 10 + u64::from(tenth);
    if up {
        tenths += 1;
    }
    format!("{}.{}", tenths / 10, tenths % 10)
}

/// A sub-agent's kind, from its name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentKind {
    /// A part's work.
    Part,
    /// A part's check.
    Verify,
    /// A part run as its own session on another account.
    Lane,
    /// Anything else.
    Other,
}

impl AgentKind {
    /// Its word.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Part => "part",
            Self::Verify => "verify",
            Self::Lane => "lane",
            Self::Other => "other",
        }
    }

    /// The kind `text` names.
    #[must_use]
    pub fn parse(text: &str) -> Self {
        match text {
            "part" => Self::Part,
            "verify" => Self::Verify,
            "lane" => Self::Lane,
            _ => Self::Other,
        }
    }
}

/// A sub-agent's kind and part from its name: "Part 3: …", "Verify 3: …", a workflow's
/// "part:3" or "verify:3", "Lane 2: …".
#[must_use]
pub fn kind_of(name: &str) -> (AgentKind, Option<f64>) {
    static KIND: LazyLock<Option<Regex>> = LazyLock::new(|| {
        Regex::new(r"(?i)^\s*(part|verify|check|review|lane)\b[\s:#-]*(\d+)?").ok()
    });
    let Some(caps) = KIND.as_ref().and_then(|re| re.captures(name)) else {
        return (AgentKind::Other, None);
    };
    let kind = match caps[1].to_lowercase().as_str() {
        "part" => AgentKind::Part,
        "lane" => AgentKind::Lane,
        _ => AgentKind::Verify,
    };
    (kind, caps.get(2).and_then(|n| n.as_str().parse().ok()))
}

/// A line for a phone: plain words, no markdown, the CLI's own notice before a sub-agent's
/// words left out, at most `n` characters.
#[must_use]
pub fn clip(text: &str, n: usize) -> String {
    static HARNESS: LazyLock<Option<Regex>> =
        LazyLock::new(|| Regex::new(r"(?i)^\s*\[harness:[^\]]*\]\s*").ok());
    static MARKS: LazyLock<Option<Regex>> = LazyLock::new(|| Regex::new(r"\*\*|__|`+").ok());
    static HEADINGS: LazyLock<Option<Regex>> =
        LazyLock::new(|| Regex::new(r"(?m)^\s*#{1,6}\s+").ok());
    static BULLETS: LazyLock<Option<Regex>> =
        LazyLock::new(|| Regex::new(r"(?m)^\s*[-*•]\s+").ok());
    let mut t = text.to_owned();
    for re in [&HARNESS, &MARKS, &HEADINGS, &BULLETS] {
        if let Some(re) = re.as_ref() {
            t = re.replace_all(&t, "").into_owned();
        }
    }
    t.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(n)
        .collect()
}

/// A sub-agent as the parts done are counted: its kind, its part and whether it is done.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Seen {
    /// Its kind.
    pub kind: AgentKind,
    /// Its part.
    pub part: Option<f64>,
    /// It finished well.
    pub done: bool,
}

/// Parts done out of all: a part whose part agent (or lane) is done, and its check when the
/// plan has one (a verify agent for that part, or one for all of them). Without a plan, the
/// part agents seen; with no parts at all, the agents done out of all.
#[must_use]
pub fn done(plan: Option<&[Part]>, agents: &[Seen]) -> (usize, usize) {
    let parts: Vec<(Option<f64>, bool)> = if let Some(parts) = plan {
        parts.iter().map(|p| (Some(p.n), p.verify)).collect()
    } else {
        let mut seen: Vec<(Option<f64>, bool)> = Vec::new();
        for a in agents {
            if a.kind == AgentKind::Part
                && a.part.is_some_and(|n| n != 0.0)
                && !seen.iter().any(|(n, _)| *n == a.part)
            {
                seen.push((a.part, false));
            }
        }
        seen
    };
    if parts.is_empty() {
        return (agents.iter().filter(|a| a.done).count(), agents.len());
    }
    let ok = |kinds: &[AgentKind], n: Option<f64>| {
        agents
            .iter()
            .any(|a| kinds.contains(&a.kind) && a.part == n && a.done)
    };
    let finished = parts
        .iter()
        .filter(|(n, verify)| {
            ok(&[AgentKind::Part, AgentKind::Lane], *n)
                && (!*verify || ok(&[AgentKind::Verify], *n) || ok(&[AgentKind::Verify], None))
        })
        .count();
    (finished, parts.len())
}

/// A sub-agent's state from the CLI's word for it.
#[must_use]
pub fn state_of(word: &str) -> Option<&'static str> {
    Some(match word {
        "completed" | "done" => "done",
        "failed" | "error" => "failed",
        "killed" | "stopped" | "cancelled" => "stopped",
        "running" => "running",
        "queued" | "pending" => "queued",
        _ => return None,
    })
}

/// Whether a sub-agent in `state` still runs.
#[must_use]
pub fn running(state: &str) -> bool {
    matches!(state, "running" | "queued")
}

/// The lead's part of a worker's brief: how an ultra run goes, with the run's caps.
#[must_use]
pub fn lead_brief(limits: &Limits, parallel: u32, cap: f64) -> String {
    format!(
        "\n\nULTRA MODE: you are the lead of an ultra run. Split this job into parts and do them in parallel with sub-agents, each checked by its own verify agent.\n\
1. Plan first. Look at the job just enough to split it, then call ultra_plan with the parts (n, a short name, size S/M/L, verify true unless it really needs no check), and the accounts to use. The plan and its estimate go on the task. If it's big (over {} tokens or {} sub-agents) or over the cap, the owner is asked: stop then, without task_done; you're resumed with their answer.\n\
2. Then run the parts with the Agent tool, at most {parallel} at a time (the server's gate refuses more; start the next when one finishes). Name each sub-agent by its part: description \"Part <n>: <name>\" for the work and \"Verify <n>: <name>\" for its check (a workflow: labels part:<n> and verify:<n>). Give each one a full brief: its part, what done looks like, the files, and the rules below.\n\
3. A verify agent checks its part's result against the brief (reads the files, runs the tests) and says pass or what's wrong; a part that fails is fixed and checked again, once.\n\
4. Spending cap: {} tokens of sub-agents for this run. Past it no new sub-agent starts and the owner is asked; then let the running ones finish, note what's done, and stop.\n\
5. Accounts: your sub-agents run on your own account. When the plan needs more room than it has, start some parts as their own sessions on another account with room (ultra_lane, one per part, then ultra_wait for them); they count as sub-agents.\n\
6. Rules for you and every sub-agent (put them in each brief): no push, merge, PR, release, deploy, message, post, payment or deleting anything without the owner's OK; prepare it and report it instead. Sub-agents don't ask the owner; the lead does, once.\n\
7. Report as one run: your summary says what each part found or changed and what its check said; finish with task_done, or task_ask when the owner must decide.",
        tokens(limits.ask_tokens),
        limits.ask_agents,
        tokens(cap),
    )
}

/// Every pattern here compiles.
#[must_use]
pub fn patterns_compile() -> bool {
    kind_of("Part 1: x").1.is_some() && clip("**a**", 10) == "a"
}
