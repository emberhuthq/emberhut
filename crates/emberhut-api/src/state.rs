//! `GET /api/state?room=`: everything the app shows, for the room it has open.
//!
//! `chat`, `earlier`, `unread`, `busy` and `live` are that room's; the rest is everyone's. A server
//! may send more than this type reads; a field this type doesn't know is skipped, and a field the server leaves out takes its default, so a newer or
//! an older server still reads. Serialized, it gives the same shape for every field it has.
//!
//! The answer carries an `ETag`. Sent back in `If-None-Match`, an unchanged state comes back as
//! 304 with no body: nothing to download or decode on a 3-second poll.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::agents::AgentRow;
use crate::approvals::Approval;
use crate::areas::Area;
use crate::auth::AuthInfo;
use crate::background::BackgroundTask;
use crate::browser::BrowserInfo;
use crate::builds::LatestBuilds;
use crate::calls::CallRecording;
use crate::chat::ChatMessage;
use crate::chrome::{ChromePairing, ChromeProfile};
use crate::health_today::HealthToday;
use crate::ids::{RoomId, TaskId};
use crate::live::Live;
use crate::plan::{FocusItem, Loop, Todo, WeekReport};
use crate::rooms::RoomRow;
use crate::runner::accounts::ClaudeAccount;
use crate::runner::agents::AgentNow;
use crate::tasks::{Goal, Issue, Project, TaskStatus};
use crate::theme::ThemeChoice;
use crate::threads::{CodingThread, ExternalThread, FollowedThread};
use crate::time::Timestamp;
use crate::usage::{AssistantAccount, UsageRow};
use crate::voice::LastCall;
use crate::wake::{AwarenessSource, Checkin, DidLine, Watch};
use crate::work::{RunnerSession, WorkRow};

/// The state of one room, and everything around it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[expect(
    clippy::struct_excessive_bools,
    reason = "the wire shape: each flag is a field the server sends"
)]
pub struct State {
    /// The room this state is for.
    pub room: RoomId,
    /// Every room's line for the switcher: Main first, then by last activity.
    pub rooms: Vec<RoomRow>,
    /// The assistant is on (not paused).
    pub on: bool,
    /// It is working in this room now.
    pub busy: bool,
    /// What it is doing in this room, while it works.
    pub live: Option<Live>,
    /// This room has answers not seen yet: send `POST /api/seen` once they are on screen.
    pub unread: bool,
    /// Events the assistant hasn't looked at yet.
    pub pending: u32,
    /// The room's newest 60 messages, oldest first.
    pub chat: Vec<ChatMessage>,
    /// Older messages than these exist ("Show earlier").
    pub earlier: bool,
    /// Every task: the open ones and this week's done ones.
    #[serde(default)]
    pub issues: Vec<Issue>,
    /// Everything in flight, waiting on the person first.
    #[serde(default)]
    pub working: Vec<WorkRow>,
    /// Sessions on the runners, every room's: running ones first.
    #[serde(default)]
    pub sessions: Vec<RunnerSession>,
    /// Today's task ids, ranked.
    #[serde(default)]
    pub today: Vec<TaskId>,
    /// The projects.
    #[serde(default)]
    pub projects: Vec<Project>,
    /// The goals, ranked.
    #[serde(default)]
    pub goals: Vec<Goal>,
    /// The theme choice: a newer `at` than the client's means fetch `GET /api/theme`.
    #[serde(default)]
    pub theme: Option<ThemeChoice>,
    /// What the app hides for now.
    #[serde(default)]
    pub ui: Option<DisplayUi>,
    /// The account the assistant's turns run on, and its use.
    #[serde(default)]
    pub account: Option<Account>,
    /// Sign-in: the mode and the signed-in devices.
    #[serde(default)]
    pub auth: Option<AuthInfo>,
    /// A call being recorded on the Mac now, if one is.
    #[serde(default)]
    pub recording: Option<CallRecording>,
    /// The server can push alerts to phones: it has the keys of APNs, FCM or both. When it
    /// can't, the app says so next to its notification setting.
    #[serde(default)]
    pub push: bool,
    /// The newest build of the app per platform: an older app offers the update.
    #[serde(default)]
    pub builds: Option<LatestBuilds>,
    /// Every coding thread the server knows about, helpers' left out: the newest 40, most
    /// recently changed first.
    #[serde(default)]
    pub threads: Vec<CodingThread>,
    /// The threads the assistant handed work to and follows, the longest followed first.
    #[serde(default)]
    pub followed: Vec<FollowedThread>,
    /// The coding-thread app's threads still running, queued or waiting, newest first (30 at
    /// most; Main: every room's). Its wire name is listed in [`crate::wire`].
    #[serde(default)]
    pub t3_running: Vec<ExternalThread>,
    /// The assistant's background work: the running ones first, then the last that ended.
    /// Not the person's tasks, which are `issues`.
    #[serde(default)]
    pub tasks: Vec<BackgroundTask>,
    /// The person's paired Chrome profiles.
    #[serde(default)]
    pub chrome: Vec<ChromeProfile>,
    /// The Chrome pairing links made in the last while: waiting, or used.
    #[serde(default)]
    pub chrome_pairing: Vec<ChromePairing>,
    /// The Chrome extension's newest version (`0` when the server doesn't know it).
    #[serde(default)]
    pub chrome_latest: Option<String>,
    /// Today's numbers from the person's phone; none before it sent any.
    #[serde(default)]
    pub health: Option<HealthToday>,
    /// Each goal's list this week: open tasks first, then this week's done ones.
    #[serde(default)]
    pub todos: Vec<Todo>,
    /// Each project's areas.
    #[serde(default)]
    pub areas: Vec<Area>,
    /// The weekly report.
    #[serde(default)]
    pub report: Option<WeekReport>,
    /// Every agent working now across the person's tasks.
    #[serde(default)]
    pub agents_now: Vec<AgentNow>,
    /// The person's own sessions, every room's, newest first.
    #[serde(default)]
    pub my_sessions: Vec<RunnerSession>,
    /// The Claude accounts the assistant and its runners use.
    #[serde(default)]
    pub accounts: Vec<ClaudeAccount>,
    /// Every Claude account the assistant may run on, with its use (the usage card).
    #[serde(default)]
    pub usage: Vec<UsageRow>,
    /// The account the assistant runs on, Auto or pinned. Its field name on the wire is listed
    /// in [`crate::wire`], which checks it.
    #[serde(default, rename = "assistantAccount")]
    pub assistant_account: Option<AssistantAccount>,
    /// The assistant's own browser: where it is and who drives; none from a server without one.
    #[serde(default)]
    pub browser: Option<BrowserInfo>,
    /// The open loops the assistant keeps: who waits on the person, promises, deadlines.
    #[serde(default)]
    pub loops: Vec<Loop>,
    /// Where this week starts (Sunday, 00:00 on the server's clock).
    #[serde(default)]
    pub week_start: Option<Timestamp>,
    /// What the assistant keeps an eye on behind the scenes, freshest first.
    #[serde(default)]
    pub focus: Vec<FocusItem>,
    /// The assistant's scheduled check-ins, soonest first.
    #[serde(default)]
    pub checkins: Vec<Checkin>,
    /// Its watches, by number.
    #[serde(default)]
    pub watches: Vec<Watch>,
    /// What it did lately, newest first (40 lines).
    #[serde(default)]
    pub did: Vec<DidLine>,
    /// Each source it reads, and how fresh it is.
    #[serde(default)]
    pub awareness: Vec<AwarenessSource>,
    /// When the assistant's awareness of the owner's chats (its open loops and notes) was last
    /// brought up to date; none: never.
    #[serde(default)]
    pub context_at: Option<Timestamp>,
    /// Each machine the server reaches for coding threads (the coding-thread app's and the
    /// runners'), and its error at the last look (`null`: it answered).
    #[serde(default, deserialize_with = "machine_errors")]
    pub machines: BTreeMap<String, Option<String>>,
    /// Voice calls and dictation work: the server has a speech-to-text and a realtime voice
    /// provider.
    #[serde(default)]
    pub voice: bool,
    /// The phone's report of the last voice call.
    #[serde(default)]
    pub last_call: Option<LastCall>,
    /// When the phone last sent health days.
    #[serde(default)]
    pub health_at: Option<Timestamp>,
    /// When the Mac last sent screen time.
    #[serde(default)]
    pub screentime_at: Option<Timestamp>,
    /// Every agent, the default agent first, with how many of its requests wait on an answer.
    /// Left out by a server without agents.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub agents: Vec<AgentRow>,
    /// The approval requests waiting on an answer, newest first. Left out when there are none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub approvals: Vec<Approval>,
}

