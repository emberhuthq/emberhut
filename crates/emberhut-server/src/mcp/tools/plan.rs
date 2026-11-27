//! The plan's tools: the whole plan (`plan`), the user's goals (`goal_set`, `goal_drop`), the
//! watch list (`focus_set`, `focus_drop`), the open loops (`loop_set`, `close_loop`) and the
//! ultimate concern (`concern_set`).

#![expect(
    clippy::option_option,
    reason = "a partial change: left out is unchanged, null clears"
)]

use emberhut_api::plan::FocusLink;
use serde::Deserialize;

use super::tasks::patch;
use super::{NoArgs, Tool, ToolContext, ToolError, ToolOutput};
use crate::convert::room_to_api;
use crate::services::plan::{FocusChange, GoalChange, LoopChange, PlanError};

impl From<PlanError> for ToolError {
    fn from(error: PlanError) -> Self {
        match error {
            PlanError::Refused(text) => Self::refused(text),
            other => Self::failed(other),
        }
    }
}

/// The user's plan.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Plan;

impl Tool for Plan {
    type Args = NoArgs;

    fn name(&self) -> &'static str {
        "plan"
    }

    fn description(&self) -> &'static str {
        "The user's plan: goals (theirs, ranked, with this week's steps), todos (open ones and this \
         week's done ones), the week's start, and projects: the watch items you keep an eye on \
         behind the scenes (id, title, line, next, goal, parkedUntil, links, timeline; not their \
         projects, see the projects tool)."
    }

    async fn call(&self, ctx: &ToolContext, _: NoArgs) -> Result<ToolOutput, ToolError> {
        let state = ctx.app.tasks().plan_state().await?;
        let focus = ctx.app.plan().focus().await.map_err(ToolError::failed)?;
        ToolOutput::json(&serde_json::json!({
            "goals": state.goals,
            "todos": state.todos,
            "weekStart": state.week_start,
            "projects": focus,
        }))
    }
}

/// Saves one of the user's goals.
#[derive(Clone, Copy, Debug)]
pub(crate) struct GoalSet;

/// `goal_set`'s arguments.
#[derive(Clone, Debug, Default, Deserialize, utoipa::ToSchema, garde::Validate)]
pub(crate) struct GoalSetArgs {
    /// The goal to change (none: a new one, with title and why).
    #[garde(range(min = 1))]
    id: Option<i64>,
    /// Its title.
    #[garde(length(max = 200))]
    title: Option<String>,
    /// Why it matters.
    #[garde(length(max = 2000))]
    why: Option<String>,
    /// 1 matters most; the others move down.
    #[garde(range(min = 1))]
    rank: Option<i64>,
    /// Your own line on how it's going this week (null clears it).
    #[serde(default, deserialize_with = "patch")]
    #[garde(skip)]
    status: Option<Option<String>>,
    /// Its 1-2 word label under its ring (null clears it).
    #[serde(default, deserialize_with = "patch")]
    #[garde(inner(inner(length(max = 16))))]
    short: Option<Option<String>>,
}

impl Tool for GoalSet {
    type Args = GoalSetArgs;

    fn name(&self) -> &'static str {
        "goal_set"
    }

    fn description(&self) -> &'static str {
        "Save one of the user's goals, only as they state or confirm it: the big, long-term things \
         they work towards (not tasks). New: title and why. rank 1 = matters most (others move \
         down). status: your own one line on how it's going this week, shown under the goal; keep \
         it true (you may always update status). short: its 1-2 word label under its ring."
    }

    async fn call(&self, ctx: &ToolContext, a: GoalSetArgs) -> Result<ToolOutput, ToolError> {
        let goal = ctx
            .app
            .plan()
            .set_goal(GoalChange {
                id: a.id,
                title: a.title,
                why: a.why,
                rank: a.rank,
                status: a.status,
                short: a.short,
            })
            .await?;
        Ok(ToolOutput::text(format!(
            "Goal #{}, rank {}.",
            goal.id, goal.rank
        )))
    }
}

/// A goal's or a Focus item's id.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema, garde::Validate)]
pub(crate) struct IdArgs {
    /// Its id.
    #[garde(range(min = 1))]
    id: i64,
}

/// Removes a goal.
#[derive(Clone, Copy, Debug)]
pub(crate) struct GoalDrop;

impl Tool for GoalDrop {
    type Args = IdArgs;

    fn name(&self) -> &'static str {
        "goal_drop"
    }

    fn description(&self) -> &'static str {
        "Remove a goal, only when the user says so. Its watch items keep going, untied."
    }

    async fn call(&self, ctx: &ToolContext, a: IdArgs) -> Result<ToolOutput, ToolError> {
        ctx.app
            .plan()
            .drop_goal(a.id)
            .await
            .map_err(ToolError::failed)?;
        Ok(ToolOutput::text("Removed."))
    }
}

