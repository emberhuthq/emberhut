//! Areas: the parts of a project that run for a while on their own (project > area > tasks >
//! subtasks, at any depth). A tree's state rolls up from its leaves: needs you (anything under
//! it waits on the person), done (every leaf done), working (something under way or partly
//! done), not started. Findings hang off their scans and are counted apart.

use std::collections::{BTreeMap, BTreeSet};

use jiff::Timestamp;
use jiff::civil::Date;

use super::board::{Health, Project, ProjectStatus};

/// Writes a line of text to a `String`.
macro_rules! put {
    ($out:expr, $($arg:tt)*) => {{
        $out.push_str(&format!($($arg)*));
        $out.push('\n');
    }};
}
use super::{AreaId, Landed, Owner, ProjectId, Status, Task, TaskId};

/// Where an area is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AreaStatus {
    /// Being worked on.
    Active,
    /// On hold.
    Paused,
    /// Finished: shown folded.
    Done,
}

impl AreaStatus {
    /// Every status.
    pub const ALL: [Self; 3] = [Self::Active, Self::Paused, Self::Done];

    /// Its word.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Paused => "paused",
            Self::Done => "done",
        }
    }

    /// The status `word` names, if any.
    #[must_use]
    pub fn parse(word: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|status| status.as_str() == word)
    }

    const fn order(self) -> u8 {
        match self {
            Self::Active => 0,
            Self::Paused => 1,
            Self::Done => 2,
        }
    }
}

/// An area of a project.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Area {
    /// Its id.
    pub id: AreaId,
    /// Its project.
    pub pid: ProjectId,
    /// Its name, unique in the project.
    pub name: String,
    /// Where it is.
    pub status: AreaStatus,
    /// What done looks like.
    pub goal: Option<String>,
    /// A short line on what it is.
    pub about: Option<String>,
    /// When it was made.
    pub created: Timestamp,
    /// When it last changed.
    pub updated: Timestamp,
}

/// Areas in list order: active, then paused, then done; by name.
pub fn sort_areas(areas: &mut [Area]) {
    areas.sort_by(|a, b| {
        a.status
            .order()
            .cmp(&b.status.order())
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
}

/// A node's state, rolled up from its leaves.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum NodeState {
    /// Every leaf done.
    Done,
    /// Something under way.
    Working,
    /// Something waits on the person.
    NeedsYou,
    /// Nothing started.
    NotStarted,
}

impl NodeState {
    /// Its word.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Done => "done",
            Self::Working => "working",
            Self::NeedsYou => "needs_you",
            Self::NotStarted => "not_started",
        }
    }

    /// In words.
    #[must_use]
    pub const fn words(self) -> &'static str {
        match self {
            Self::Done => "done",
            Self::Working => "working",
            Self::NeedsYou => "needs you",
            Self::NotStarted => "not started",
        }
    }

    const fn mark(self) -> &'static str {
        match self {
            Self::Done => "✓",
            Self::Working => "~",
            Self::NeedsYou => "!",
            Self::NotStarted => "○",
        }
    }

    /// A task's own state, from its status.
    #[must_use]
    pub const fn own(status: Status) -> Self {
        match status {
            Status::Done => Self::Done,
            Status::Waiting => Self::NeedsYou,
            Status::Working => Self::Working,
            Status::Todo | Status::Canceled => Self::NotStarted,
        }
    }

    /// A state the filters take: `waiting` and `todo` read as needs you and not started.
    #[must_use]
    pub fn parse_filter(word: &str) -> Option<Self> {
        match word.trim().to_lowercase().replace(' ', "_").as_str() {
            "waiting" | "needs_you" => Some(Self::NeedsYou),
            "todo" | "not_started" => Some(Self::NotStarted),
            "working" => Some(Self::Working),
            "done" => Some(Self::Done),
            _ => None,
        }
    }
}

