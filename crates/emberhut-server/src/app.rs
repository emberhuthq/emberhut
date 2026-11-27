//! The app: its state (the services, wired once) and the router with its middleware.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::extract::{MatchedPath, State};
use axum::http::{HeaderName, Method, Request, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use emberhut_api::paths;
use emberhut_core::lag::masked_path;
use emberhut_core::{Clock, OsRandom, RandomSource, SystemClock};
use emberhut_db::Database;
use emberhut_identity::Scheme;
use tokio_util::sync::CancellationToken;
use tower::ServiceBuilder;
use tower_http::catch_panic::CatchPanicLayer;
use tower_http::compression::CompressionLayer;
use tower_http::compression::predicate::{
    DefaultPredicate, NotForContentType, Predicate, SizeAbove,
};
use tower_http::request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer};
use tower_http::sensitive_headers::SetSensitiveRequestHeadersLayer;
use tower_http::trace::{DefaultOnResponse, TraceLayer};
use tracing::Level;

use crate::config::AuthMode;
use crate::error::ApiError;
use crate::identity::Machine;
use crate::mcp::tokens::AgentTokens;
use crate::routes;
use crate::services::accounts::AccountService;
use crate::services::activity::ActivityLog;
use crate::services::agents::scope::AgentRoots;
use crate::services::agents::slack::{AgentSlack, NoAgentSlack};
use crate::services::agents::{AgentConversations, AgentService, ConversationParts};
use crate::services::approvals::channels::ApprovalDeps;
use crate::services::approvals::{ApprovalParts, ApprovalService};
use crate::services::auth::AuthService;
use crate::services::awareness::AwarenessService;
use crate::services::backup::BackupService;
use crate::services::browser::desk::{CardGate, NoDesk, StepGate, TaskDesk};
use crate::services::browser::engine::{BrowserEngine, NoBrowser};
use crate::services::browser::files::Places;
use crate::services::browser::surface::{BrowserLog, Kit};
use crate::services::browser::{BrowserParts, BrowserService};
use crate::services::calendar::CalendarService;
use crate::services::calls::CallService;
use crate::services::chat_model::{ChatModel, NoChatModel};
use crate::services::chrome::{ChromeParts, ChromeService};
use crate::services::coding_app::{CodingThreadApp, NoCodingApp};
use crate::services::context::ContextService;
use crate::services::dark::{DarkModels, DarkService};
use crate::services::display::DisplayService;
use crate::services::environments::EnvironmentService;
use crate::services::facts::FactService;
use crate::services::family::{
    FamilyDirectory, FamilyService, NoFamily, NoReminders, RoomReminders,
};
use crate::services::guard::{Guard, GuardConfig, GuardParts, Judge, NoJudge, RepoRemotes};
use crate::services::hands::HandsService;
use crate::services::health::{LoopMonitor, measure};
use crate::services::health_data::HealthDataService;
use crate::services::history::HistoryService;
use crate::services::jobs::JobService;
use crate::services::learning::LearningService;
use crate::services::live::LiveHub;
use crate::services::live_call::LiveCallService;
use crate::services::mac::{MacHelper, MacService, NoMacHelper};
use crate::services::machines::MachineService;
use crate::services::messaging::hooks::{ServerTasks, WhatsAppAnswers};
use crate::services::messaging::{Messaging, MessagingDeps, MessagingParts};
use crate::services::notes::NoteService;
use crate::services::pairing::PairingService;
use crate::services::parties::PartyService;
use crate::services::personal::{PersonalDependencies, PersonalServices};
use crate::services::plan::PlanService;
use crate::services::presence::Presence;
use crate::services::push::{PushSenders, PushService};
use crate::services::recall::{Embedder, NoEmbedder, RecallParts, RecallService};
use crate::services::report::ReportLog;
use crate::services::rooms::RoomsService;
use crate::services::routines::RoutineService;
use crate::services::runner_api::{NoRunners, RunnerApi};
use crate::services::runners::{RunnerParts, RunnerService};
use crate::services::same_task::SameTaskService;
use crate::services::schedule_sources::AssistantSchedule;
use crate::services::scheduled::{ScheduleSource, ScheduledService};
use crate::services::secret_links::{SecretLinkParts, SecretLinkService};
use crate::services::settings_build::BuildService;
use crate::services::state::{Sections, StateService};
use crate::services::switches::{SwitchParts, SwitchService};
use crate::services::talk::{Assistant, Silent, TalkParts, TalkService};
use crate::services::tasks::{
    ArtifactFiles, Ffmpeg, NoWorkers, TaskParts, TaskService, TaskWorkers, Thumbnailer,
};
use crate::services::theme::ThemeService;
use crate::services::threads::ThreadService;
use crate::services::transfer::{Roots, TransferParts, TransferService};
use crate::services::uploads::UploadService;
use crate::services::voice::VoiceService;
use crate::services::wake::{WakeParts, WakeService};
use crate::services::workers::{LateWorkers, WorkerParts, WorkerRuntime, WorkerService};
use crate::services::workers_view::{AssistantTasks, NoWorkers as NoWorkerViews, WorkerSessions};

/// The header that carries a request's id, set when the client sent none and echoed back.
const REQUEST_ID: HeaderName = HeaderName::from_static("x-request-id");

/// Bodies at least this big are gzipped for a client that takes it: the state, mostly.
const COMPRESS_FROM: u64 = 16 * 1024;

/// The longest request id taken from a client; a longer one is replaced.
const REQUEST_ID_LIMIT: usize = 64;

/// Everything a handler can reach: the services, wired once at start-up. Cheap to clone.
#[derive(Clone, Debug)]
pub struct AppState {
    inner: Arc<Inner>,
}

#[derive(Debug)]
struct Inner {
    auth: AuthService,
    pairing: PairingService,
    state: StateService,
    talk: TalkService,
    threads: ThreadService,
    live: LiveHub,
    presence: Presence,
    report_log: ReportLog,
    display: DisplayService,
    theme: ThemeService,
    agent_tokens: AgentTokens,
    history: HistoryService,
    uploads: UploadService,
    lag: LoopMonitor,
    push: PushService,
    secret_links: SecretLinkService,
    clock: Arc<dyn Clock>,
    auth_mode: AuthMode,
    stopping: CancellationToken,
    mac: MacService,
    backups: BackupService,
    builds: BuildService,
    activity: ActivityLog,
    plan: PlanService,
    tasks: TaskService,
    scheduled: ScheduledService,
    program_key: Option<String>,
    rooms: RoomsService,
    routines: RoutineService,
    wake: WakeService,
    meeting_prep: crate::services::meeting_prep::MeetingPrep,
    facts: FactService,
    notes: NoteService,
    learning: LearningService,
    recall: RecallService,
    context: ContextService,
    runners: RunnerService,
    machines: MachineService,
    accounts: AccountService,
    transfer: TransferService,
    runner_mcp: Option<PathBuf>,
    worker_sessions: Arc<dyn WorkerSessions>,
    assistant_tasks: Arc<dyn AssistantTasks>,
    jobs: JobService,
    switches: SwitchService,
    environments: EnvironmentService,
    personal: PersonalServices,
    messaging: Messaging,
    guard: Guard,
    workers: WorkerService,
    worker_tokens: crate::mcp::worker_tokens::WorkerTokens,
    browser: BrowserService,
    hands: HandsService,
    parties: PartyService,
    dark: DarkService,
    family: FamilyService,
    machine: Option<Arc<Machine>>,
    pairing_routes: Vec<String>,
    direct_scheme: Scheme,
    agents: AgentService,
    approvals: ApprovalService,
    agent_conversations: AgentConversations,
}

