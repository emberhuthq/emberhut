//! The agents on a task's run, as its page shows them: the run's summary (ultra mode's parts,
//! state and spending), its lead, and every sub-agent in a tree (a part's check under its part,
//! a workflow's agents under the workflow), each with how long it ran and what it used.

use emberhut_api::ultra::{AgentKind, AgentState, UltraAgent, UltraLead, UltraRun, UltraState};
use jiff::Timestamp;

use super::clock;
use super::counts::tokens;
use super::words::Tone;

/// An agent's state in words.
#[must_use]
pub(crate) fn state_name(state: &AgentState) -> String {
    match state {
        AgentState::Running => "Running".to_owned(),
        AgentState::Queued => "Queued".to_owned(),
        AgentState::Done => "Done".to_owned(),
        AgentState::Failed => "Failed".to_owned(),
        AgentState::Stopped => "Stopped".to_owned(),
        AgentState::Other(word) => crate::format::capitalized(word),
        other => crate::format::capitalized(other.as_str()),
    }
}

/// How deep `agent` sits in the tree: 0 with no lead, 1 under the lead, 2 under a part (its
/// check) or a workflow.
#[must_use]
pub(crate) fn level(agent: &UltraAgent, run: &UltraRun) -> usize {
    if agent.workflow {
        return 2;
    }
    if agent.kind == Some(AgentKind::Verify)
        && let Some(part) = agent.part
        && run
            .agents
            .iter()
            .any(|other| other.kind == Some(AgentKind::Part) && other.part == Some(part))
    {
        return 2;
    }
    usize::from(run.lead.is_some())
}

/// The summary's first line: `3 of 5 parts done`.
#[must_use]
pub(crate) fn parts_line(run: &UltraRun) -> String {
    if run.total > 0 {
        format!("{} of {} parts done", run.done, run.total)
    } else {
        "No parts yet".to_owned()
    }
}

/// Where the run is, and its colour: `Planning`, `Waits for your OK`, `At its spending cap`,
/// `2 running`, `Done`.
#[must_use]
pub(crate) fn run_state(run: &UltraRun) -> (String, Tone) {
    let all_done = run.total > 0 && run.done == run.total;
    let text = match &run.state {
        Some(UltraState::Planning) => "Planning".to_owned(),
        Some(UltraState::Asking) => "Waits for your OK".to_owned(),
        Some(UltraState::Capped) => "At its spending cap".to_owned(),
        _ if run.running > 0 => format!("{} running", run.running),
        _ if run.agents.is_empty() => "Not started".to_owned(),
        _ if all_done => "Done".to_owned(),
        _ => "Between agents".to_owned(),
    };
    let tone = if matches!(run.state, Some(UltraState::Asking | UltraState::Capped)) {
        Tone::Accent
    } else if run.running > 0 || all_done {
        Tone::Success
    } else {
        Tone::Muted
    };
    (text, tone)
}

/// The summary's bar: parts done of all.
#[must_use]
pub(crate) fn parts_done(run: &UltraRun) -> f64 {
    if run.total == 0 {
        0.0
    } else {
        f64::from(run.done) / f64::from(run.total)
    }
}

/// What it spent: `350k of 2M tokens · 4 at a time · plan 3 agents, ~900k`.
#[must_use]
pub(crate) fn usage_line(run: &UltraRun) -> String {
    let plan = run.plan.map_or_else(String::new, |plan| {
        format!(
            " · plan {} agents, ~{}",
            crate::format::whole(plan.agents),
            tokens(plan.tokens)
        )
    });
    format!(
        "{} of {} tokens · {} at a time{plan}",
        tokens(run.spent),
        tokens(run.budget),
        run.parallel
    )
}

/// The Agents header's count while something runs: `3 working`.
#[must_use]
pub(crate) fn working_line(run: &UltraRun) -> Option<String> {
    run.is_live().then(|| {
        let lead = u32::from(run.lead.as_ref().is_some_and(|lead| lead.running));
        format!("{} working", run.running + lead)
    })
}

/// The lead's small line: `the Mac · 2 helpers · 120k tokens · $0.40`.
#[must_use]
pub(crate) fn lead_line(lead: &UltraLead, helpers: usize) -> String {
    let helpers = match helpers {
        0 => None,
        1 => Some("1 helper".to_owned()),
        n => Some(format!("{n} helpers")),
    };
    [
        (!lead.place.is_empty()).then(|| lead.place.clone()),
        helpers,
        lead.tokens.map(|count| format!("{} tokens", tokens(count))),
        lead.cost.map(|cost| format!("${cost:.2}")),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" · ")
}

/// What the lead does now, or its state once it stopped.
#[must_use]
pub(crate) fn lead_doing(lead: &UltraLead) -> String {
    if lead.running {
        lead.doing.clone().unwrap_or_else(|| "Thinking…".to_owned())
    } else {
        state_name(&lead.state)
    }
}