/// A Focus item's link.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema)]
pub(crate) struct LinkArg {
    /// Thread, Watch, Loop, Ticket, PR, Person, Note, ...
    kind: String,
    /// Plain words.
    label: String,
}

/// A Focus item's section.
#[derive(Clone, Copy, Debug, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SectionArg {
    /// It needs the user.
    NeedsYou,
    /// It moves.
    Moving,
    /// The user's own life.
    Personal,
    /// Nothing happens.
    Quiet,
}

impl SectionArg {
    const fn word(self) -> &'static str {
        match self {
            Self::NeedsYou => "needs_you",
            Self::Moving => "moving",
            Self::Personal => "personal",
            Self::Quiet => "quiet",
        }
    }
}

/// Adds or updates a watch item.
#[derive(Clone, Copy, Debug)]
pub(crate) struct FocusSet;

/// `focus_set`'s arguments.
#[derive(Clone, Debug, Default, Deserialize, utoipa::ToSchema, garde::Validate)]
pub(crate) struct FocusSetArgs {
    /// The item to change (none: a new one, with title and line).
    #[garde(skip)]
    id: Option<i64>,
    /// Its title.
    #[garde(length(max = 200))]
    title: Option<String>,
    /// `needs_you`, moving, personal or quiet.
    #[garde(skip)]
    section: Option<SectionArg>,
    /// Where it stands now, one or two plain sentences.
    #[garde(length(max = 2000))]
    line: Option<String>,
    /// Its next step (null clears it).
    #[serde(default, deserialize_with = "patch")]
    #[garde(skip)]
    next: Option<Option<String>>,
    /// 1-3 short chips.
    #[garde(length(max = 4))]
    tags: Option<Vec<String>>,
    /// The goal it serves (null: none).
    #[serde(default, deserialize_with = "patch")]
    #[garde(skip)]
    goal: Option<Option<i64>>,
    /// The date the user parked it until (null brings it back).
    #[serde(default, deserialize_with = "patch")]
    #[garde(skip)]
    parked_until: Option<Option<String>>,
    /// Their reason, in a few words.
    #[serde(default, deserialize_with = "patch")]
    #[garde(skip)]
    parked_why: Option<Option<String>>,
    /// What it ties to.
    #[garde(length(max = 12))]
    links: Option<Vec<LinkArg>>,
    /// Only when the user asks to keep it.
    #[garde(skip)]
    pinned: Option<bool>,
    /// A short timeline line.
    #[garde(length(max = 500))]
    happened: Option<String>,
}

impl Tool for FocusSet {
    type Args = FocusSetArgs;

    fn name(&self) -> &'static str {
        "focus_set"
    }

    fn description(&self) -> &'static str {
        "Add a watch item (no id: title and line) or update one (id; fields left out stay). A watch \
         item is anything you keep an eye on behind the scenes; the user's projects and their tasks \
         are project_set and task_create, not this. line: where it stands now. next: its next \
         step. goal: the goal id it serves. section: needs_you, moving, personal, quiet. \
         parked_until: the date the user parks it until (null brings it back), with parked_why. \
         tags: 1-3 chips. links: what it ties to. happened: a short timeline line. pinned: only \
         when the user asks to keep it."
    }

    async fn call(&self, ctx: &ToolContext, a: FocusSetArgs) -> Result<ToolOutput, ToolError> {
        let new = a.id.is_none();
        let parked = a.parked_until.clone().flatten().filter(|p| !p.is_empty());
        let item = ctx
            .app
            .plan()
            .set_focus(FocusChange {
                id: a.id,
                title: a.title,
                section: a.section.map(|s| s.word().to_owned()),
                line: a.line,
                next: a.next,
                tags: a.tags,
                links: a.links.map(|links| {
                    links
                        .into_iter()
                        .map(|l| FocusLink {
                            kind: l.kind,
                            label: l.label,
                        })
                        .collect()
                }),
                pinned: a.pinned,
                happened: a.happened,
                goal: a.goal,
                parked_until: a.parked_until,
                parked_why: a.parked_why,
            })
            .await?;
        let activity = ctx.app.activity();
        if new {
            activity
                .note(format!("Keeping an eye on {}", item.title))
                .await
                .map_err(ToolError::failed)?;
        }
        if let Some(parked) = parked {
            activity
                .note(format!(
                    "Parked {} until {}",
                    item.title,
                    parked.chars().take(10).collect::<String>()
                ))
                .await
                .map_err(ToolError::failed)?;
        }
        Ok(ToolOutput::text(format!("Saved as #{}.", item.id)))
    }
}