/// What the services are built from, besides the database: the dependencies a test swaps.
#[derive(Clone, Debug)]
pub struct Dependencies {
    /// What time it is.
    pub clock: Arc<dyn Clock>,
    /// Where secrets get their randomness.
    pub random: Arc<dyn RandomSource>,
    /// What answers the person.
    pub assistant: Arc<dyn Assistant>,
    /// The push services (none by default).
    pub push: PushSenders,
    /// Where secrets typed into secret links go.
    pub secrets_dir: PathBuf,
    /// The address links point at, without a final `/`.
    pub public_url: String,
    /// The helper on the owner's Mac.
    pub mac: Arc<dyn MacHelper>,
    /// The coding-thread app on the person's machines.
    pub coding_app: Arc<dyn CodingThreadApp>,
    /// The workers' area: a worker started on a task, or stopped.
    pub workers: Arc<dyn TaskWorkers>,
    /// The server's time zone: days, the week, deadlines, routines and quiet hours are its.
    pub zone: jiff::tz::TimeZone,
    /// The server user's home: a task's files may come from under it, and files sent to a
    /// runner may be in it.
    pub home: Option<PathBuf>,
    /// What makes the thumbnails of a task's pictures and videos.
    pub thumbnailer: Arc<dyn Thumbnailer>,
    /// The key a program on this machine hands the assistant a job with (`POST /api/tasks`);
    /// none: no program can.
    pub program_key: Option<String>,
    /// What the Scheduled page lists, besides what the server schedules itself.
    pub schedule: Vec<Arc<dyn ScheduleSource>>,
    /// The embedding model recall searches by meaning with (none: by words alone).
    pub embedder: Arc<dyn Embedder>,
    /// The folder of agent transcripts recall indexes, if any.
    pub transcripts: Option<PathBuf>,
    /// The runners on the person's machines.
    pub runners: Arc<dyn RunnerApi>,
    /// The file of MCP servers the runners' sessions get too (`{ "mcpServers": { ... } }`).
    pub runner_mcp: Option<PathBuf>,
    /// The assistant's workers, as the sessions `w-<task>` show them, when no worker runtime
    /// runs (no agent CLI is configured): `NoWorkers` by default. With one, the workers are
    /// shown by the workers service itself.
    pub worker_sessions: Arc<dyn WorkerSessions>,
    /// The assistant's own background tasks (`stopTask`): the agent runtime's, `NoWorkers`
    /// (the default) for the silent assistant.
    pub assistant_tasks: Arc<dyn AssistantTasks>,
    /// The files the server keeps keys and tokens in besides its secrets folder (the
    /// configured token and key files): never sent to a runner's machine. The database, its
    /// journals and the machine's key beside it are added to them.
    pub private_files: Vec<PathBuf>,
    /// The providers of the calls, voice, calendar and health services.
    pub personal: PersonalDependencies,
    /// The outside services the owner's messages come from and go to.
    pub messaging: MessagingDeps,
    /// The guard rails' setup and outside services.
    pub guard: GuardDeps,
    /// How the server runs the assistant's workers.
    pub worker_setup: WorkerSetup,
    /// The browser the assistant drives (none set up by default).
    pub browser: Arc<dyn BrowserEngine>,
    /// The browser's live view address, for the app.
    pub browser_view: Option<String>,
    /// The tasks, as the browser and the Mac tools reach them (none by default).
    pub desk: Arc<dyn TaskDesk>,
    /// The owner's OK on final steps; none: Go ahead cards on the step's task ([`CardGate`]).
    pub gate: Option<Arc<dyn StepGate>>,
    /// The owner's home folder: where browser uploads may come from and downloads go; none:
    /// `home`.
    pub browser_home: Option<PathBuf>,
    /// The browser extension's build, served to the owner's Chrome profiles.
    pub chrome_extension_zip: Option<PathBuf>,
    /// The address the owner's Chrome profiles reach the server at; none: `public_url`.
    pub chrome_url: Option<String>,
    /// The model parties talk through.
    pub party_model: Arc<dyn ChatModel>,
    /// The models the off-the-record room talks with.
    pub dark: DarkModels,
    /// Who a family member's device key belongs to.
    pub family_directory: Arc<dyn FamilyDirectory>,
    /// The check-ins each room asked for.
    pub room_reminders: Arc<dyn RoomReminders>,
    /// The machine's identity: its key and name (ADR 0007). None (the default, for tests):
    /// pairing answers carry no `machine` and no link, as a legacy host's, and the machine
    /// signs no link statement.
    pub machine: Option<Arc<Machine>>,
    /// The routes a pairing link names, best first (none by default).
    pub pairing_routes: Vec<String>,
    /// The outside services approvals are routed through, and the server's public address
    /// (none by default: approvals are made in the apps only).
    pub approvals: ApprovalDeps,
    /// Slack, as configured agents' own bots post and read there.
    pub agent_slack: Arc<dyn AgentSlack>,
}

/// How the server runs the assistant's workers: the CLI (none: no workers, and the
/// `workers` and `worker_sessions` dependencies stand in), the ultra runs' caps, the runner
/// lanes start on, and a ceiling on workers at once.
#[derive(Clone, Debug, Default)]
pub struct WorkerSetup {
    /// The CLI and how it starts.
    pub runtime: Option<WorkerRuntime>,
    /// The ultra runs' caps.
    pub limits: emberhut_core::ultra::Limits,
    /// The runner machine an ultra run's lanes start on; none: the first runner configured.
    pub lane_machine: Option<String>,
    /// The most workers at once; none: no ceiling.
    pub max_working: Option<usize>,
}

/// The guard rails' setup and outside services: what a test swaps.
#[derive(Clone, Debug)]
pub struct GuardDeps {
    /// The setup.
    pub config: GuardConfig,
    /// The ask guard's judge model (none: the guard is off).
    pub judge: Arc<dyn Judge>,
    /// A folder's git remotes.
    pub remotes: Arc<dyn RepoRemotes>,
}