/// A node's state from its own and its leaves': needs you over everything, done when every leaf
/// is, working when anything is under way (or some done and some not), else not started.
#[must_use]
pub fn roll(own: NodeState, leaves: &[NodeState]) -> NodeState {
    if leaves.is_empty() {
        return own;
    }
    if own == NodeState::NeedsYou || leaves.contains(&NodeState::NeedsYou) {
        return NodeState::NeedsYou;
    }
    if leaves.iter().all(|leaf| *leaf == NodeState::Done) {
        return NodeState::Done;
    }
    if own == NodeState::Working
        || leaves.contains(&NodeState::Working)
        || leaves.contains(&NodeState::Done)
    {
        return NodeState::Working;
    }
    NodeState::NotStarted
}

/// A scan's findings, counted.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FindingCount {
    /// All of them.
    pub total: u32,
    /// Not landed, partly landed or unchecked.
    pub open: u32,
}

/// A task in a tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Node {
    /// The task.
    pub id: TaskId,
    /// Its title.
    pub title: String,
    /// Whose it is.
    pub owner: Owner,
    /// Its own status.
    pub status: Status,
    /// Rolled up from its leaves.
    pub state: NodeState,
    /// What the person should do, while it waits on them.
    pub action: Option<String>,
    /// When it is due, while open.
    pub due: Option<Date>,
    /// When it was done.
    pub done_at: Option<Timestamp>,
    /// A scan's findings.
    pub findings: Option<FindingCount>,
    /// Its subtasks.
    pub kids: Vec<Node>,
}

fn leaves_of(node: &Node, out: &mut Vec<NodeState>) {
    if node.kids.is_empty() {
        out.push(node.state);
    } else {
        for kid in &node.kids {
            leaves_of(kid, out);
        }
    }
}

fn walk<'a>(nodes: &'a [Node], out: &mut Vec<&'a Node>) {
    for node in nodes {
        out.push(node);
        walk(&node.kids, out);
    }
}

/// Tasks (no findings, no canceled ones) as trees under the roots, with each node's rolled-up
/// state; and every leaf's state.
#[must_use]
pub fn build(
    rows: &[&Task],
    is_root: impl Fn(&Task) -> bool,
    scans: &BTreeMap<TaskId, FindingCount>,
) -> (Vec<Node>, Vec<NodeState>) {
    let mut kids: BTreeMap<TaskId, Vec<&Task>> = BTreeMap::new();
    for row in rows {
        if let Some(parent) = row.parent {
            kids.entry(parent).or_default().push(row);
        }
    }
    for list in kids.values_mut() {
        list.sort_by_key(|t| t.id);
    }
    let mut seen = BTreeSet::new();
    let mut leaves = Vec::new();
    let mut roots: Vec<&Task> = rows.iter().copied().filter(|t| is_root(t)).collect();
    roots.sort_by_key(|t| t.id);
    let nodes = roots
        .into_iter()
        .map(|t| node(t, 0, &kids, scans, &mut seen, &mut leaves))
        .collect();
    (nodes, leaves)
}

fn node(
    t: &Task,
    depth: usize,
    kids: &BTreeMap<TaskId, Vec<&Task>>,
    scans: &BTreeMap<TaskId, FindingCount>,
    seen: &mut BTreeSet<TaskId>,
    leaves: &mut Vec<NodeState>,
) -> Node {
    seen.insert(t.id);
    let mut under = Vec::new();
    if depth < 50
        && let Some(list) = kids.get(&t.id)
    {
        for kid in list {
            if !seen.contains(&kid.id) {
                under.push(node(kid, depth + 1, kids, scans, seen, leaves));
            }
        }
    }
    let own = NodeState::own(t.status);
    let state = if under.is_empty() {
        leaves.push(own);
        own
    } else {
        let mut mine = Vec::new();
        for kid in &under {
            leaves_of(kid, &mut mine);
        }
        roll(own, &mine)
    };
    Node {
        id: t.id,
        title: t.title.clone(),
        owner: t.owner,
        status: t.status,
        state,
        action: (t.status == Status::Waiting)
            .then(|| t.action.clone())
            .flatten(),
        due: if t.status == Status::Done {
            None
        } else {
            t.due
        },
        done_at: if t.status == Status::Done {
            t.done_at
        } else {
            None
        },
        findings: scans.get(&t.id).copied(),
        kids: under,
    }
}

