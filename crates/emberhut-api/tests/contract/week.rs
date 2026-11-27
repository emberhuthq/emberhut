//! The week in the state: the goals' lists, the weekly report, the projects' areas, and what a
//! waiting row adds for its card (its area, its picture, an X post to reply to).

use emberhut_api::areas::{AreaStatus, WorkStage};
use emberhut_api::ids::{AreaId, GoalId, ProjectId, TaskId};
use emberhut_api::state::State;
use emberhut_api::task_message::MessageTo;
use emberhut_api::tasks::{Landed, ProjectHealth};

use crate::support::read;

#[test]
fn reads_the_goals_lists_and_their_week() {
    let state: State = read("state.json");
    let goal = &state.goals[0];
    assert_eq!(goal.week.by("owner"), 1);
    assert_eq!(goal.week.by(MessageTo::ASSISTANT), 2);
    assert_eq!(goal.week.by("someone else"), 0);
    assert!(goal.at.is_some());
    let todo = &state.todos[0];
    assert_eq!(
        (todo.id, todo.goal, todo.is_done()),
        (TaskId::new(12), Some(GoalId::new(1)), false)
    );
    assert!(todo.today);
}

#[test]
fn reads_the_weekly_report() {
    let state: State = read("state.json");
    let report = state.report.unwrap();
    assert_eq!(report.concern.unwrap().title, "A made-up concern");
    assert_eq!(report.weeks.len(), 8);
    assert_eq!(report.week_starts.unwrap().len(), 8);
    assert!(report.since.is_some());
    let goal = &report.goals[0];
    assert_eq!(goal.projects[0].id, ProjectId::new(2));
    assert_eq!(goal.projects[0].health, Some(ProjectHealth::OnTrack));
    assert_eq!(report.done[0].project.as_deref(), Some("Example project"));
}

#[test]
fn reads_the_areas() {
    let state: State = read("state.json");
    let [area, done] = state.areas.as_slice() else {
        panic!("two areas")
    };
    assert_eq!((area.id, area.pid), (AreaId::new(9), ProjectId::new(2)));
    assert_eq!(area.state, WorkStage::NeedsYou);
    assert_eq!(area.open(), 2);
    assert_eq!(area.questions[0].action.as_deref(), Some("Say yes"));
    assert_eq!(area.findings.open, 1);
    assert_eq!(area.findings.unlanded[0].landed, Landed::NotLanded);
    assert_eq!(area.tree[0].kids[0].state, WorkStage::Done);
    assert_eq!(done.status, AreaStatus::Done);
    assert_eq!(done.tree.len(), 0);
    assert_eq!(state.issues[0].area, Some(AreaId::new(9)));
}

#[test]
fn reads_what_a_waiting_row_adds_for_its_card() {
    let state: State = read("state.json");
    let row = &state.working[0];
    assert_eq!(row.area, Some(AreaId::new(9)));
    assert_eq!(row.thumb.as_deref(), Some("/api/artifacts/7/thumb"));
    let reply = row.reply.as_ref().unwrap();
    assert_eq!(reply.kind, "x_reply");
    let x = reply.x.as_ref().unwrap();
    assert_eq!(x.author.as_deref(), Some("example"));
    assert_eq!(x.followers, Some(1200));
    assert_eq!(x.replies.len(), 2);
    assert_eq!(
        x.replies[0].intent_url.as_deref(),
        Some("https://x.com/intent/tweet?in_reply_to=1")
    );
    assert_eq!(x.metrics.unwrap().likes, Some(40));
    assert_eq!(x.room.as_ref().unwrap().top_replies[0].likes, Some(12));
    assert!(x.posted.is_none());
    assert!(state.working[1].thumb.is_none() && state.working[1].reply.is_none());
}
