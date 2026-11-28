//! The app's state for a room (`GET /api/state`): what it shows, built from the stores and the
//! live values on each request.

use std::collections::BTreeMap;
use std::sync::Arc;

use emberhut_api as api;
use emberhut_api::ids::ProjectId;
use emberhut_api::rooms::RoomRow;
use emberhut_api::state::State;
use emberhut_api::tasks::ProjectStatus;
use emberhut_core::message::{Author, preview};
use emberhut_core::upload::StoredFile;
use emberhut_core::{Clock, MessageId, RoomId};
use emberhut_db::{DbError, LogStore, MessageRefStore, RoomLine, RoomStore, SnapshotStore};

use crate::config::AuthMode;
use crate::convert::{device_to_api, room_to_api, timestamp_to_api};
use crate::services::accounts::AccountService;
use crate::services::auth::{AuthError, AuthService};
use crate::services::awareness::AwarenessService;
use crate::services::browser::BrowserService;
use crate::services::calls::CallService;
use crate::services::chat::{chat_lines, one_line};
use crate::services::context::ContextService;
use crate::services::display::DisplayService;
use crate::services::ember_name::EmberNameService;
use crate::services::health_data::HealthDataService;
use crate::services::live::LiveHub;
use crate::services::plan::PlanService;
use crate::services::push::PushService;
use crate::services::runners::RunnerService;
use crate::services::settings_build::BuildService;
use crate::services::tasks::TaskService;
use crate::services::theme::ThemeService;
use crate::services::threads::{ThreadError, ThreadService};
use crate::services::voice::VoiceService;
use crate::services::wake::WakeService;

/// Characters of a room's last message shown in the switcher.
const LAST_LENGTH: usize = 120;
/// A room's last line when its last message is files with no text.
const SENT_A_FILE: &str = "Sent a file";
/// Lines of the activity log in the state's `did`.
const DID_LINES: u64 = 40;