/// Takes a watch item off the plan.
#[derive(Clone, Copy, Debug)]
pub(crate) struct FocusDrop;

/// `focus_drop`'s arguments.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema, garde::Validate)]
pub(crate) struct FocusDropArgs {
    /// The item.
    #[garde(range(min = 1))]
    id: i64,
    /// The owner asked for it (a pinned item only then).
    #[serde(default)]
    #[garde(skip)]
    owner_asked: bool,
}

impl Tool for FocusDrop {
    type Args = FocusDropArgs;

    fn name(&self) -> &'static str {
        "focus_drop"
    }

    fn description(&self) -> &'static str {
        "Take a watch item off the plan when it is over or no longer matters. Pinned ones only \
         when the user asks (owner_asked: true)."
    }

    async fn call(&self, ctx: &ToolContext, a: FocusDropArgs) -> Result<ToolOutput, ToolError> {
        ctx.app.plan().drop_focus(a.id, a.owner_asked).await?;
        Ok(ToolOutput::text("Dropped."))
    }
}

/// Keeps an open loop.
#[derive(Clone, Copy, Debug)]
pub(crate) struct LoopSet;

/// `loop_set`'s arguments.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema, garde::Validate)]
pub(crate) struct LoopSetArgs {
    /// The loop to change (none: a new one).
    #[garde(skip)]
    id: Option<i64>,
    /// Who it is about.
    #[garde(length(min = 1, max = 200))]
    who: String,
    /// What is open.
    #[garde(length(min = 1, max = 2000))]
    what: String,
    /// When it is due (a date, or null).
    #[serde(default, deserialize_with = "patch")]
    #[garde(skip)]
    due: Option<Option<String>>,
    /// Where it came from, in words.
    #[garde(length(max = 200))]
    source: Option<String>,
    /// The chat's id, so the user's reply there flags it maybe answered.
    #[garde(length(max = 200))]
    chat: Option<String>,
}

impl Tool for LoopSet {
    type Args = LoopSetArgs;

    fn name(&self) -> &'static str {
        "loop_set"
    }

    fn description(&self) -> &'static str {
        "Keep an open loop in your context: someone waiting on the user, a promise they made, a \
         deadline. New: no id. who, what, due (a date or null), source (where it came from), chat: \
         the chat's id, so their reply there flags it maybe answered."
    }

    async fn call(&self, ctx: &ToolContext, a: LoopSetArgs) -> Result<ToolOutput, ToolError> {
        let id = ctx
            .app
            .plan()
            .set_loop(LoopChange {
                id: a.id,
                who: a.who,
                what: a.what,
                due: a.due.flatten(),
                source: a.source,
                chat: a.chat,
                room: Some(room_to_api(ctx.room)),
            })
            .await
            .map_err(ToolError::failed)?;
        Ok(ToolOutput::text(format!("Loop #{id}.")))
    }
}

/// Closes an open loop.
#[derive(Clone, Copy, Debug)]
pub(crate) struct CloseLoop;

/// `close_loop`'s arguments.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema, garde::Validate)]
pub(crate) struct CloseLoopArgs {
    /// The loop.
    #[garde(skip)]
    id: i64,
}

impl Tool for CloseLoop {
    type Args = CloseLoopArgs;

    fn name(&self) -> &'static str {
        "close_loop"
    }

    fn description(&self) -> &'static str {
        "Close an open loop that is done or no longer matters."
    }

    async fn call(&self, ctx: &ToolContext, a: CloseLoopArgs) -> Result<ToolOutput, ToolError> {
        ctx.app
            .plan()
            .close_loop(a.id)
            .await
            .map_err(ToolError::failed)?;
        Ok(ToolOutput::text("Closed."))
    }
}

/// Saves the ultimate concern.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ConcernSet;

/// `concern_set`'s arguments.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema, garde::Validate)]
pub(crate) struct ConcernSetArgs {
    /// What it is.
    #[garde(length(min = 1, max = 300))]
    title: String,
    /// Why.
    #[garde(length(min = 1, max = 2000))]
    why: String,
}

impl Tool for ConcernSet {
    type Args = ConcernSetArgs;

    fn name(&self) -> &'static str {
        "concern_set"
    }

    fn description(&self) -> &'static str {
        "Save the user's ultimate concern (the one thing all their goals serve, shown in their \
         weekly report), only as they state or confirm it."
    }

    async fn call(&self, ctx: &ToolContext, a: ConcernSetArgs) -> Result<ToolOutput, ToolError> {
        ctx.app
            .plan()
            .set_concern(a.title, a.why)
            .await
            .map_err(ToolError::failed)?;
        Ok(ToolOutput::text("Saved."))
    }
}