impl Default for GuardDeps {
    /// No repository always let through, nobody messaged without an OK, no judge, and
    /// `git remote -v` for a folder's remotes.
    fn default() -> Self {
        Self {
            config: GuardConfig::default(),
            judge: Arc::new(NoJudge),
            remotes: Arc::new(crate::services::guard::GitRemotes),
        }
    }
}

impl Default for Dependencies {
    /// The system's clock and randomness, the silent stand-in assistant, no push services,
    /// `secrets` for the secrets, the default listen address for links, no Mac helper and no
    /// coding-thread app, no model for parties or the off-the-record room, no family members,
    /// no check-ins, and no identity of the machine's own.
    fn default() -> Self {
        Self {
            clock: Arc::new(SystemClock),
            random: Arc::new(OsRandom),
            assistant: Arc::new(Silent),
            push: PushSenders::default(),
            secrets_dir: PathBuf::from("secrets"),
            public_url: "http://127.0.0.1:18990".to_owned(),
            mac: Arc::new(NoMacHelper),
            coding_app: Arc::new(NoCodingApp),
            workers: Arc::new(NoWorkers),
            zone: jiff::tz::TimeZone::try_system().unwrap_or(jiff::tz::TimeZone::UTC),
            home: std::env::var_os("HOME").map(PathBuf::from),
            thumbnailer: Arc::new(Ffmpeg::new(PathBuf::from("ffmpeg"))),
            program_key: None,
            schedule: Vec::new(),
            embedder: Arc::new(NoEmbedder),
            transcripts: None,
            runners: Arc::new(NoRunners),
            runner_mcp: None,
            worker_sessions: Arc::new(NoWorkerViews),
            assistant_tasks: Arc::new(NoWorkerViews),
            private_files: Vec::new(),
            personal: PersonalDependencies::default(),
            messaging: MessagingDeps::default(),
            guard: GuardDeps::default(),
            worker_setup: WorkerSetup::default(),
            browser: Arc::new(NoBrowser),
            browser_view: None,
            desk: Arc::new(NoDesk),
            gate: None,
            browser_home: None,
            chrome_extension_zip: None,
            chrome_url: None,
            party_model: Arc::new(NoChatModel),
            dark: DarkModels::default(),
            family_directory: Arc::new(NoFamily),
            room_reminders: Arc::new(NoReminders),
            machine: None,
            pairing_routes: Vec::new(),
            approvals: ApprovalDeps::default(),
            agent_slack: Arc::new(NoAgentSlack),
        }
    }
}

