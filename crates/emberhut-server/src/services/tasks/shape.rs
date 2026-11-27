//! Tasks, projects and areas as the contract writes them.

use emberhut_api as api;
use emberhut_api::ids::{ArtifactId, GoalId, ProjectId, TaskId};
use emberhut_api::time::Day;
use emberhut_core::task::area::{AreaTree, Node};
use emberhut_core::task::board::{Project, ProjectLine, Scored};
use emberhut_core::task::live::Live;
use emberhut_core::task::rollup::Rollup;
use emberhut_core::task::{Task, TaskId as CoreTaskId};
use emberhut_db::StoredArtifact;

use crate::convert::timestamp_to_api;

/// The contract's task id for a domain one.
pub(crate) fn task_to_api(id: CoreTaskId) -> TaskId {
    TaskId::new(id.get())
}

/// A day as the contract writes it.
pub(crate) fn day(date: jiff::civil::Date) -> Day {
    Day::new(date.to_string())
}

/// A count as the contract writes it: a whole number from 0 up; anything else is no count.
fn count(n: Option<i64>) -> Option<u32> {
    n.and_then(|n| u32::try_from(n).ok())
}

/// A rollup as the contract writes it.
pub(crate) fn rollup(r: Rollup) -> api::tasks::Rollup {
    api::tasks::Rollup {
        done: r.done,
        total: r.total,
        waiting: r.waiting.into_iter().map(task_to_api).collect(),
        progress: count(r.progress),
        of: count(r.of),
        left: count(r.left),
        unit: r.unit,
    }
}

/// A live state as the contract writes it.
pub(crate) fn live(l: Live) -> api::task_live::TaskLive {
    api::task_live::TaskLive {
        state: api::task_live::LiveState::from(l.state.as_str()),
        since: l.since.map(timestamp_to_api),
        alarm: false,
        inferred: l.inferred,
        run: l.run.map(|run| api::task_live::LiveRun {
            machine: api::ids::MachineId::new(run.machine),
            place: Some(run.place),
            thread: run.thread,
            title: None,
            worker: run.worker,
            requested: run.requested.map(timestamp_to_api),
            started: run.started.map(timestamp_to_api),
        }),
        runner: None,
        until: l.until.map(api::time::Timestamp::from),
        by: l.by.map(task_to_api),
        watch: None,
        checkin: None,
        checking: None,
        last: l.last.map(|last| api::task_live::LiveLast {
            ended: last.ended.map(timestamp_to_api),
            line: last.line,
        }),
    }
}

/// An artifact as the app lists it: a file opens at its id's URL, a link (or an X post) where
/// it points.
pub(crate) fn artifact(a: &StoredArtifact) -> api::tasks::Artifact {
    let id = a.id.get();
    let x: Option<api::x_reply::XReply> = (a.kind == emberhut_db::X_REPLY)
        .then(|| {
            a.data
                .as_deref()
                .and_then(|data| serde_json::from_str(data).ok())
        })
        .flatten();
    let outside = a.kind == "link" || a.kind == emberhut_db::X_REPLY;
    let name = if a.kind == emberhut_db::X_REPLY {
        format!(
            "@{}",
            x.as_ref()
                .and_then(|x| x.author.clone())
                .unwrap_or_else(|| "post".to_owned())
        )
    } else if a.kind == "link" {
        let bare = a.reference.trim_start_matches("https://");
        let bare = bare.strip_prefix("www.").unwrap_or(bare);
        bare.trim_end_matches('/').chars().take(80).collect()
    } else {
        a.reference
            .rsplit('/')
            .next()
            .unwrap_or_default()
            .to_owned()
    };
    api::tasks::Artifact {
        id: ArtifactId::new(id),
        kind: a.kind.clone(),
        caption: a.caption.clone(),
        at: timestamp_to_api(a.at),
        name,
        url: if outside {
            a.reference.clone()
        } else {
            format!("/api/artifacts/{id}")
        },
        thumb: (a.kind == "image" || a.kind == "video")
            .then(|| format!("/api/artifacts/{id}/thumb")),
        x,
    }
}

/// What a task carries besides its own row, for the contract.
pub(crate) struct Extras {
    /// Its project's name.
    pub(crate) project: Option<String>,
    /// Its subtasks at a glance.
    pub(crate) sub: Option<Rollup>,
    /// What it produced.
    pub(crate) artifacts: Vec<api::tasks::Artifact>,
    /// What is happening on it.
    pub(crate) live: Option<Live>,
    /// Its place on the board.
    pub(crate) scored: Option<Scored>,
    /// The person's own request for it.
    pub(crate) requested: Option<api::tasks::Requested>,
    /// Someone else's.
    pub(crate) requested_by: Option<api::tasks::RequestedBy>,
    /// The person's last tap on its send card.
    pub(crate) send: Option<api::tasks::TaskSend>,
}