/// An area's leaves by state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    /// Done.
    pub done: u32,
    /// Under way.
    pub working: u32,
    /// Waiting on the person.
    pub needs_you: u32,
    /// Not started.
    pub not_started: u32,
}

impl Counts {
    /// `leaves`, counted.
    #[must_use]
    pub fn of(leaves: &[NodeState]) -> Self {
        let n = |state| u32::try_from(leaves.iter().filter(|l| **l == state).count()).unwrap_or(0);
        Self {
            done: n(NodeState::Done),
            working: n(NodeState::Working),
            needs_you: n(NodeState::NeedsYou),
            not_started: n(NodeState::NotStarted),
        }
    }

    /// Every leaf.
    #[must_use]
    pub const fn total(self) -> u32 {
        self.done + self.working + self.needs_you + self.not_started
    }
}

/// An open finding as its area lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FindingItem {
    /// The finding.
    pub id: TaskId,
    /// Its title.
    pub title: String,
    /// Its state.
    pub landed: Landed,
    /// The scan it came from.
    pub scan: TaskId,
    /// The scan's title.
    pub scan_title: String,
    /// The task that fixes it.
    pub fixed_by: Option<TaskId>,
    /// That task's title.
    pub fixed_by_title: Option<String>,
    /// That task's status.
    pub fixed_by_status: Option<Status>,
}

/// An area's findings by state, the open ones listed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FindingsSummary {
    /// All of them.
    pub total: u32,
    /// Landed.
    pub landed: u32,
    /// Partly landed.
    pub partly: u32,
    /// Not landed.
    pub not_landed: u32,
    /// Superseded.
    pub superseded: u32,
    /// Not checked yet.
    pub unchecked: u32,
    /// The open ones: not landed first, then partly, then unchecked.
    pub unlanded: Vec<FindingItem>,
}

impl FindingsSummary {
    /// The open ones' number.
    #[must_use]
    pub const fn open(&self) -> u32 {
        self.partly + self.not_landed + self.unchecked
    }
}

/// An area with its tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AreaTree {
    /// The area.
    pub area: Area,
    /// Rolled up from its leaves.
    pub state: NodeState,
    /// Its leaves by state.
    pub counts: Counts,
    /// What waits on the person in it, at any depth.
    pub questions: Vec<Node>,
    /// Its findings.
    pub findings: FindingsSummary,
    /// Its tasks, as trees.
    pub tree: Vec<Node>,
}

/// The findings of `findings`, counted per scan.
#[must_use]
pub fn scan_counts(findings: &[&Task]) -> BTreeMap<TaskId, FindingCount> {
    let mut scans: BTreeMap<TaskId, FindingCount> = BTreeMap::new();
    for f in findings {
        let count = scans.entry(f.parent.unwrap_or(TaskId::new(0))).or_default();
        count.total += 1;
        if f.landed.unwrap_or(Landed::Unchecked).is_open() {
            count.open += 1;
        }
    }
    scans
}

/// `area`'s tree from its rows (its tasks and findings, canceled ones left out); `lookup` finds
/// a scan or a fixing task that sits outside the area.
#[must_use]
pub fn tree<'a>(
    area: &Area,
    rows: &'a [Task],
    lookup: impl Fn(TaskId) -> Option<&'a Task>,
) -> AreaTree {
    let rows: Vec<&Task> = rows
        .iter()
        .filter(|t| t.status != Status::Canceled)
        .collect();
    let by_id: BTreeMap<TaskId, &Task> = rows.iter().map(|t| (t.id, *t)).collect();
    let tasks: Vec<&Task> = rows.iter().copied().filter(|t| !t.finding).collect();
    let findings: Vec<&Task> = rows.iter().copied().filter(|t| t.finding).collect();
    let scans = scan_counts(&findings);
    let ids: BTreeSet<TaskId> = tasks.iter().map(|t| t.id).collect();
    let (roots, leaves) = build(
        &tasks,
        |t| t.parent.is_none_or(|parent| !ids.contains(&parent)),
        &scans,
    );
    let mut all = Vec::new();
    walk(&roots, &mut all);
    let questions = all
        .into_iter()
        .filter(|n| n.status == Status::Waiting)
        .map(|n| Node {
            kids: Vec::new(),
            ..n.clone()
        })
        .collect();
    let find = |id: TaskId| by_id.get(&id).copied().or_else(|| lookup(id));
    AreaTree {
        area: area.clone(),
        state: if roots.is_empty() {
            NodeState::NotStarted
        } else {
            roll(NodeState::NotStarted, &leaves)
        },
        counts: Counts::of(&leaves),
        questions,
        findings: summarize(&findings, find),
        tree: roots,
    }
}