impl AppState {
    /// Every service over `db`, wired once from `dependencies`.
    #[must_use]
    #[expect(
        clippy::too_many_lines,
        reason = "every service, wired once in one place"
    )]
    pub fn new(db: &Database, dependencies: Dependencies, auth_mode: AuthMode) -> Self {
        let Dependencies {
            clock,
            random,
            assistant,
            push,
            secrets_dir,
            public_url,
            mac,
            coding_app,
            workers,
            zone,
            home,
            thumbnailer,
            program_key,
            schedule,
            embedder,
            transcripts,
            runners,
            runner_mcp,
            worker_sessions,
            assistant_tasks,
            mut private_files,
            personal,
            mut messaging,
            guard,
            worker_setup,
            browser,
            browser_view,
            desk,
            gate,
            browser_home,
            chrome_extension_zip,
            chrome_url,
            party_model,
            dark,
            family_directory,
            room_reminders,
            machine,
            pairing_routes,
            approvals: approval_deps,
            agent_slack,
        } = dependencies;
        let WorkerSetup {
            runtime: worker_runtime,
            limits: worker_limits,
            lane_machine,
            max_working,
        } = worker_setup;
        let late_workers = LateWorkers::default();
        let workers: Arc<dyn TaskWorkers> = if worker_runtime.is_some() {
            Arc::new(late_workers.clone())
        } else {
            workers
        };
        let worker_tokens = crate::mcp::worker_tokens::WorkerTokens::new(Arc::clone(&random));
        let (secrets_dir_for_messaging, public_url_for_messaging) =
            (secrets_dir.clone(), public_url.clone());
        let direct_scheme = direct_scheme(&public_url);
        private_files.extend(crate::services::transfer::beside_database(db.path()));
        let private_dir = secrets_dir.clone();
        let home_for_guard = home.clone();
        let activity = ActivityLog::new(db.clone(), Arc::clone(&clock));
        let plan = PlanService::new(db.settings(), db.log(), Arc::clone(&clock));
        let thumbs = db
            .path()
            .parent()
            .map_or_else(|| PathBuf::from("thumbs"), |dir| dir.join("thumbs"));
        let tasks = TaskService::new(TaskParts {
            db: db.clone(),
            clock: Arc::clone(&clock),
            zone: zone.clone(),
            plan: plan.clone(),
            activity: activity.clone(),
            files: ArtifactFiles::new(home.clone(), &secrets_dir, thumbs, thumbnailer),
            workers,
        });
        let personal =
            PersonalServices::new(db, &tasks, &activity, &clock, &random, &mac, personal);
        let auth = AuthService::new(db.sessions(), Arc::clone(&clock), Arc::clone(&random));
        let pairing = PairingService::new(db.pairing(), Arc::clone(&clock), Arc::clone(&random));
        // What never leaves the server, the same for a file shown in the chat and one sent to
        // a runner's machine.
        let private = crate::services::private::PrivatePaths::new(
            std::iter::once(private_dir)
                .chain(
                    home.iter()
                        .flat_map(|home| crate::services::private::in_home(home)),
                )
                .chain(private_files),
        );
        let data_dir = db
            .path()
            .parent()
            .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
        let work_dir = data_dir.join(crate::services::agents::scope::WORK_FOLDER);
        let agent_roots = AgentRoots::new(data_dir.clone(), work_dir.clone(), db.tasks());
        let approvals = ApprovalService::new(ApprovalParts {
            approvals: db.approvals(),
            agents: db.agents(),
            clock: Arc::clone(&clock),
            random: Arc::clone(&random),
            zone: zone.clone(),
            secrets_dir: secrets_dir.clone(),
            private: private.clone(),
            deps: approval_deps,
            roots: agent_roots.clone(),
        });
        let agents = AgentService::new(db.agents(), db.approvals(), Arc::clone(&clock));
        let uploads = UploadService::new(
            db.files(),
            UploadService::folder_beside(db.path()),
            private.clone(),
            Arc::clone(&clock),
            Arc::clone(&random),
        );
        let history =
            HistoryService::new(db.snapshots(), Arc::clone(&clock)).with_refs(db.message_refs());
        let live = LiveHub::default();
        let display = DisplayService::new(db.settings(), db.log(), Arc::clone(&clock));
        let theme = ThemeService::new(db.settings(), db.log(), Arc::clone(&clock));
        let browser_secrets = secrets_dir.clone();
        let browser_url = chrome_url.unwrap_or_else(|| public_url.clone());
        let secret_links = SecretLinkService::new(SecretLinkParts {
            links: db.secret_links(),
            messages: db.messages(),
            clock: Arc::clone(&clock),
            random: Arc::clone(&random),
            secrets_dir,
            public_url,
        });
        let presence = Presence::new(Arc::clone(&clock));
        let push = PushService::new(
            db.push(),
            db.rooms(),
            Arc::clone(&clock),
            presence.clone(),
            push,
        )
        .with_family(db.family());
        let builds = BuildService::new(db.builds());
        let (browser, kit) = browser_service(
            db,
            BrowserWiring {
                engine: browser,
                live_view: browser_view,
                desk,
                gate,
                home: browser_home.or_else(|| home.clone()),
                secrets_dir: browser_secrets,
                extension_zip: chrome_extension_zip,
                chrome_url: browser_url,
                mac: Arc::clone(&mac),
            },
            Kit {
                desk: Arc::new(NoDesk),
                gate: Arc::new(CardGate::new(Arc::new(NoDesk), Arc::clone(&clock))),
                push: Some(push.clone()),
                activity: activity.clone(),
                clock: Arc::clone(&clock),
                logs: BrowserLog::new(&data_dir_of(db)),
            },
            &random,
        );
        let hands = HandsService::new(Arc::clone(&mac), kit, db.settings(), &data_dir_of(db));
        let threads = ThreadService::new(db.threads(), Arc::clone(&clock), coding_app);
        let stopping = CancellationToken::new();
        let routines =
            RoutineService::new(db.settings(), db.events(), Arc::clone(&clock), zone.clone());
        let notes = NoteService::new(db.notes(), db.settings(), Arc::clone(&clock), zone.clone());
        let learning = LearningService::new(
            db.corrections(),
            db.settings(),
            db.conversation(),
            Arc::clone(&clock),
        );
        let agent_tokens = AgentTokens::new(Arc::clone(&random));
        let talk = TalkService::new(TalkParts {
            messages: db.messages(),
            rooms: db.rooms(),
            live: live.clone(),
            clock: Arc::clone(&clock),
            assistant,
            stopping: stopping.clone(),
            push: push.clone(),
            tokens: agent_tokens.clone(),
            routines: routines.clone(),
            activity: activity.clone(),
            refs: db.message_refs(),
            same_task: SameTaskService::new(db.messages(), db.rooms(), Arc::clone(&clock)),
            settings: db.settings(),
            notes: notes.clone(),
            learning: learning.clone(),
            agents: db.agents(),
        });
        let rooms = RoomsService::new(db.rooms(), db.messages(), db.events(), Arc::clone(&clock));
        let wake = WakeService::new(WakeParts {
            settings: db.settings(),
            events: db.events(),
            rooms: db.rooms(),
            talk: talk.clone(),
            threads: threads.clone(),
            routines: routines.clone(),
            room_service: rooms.clone(),
            activity: activity.clone(),
            clock: Arc::clone(&clock),
        });
        let agent_conversations = AgentConversations::new(ConversationParts {
            agents: db.agents(),
            messages: db.messages(),
            secrets: crate::services::messaging::secrets::Secrets::new(
                secrets_dir_for_messaging.clone(),
            ),
            slack: agent_slack,
            talk: talk.clone(),
            wake: wake.clone(),
            private: private.clone(),
            roots: agent_roots.clone(),
            clock: Arc::clone(&clock),
        });
        // The assistant's own watches, check-ins and routines are on the Scheduled page too.
        let mut schedule = schedule;
        schedule.push(Arc::new(AssistantSchedule::new(
            wake.clone(),
            routines.clone(),
            db.tasks(),
        )));
        let scheduled = ScheduledService::new(schedule, db.tasks());
        let facts = FactService::new(
            FactService::folder_beside(db.path()),
            Arc::clone(&clock),
            db.events(),
        );
        let recall = RecallService::new(RecallParts {
            store: db.recall(),
            conversation: db.conversation(),
            notes: db.notes(),
            facts: facts.clone(),
            tasks: db.tasks(),
            voice: db.voice_notes(),
            embedder,
            transcripts,
            zone,
        });
        let context = ContextService::new(db.settings(), Arc::clone(&clock));
        let RunnerArea {
            transfer,
            runner_service,
            machines,
            jobs,
            accounts,
            switches,
            environments,
        } = runner_area(RunnerInputs {
            db,
            activity: &activity,
            clock: &clock,
            random: &random,
            threads: &threads,
            uploads: uploads.dir().to_path_buf(),
            home,
            private,
            runners,
            assistant_tasks: &assistant_tasks,
        });
        let guard = Guard::new(GuardParts {
            db: db.clone(),
            tasks: tasks.clone(),
            clock: Arc::clone(&clock),
            zone: tasks.zone().clone(),
            config: GuardConfig {
                home: guard.config.home.clone().or_else(|| home_for_guard.clone()),
                ..guard.config
            },
            judge: guard.judge,
            remotes: guard.remotes,
        });
        guard.use_memory(learning.clone(), facts.clone());
        guard.use_agents(approvals.clone(), db.agents(), agent_roots.clone());
        messaging.oks = Arc::new(crate::services::guard::send_tap::GuardOks::new(
            guard.clone(),
            messaging.oks,
        ));
        tasks.hand_answers_to(Arc::new(crate::services::guard::send_tap::GuardAnswers(
            guard.clone(),
        )));
        let workers = WorkerService::new(WorkerParts {
            db: db.clone(),
            tasks: tasks.clone(),
            guard: guard.clone(),
            runners: runner_service.clone(),
            machines: machines.clone(),
            activity: activity.clone(),
            clock: Arc::clone(&clock),
            tokens: worker_tokens.clone(),
            work_dir: work_dir.clone(),
            runtime: worker_runtime.clone(),
            limits: worker_limits,
            lane_machine: lane_machine.clone(),
            max_working,
        });
        late_workers.set(workers.clone());
        tasks.hand_answers_to(Arc::new(crate::services::workers::ultra::UltraAnswers(
            workers.clone(),
        )));
        let worker_sessions: Arc<dyn WorkerSessions> = if worker_runtime.is_some() {
            Arc::new(workers.clone())
        } else {
            worker_sessions
        };
        let state = StateService::new(
            db.rooms(),
            db.snapshots(),
            live.clone(),
            auth.clone(),
            auth_mode,
            Arc::clone(&clock),
            Sections {
                display: display.clone(),
                theme: theme.clone(),
                builds: builds.clone(),
                threads: threads.clone(),
                tasks: tasks.clone(),
                plan: plan.clone(),
                wake: wake.clone(),
                log: db.log(),
                awareness: AwarenessService::new(threads.clone()),
                refs: db.message_refs(),
                context: context.clone(),
                runners: runner_service.clone(),
                accounts: accounts.clone(),
                calls: personal.calls.clone(),
                voice: personal.voice.clone(),
                health_data: personal.health_data.clone(),
                workers: workers.clone(),
                browser: browser.clone(),
                agents: agents.clone(),
                approvals: approvals.clone(),
            },
        )
        .with_push(push.clone());
        let backups = BackupService::new(db.backups(), Arc::clone(&clock), stopping.clone());
        let messaging = Messaging::new(
            db,
            messaging,
            MessagingParts {
                card_oks_only: auth_mode == AuthMode::Report,
                clock: Arc::clone(&clock),
                random: Arc::clone(&random),
                push: push.clone(),
                secrets_dir: secrets_dir_for_messaging,
                public_url: public_url_for_messaging,
                tasks: Arc::new(ServerTasks::new(
                    tasks.clone(),
                    db.rooms(),
                    db.events(),
                    push.clone(),
                    Arc::clone(&clock),
                )),
                facts: Some(facts.clone()),
            },
        );
        tasks.hand_answers_to(Arc::new(WhatsAppAnswers(messaging.whatsapp().clone())));
        guard.use_messaging(messaging.clone());
        let (parties, dark, family) = side_apps(
            db,
            &clock,
            &stopping,
            &theme,
            SideParts {
                party_model,
                dark,
                family_directory,
                room_reminders,
            },
        );
        let meeting_prep = crate::services::meeting_prep::MeetingPrep::new(
            personal.calendar.clone(),
            tasks.clone(),
            db.settings(),
            Arc::clone(&clock),
        );
        Self {
            inner: Arc::new(Inner {
                auth,
                pairing,
                state,
                talk,
                threads,
                live,
                presence,
                report_log: ReportLog::new(Arc::clone(&clock)),
                display,
                theme,
                agent_tokens,
                history,
                uploads,
                lag: LoopMonitor::new(),
                push,
                secret_links,
                clock,
                auth_mode,
                stopping,
                mac: MacService::new(mac),
                backups,
                builds,
                activity,
                plan,
                tasks,
                scheduled,
                program_key,
                rooms,
                routines,
                meeting_prep,
                wake,
                facts,
                notes,
                learning,
                recall,
                context,
                runners: runner_service,
                machines,
                accounts,
                transfer,
                runner_mcp,
                worker_sessions,
                assistant_tasks,
                jobs,
                switches,
                environments,
                personal,
                messaging,
                guard,
                workers,
                worker_tokens,
                browser,
                hands,
                parties,
                dark,
                family,
                machine,
                pairing_routes,
                direct_scheme,
                agents,
                approvals,
                agent_conversations,
            }),
        }
    }

    /// The machine's identity, when the server has one (always, when it serves).
    #[must_use]
    pub fn machine(&self) -> Option<&Machine> {
        self.inner.machine.as_deref()
    }

    /// The routes a pairing link names, best first.
    #[must_use]
    pub fn pairing_routes(&self) -> &[String] {
        &self.inner.pairing_routes
    }

    /// The scheme a direct listener's requests come in with, which a probe's host is
    /// normalized against: the public URL's when it is `https` (the listener sits behind a TLS
    /// proxy), and `http` otherwise.
    #[must_use]
    pub fn direct_scheme(&self) -> Scheme {
        self.inner.direct_scheme
    }

    /// Agents, approvers and trusted requesters, as the owner configures them.
    #[must_use]
    pub fn agents(&self) -> &AgentService {
        &self.inner.agents
    }

    /// Configured agents' conversations in their channels: inbound messages, their answers
    /// and the threads they read.
    #[must_use]
    pub fn agent_conversations(&self) -> &AgentConversations {
        &self.inner.agent_conversations
    }

    /// Approval requests: asked for, routed, decided, used and expired.
    #[must_use]
    pub fn approvals(&self) -> &ApprovalService {
        &self.inner.approvals
    }

    /// The assistant's workers and ultra runs.
    #[must_use]
    pub fn workers(&self) -> &WorkerService {
        &self.inner.workers
    }

    /// The workers' tokens: the bearer a worker calls the worker endpoint and the gate with.
    #[must_use]
    pub fn worker_tokens(&self) -> &crate::mcp::worker_tokens::WorkerTokens {
        &self.inner.worker_tokens
    }

    /// The guard rails: the owner's OKs, outward steps, Go ahead cards, the ask guard and the
    /// cards' Send it.
    #[must_use]
    pub fn guard(&self) -> &Guard {
        &self.inner.guard
    }

    /// The facts: the context folder.
    #[must_use]
    pub fn facts(&self) -> &FactService {
        &self.inner.facts
    }

    /// The quick notes and the diary.
    #[must_use]
    pub fn notes(&self) -> &NoteService {
        &self.inner.notes
    }

    /// The corrections, standing rules, trial rules and the chat log.
    #[must_use]
    pub fn learning(&self) -> &LearningService {
        &self.inner.learning
    }

    /// Recall: the search over everything said.
    #[must_use]
    pub fn recall(&self) -> &RecallService {
        &self.inner.recall
    }

    /// The assistant's running awareness (its open loops and notes).
    #[must_use]
    pub fn context(&self) -> &ContextService {
        &self.inner.context
    }

    /// Device sessions.
    #[must_use]
    pub fn auth(&self) -> &AuthService {
        &self.inner.auth
    }

    /// Pairing codes.
    #[must_use]
    pub fn pairing(&self) -> &PairingService {
        &self.inner.pairing
    }

    /// The app's state for a room.
    #[must_use]
    pub fn state(&self) -> &StateService {
        &self.inner.state
    }

    /// Saying something, and the assistant's turns.
    #[must_use]
    pub fn talk(&self) -> &TalkService {
        &self.inner.talk
    }

    /// Coding threads and the ones the assistant follows.
    #[must_use]
    pub fn threads(&self) -> &ThreadService {
        &self.inner.threads
    }

    /// What the assistant is doing in each room.
    #[must_use]
    pub fn live(&self) -> &LiveHub {
        &self.inner.live
    }

    /// Which rooms are on screen.
    #[must_use]
    pub fn presence(&self) -> &Presence {
        &self.inner.presence
    }

    /// The phones' push addresses, and pushing to them.
    #[must_use]
    pub fn push(&self) -> &PushService {
        &self.inner.push
    }

    /// One-time secret links.
    #[must_use]
    pub fn secret_links(&self) -> &SecretLinkService {
        &self.inner.secret_links
    }

    /// Report mode's notes of requests without a session.
    #[must_use]
    pub fn report_log(&self) -> &ReportLog {
        &self.inner.report_log
    }

    /// What the app shows and hides.
    #[must_use]
    pub fn display(&self) -> &DisplayService {
        &self.inner.display
    }

    /// The theme choice.
    #[must_use]
    pub fn theme(&self) -> &ThemeService {
        &self.inner.theme
    }

    /// The rooms' agent tokens, which let a room's agent session call the MCP endpoint.
    #[must_use]
    pub fn agent_tokens(&self) -> &AgentTokens {
        &self.inner.agent_tokens
    }

    /// Pages of a room's chat.
    #[must_use]
    pub fn history(&self) -> &HistoryService {
        &self.inner.history
    }

    /// The chat's files.
    #[must_use]
    pub fn uploads(&self) -> &UploadService {
        &self.inner.uploads
    }

    /// How late the runtime is, and the work that held it.
    #[must_use]
    pub fn lag(&self) -> &LoopMonitor {
        &self.inner.lag
    }

    /// What time it is.
    #[must_use]
    pub fn clock(&self) -> &dyn Clock {
        self.inner.clock.as_ref()
    }

    /// What happens to a request that needs a session and has none.
    #[must_use]
    pub fn auth_mode(&self) -> AuthMode {
        self.inner.auth_mode
    }

    /// The owner's Mac, through its helper.
    #[must_use]
    pub fn mac(&self) -> &MacService {
        &self.inner.mac
    }

    /// The server's backups.
    #[must_use]
    pub fn backups(&self) -> &BackupService {
        &self.inner.backups
    }

    /// The newest builds of the app.
    #[must_use]
    pub fn builds(&self) -> &BuildService {
        &self.inner.builds
    }

    /// The activity log.
    #[must_use]
    pub fn activity(&self) -> &ActivityLog {
        &self.inner.activity
    }

    /// The person's plan: goals, the Focus list, open loops, the concern, background jobs.
    #[must_use]
    pub fn plan(&self) -> &PlanService {
        &self.inner.plan
    }

    /// The tasks, their projects and areas.
    #[must_use]
    pub fn tasks(&self) -> &TaskService {
        &self.inner.tasks
    }

    /// The Scheduled page.
    #[must_use]
    pub fn scheduled(&self) -> &ScheduledService {
        &self.inner.scheduled
    }

    /// The key programs on this machine use to hand the assistant a job.
    #[must_use]
    pub fn program_key(&self) -> Option<&str> {
        self.inner.program_key.as_deref()
    }

    /// The rooms as the assistant sees them: finding one, handing something to another.
    #[must_use]
    pub fn rooms(&self) -> &RoomsService {
        &self.inner.rooms
    }

    /// The routines and quiet hours.
    #[must_use]
    pub fn routines(&self) -> &RoutineService {
        &self.inner.routines
    }

    /// The wake rule: the switch, check-ins, watches and events.
    #[must_use]
    pub fn wake(&self) -> &WakeService {
        &self.inner.wake
    }

    /// Meeting prep: a task to prepare the owner before each meeting.
    #[must_use]
    pub fn meeting_prep(&self) -> &crate::services::meeting_prep::MeetingPrep {
        &self.inner.meeting_prep
    }

    /// The runners' sessions.
    #[must_use]
    pub fn runners(&self) -> &RunnerService {
        &self.inner.runners
    }

    /// The runner machines' samples and states.
    #[must_use]
    pub fn machines(&self) -> &MachineService {
        &self.inner.machines
    }

    /// The assistant's accounts.
    #[must_use]
    pub fn accounts(&self) -> &AccountService {
        &self.inner.accounts
    }

    /// Files to and from the runners' machines.
    #[must_use]
    pub fn transfer(&self) -> &TransferService {
        &self.inner.transfer
    }

    /// The assistant's workers, as the sessions `w-<task>` show them.
    #[must_use]
    pub fn worker_sessions(&self) -> &dyn WorkerSessions {
        self.inner.worker_sessions.as_ref()
    }

    /// The MCP servers and skills switches.
    #[must_use]
    pub fn switches(&self) -> &SwitchService {
        &self.inner.switches
    }

    /// The Environments page.
    #[must_use]
    pub fn environments(&self) -> &EnvironmentService {
        &self.inner.environments
    }

    /// The background jobs on the runners' machines.
    #[must_use]
    pub fn jobs(&self) -> &JobService {
        &self.inner.jobs
    }

    /// The assistant's own background tasks.
    #[must_use]
    pub fn assistant_tasks(&self) -> &dyn AssistantTasks {
        self.inner.assistant_tasks.as_ref()
    }

    /// The file of MCP servers the runners' sessions get too, when one is set.
    #[must_use]
    pub fn runner_mcp(&self) -> Option<&Path> {
        self.inner.runner_mcp.as_deref()
    }

    /// The owner's calendar and reminders.
    #[must_use]
    pub fn calendar(&self) -> &CalendarService {
        &self.inner.personal.calendar
    }

    /// Recorded and shared calls, and their transcripts.
    #[must_use]
    pub fn calls(&self) -> &CallService {
        &self.inner.personal.calls
    }

    /// Calls being recorded: their live transcript and copilot.
    #[must_use]
    pub fn live_calls(&self) -> &LiveCallService {
        &self.inner.personal.live_calls
    }

    /// Voice calls and dictation.
    #[must_use]
    pub fn voice(&self) -> &VoiceService {
        &self.inner.personal.voice
    }

    /// The owner's health and screen time.
    #[must_use]
    pub fn health_data(&self) -> &HealthDataService {
        &self.inner.personal.health_data
    }

    /// The owner's messages from outside the app, and the assistant's own WhatsApp number.
    #[must_use]
    pub fn messaging(&self) -> &Messaging {
        &self.inner.messaging
    }

    /// The browser the assistant drives.
    #[must_use]
    pub fn browser(&self) -> &BrowserService {
        &self.inner.browser
    }

    /// The assistant's Mac tools, through the Mac helper.
    #[must_use]
    pub fn hands(&self) -> &HandsService {
        &self.inner.hands
    }

    /// The owner's Chrome profiles with the browser extension.
    #[must_use]
    pub fn chrome(&self) -> &ChromeService {
        self.inner.browser.chrome()
    }

    /// Parties: AI personas talking about a topic.
    #[must_use]
    pub fn parties(&self) -> &PartyService {
        &self.inner.parties
    }

    /// Family members' rooms, theme and phones.
    #[must_use]
    pub fn family(&self) -> &FamilyService {
        &self.inner.family
    }

    /// The off-the-record room.
    #[must_use]
    pub fn dark(&self) -> &DarkService {
        &self.inner.dark
    }

    /// Cancelled when the server starts shutting down, so long-lived responses (live streams)
    /// can end instead of holding the shutdown up.
    #[must_use]
    pub fn stopping(&self) -> &CancellationToken {
        &self.inner.stopping
    }
}