/// A task as the contract writes it.
pub(crate) fn issue(t: &Task, extras: Extras) -> api::tasks::Issue {
    let decision = t
        .decision
        .as_deref()
        .and_then(|d| serde_json::from_str::<api::tasks::Decision>(d).ok());
    api::tasks::Issue {
        id: task_to_api(t.id),
        key: t.key(),
        title: t.title.clone(),
        owner: t.owner.as_str().to_owned(),
        status: api::tasks::TaskStatus::from(t.status.as_str()),
        goal: t.goal.map(|g| GoalId::new(g.get())),
        project: extras.project.or_else(|| t.project.clone()),
        pid: t.pid.map(|p| ProjectId::new(p.get())),
        priority: t.priority,
        due: t.due.map(day),
        today: t.today,
        decide_by: t.decide_by.map(day),
        decision,
        action: t.action.clone(),
        updated: timestamp_to_api(t.updated),
        done_at: t.done_at.map(timestamp_to_api),
        parent: t.parent.map(task_to_api),
        artifacts: extras.artifacts,
        deadline: extras.scored.and_then(|s| s.deadline).map(|d| day(d.date)),
        hard_deadline: extras
            .scored
            .and_then(|s| s.deadline)
            .is_some_and(|d| d.hard),
        score: extras.scored.map_or(0.0, |s| s.score),
        body: t.body.clone(),
        hard: t.hard,
        size: t.size.map(|s| api::tasks::TaskSize::from(s.as_str())),
        parked_until: t.parked_until.clone(),
        created: Some(timestamp_to_api(t.created)),
        last: t.last.clone(),
        progress: count(t.progress),
        total: count(t.total),
        unit: t.unit.clone(),
        sub: extras.sub.map(rollup),
        tracked: t.tracked,
        eta: t.eta.clone(),
        ultra: t.ultra,
        agents: None,
        live: extras.live.map(live),
        area: t.area.map(|a| api::ids::AreaId::new(a.get())),
        kind: t.finding.then(|| "finding".to_owned()),
        landed: t.landed.map(|l| api::tasks::Landed::from(l.as_str())),
        fixed_by: t.fixed_by.map(task_to_api),
        requested: extras.requested,
        requested_by: extras.requested_by,
        send: extras.send,
    }
}

/// A project as the board shows it.
pub(crate) fn project(p: &Project, line: Option<&ProjectLine>) -> api::tasks::Project {
    api::tasks::Project {
        id: ProjectId::new(p.id.get()),
        name: p.name.clone(),
        goal: p.goal.map(|g| GoalId::new(g.get())),
        priority: p.priority,
        kind: api::tasks::ProjectKind::from(p.kind.as_str()),
        deadline: p.deadline.map(day),
        hard: p.hard,
        status: api::tasks::ProjectStatus::from(p.status.as_str()),
        paused_until: p.paused_until.map(day),
        open: line.map_or(0, |l| l.open),
        done_week: line.map_or(0, |l| l.done_week),
        health: line
            .and_then(|l| l.health)
            .map(|h| api::tasks::ProjectHealth::from(h.as_str())),
        next: line.and_then(|l| l.next).map(day),
        hours_left: line.map_or(0.0, |l| l.hours_left),
    }
}

fn node(n: &Node) -> api::areas::TreeNode {
    api::areas::TreeNode {
        id: task_to_api(n.id),
        title: n.title.clone(),
        owner: n.owner.as_str().to_owned(),
        status: n.status.as_str().to_owned(),
        state: api::areas::WorkStage::from(n.state.as_str()),
        action: n.action.clone(),
        due: n.due.map(day),
        done_at: n.done_at.map(timestamp_to_api),
        findings: n.findings.map(|f| api::areas::FindingCount {
            total: f.total,
            open: f.open,
        }),
        kids: n.kids.iter().map(node).collect(),
    }
}

/// An area's tree as the contract writes it.
pub(crate) fn area(t: &AreaTree) -> api::areas::Area {
    let f = &t.findings;
    api::areas::Area {
        id: api::ids::AreaId::new(t.area.id.get()),
        pid: ProjectId::new(t.area.pid.get()),
        name: t.area.name.clone(),
        status: api::areas::AreaStatus::from(t.area.status.as_str()),
        goal: t.area.goal.clone(),
        about: t.area.about.clone(),
        state: api::areas::WorkStage::from(t.state.as_str()),
        counts: api::areas::AreaCounts {
            done: t.counts.done,
            working: t.counts.working,
            needs_you: t.counts.needs_you,
            not_started: t.counts.not_started,
            total: t.counts.done + t.counts.working + t.counts.needs_you + t.counts.not_started,
        },
        questions: t.questions.iter().map(node).collect(),
        findings: api::areas::FindingsSummary {
            total: f.total,
            landed: f.landed,
            partly: f.partly,
            not_landed: f.not_landed,
            superseded: f.superseded,
            unchecked: f.unchecked,
            open: f.not_landed + f.partly + f.unchecked,
            unlanded: f
                .unlanded
                .iter()
                .map(|u| api::areas::FindingItem {
                    id: task_to_api(u.id),
                    title: u.title.clone(),
                    landed: api::tasks::Landed::from(u.landed.as_str()),
                    scan: task_to_api(u.scan),
                    scan_title: u.scan_title.clone(),
                    fixed_by: u.fixed_by.map(task_to_api),
                    fixed_by_title: u.fixed_by_title.clone(),
                    fixed_by_status: u.fixed_by_status.map(|s| s.as_str().to_owned()),
                })
                .collect(),
        },
        tree: t.tree.iter().map(node).collect(),
    }
}