/// A done area, folded: no tree.
#[must_use]
pub fn bare(area: &Area) -> AreaTree {
    AreaTree {
        area: area.clone(),
        state: NodeState::Done,
        counts: Counts::default(),
        questions: Vec::new(),
        findings: FindingsSummary::default(),
        tree: Vec::new(),
    }
}

/// `findings` by state, the open ones listed with their scan and their fix.
#[must_use]
pub fn summarize<'a>(
    findings: &[&Task],
    find: impl Fn(TaskId) -> Option<&'a Task>,
) -> FindingsSummary {
    let mut s = FindingsSummary {
        total: u32::try_from(findings.len()).unwrap_or(u32::MAX),
        ..FindingsSummary::default()
    };
    for f in findings {
        match f.landed.unwrap_or(Landed::Unchecked) {
            Landed::Landed => s.landed += 1,
            Landed::Partly => s.partly += 1,
            Landed::NotLanded => s.not_landed += 1,
            Landed::Superseded => s.superseded += 1,
            Landed::Unchecked => s.unchecked += 1,
        }
    }
    let rank = |l: Landed| match l {
        Landed::NotLanded => 0,
        Landed::Partly => 1,
        _ => 2,
    };
    let mut open: Vec<&&Task> = findings
        .iter()
        .filter(|f| f.landed.unwrap_or(Landed::Unchecked).is_open())
        .collect();
    open.sort_by_key(|f| (rank(f.landed.unwrap_or(Landed::Unchecked)), f.parent, f.id));
    s.unlanded = open
        .into_iter()
        .map(|f| {
            let scan = f.parent.unwrap_or(TaskId::new(0));
            let fix = f.fixed_by.and_then(&find);
            FindingItem {
                id: f.id,
                title: f.title.clone(),
                landed: f.landed.unwrap_or(Landed::Unchecked),
                scan,
                scan_title: find(scan).map_or_default(|t| t.title.clone()),
                fixed_by: f.fixed_by,
                fixed_by_title: fix.map(|t| t.title.clone()),
                fixed_by_status: fix.map(|t| t.status),
            }
        })
        .collect();
    s
}

/// "12 tasks: 8 done · 2 working · 1 needs you · 1 not started".
#[must_use]
pub fn count_line(c: Counts) -> String {
    if c.total() == 0 {
        return "no tasks yet".to_owned();
    }
    let parts: Vec<String> = [
        (c.done, "done"),
        (c.working, "working"),
        (c.needs_you, "needs you"),
        (c.not_started, "not started"),
    ]
    .into_iter()
    .filter(|(n, _)| *n > 0)
    .map(|(n, word)| format!("{n} {word}"))
    .collect();
    format!(
        "{} task{}: {}",
        c.total(),
        if c.total() == 1 { "" } else { "s" },
        parts.join(" · ")
    )
}