/// What [`runner_area`] builds the runners' services from.
struct RunnerInputs<'a> {
    db: &'a Database,
    activity: &'a ActivityLog,
    clock: &'a Arc<dyn Clock>,
    random: &'a Arc<dyn RandomSource>,
    threads: &'a ThreadService,
    uploads: PathBuf,
    home: Option<PathBuf>,
    private: crate::services::private::PrivatePaths,
    runners: Arc<dyn RunnerApi>,
    assistant_tasks: &'a Arc<dyn AssistantTasks>,
}

/// The runners' services, wired together.
struct RunnerArea {
    transfer: TransferService,
    runner_service: RunnerService,
    machines: MachineService,
    jobs: JobService,
    accounts: AccountService,
    switches: SwitchService,
    environments: EnvironmentService,
}

/// The runners' services: their sessions, machines, accounts, files, jobs, switches and the
/// Environments page.
fn runner_area(inputs: RunnerInputs<'_>) -> RunnerArea {
    let RunnerInputs {
        db,
        activity,
        clock,
        random,
        threads,
        uploads,
        home,
        private,
        runners,
        assistant_tasks,
    } = inputs;
    let transfer = TransferService::new(TransferParts {
        api: Arc::clone(&runners),
        pulls: db.pulls(),
        activity: activity.clone(),
        roots: Roots {
            uploads,
            home,
            private,
        },
        clock: Arc::clone(clock),
        random: Arc::clone(random),
    });
    let runner_service = RunnerService::new(RunnerParts {
        api: runners,
        threads: threads.clone(),
        store: db.threads(),
        usage: db.usage(),
        activity: activity.clone(),
        transfer: transfer.clone(),
        clock: Arc::clone(clock),
    });
    let machines = MachineService::new(
        db.machines(),
        db.threads(),
        db.usage(),
        runner_service.clone(),
        Arc::clone(clock),
    );
    let jobs = JobService::new(runner_service.clone(), activity.clone(), Arc::clone(clock));
    let accounts = AccountService::new(
        db.usage(),
        db.prefs(),
        runner_service.clone(),
        activity.clone(),
        Arc::clone(clock),
    );
    let switches = SwitchService::new(SwitchParts {
        prefs: db.prefs(),
        runners: runner_service.clone(),
        threads: threads.clone(),
        accounts: accounts.clone(),
        session: Arc::clone(assistant_tasks),
        activity: activity.clone(),
        clock: Arc::clone(clock),
    });
    let environments = EnvironmentService::new(
        runner_service.clone(),
        threads.clone(),
        db.threads(),
        accounts.clone(),
        Arc::clone(clock),
    );
    RunnerArea {
        transfer,
        runner_service,
        machines,
        jobs,
        accounts,
        switches,
        environments,
    }
}

