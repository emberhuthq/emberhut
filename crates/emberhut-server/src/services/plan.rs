//! The person's plan around the tasks, kept as settings: the
//! goals (ranked, set only as the person states them), the watch list (Focus items), the open
//! loops, the ultimate concern, and the assistant's own background jobs.

use std::sync::Arc;

use emberhut_api::background::{BackgroundTask, WorkStatus};
use emberhut_api::ids::{GoalId, RoomId};
use emberhut_api::plan::{Concern, FocusEvent, FocusItem, FocusLink, Loop};
use emberhut_api::tasks::Goal;
use emberhut_core::Clock;
use emberhut_core::plan::{LOOPS_KEPT, TIMELINE_LINES, parked, reranked};
use emberhut_db::{DbError, LogStore, SettingKey, SettingStore};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::convert::timestamp_to_api;

/// A goal as it is kept: `status` and `short` stay out until set, and keep a `null` given.
#[expect(clippy::option_option, reason = "left out until set, and a null kept")]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct StoredGoal {
    id: i64,
    title: String,
    why: String,
    rank: i32,
    at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    status: Option<Option<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    short: Option<Option<String>>,
}

/// The assistant's context as it is kept: its loops beside what other
/// areas keep there (its notes, when the scout last read), which stay as they are.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct Context {
    #[serde(default)]
    loops: Vec<Loop>,
    #[serde(flatten)]
    rest: Map<String, Value>,
}

/// The goals, ranked.
const GOALS: SettingKey<Vec<StoredGoal>> = SettingKey::new("goals");
/// The last goal id handed out.
const GOAL_SEQ: SettingKey<i64> = SettingKey::new("goalSeq");
/// The Focus items.
const FOCUS: SettingKey<Vec<FocusItem>> = SettingKey::new("focus");
/// The last Focus id handed out.
const FOCUS_SEQ: SettingKey<i64> = SettingKey::new("focusSeq");
/// The open loops, in the assistant's context.
const CONTEXT: SettingKey<Context> = SettingKey::new("context");
/// The last loop id handed out.
const LOOP_SEQ: SettingKey<i64> = SettingKey::new("loopSeq");
/// The ultimate concern.
const CONCERN: SettingKey<Concern> = SettingKey::new("concern");
/// The assistant's background jobs.
const JOBS: SettingKey<Vec<BackgroundTask>> = SettingKey::new("tasks");

/// The background jobs kept, newest last.
const JOBS_KEPT: usize = 20;
/// The finished background jobs the state shows.
const JOBS_SHOWN: usize = 8;

/// Why a change to the plan was refused.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum PlanError {
    /// The change breaks a rule; the text says which.
    #[error("{0}")]
    Refused(String),
    /// The database failed.
    #[error(transparent)]
    Db(#[from] DbError),
}

/// A goal to save: new (no id: title and why) or changed. A field left out stays; `status`
/// and `short` given as `None` are cleared.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GoalChange {
    /// The goal to change.
    pub id: Option<i64>,
    /// Its title.
    pub title: Option<String>,
    /// Why it matters.
    pub why: Option<String>,
    /// 1 matters most; the others move down.
    pub rank: Option<i64>,
    /// The assistant's line on how it goes.
    pub status: Option<Option<String>>,
    /// The label under its ring.
    pub short: Option<Option<String>>,
}

/// A Focus item to save: new (no id: title and line) or changed. Left out: unchanged.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FocusChange {
    /// The item to change.
    pub id: Option<i64>,
    /// Its title.
    pub title: Option<String>,
    /// `needs_you`, `moving`, `personal` or `quiet`.
    pub section: Option<String>,
    /// Where it stands.
    pub line: Option<String>,
    /// Its next step.
    pub next: Option<Option<String>>,
    /// Its chips.
    pub tags: Option<Vec<String>>,
    /// What it ties to.
    pub links: Option<Vec<FocusLink>>,
    /// Kept by the person.
    pub pinned: Option<bool>,
    /// A line for its timeline.
    pub happened: Option<String>,
    /// The goal it serves.
    pub goal: Option<Option<i64>>,
    /// Quiet until then.
    pub parked_until: Option<Option<String>>,
    /// Why.
    pub parked_why: Option<Option<String>>,
}

/// A loop to keep: new (no id) or changed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LoopChange {
    /// The loop to change.
    pub id: Option<i64>,
    /// Who it is about.
    pub who: String,
    /// What is open.
    pub what: String,
    /// When it is due.
    pub due: Option<String>,
    /// Where it came from.
    pub source: Option<String>,
    /// Its chat's id.
    pub chat: Option<String>,
    /// The room that opened it.
    pub room: Option<RoomId>,
}