fn line(out: &mut String, n: &Node, indent: &str, all: bool) {
    let mut bits = Vec::new();
    if n.owner == Owner::Person {
        bits.push("the person's".to_owned());
    }
    if !n.kids.is_empty() && n.state != NodeState::own(n.status) {
        bits.push(format!("itself {}", n.status.as_str()));
    }
    if let Some(action) = &n.action {
        bits.push(format!("for the person: {action}"));
    }
    if let Some(due) = n.due {
        bits.push(format!("due {due}"));
    }
    if let Some(f) = n.findings {
        bits.push(format!("findings {} open of {}", f.open, f.total));
    }
    let bits = if bits.is_empty() {
        String::new()
    } else {
        format!(" ({})", bits.join("; "))
    };
    put!(
        out,
        "{indent}{} {} {}{bits}",
        n.state.mark(),
        n.id.key(),
        n.title
    );
    let inner = format!("{indent}  ");
    for kid in n.kids.iter().filter(|k| all || k.state != NodeState::Done) {
        line(out, kid, &inner, all);
    }
    fold(out, &n.kids, &inner, all);
}

fn fold(out: &mut String, nodes: &[Node], indent: &str, all: bool) {
    let done: Vec<String> = nodes
        .iter()
        .filter(|k| k.state == NodeState::Done)
        .map(|k| k.id.key())
        .collect();
    if !all && !done.is_empty() {
        put!(out, "{indent}✓ {} done: {}", done.len(), done.join(", "));
    }
}

/// An area as a few lines: header, counts, questions, findings not landed, then its open tree
/// (done work folded into "✓ n done" lines; `all`: every node).
#[must_use]
pub fn outline(a: &AreaTree, all: bool, indent: &str, list_findings: bool) -> String {
    let mut out = String::new();
    let area = &a.area;
    let state = if area.status == AreaStatus::Active {
        format!(", {}", a.state.words())
    } else {
        String::new()
    };
    let about = area
        .about
        .as_ref()
        .map_or_default(|about| format!(": {about}"));
    put!(
        out,
        "{indent}Area {} \"{}\" ({}{state}){about}",
        area.id,
        area.name,
        area.status.as_str()
    );
    if let Some(goal) = &area.goal {
        put!(out, "{indent}  Goal: {goal}");
    }
    if area.status == AreaStatus::Done && a.tree.is_empty() {
        return out;
    }
    put!(out, "{indent}  {}", count_line(a.counts));
    for q in &a.questions {
        put!(
            out,
            "{indent}  Needs the person: {} {}: {}",
            q.id.key(),
            q.title,
            q.action.as_deref().unwrap_or("(no action)")
        );
    }
    let f = &a.findings;
    if f.total > 0 {
        let parts: Vec<String> = [
            (f.landed, "landed"),
            (f.partly, "partly landed"),
            (f.not_landed, "not landed"),
            (f.superseded, "superseded"),
            (f.unchecked, "unchecked"),
        ]
        .into_iter()
        .filter(|(n, _)| *n > 0)
        .map(|(n, word)| format!("{n} {word}"))
        .collect();
        put!(
            out,
            "{indent}  Findings: {} ({})",
            f.total,
            parts.join(", ")
        );
        if list_findings {
            for u in &f.unlanded {
                let fix = match (u.fixed_by, u.fixed_by_status) {
                    (Some(fix), status) => format!(
                        ", fix {} {}",
                        fix.key(),
                        status.map_or("unknown", Status::as_str)
                    ),
                    (None, _) => ", no fix task".to_owned(),
                };
                put!(
                    out,
                    "{indent}    {}: {} {} (from {}{fix})",
                    u.landed.words(),
                    u.id.key(),
                    u.title,
                    u.scan.key()
                );
            }
        } else if f.open() > 0 {
            put!(
                out,
                "{indent}    {} not landed yet: tasks area={} lists them",
                f.open(),
                area.id
            );
        }
    }
    let inner = format!("{indent}  ");
    for n in a.tree.iter().filter(|n| all || n.state != NodeState::Done) {
        line(&mut out, n, &inner, all);
    }
    fold(&mut out, &a.tree, &inner, all);
    out
}

/// The filters of the `tasks` tool and the app's filter bar: a project (`Some(None)`: no
/// project), an area, states and an owner. Any left out: all.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TaskFilter {
    /// Only this project's (`Some(None)`: tasks with no project).
    pub pid: Option<Option<ProjectId>>,
    /// Only this area's.
    pub area: Option<AreaId>,
    /// Only tasks in these states (their own status, not their leaves').
    pub states: BTreeSet<NodeState>,
    /// Only this owner's.
    pub owner: Option<Owner>,
}