/// What the browser is wired from.
struct BrowserWiring {
    engine: Arc<dyn BrowserEngine>,
    live_view: Option<String>,
    desk: Arc<dyn TaskDesk>,
    gate: Option<Arc<dyn StepGate>>,
    home: Option<PathBuf>,
    secrets_dir: PathBuf,
    extension_zip: Option<PathBuf>,
    chrome_url: String,
    mac: Arc<dyn MacHelper>,
}

/// The data folder: the database's.
fn data_dir_of(db: &Database) -> PathBuf {
    let dir = db
        .path()
        .parent()
        .map_or_else(PathBuf::new, Path::to_path_buf);
    std::path::absolute(&dir).unwrap_or(dir)
}

/// The browser and the owner's Chrome profiles, over `kit` (its desk and gate replaced by the
/// wiring's).
fn browser_service(
    db: &Database,
    wiring: BrowserWiring,
    mut kit: Kit,
    random: &Arc<dyn RandomSource>,
) -> (BrowserService, Kit) {
    let gate = wiring.gate.unwrap_or_else(|| {
        Arc::new(CardGate::new(
            Arc::clone(&wiring.desk),
            Arc::clone(&kit.clock),
        ))
    });
    kit.desk = wiring.desk;
    kit.gate = gate;
    let chrome = ChromeService::new(ChromeParts {
        store: db.chrome(),
        kit: kit.clone(),
        random: Arc::clone(random),
        base_url: wiring.chrome_url.trim_end_matches('/').to_owned(),
        extension_zip: wiring.extension_zip,
        mac: wiring.mac,
    });
    let browser = BrowserService::new(BrowserParts {
        engine: wiring.engine,
        kit: kit.clone(),
        settings: db.settings(),
        places: Places {
            home: wiring.home,
            data_dir: data_dir_of(db),
            secrets_dir: wiring.secrets_dir,
        },
        live_view: wiring.live_view,
        chrome,
    });
    (browser, kit)
}