/// Keeps the plan.
#[derive(Clone, Debug)]
pub struct PlanService {
    settings: SettingStore,
    log: LogStore,
    clock: Arc<dyn Clock>,
}

impl PlanService {
    /// A plan kept in `settings`, its changes noted in `log`.
    #[must_use]
    pub fn new(settings: SettingStore, log: LogStore, clock: Arc<dyn Clock>) -> Self {
        Self {
            settings,
            log,
            clock,
        }
    }

    fn now(&self) -> String {
        timestamp_to_api(self.clock.now()).as_str().to_owned()
    }

    async fn note(&self, text: String) -> Result<(), DbError> {
        self.log.add(text, self.clock.now()).await?;
        Ok(())
    }

    async fn next_id(&self, key: &SettingKey<i64>) -> Result<i64, DbError> {
        let id = self.settings.get(key).await?.unwrap_or(0) + 1;
        self.settings.set(key, &id).await?;
        Ok(id)
    }

    async fn stored_goals(&self) -> Result<Vec<StoredGoal>, DbError> {
        let mut goals = self.settings.get(&GOALS).await?.unwrap_or_default();
        goals.sort_by_key(|g| g.rank);
        Ok(goals)
    }

    /// The goals, ranked (their week's steps are the tasks' to count).
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn goals(&self) -> Result<Vec<Goal>, DbError> {
        Ok(self
            .stored_goals()
            .await?
            .into_iter()
            .map(|g| Goal {
                id: GoalId::new(g.id),
                title: g.title,
                why: g.why,
                rank: g.rank,
                status: g.status.flatten(),
                short: g.short.flatten(),
                at: Some(g.at.into()),
                week: emberhut_api::plan::GoalWeek::default(),
            })
            .collect())
    }

    /// Saves a goal; the others move down to make room for a rank. Returns the goal.
    ///
    /// # Errors
    ///
    /// [`PlanError::Refused`] for a goal that isn't there, or a new one without a title and a
    /// why; [`PlanError::Db`].
    pub async fn set_goal(&self, change: GoalChange) -> Result<Goal, PlanError> {
        let mut list = self.stored_goals().await?;
        let now = self.now();
        let index = if let Some(id) = change.id.filter(|id| *id != 0) {
            list.iter()
                .position(|g| g.id == id)
                .ok_or_else(|| PlanError::Refused(format!("no goal {id}")))?
        } else {
            let (Some(title), Some(why)) = (
                change.title.clone().filter(|t| !t.is_empty()),
                change.why.clone().filter(|w| !w.is_empty()),
            ) else {
                return Err(PlanError::Refused(
                    "a new goal needs a title and a why".to_owned(),
                ));
            };
            let id = self.next_id(&GOAL_SEQ).await?;
            list.push(StoredGoal {
                id,
                title,
                why,
                rank: i32::try_from(list.len() + 1).unwrap_or(i32::MAX),
                at: now.clone(),
                status: None,
                short: None,
            });
            list.len() - 1
        };
        let goal = &mut list[index];
        if let Some(title) = change.title {
            goal.title = title;
        }
        if let Some(why) = change.why {
            goal.why = why;
        }
        if let Some(status) = change.status {
            goal.status = Some(status);
        }
        if let Some(short) = change.short {
            goal.short = Some(short);
        }
        goal.at = now;
        let id = goal.id;
        let order = reranked(list.len(), index, change.rank);
        let mut ranked: Vec<StoredGoal> = order.into_iter().map(|i| list[i].clone()).collect();
        for (rank, goal) in ranked.iter_mut().enumerate() {
            goal.rank = i32::try_from(rank + 1).unwrap_or(i32::MAX);
        }
        self.settings.set(&GOALS, &ranked).await?;
        let saved = self
            .goals()
            .await?
            .into_iter()
            .find(|g| g.id.get() == id)
            .ok_or_else(|| PlanError::Refused(format!("no goal {id}")))?;
        self.note(format!(
            "Goal {}: {}",
            if change.id.is_some() {
                "updated"
            } else {
                "added"
            },
            saved.title
        ))
        .await?;
        Ok(saved)
    }