impl TaskFilter {
    /// Whether it cuts anything.
    #[must_use]
    pub fn any(&self) -> bool {
        self.pid.is_some() || self.area.is_some() || !self.states.is_empty() || self.owner.is_some()
    }

    /// The task itself matches.
    #[must_use]
    pub fn matches(&self, n: &Node) -> bool {
        (self.states.is_empty() || self.states.contains(&NodeState::own(n.status)))
            && self.owner.is_none_or(|owner| n.owner == owner)
    }
}

/// The trees cut to what matches: a task stays when it matches or something under it does.
#[must_use]
pub fn prune(nodes: &[Node], f: &TaskFilter) -> Vec<Node> {
    nodes
        .iter()
        .map(|n| Node {
            kids: prune(&n.kids, f),
            ..n.clone()
        })
        .filter(|n| f.matches(n) || !n.kids.is_empty())
        .collect()
}

/// A project's work outside its areas, as trees: `open` are the board's tasks.
#[must_use]
pub fn loose(
    pid: Option<ProjectId>,
    open: &[Task],
    scans: &BTreeMap<TaskId, FindingCount>,
) -> Vec<Node> {
    let mine: Vec<&Task> = open
        .iter()
        .filter(|t| t.pid == pid && t.area.is_none() && !t.finding && t.status != Status::Canceled)
        .collect();
    let ids: BTreeSet<TaskId> = mine.iter().map(|t| t.id).collect();
    build(
        &mine,
        |t| t.parent.is_none_or(|parent| !ids.contains(&parent)),
        scans,
    )
    .0
}

/// A project's line on the overview.
#[derive(Clone, Debug, PartialEq)]
pub struct OverviewProject<'a> {
    /// The project.
    pub project: &'a Project,
    /// Its health.
    pub health: Option<Health>,
}

/// What the overview is made of: the projects in board order, every area's tree, the board's
/// tasks, the findings counted per scan, and Today.
#[derive(Clone, Debug)]
pub struct OverviewParts<'a> {
    /// The projects.
    pub projects: Vec<OverviewProject<'a>>,
    /// The areas' trees.
    pub areas: Vec<AreaTree>,
    /// The board's tasks.
    pub open: &'a [Task],
    /// Findings per scan.
    pub scans: BTreeMap<TaskId, FindingCount>,
    /// Today's ids.
    pub today: &'a [TaskId],
}