/// What the side apps are built from, besides the database, clock and theme.
struct SideParts {
    party_model: Arc<dyn ChatModel>,
    dark: DarkModels,
    family_directory: Arc<dyn FamilyDirectory>,
    room_reminders: Arc<dyn RoomReminders>,
}

/// The side apps: parties, the off-the-record room (kept beside the database) and family
/// members' rooms.
fn side_apps(
    db: &Database,
    clock: &Arc<dyn Clock>,
    stopping: &CancellationToken,
    theme: &ThemeService,
    parts: SideParts,
) -> (PartyService, DarkService, FamilyService) {
    let parties = PartyService::new(
        db.parties(),
        Arc::clone(clock),
        parts.party_model,
        stopping.clone(),
    );
    let folder = db.path().parent().unwrap_or_else(|| Path::new(""));
    let dark = DarkService::new(folder, Arc::clone(clock), parts.dark, stopping.clone());
    let family = FamilyService::new(
        db.family(),
        Arc::clone(clock),
        parts.family_directory,
        parts.room_reminders,
        theme.clone(),
    );
    (parties, dark, family)
}

/// The whole HTTP app: every route, behind the middleware every request goes through.
///
/// From the outside in: secrets in headers are kept out of logs; each request gets an id
/// (the client's `x-request-id` when it is at most 64 visible ASCII characters, else a new
/// UUID) that its log span carries and its response echoes; each response is logged with its
/// status and latency; a panicking handler answers 500; a body over 16 KiB is gzipped for a
/// client that takes it; a request that takes longer than `request_timeout` answers 503 (an
/// upload or a backup excepted); and every poll of a handler that holds a worker 50 ms or more
/// is noted for `GET /api/loop`.
pub fn router(state: AppState, request_timeout: Duration) -> Router {
    with_middleware(routes::router(), state, request_timeout)
}

/// The scheme of a direct listener whose links point at `public_url` ([`AppState::direct_scheme`]).
fn direct_scheme(public_url: &str) -> Scheme {
    match url::Url::parse(public_url) {
        Ok(url) if url.scheme() == "https" => Scheme::Https,
        _ => Scheme::Http,
    }
}

fn with_middleware(routes: Router<AppState>, state: AppState, request_timeout: Duration) -> Router {
    let middleware = ServiceBuilder::new()
        .layer(SetSensitiveRequestHeadersLayer::new([
            header::AUTHORIZATION,
            header::COOKIE,
        ]))
        .layer(axum::middleware::map_request(drop_unusable_request_id))
        .layer(SetRequestIdLayer::new(REQUEST_ID, MakeRequestUuid))
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(request_span)
                .on_response(DefaultOnResponse::new().level(Level::INFO)),
        )
        .layer(PropagateRequestIdLayer::new(REQUEST_ID))
        .layer(CatchPanicLayer::custom(panicked))
        .layer(CompressionLayer::new().compress_when(compress_when()))
        .layer(axum::middleware::from_fn_with_state(
            request_timeout,
            time_limit,
        ));
    routes
        .layer(axum::middleware::from_fn_with_state(state.clone(), measure))
        .layer(middleware)
        .with_state(state)
}