/// Why the state could not be built.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum StateError {
    /// The database failed.
    #[error("database error")]
    Database(#[from] DbError),
    /// The device list could not be read.
    #[error("could not list the devices")]
    Devices(#[from] AuthError),
    /// The coding threads could not be read.
    #[error("could not read the coding threads")]
    Threads(#[from] ThreadError),
    /// A section's service failed (`.map_err(StateError::section)?` where its error is not a
    /// database error).
    #[error("a section of the state could not be read")]
    Section(#[source] Box<dyn std::error::Error + Send + Sync>),
}

impl StateError {
    /// A section's failure, caused by `error`.
    pub fn section(error: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self::Section(Box::new(error))
    }
}

/// The services the state's sections are read from, besides the rooms and the chat.
///
/// Filling a state key is three lines: the area's service as a field here, its line where
/// `AppState::new` makes this, and the key read from it in [`StateService::state`]'s `State`,
/// in place of its stand-in.
#[derive(Clone, Debug)]
pub struct Sections {
    /// `ui`: what the app hides.
    pub display: DisplayService,
    /// `theme`: the theme choice.
    pub theme: ThemeService,
    /// `emberName`: what the owner calls their ember.
    pub ember_name: EmberNameService,
    /// `builds`: the newest build of the app per platform.
    pub builds: BuildService,
    /// `threads`, `followed` and `t3Running`: the coding-thread app's threads.
    pub threads: ThreadService,
    /// `issues`, `today`, `projects`, `goals`, `todos`, `weekStart`, `areas` and `report`.
    pub tasks: TaskService,
    /// `loops`, `focus` and `tasks` (the assistant's background jobs).
    pub plan: PlanService,
    /// `on`, `pending`, `checkins` and `watches`: the wake rule.
    pub wake: WakeService,
    /// `did`: the activity log.
    pub log: LogStore,
    /// `awareness`: how fresh each source is.
    pub awareness: AwarenessService,
    /// The chips on `chat`'s answers.
    pub refs: MessageRefStore,
    /// `contextAt`: when the assistant's awareness was last brought up to date.
    pub context: ContextService,
    /// `sessions`, `mySessions` and the runners' part of `machines`: the runners' sessions.
    pub runners: RunnerService,
    /// `accounts`, `usage`, `assistantAccount` and `account`: the assistant's accounts.
    pub accounts: AccountService,
    /// `recording`: the call being recorded on the Mac.
    pub calls: CallService,
    /// `voice` and `lastCall`: whether dictation works, and the last voice call.
    pub voice: VoiceService,
    /// `healthAt`, `health` and `screentimeAt`: the phone's health and the Mac's screen time.
    pub health_data: HealthDataService,
    /// `working` and `agentsNow`: the work in flight, and the agents running now.
    pub workers: crate::services::workers::WorkerService,
    /// `browser`, `chrome`, `chromePairing` and `chromeLatest`: the assistant's browser and the
    /// owner's Chrome profiles.
    pub browser: BrowserService,
    /// `agents`: the configured agents, when there is one besides the default.
    pub agents: crate::services::agents::AgentService,
    /// `approvals`: the requests waiting on an answer.
    pub approvals: crate::services::approvals::ApprovalService,
}

/// Builds the state.
#[derive(Clone, Debug)]
pub struct StateService {
    rooms: RoomStore,
    snapshots: SnapshotStore,
    live: LiveHub,
    auth: AuthService,
    auth_mode: AuthMode,
    clock: Arc<dyn Clock>,
    sections: Sections,
    push: Option<PushService>,
}

impl StateService {
    /// A service over the stores, the live values, the device sessions and the sections'
    /// services, telling the time with `clock`.
    #[must_use]
    pub fn new(
        rooms: RoomStore,
        snapshots: SnapshotStore,
        live: LiveHub,
        auth: AuthService,
        auth_mode: AuthMode,
        clock: Arc<dyn Clock>,
        sections: Sections,
    ) -> Self {
        Self {
            rooms,
            snapshots,
            live,
            auth,
            auth_mode,
            clock,
            sections,
            push: None,
        }
    }

    /// The same service, saying in the state's `push` whether `push` can push.
    #[must_use]
    pub fn with_push(mut self, push: PushService) -> Self {
        self.push = Some(push);
        self
    }

    /// The room `asked` for, when it exists; Main otherwise (the contract has no error for an
    /// unknown room).
    ///
    /// # Errors
    ///
    /// [`StateError::Database`].
    pub async fn resolve(&self, asked: Option<RoomId>) -> Result<RoomId, StateError> {
        match asked {
            Some(room) if self.rooms.get(room).await?.is_some() => Ok(room),
            _ => Ok(RoomId::MAIN),
        }
    }

    /// The state for `room` as a request by `viewer` reads it: [`Self::state`], without what
    /// only a signed-in device reads (the agents and the pending approvals) for a request
    /// without a session, which report mode lets through.
    ///
    /// # Errors
    ///
    /// [`StateError`] when a store fails.
    pub async fn state_for(
        &self,
        room: RoomId,
        viewer: Option<emberhut_core::DeviceId>,
    ) -> Result<State, StateError> {
        let mut state = self.state(room).await?;
        if viewer.is_none() {
            state.agents.clear();
            state.approvals.clear();
        }
        Ok(state)
    }

    /// The state for `room` (which must exist: see [`Self::resolve`]), as a signed-in device
    /// reads it.
    ///
    /// # Errors
    ///
    /// [`StateError`] when a store fails.
    pub async fn state(&self, room: RoomId) -> Result<State, StateError> {
        let now = self.clock.now();
        let snapshot = self.snapshots.read(room).await?;
        let unread = snapshot
            .rooms
            .iter()
            .any(|line| line.room.id == room && line.unread > 0);
        let mut chat = chat_lines(&snapshot.chat, &snapshot.quoted, &snapshot.files, now);
        crate::services::refs::fill(&self.sections.refs, &mut chat).await?;
        let earlier = snapshot.earlier;
        let live = self.live.now(room);
        let threads = self.sections.threads.recent().await?;
        let followed = self.sections.threads.followed().await?;
        let running = self.sections.threads.running(room).await?;
        let plan = self
            .sections
            .tasks
            .plan_state()
            .await
            .map_err(StateError::section)?;

        let rows = self.room_rows(&snapshot, &plan, now);
        let working = self.sections.workers.working(&plan.issues, &threads);
        Ok(State {
            room: room_to_api(room),
            rooms: rows,
            on: self.sections.wake.is_on().await?,
            busy: live.is_some(),
            live,
            unread,
            pending: u32::try_from(self.sections.wake.pending().await?).unwrap_or(u32::MAX),
            chat,
            earlier,
            issues: plan.issues,
            working,
            agents_now: self.sections.workers.agents_now().await?,
            sessions: self.sections.runners.state_sessions().await?,
            today: plan.today,
            projects: plan.projects,
            goals: plan.goals,
            theme: Some(self.sections.theme.choice().await?),
            ember_name: Some(self.sections.ember_name.name().await?),
            ui: Some(self.sections.display.ui().await?),
            account: Some(self.sections.accounts.state_account().await?),
            auth: Some(self.auth_info().await?),
            push: match &self.push {
                Some(push) => push.ready().await,
                None => false,
            },
            builds: Some(self.sections.builds.latest().await?),
            threads,
            followed,
            t3_running: running,
            loops: self.sections.plan.loops().await?,
            todos: plan.todos,
            week_start: plan.week_start,
            focus: self.sections.plan.focus().await?,
            areas: plan.areas,
            report: plan.report,
            tasks: self.sections.plan.jobs().await?,
            checkins: self.sections.wake.checkins().await?,
            watches: self.sections.wake.watches().await?,
            did: self.did(now).await?,
            awareness: self.sections.awareness.sources(now).await?,
            context_at: self.sections.context.context_at().await?,
            my_sessions: self.sections.runners.my_sessions().await?,
            machines: self.machine_errors(),
            accounts: self.sections.accounts.all().await?,
            usage: self.sections.accounts.usage_card().await?,
            assistant_account: Some(self.sections.accounts.assistant().await?),
            voice: self.sections.voice.ready().await,
            last_call: self.sections.voice.last_call().await?,
            recording: self.sections.calls.recording().await?,
            health_at: self.sections.health_data.health_at().await?,
            health: Some(
                self.sections
                    .health_data
                    .now()
                    .await
                    .map_err(StateError::section)?,
            ),
            screentime_at: self.sections.health_data.screentime_at().await?,
            browser: Some(self.sections.browser.summary().await),
            chrome: self.sections.browser.chrome().summary().await?,
            chrome_pairing: self.sections.browser.chrome().pairings(),
            chrome_latest: Some(self.sections.browser.chrome().latest()),
            agents: self.sections.agents.state_rows().await?,
            approvals: self
                .sections
                .approvals
                .pending()
                .await
                .map_err(StateError::section)?,
        })
    }

    /// The activity log's newest [`DID_LINES`] lines, newest first.
    async fn did(&self, now: jiff::Timestamp) -> Result<Vec<api::wake::DidLine>, DbError> {
        Ok(self
            .sections
            .log
            .recent(DID_LINES)
            .await?
            .into_iter()
            .map(|line| api::wake::DidLine {
                time: emberhut_core::message::age_label(line.at, now),
                at: timestamp_to_api(line.at),
                text: line.text,
            })
            .collect())
    }

    /// Each machine the server reaches for coding threads and its error at the last look: the
    /// coding-thread app's machines, then the runners.
    fn machine_errors(&self) -> BTreeMap<String, Option<String>> {
        let mut errors = self.sections.threads.machine_errors();
        errors.extend(self.sections.runners.errors());
        errors
    }

    /// Every room's line for the switcher, in its order, with its project and its asks from
    /// the board `plan`.
    fn room_rows(
        &self,
        snapshot: &emberhut_db::Snapshot,
        plan: &crate::services::tasks::PlanState,
        now: jiff::Timestamp,
    ) -> Vec<RoomRow> {
        let mut rows: Vec<RoomRow> = snapshot
            .rooms
            .iter()
            .map(|line| self.room_row(line, &snapshot.files))
            .collect();
        let projects_of: BTreeMap<i64, ProjectId> = snapshot
            .rooms
            .iter()
            .filter_map(|line| Some((line.room.id.get(), ProjectId::new(line.room.project?.get()))))
            .collect();
        let today = now
            .to_zoned_datetime(self.sections.tasks.zone().clone())
            .date();
        crate::services::room_asks::fill(
            &mut rows,
            &projects_of,
            &plan.projects,
            &plan.issues,
            today,
        );
        sort_rooms(&mut rows);
        rows
    }

    /// A room's line for the switcher.
    fn room_row(&self, entry: &RoomLine, files: &BTreeMap<MessageId, Vec<StoredFile>>) -> RoomRow {
        let RoomLine { room, last, unread } = entry;
        let live = self.live.now(room.id);
        RoomRow {
            id: room_to_api(room.id),
            name: room.name.clone(),
            project: None,
            goal: None,
            active: true,
            status: ProjectStatus::Active,
            paused_until: None,
            unread: u32::try_from(*unread).unwrap_or(u32::MAX),
            needs: 0,
            ask: None,
            ask_id: None,
            busy: live.is_some(),
            doing: live.and_then(|live| live.doing),
            last: last.as_ref().map(|message| {
                let text = one_line(&message.text);
                if text.is_empty() && files.contains_key(&message.id) {
                    SENT_A_FILE.to_owned()
                } else {
                    preview(&text, LAST_LENGTH)
                }
            }),
            last_at: last
                .as_ref()
                .map(|message| timestamp_to_api(message.created_at)),
            last_mine: last
                .as_ref()
                .is_some_and(|message| message.author == Author::User),
        }
    }

    /// Sign-in as the state shows it: the mode and the signed-in devices.
    ///
    /// # Errors
    ///
    /// [`StateError::Devices`] when the devices can't be read.
    pub async fn auth_info(&self) -> Result<api::auth::AuthInfo, StateError> {
        let devices = self.auth.devices().await?;
        Ok(api::auth::AuthInfo {
            mode: self.auth_mode.to_api(),
            // emberhut binds no outside identity: a server with a signed-in device is in use.
            bound: !devices.is_empty(),
            devices: devices
                .into_iter()
                .map(|device| api::auth::Device {
                    id: device_to_api(device.id),
                    name: Some(device.name),
                    model: Some(device.model),
                    created: Some(timestamp_to_api(device.created_at)),
                    seen: Some(timestamp_to_api(device.seen_at)),
                })
                .collect(),
        })
    }
}

/// Main first, then the rooms most recently talked in, then the rooms with nothing said by name.
fn sort_rooms(rows: &mut [RoomRow]) {
    rows.sort_by(|a, b| {
        b.is_main()
            .cmp(&a.is_main())
            .then_with(|| b.last_at.cmp(&a.last_at))
            .then_with(|| a.name.cmp(&b.name))
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: i64, name: &str, last_at: Option<&str>) -> RoomRow {
        RoomRow {
            id: api::ids::RoomId::new(id),
            name: name.to_owned(),
            project: None,
            goal: None,
            active: true,
            status: ProjectStatus::Active,
            paused_until: None,
            unread: 0,
            needs: 0,
            ask: None,
            ask_id: None,
            busy: false,
            doing: None,
            last: None,
            last_at: last_at.map(api::time::Timestamp::from),
            last_mine: false,
        }
    }

    #[test]
    fn main_first_then_by_last_activity_then_by_name() {
        let mut rows = [
            row(4, "Room 4", None),
            row(2, "Room 2", Some("2026-11-24T10:00:00.000Z")),
            row(3, "Room 3", Some("2026-11-24T11:00:00.000Z")),
            row(5, "Atrium", None),
            row(1, "Main", None),
        ];
        sort_rooms(&mut rows);
        let ids: Vec<i64> = rows.iter().map(|r| r.id.get()).collect();
        assert_eq!(ids, [1, 3, 2, 5, 4]);
    }
}