/// The `tasks` tool's overview: every project with open work, its areas (active ones in full,
/// paused and done ones as a line) and its tasks outside them, as outlines; then Today. A filter
/// cuts it to the matching tasks (with their parents), done ones spelled out only when the
/// states ask for them.
#[must_use]
#[expect(
    clippy::too_many_lines,
    reason = "one outline of every project, in a fixed order"
)]
pub fn overview(parts: &OverviewParts<'_>, f: &TaskFilter) -> String {
    let only_pid: Option<Option<ProjectId>> = match f.area {
        Some(area) => Some(
            parts
                .areas
                .iter()
                .find(|a| a.area.id == area)
                .map(|a| a.area.pid),
        ),
        None => f.pid,
    };
    let cut = f.any();
    let all = cut && f.states.contains(&NodeState::Done);
    let mut out = String::from(
        "Marks: ! needs the person, ~ working, ○ not started, ✓ done. Done work is folded (\"✓ n done: ...\"); tasks id=N opens one, tasks area=N one area in full.\n",
    );
    if cut {
        let mut said = Vec::new();
        match f.pid {
            Some(None) => said.push("no project".to_owned()),
            Some(Some(pid)) => said.push(format!("project {pid}")),
            None => {}
        }
        if let Some(area) = f.area {
            said.push(format!("area {area}"));
        }
        if !f.states.is_empty() {
            let states: Vec<&str> = f.states.iter().map(|s| s.as_str()).collect();
            said.push(format!("status {}", states.join("/")));
        }
        if let Some(owner) = f.owner {
            said.push(format!("owner {}", owner.as_str()));
        }
        put!(
            out,
            "Filtered: {}. A task shows when it matches or something under it does.",
            said.join(", ")
        );
    }
    let mut shown = 0;
    for p in &parts.projects {
        let project = p.project;
        if only_pid.is_some_and(|only| only != Some(project.id)) {
            continue;
        }
        let mut mine: Vec<AreaTree> = parts
            .areas
            .iter()
            .filter(|a| a.area.pid == project.id && f.area.is_none_or(|id| a.area.id == id))
            .cloned()
            .collect();
        let mut free = if f.area.is_some() {
            Vec::new()
        } else {
            loose(Some(project.id), parts.open, &parts.scans)
        };
        if cut {
            mine = mine
                .into_iter()
                .filter(|a| a.area.status != AreaStatus::Done || all)
                .map(|a| AreaTree {
                    tree: prune(&a.tree, f),
                    questions: a
                        .questions
                        .iter()
                        .filter(|q| f.matches(q))
                        .cloned()
                        .collect(),
                    ..a
                })
                .filter(|a| !a.tree.is_empty())
                .collect();
            free = prune(&free, f);
            if mine.is_empty() && free.is_empty() {
                continue;
            }
        } else if only_pid.is_none()
            && free.iter().all(|n| n.state == NodeState::Done)
            && !mine.iter().any(|a| a.area.status == AreaStatus::Active)
        {
            continue;
        }
        shown += 1;
        let health = p.health.map_or_default(|h| format!(", {}", h.as_str()));
        let ends = project
            .deadline
            .map_or_default(|d| format!(", ends {d}{}", if project.hard { " hard" } else { "" }));
        let status = if project.status == ProjectStatus::Active {
            "active"
        } else {
            project.status.as_str()
        };
        put!(
            out,
            "\nProject {} {} ({status}{health}{ends})",
            project.id,
            project.name
        );
        for a in &mine {
            if a.area.status == AreaStatus::Active || cut {
                out.push_str(&outline(a, all, "  ", false));
            } else {
                let open = if a.findings.open() > 0 {
                    format!(": {} findings not landed", a.findings.open())
                } else {
                    String::new()
                };
                put!(
                    out,
                    "  Area {} \"{}\" ({}){open}",
                    a.area.id,
                    a.area.name,
                    a.area.status.as_str()
                );
            }
        }
        if !free.is_empty() {
            let indent = if mine.is_empty() { "  " } else { "    " };
            if !mine.is_empty() {
                out.push_str("  Not in an area:\n");
            }
            for n in free.iter().filter(|n| all || n.state != NodeState::Done) {
                line(&mut out, n, indent, all);
            }
            let done = free.iter().filter(|n| n.state == NodeState::Done).count();
            if done > 0 && !all {
                put!(out, "{indent}✓ {done} done this week");
            }
        }
    }
    if matches!(only_pid, None | Some(None)) && f.area.is_none() {
        let mut none = loose(None, parts.open, &parts.scans);
        if cut {
            none = prune(&none, f);
        }
        if !none.is_empty() {
            shown += 1;
            out.push_str("\nNo project\n");
            for n in none.iter().filter(|n| all || n.state != NodeState::Done) {
                line(&mut out, n, "  ", all);
            }
        }
    }
    if cut && shown == 0 {
        out.push_str("\nNothing matches.\n");
    }
    if !cut {
        let today: Vec<String> = parts.today.iter().map(|id| id.key()).collect();
        put!(
            out,
            "\nToday: {}",
            if today.is_empty() {
                "nothing".to_owned()
            } else {
                today.join(", ")
            }
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task::tests::task;
    use crate::task::wire::KEY_PREFIX;

    fn area() -> Area {
        Area {
            id: AreaId::new(3),
            pid: ProjectId::new(1),
            name: "Harness".to_owned(),
            status: AreaStatus::Active,
            goal: Some("Every check green".to_owned()),
            about: None,
            created: Timestamp::UNIX_EPOCH,
            updated: Timestamp::UNIX_EPOCH,
        }
    }

    fn in_area(id: i64, parent: Option<i64>, status: Status) -> Task {
        let mut t = task(id);
        t.area = Some(AreaId::new(3));
        t.pid = Some(ProjectId::new(1));
        t.parent = parent.map(TaskId::new);
        t.status = status;
        if status == Status::Waiting {
            t.action = Some("Pick one".to_owned());
        }
        t
    }

    #[test]
    fn a_tree_rolls_up_from_its_leaves() {
        let mut finding = in_area(6, Some(4), Status::Todo);
        finding.finding = true;
        finding.landed = Some(Landed::NotLanded);
        finding.fixed_by = Some(TaskId::new(5));
        let rows = [
            in_area(1, None, Status::Working),
            in_area(2, Some(1), Status::Done),
            in_area(3, Some(1), Status::Waiting),
            in_area(4, None, Status::Done),
            in_area(5, None, Status::Todo),
            finding,
            in_area(7, None, Status::Canceled),
        ];
        let tree = tree(&area(), &rows, |_| None);
        assert_eq!(tree.state, NodeState::NeedsYou);
        assert_eq!(
            tree.counts,
            Counts {
                done: 2,
                working: 0,
                needs_you: 1,
                not_started: 1
            }
        );
        assert_eq!(tree.tree.len(), 3);
        assert_eq!(tree.tree[0].state, NodeState::NeedsYou);
        assert_eq!(tree.questions.len(), 1);
        assert_eq!(tree.questions[0].action.as_deref(), Some("Pick one"));
        assert_eq!(tree.findings.not_landed, 1);
        assert_eq!(
            tree.findings.unlanded[0].fixed_by_title.as_deref(),
            Some("Task 5")
        );
        assert_eq!(
            tree.tree[1].findings,
            Some(FindingCount { total: 1, open: 1 })
        );
        let text = outline(&tree, false, "", true);
        assert!(
            text.contains("Area 3 \"Harness\" (active, needs you)"),
            "{text}"
        );
        assert!(text.contains("  Goal: Every check green"), "{text}");
        assert!(
            text.contains("4 tasks: 2 done · 1 needs you · 1 not started"),
            "{text}"
        );
        assert!(
            text.contains(&format!(
                "not landed: {KEY_PREFIX}6 Task 6 (from {KEY_PREFIX}4, fix {KEY_PREFIX}5 todo)"
            )),
            "{text}"
        );
        assert!(text.contains(&format!("✓ 1 done: {KEY_PREFIX}4")), "{text}");
    }

    #[test]
    fn roll_puts_needs_you_over_everything() {
        use NodeState::{Done, NeedsYou, NotStarted, Working};
        assert_eq!(roll(NotStarted, &[]), NotStarted);
        assert_eq!(roll(Working, &[Done, NeedsYou]), NeedsYou);
        assert_eq!(roll(NotStarted, &[Done, Done]), Done);
        assert_eq!(roll(NotStarted, &[Done, NotStarted]), Working);
        assert_eq!(roll(NotStarted, &[NotStarted]), NotStarted);
        assert_eq!(count_line(Counts::default()), "no tasks yet");
        assert_eq!(NodeState::parse_filter("waiting"), Some(NeedsYou));
        assert_eq!(NodeState::parse_filter("blocked"), None);
    }

    #[test]
    fn a_filter_keeps_the_parents_of_what_matches() {
        let rows = [
            in_area(1, None, Status::Working),
            in_area(2, Some(1), Status::Waiting),
            in_area(3, None, Status::Todo),
        ];
        let t = tree(&area(), &rows, |_| None);
        let f = TaskFilter {
            states: BTreeSet::from([NodeState::NeedsYou]),
            ..TaskFilter::default()
        };
        let cut = prune(&t.tree, &f);
        assert_eq!(cut.len(), 1);
        assert_eq!(cut[0].kids[0].id, TaskId::new(2));
    }
}