/// When to gzip an answer: when the client takes gzip and the body is [big enough](COMPRESS_FROM).
/// tower-http's default predicate also leaves out images, gRPC and event streams, so the live
/// stream is never held back to be compressed; and a body that is gzip already (the backup) is
/// not gzipped twice.
fn compress_when() -> impl Predicate {
    DefaultPredicate::new()
        .and(SizeAbove::new(COMPRESS_FROM))
        .and(NotForContentType::const_new("application/gzip"))
}

/// Drops a client's request id that is too long or not plain visible ASCII, so the server
/// makes its own: the id is logged and echoed, and a client should not choose what goes into
/// the logs.
async fn drop_unusable_request_id<B>(mut request: Request<B>) -> Request<B> {
    let usable = request.headers().get(REQUEST_ID).is_none_or(|id| {
        let id = id.as_bytes();
        !id.is_empty() && id.len() <= REQUEST_ID_LIMIT && id.iter().all(u8::is_ascii_graphic)
    });
    if !usable {
        request.headers_mut().remove(REQUEST_ID);
    }
    request
}

/// The span every request's log lines belong to. It names the route that matched (`/s/{token}`),
/// never the path as sent: a path can carry a credential (a one-time link's token), and the
/// logs are not a place for one. A path no route matched is logged with its ids and keys
/// masked ([`masked_path`]).
fn request_span<B>(request: &Request<B>) -> tracing::Span {
    let id = request
        .headers()
        .get(REQUEST_ID)
        .and_then(|id| id.to_str().ok())
        .unwrap_or("-");
    tracing::info_span!(
        "request",
        method = %request.method(),
        path = %logged_path(request),
        id,
    )
}

/// The path a request's log lines name: the matched route's template, else the masked path.
fn logged_path<B>(request: &Request<B>) -> String {
    request.extensions().get::<MatchedPath>().map_or_else(
        || masked_path(request.uri().path()),
        |matched| matched.as_str().to_owned(),
    )
}

/// The answer to a handler's panic, which is logged here.
#[expect(
    clippy::needless_pass_by_value,
    reason = "CatchPanicLayer hands the panic over by value"
)]
fn panicked(panic: Box<dyn std::any::Any + Send + 'static>) -> Response {
    let message = panic
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| panic.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("(not a string)");
    tracing::error!(panic = message, "request handler panicked");
    ApiError::Internal.into_response()
}

/// Answers 503 for a request that takes longer than `limit`. An upload is let run: a phone's
/// video takes minutes to arrive, and an upload that stops arriving is given up on by the
/// uploads service itself ([`crate::services::uploads::IDLE`]). So are a worker's endpoint and
/// its gate, and the few routes that wait on something slow by design ([`routes::takes_long`]:
/// a call's audio, a transcription).
async fn time_limit(State(limit): State<Duration>, request: Request<Body>, next: Next) -> Response {
    // An upload reads its body as it comes, and a backup copies the database before its
    // first byte: both take as long as the data does, so neither has the time limit. A worker's
    // tools wait by design (`ultra_wait` for its lanes, `task_ask` for the ask guard's judge),
    // and the gate asks the owner before it answers; each bounds its own wait.
    let path = request.uri().path();
    let unlimited = (request.method() == Method::POST
        && [paths::UPLOADS, paths::MCP_WORKER, paths::ULTRA_GATE].contains(&path))
        || (request.method() == Method::GET && path == paths::BACKUP);
    if unlimited {
        return next.run(request).await;
    }
    if request.method() == Method::POST && routes::takes_long(request.uri().path()) {
        return next.run(request).await;
    }
    tokio::time::timeout(limit, next.run(request))
        .await
        .unwrap_or_else(|_| ApiError::Timeout.into_response())
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::StatusCode;
    use axum::routing::get;
    use emberhut_db::DatabaseUrl;
    use http_body_util::BodyExt as _;
    use serde_json::{Value, json};
    use tower::ServiceExt as _;

    use super::*;

    /// The middleware around two misbehaving routes.
    async fn misbehaving(dir: &tempfile::TempDir) -> Router {
        let url: DatabaseUrl = format!("sqlite://{}", dir.path().join("emberhut.db").display())
            .parse()
            .unwrap();
        let db = Database::connect(&url).await.unwrap();
        let state = AppState::new(&db, Dependencies::default(), AuthMode::Enforce);
        #[expect(
            clippy::disallowed_methods,
            reason = "test-only routes that misbehave on purpose; they are not part of the API"
        )]
        let routes = Router::new()
            .route(
                "/slow",
                get(|| async { tokio::time::sleep(Duration::from_secs(60)).await }),
            )
            .route("/panic", get(|| async { panic!("a handler bug") }))
            .route(
                paths::MCP_WORKER,
                axum::routing::post(|| async {
                    tokio::time::sleep(Duration::from_secs(60)).await;
                    "ok"
                }),
            )
            .route(
                paths::ULTRA_GATE,
                axum::routing::post(|| async {
                    tokio::time::sleep(Duration::from_secs(60)).await;
                    "ok"
                }),
            );
        with_middleware(routes, state, Duration::from_secs(1))
    }

    async fn answer(app: Router, path: &str) -> (StatusCode, Value) {
        let request = Request::get(path).body(Body::empty()).unwrap();
        let response = app.oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (status, serde_json::from_slice(&bytes).unwrap())
    }

    #[tokio::test]
    async fn a_slow_request_is_answered_with_503() {
        let dir = tempfile::tempdir().unwrap();
        let app = misbehaving(&dir).await;
        // Paused only now: opening the database waits on real I/O.
        tokio::time::pause();
        let (status, body) = answer(app, "/slow").await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(
            body,
            json!({ "error": "The server took too long to answer. Try again." })
        );
    }

    #[tokio::test]
    async fn a_worker_s_endpoint_and_its_gate_have_no_time_limit() {
        let dir = tempfile::tempdir().unwrap();
        let app = misbehaving(&dir).await;
        tokio::time::pause();
        for path in [paths::MCP_WORKER, paths::ULTRA_GATE] {
            let request = Request::post(path).body(Body::empty()).unwrap();
            let response = app.clone().oneshot(request).await.unwrap();
            assert_eq!(response.status(), StatusCode::OK, "{path}");
        }
    }

    #[tokio::test]
    async fn a_panicking_handler_is_answered_with_500() {
        let dir = tempfile::tempdir().unwrap();
        let (status, body) = answer(misbehaving(&dir).await, "/panic").await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(
            body,
            json!({ "error": "Something went wrong on the server." })
        );
    }
}