/// A sub-agent's small line: its state (unless running), tokens and steps.
#[must_use]
pub(crate) fn agent_line(agent: &UltraAgent) -> String {
    let running = agent.state == Some(AgentState::Running);
    #[expect(
        clippy::cast_possible_truncation,
        reason = "a step count is far inside i64's range"
    )]
    let steps = agent.tools.map(|steps| steps.round() as i64);
    [
        agent.state.as_ref().filter(|_| !running).map(state_name),
        agent
            .tokens
            .map(|count| format!("{} tokens", tokens(count))),
        steps.map(|steps| {
            if steps == 1 {
                "1 step".to_owned()
            } else {
                format!("{steps} steps")
            }
        }),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" · ")
}

/// How long it ran: from `started` to `ended`, or to `now` while it runs (the view redraws
/// it every second); none before it started.
#[must_use]
pub(crate) fn elapsed(
    started: Option<&emberhut_api::time::Timestamp>,
    ended: Option<&emberhut_api::time::Timestamp>,
    running: bool,
    now: Timestamp,
) -> Option<String> {
    let start = started.and_then(clock::instant)?;
    let end = if running {
        now
    } else {
        ended.and_then(clock::instant)?
    };
    Some(clock::span(start, end))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn run(extra: serde_json::Value) -> UltraRun {
        let mut base = json!({"on": true, "state": "running", "parallel": 4, "budget": 2_000_000.0,
            "spent": 350_000.0, "done": 1, "total": 3, "running": 1, "agents": []});
        if let (Some(base), serde_json::Value::Object(extra)) = (base.as_object_mut(), extra) {
            base.extend(extra);
        }
        serde_json::from_value(base).unwrap()
    }

    fn agent(id: i64, kind: &str, part: u32, workflow: bool) -> serde_json::Value {
        json!({"id": id, "name": format!("A{id}"), "kind": kind, "part": part, "state": "running",
            "last": null, "tokens": 1000.0, "tools": 1.0, "workflow": workflow})
    }

    #[test]
    fn a_check_sits_under_its_part_and_the_rest_under_the_lead() {
        let alone = run(json!({"agents": [agent(1, "part", 1, false)]}));
        assert_eq!(level(&alone.agents[0], &alone), 0, "no lead: at the root");
        let lead =
            json!({"viewer": "w-1", "name": "The worker", "state": "running", "running": true});
        let run = run(json!({"lead": lead, "agents": [
            agent(1, "part", 1, false), agent(2, "verify", 1, false),
            agent(3, "verify", 9, false), agent(4, "other", 0, true)]}));
        let levels: Vec<usize> = run.agents.iter().map(|a| level(a, &run)).collect();
        assert_eq!(levels, [1, 2, 1, 2]);
        assert_eq!(working_line(&run).as_deref(), Some("2 working"));
    }

    #[test]
    fn the_summary_says_parts_state_and_spending() {
        let running = run(json!({"plan": {"agents": 3.0, "tokens": 900_000.0}}));
        assert_eq!(parts_line(&running), "1 of 3 parts done");
        assert_eq!(run_state(&running), ("1 running".to_owned(), Tone::Success));
        assert_eq!(
            usage_line(&running),
            "350k of 2M tokens · 4 at a time · plan 3 agents, ~900k"
        );
        let asking = run(json!({"state": "asking", "running": 0}));
        assert_eq!(
            run_state(&asking),
            ("Waits for your OK".to_owned(), Tone::Accent)
        );
        let quiet = run(json!({"state": null, "running": 0, "total": 0, "done": 0}));
        assert_eq!(run_state(&quiet).0, "Not started");
        assert_eq!(parts_line(&quiet), "No parts yet");
        assert!(working_line(&quiet).is_none());
    }

    #[test]
    fn rows_say_what_each_agent_used() {
        let lead: UltraLead = serde_json::from_value(json!({"viewer": "w-1", "name": "The worker",
            "place": "the Mac", "state": "done", "running": false, "tokens": 120_000.0, "cost": 0.4})).unwrap();
        assert_eq!(
            lead_line(&lead, 2),
            "the Mac · 2 helpers · 120k tokens · $0.40"
        );
        assert_eq!(lead_doing(&lead), "Done");
        let done: UltraAgent =
            serde_json::from_value(json!({"id": 1, "name": "A1", "kind": "part",
            "part": 1, "state": "done", "last": "Wrote it", "tokens": 12_000.0, "tools": 1.0}))
            .unwrap();
        assert_eq!(agent_line(&done), "Done · 12k tokens · 1 step");
        let now: Timestamp = "2026-11-25T10:05:00Z".parse().unwrap();
        let started = emberhut_api::time::Timestamp::new("2026-11-25T08:00:00Z");
        assert_eq!(
            elapsed(Some(&started), None, true, now).as_deref(),
            Some("2h 5m")
        );
        assert_eq!(elapsed(Some(&started), None, false, now), None);
    }
}