impl State {
    /// The tasks that are open (not done or canceled) and not subtasks.
    pub fn open_tasks(&self) -> impl Iterator<Item = &Issue> {
        self.issues.iter().filter(|t| {
            t.parent.is_none() && !matches!(t.status, TaskStatus::Done | TaskStatus::Canceled)
        })
    }

    /// This state's line for its own room.
    #[must_use]
    pub fn current_room(&self) -> Option<&RoomRow> {
        self.rooms.iter().find(|r| r.id == self.room)
    }
}

/// The state's `machines`, read leniently: an entry whose value is neither text nor `null`
/// (a newer server's) is left out.
fn machine_errors<'de, D>(deserializer: D) -> Result<BTreeMap<String, Option<String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let all: BTreeMap<String, serde_json::Value> = Deserialize::deserialize(deserializer)?;
    Ok(all
        .into_iter()
        .filter_map(|(machine, error)| match error {
            serde_json::Value::Null => Some((machine, None)),
            serde_json::Value::String(error) => Some((machine, Some(error))),
            _ => None,
        })
        .collect())
}

/// What the app hides for now (the data stays; only the screens hide it).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct DisplayUi {
    /// Hide the goals.
    #[serde(default)]
    pub hide_goals: bool,
    /// Hide the projects.
    #[serde(default)]
    pub hide_projects: bool,
    /// Tasks carry plans: show tasks as the work running, each with its plan, and the
    /// sessions inside them rather than as a list of their own.
    #[serde(default)]
    pub task_plans: bool,
}

/// The account the assistant's turns run on and its use, from the last turn. Every field is
/// `null` until the first turn.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Account {
    /// The account's id.
    #[serde(default)]
    pub id: Option<String>,
    /// Its email.
    #[serde(default)]
    pub email: Option<String>,
    /// Its plan.
    #[serde(default)]
    pub plan: Option<String>,
    /// Whether it is logged in.
    #[serde(default)]
    pub logged_in: Option<bool>,
    /// Its use of the 5-hour window, 0 to 1.
    #[serde(default)]
    pub used5h: Option<f64>,
    /// When that window resets.
    #[serde(default)]
    pub resets5h: Option<Timestamp>,
    /// Its use of the week, 0 to 1.
    #[serde(default)]
    pub used7d: Option<f64>,
    /// When the week resets.
    #[serde(default)]
    pub resets7d: Option<Timestamp>,
    /// It is out until then.
    #[serde(default)]
    pub limited_until: Option<Timestamp>,
    /// When these figures were read.
    #[serde(default)]
    pub at: Option<Timestamp>,
}