    /// Removes a goal; the Focus items it had keep going, untied.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn drop_goal(&self, id: i64) -> Result<(), DbError> {
        let mut list: Vec<StoredGoal> = self
            .stored_goals()
            .await?
            .into_iter()
            .filter(|g| g.id != id)
            .collect();
        for (rank, goal) in list.iter_mut().enumerate() {
            goal.rank = i32::try_from(rank + 1).unwrap_or(i32::MAX);
        }
        self.settings.set(&GOALS, &list).await?;
        let mut focus = self.settings.get(&FOCUS).await?.unwrap_or_default();
        for item in &mut focus {
            if item.goal.is_some_and(|goal| goal.get() == id) {
                item.goal = None;
            }
        }
        self.settings.set(&FOCUS, &focus).await?;
        self.note("Goal removed".to_owned()).await
    }

    /// The Focus items, freshest first.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn focus(&self) -> Result<Vec<FocusItem>, DbError> {
        let mut items = self.settings.get(&FOCUS).await?.unwrap_or_default();
        items.sort_by(|a, b| b.at.cmp(&a.at));
        Ok(items)
    }

    /// Saves a Focus item: `happened` adds a line to the top of its timeline. Returns it.
    ///
    /// # Errors
    ///
    /// [`PlanError::Refused`] for an item that isn't there, or a new one without a title and a
    /// line; [`PlanError::Db`].
    pub async fn set_focus(&self, change: FocusChange) -> Result<FocusItem, PlanError> {
        let mut list = self.settings.get(&FOCUS).await?.unwrap_or_default();
        let now = self.now();
        let index = if let Some(id) = change.id.filter(|id| *id != 0) {
            list.iter()
                .position(|f| f.id == id)
                .ok_or_else(|| PlanError::Refused(format!("no focus item {id}")))?
        } else {
            let (Some(title), Some(line)) = (
                change.title.clone().filter(|t| !t.is_empty()),
                change.line.clone().filter(|l| !l.is_empty()),
            ) else {
                return Err(PlanError::Refused(
                    "a new item needs a title and a line".to_owned(),
                ));
            };
            let id = self.next_id(&FOCUS_SEQ).await?;
            list.push(FocusItem {
                goal: None,
                parked_until: None,
                parked_why: None,
                id,
                title,
                section: change
                    .section
                    .clone()
                    .unwrap_or_else(|| "moving".to_owned()),
                line,
                next: None,
                tags: Vec::new(),
                links: Vec::new(),
                pinned: false,
                since: now.clone().into(),
                at: now.clone().into(),
                timeline: Vec::new(),
            });
            list.len() - 1
        };
        let item = &mut list[index];
        if let Some(title) = change.title {
            item.title = title;
        }
        if let Some(section) = change.section {
            item.section = section;
        }
        if let Some(line) = change.line {
            item.line = line;
        }
        if let Some(next) = change.next {
            item.next = next;
        }
        if let Some(tags) = change.tags {
            item.tags = tags;
        }
        if let Some(links) = change.links {
            item.links = links;
        }
        if let Some(pinned) = change.pinned {
            item.pinned = pinned;
        }
        if let Some(goal) = change.goal {
            item.goal = goal.map(GoalId::new);
        }
        if let Some(until) = change.parked_until {
            item.parked_until = until;
        }
        if let Some(why) = change.parked_why {
            item.parked_why = why;
        }
        item.at = now.clone().into();
        if let Some(happened) = change.happened.filter(|h| !h.is_empty()) {
            item.timeline.insert(
                0,
                FocusEvent {
                    at: now.into(),
                    text: happened,
                },
            );
            item.timeline.truncate(TIMELINE_LINES);
        }
        let saved = item.clone();
        self.settings.set(&FOCUS, &list).await?;
        Ok(saved)
    }

    /// Takes a Focus item off the plan. The assistant drops only what the person didn't pin,
    /// unless they asked; the app's drop is theirs.
    ///
    /// # Errors
    ///
    /// [`PlanError::Refused`] for an item that isn't there, or a pinned one the person didn't
    /// ask to drop; [`PlanError::Db`].
    pub async fn drop_focus(&self, id: i64, by_person: bool) -> Result<(), PlanError> {
        let list = self.settings.get(&FOCUS).await?.unwrap_or_default();
        let item = list
            .iter()
            .find(|f| f.id == id)
            .ok_or_else(|| PlanError::Refused(format!("no focus item {id}")))?;
        if item.pinned && !by_person {
            return Err(PlanError::Refused(format!(
                "\"{}\" is pinned by the person: drop it only when they ask (owner_asked: true)",
                item.title
            )));
        }
        let kept: Vec<FocusItem> = list.into_iter().filter(|f| f.id != id).collect();
        self.settings.set(&FOCUS, &kept).await?;
        Ok(())
    }

    /// Pins a Focus item (or unpins it).
    ///
    /// # Errors
    ///
    /// [`PlanError::Refused`] for an item that isn't there; [`PlanError::Db`].
    pub async fn pin_focus(&self, id: i64, pinned: bool) -> Result<(), PlanError> {
        let mut list = self.settings.get(&FOCUS).await?.unwrap_or_default();
        let item = list
            .iter_mut()
            .find(|f| f.id == id)
            .ok_or_else(|| PlanError::Refused(format!("no focus item {id}")))?;
        item.pinned = pinned;
        self.settings.set(&FOCUS, &list).await?;
        Ok(())
    }

    /// Whether a Focus item is parked now.
    #[must_use]
    pub fn is_parked(&self, item: &FocusItem) -> bool {
        parked(item.parked_until.as_deref(), &self.now())
    }

    /// The open loops.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn loops(&self) -> Result<Vec<Loop>, DbError> {
        Ok(self.settings.get(&CONTEXT).await?.unwrap_or_default().loops)
    }

    /// Keeps a loop (the newest 80 stay); returns its id.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn set_loop(&self, change: LoopChange) -> Result<i64, DbError> {
        let mut context = self.settings.get(&CONTEXT).await?.unwrap_or_default();
        let now = self.now();
        let known = change
            .id
            .filter(|id| *id != 0)
            .and_then(|id| context.loops.iter_mut().find(|l| l.id == id));
        let id = if let Some(known) = known {
            known.who = change.who;
            known.what = change.what;
            if change.due.is_some() {
                known.due = change.due;
            }
            if let Some(source) = change.source {
                known.source = source;
            }
            if change.chat.is_some() {
                known.chat = change.chat;
            }
            known.touched = Some(now.into());
            known.maybe = None;
            known.id
        } else {
            let id = self.next_id(&LOOP_SEQ).await?;
            context.loops.push(Loop {
                id,
                who: change.who,
                what: change.what,
                due: change.due,
                source: change.source.unwrap_or_default(),
                since: now.into(),
                chat: change.chat.filter(|c| !c.is_empty()),
                touched: None,
                maybe: None,
                room: change.room.filter(|room| !room.is_main()),
            });
            id
        };
        let excess = context.loops.len().saturating_sub(LOOPS_KEPT);
        context.loops.drain(..excess);
        self.settings.set(&CONTEXT, &context).await?;
        Ok(id)
    }

    /// Closes a loop.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn close_loop(&self, id: i64) -> Result<(), DbError> {
        let mut context = self.settings.get(&CONTEXT).await?.unwrap_or_default();
        context.loops.retain(|l| l.id != id);
        self.settings.set(&CONTEXT, &context).await
    }

    /// The ultimate concern, once the person stated it.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn concern(&self) -> Result<Option<Concern>, DbError> {
        self.settings.get(&CONCERN).await
    }

    /// Saves the ultimate concern.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn set_concern(&self, title: String, why: String) -> Result<(), DbError> {
        self.note(format!("Ultimate concern: {title}")).await?;
        self.settings.set(&CONCERN, &Concern { title, why }).await
    }

    /// The background jobs the state shows: the running ones, then the last ones finished,
    /// newest first.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn jobs(&self) -> Result<Vec<BackgroundTask>, DbError> {
        let jobs = self.settings.get(&JOBS).await?.unwrap_or_default();
        let (running, ended): (Vec<_>, Vec<_>) =
            jobs.into_iter().partition(BackgroundTask::is_running);
        Ok(running
            .into_iter()
            .chain(ended.into_iter().rev().take(JOBS_SHOWN))
            .collect())
    }

    /// A room's session has these jobs running now (its list replaces the running ones it had;
    /// other rooms' stay). For the assistant's session to call as its jobs change.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn jobs_running(
        &self,
        room: RoomId,
        live: Vec<BackgroundTask>,
    ) -> Result<(), DbError> {
        let jobs = self.settings.get(&JOBS).await?.unwrap_or_default();
        let mut kept: Vec<BackgroundTask> = jobs
            .into_iter()
            .filter(|job| !job.is_running() || job.room.unwrap_or_default() != room)
            .collect();
        kept.extend(live.into_iter().map(|job| BackgroundTask {
            status: WorkStatus::Running,
            room: Some(room),
            ..job
        }));
        self.save_jobs(kept).await
    }

    /// A job ended with `status` and its summary (500 characters at most).
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn job_ended(&self, id: &str, status: &str, summary: &str) -> Result<(), DbError> {
        let mut jobs = self.settings.get(&JOBS).await?.unwrap_or_default();
        let Some(index) = jobs.iter().position(|job| job.id == id) else {
            return Ok(());
        };
        let mut job = jobs.remove(index);
        job.status = WorkStatus::from(status);
        job.ended_at = Some(self.now().into());
        job.summary = Some(summary.chars().take(500).collect());
        let said = match status {
            "completed" => "Finished",
            "failed" => "Failed",
            _ => "Stopped",
        };
        let line = format!("{said}: {}", job.description);
        jobs.push(job);
        self.save_jobs(jobs).await?;
        self.note(line).await
    }

    async fn save_jobs(&self, mut jobs: Vec<BackgroundTask>) -> Result<(), DbError> {
        let excess = jobs.len().saturating_sub(JOBS_KEPT);
        jobs.drain(..excess);
        self.settings.set(&JOBS, &jobs).await
    }
}
