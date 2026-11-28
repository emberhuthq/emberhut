//! A scripted, in-memory [`Backend`] for the store's tests: state polls, marks seen and
//! messages answer from queues, and every call is recorded.

use std::collections::{HashMap, VecDeque};
use std::io::Write;
use std::ops::ControlFlow;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use emberhut_api::agent_provider::{AgentProvider, ProviderRequest, ProviderTest};
use emberhut_api::approvals::{
    ApprovalReply, ApprovalsPage, ApprovalsQuery, Decision, DecisionRequest, WithdrawRequest,
};
use emberhut_api::auth::PairingCode;
use emberhut_api::calls::RecordCallRequest;
use emberhut_api::chat::{ChatFile, ChatPage, FileKind};
use emberhut_api::ember_name::EmberName;
use emberhut_api::headers::ETag;
use emberhut_api::ids::{ApprovalId, DeviceId, MessageId, RoomId, TaskId};
use emberhut_api::live::LiveEvent;
use emberhut_api::machines::MachinesPage;
use emberhut_api::projects::ProjectSetRequest;
use emberhut_api::talk::{TalkReply, TalkRequest};
use emberhut_api::task_actions::TaskAction;
use emberhut_api::task_message::{MessageTo, TaskMessageReply, TaskMessageRequest};
use emberhut_api::task_page::TaskPage;
use emberhut_api::task_search::TaskQuery;
use emberhut_api::tasks::Issue;
use emberhut_api::theme::{ThemeChange, ThemePage};
use emberhut_api::time::Timestamp;

use super::{Backend, Error, RoomEvent, Serving, StateReply, UploadSource};
use crate::config::ServerUrl;
use emberhut_api::browser::{BrowserInfo, BrowserInput, BrowserSession};
use emberhut_api::calendar::{CalendarPage, CalendarQuery};
use emberhut_api::chrome::{ChromeAction, ChromeReply};
use emberhut_api::dark::{DarkEvent, DarkRoom};
use emberhut_api::environments::{EnvActionRequest, EnvPage};
use emberhut_api::facts::FactsPage;
use emberhut_api::ids::BrowserDeviceId;
use emberhut_api::ids::MachineJobId;
use emberhut_api::ids::PartyId;
use emberhut_api::jobs::{JobAction, JobsPage};
use emberhut_api::notes::{NoteSave, NoteSaved, NotesPage};
use emberhut_api::party::{NewParty, PartiesPage, Party, PartyAction, PartyEvent};
use emberhut_api::scheduled::ScheduledPage;
use emberhut_api::settings::SettingsAction;
use emberhut_api::switches::{SwitchAction, SwitchPage};
use emberhut_api::usage::{UseAccountReply, UseAccountRequest};

pub(crate) mod account;
pub(crate) mod approvals;
pub(crate) mod browser;
pub(crate) mod days;
pub(crate) mod party;
pub(crate) mod provider;
pub(crate) mod releases;
pub(crate) mod sessions;
pub(crate) mod settings;
pub(crate) mod setup;
pub(crate) mod working;

use self::account::AccountReplies;
use self::approvals::ApprovalReplies;
use self::browser::BrowserReplies;
use self::days::DaysReplies;
use self::party::PartyReplies;
use self::provider::ProviderReplies;
use self::releases::ReleaseReplies;
use self::sessions::SessionReplies;
use self::settings::SettingsReplies;
use self::setup::SetupReplies;
use self::working::WorkingReplies;
use emberhut_api::ids::SessionId;
use emberhut_api::runner::actions::{SendToSession, SentToSession, SessionSettings};
use emberhut_api::runner::new_session::{
    BranchList, BranchQuery, FolderList, FolderQuery, OpenSession, OpenedSession,
};
use emberhut_api::runner::transcript::{SessionLive, StepsQuery, Transcript};
use emberhut_api::threads::{ActivityQuery, ThreadActivity};

/// A call the store made.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Call {
    /// A state poll for a room, with or without a fingerprint, and whether it said the room
    /// was being looked at.
    State(RoomId, Option<String>, bool),
    /// The room marked seen.
    Seen(RoomId),
    /// A message, as sent.
    Talk(TalkRequest),
    /// A file stored, by its name.
    Upload(String),
    /// An upload stopped by the app, by its name.
    UploadStopped(String),
    /// A file fetched to disk, by its path.
    Download(String),
    /// An answer.
    Decide(TaskId, String),
    /// A page of a room's chat before the message given.
    History(RoomId, MessageId),
    /// A pairing code.
    Pair(String),
    /// A pairing from a link: its code, and the machine it names.
    PairWithMachine(String, emberhut_identity::MachineId),
    /// A link statement asked of the server.
    LinkStatement(emberhut_api::connect::LinkStatementRequest),
    /// A theme change, as sent.
    ChangeTheme(ThemeChange),
    /// A rename of the ember: the name sent (none: back to the default).
    RenameEmber(Option<String>),
    /// The machines.
    Machines,
    /// A file, by its path.
    File(String),
    /// A room's catch-up, after the message given (none: its newest page).
    CatchUp(RoomId, Option<MessageId>),
    /// A project's status set.
    SetProject(ProjectSetRequest),
    /// A device signed out.
    Revoke(DeviceId),
    /// This device's session forgotten here.
    SignOutHere,
    /// A pairing code made for another device.
    NewPairingCode,
    /// The turn in a room stopped.
    Interrupt(RoomId),
    /// Recording a call started or stopped.
    Recording(RecordCallRequest),
    /// A recording turned into text: its name, the words told, and its size.
    Transcribe(String, Vec<String>, usize),
    /// One of the user's changes.
    Settings(SettingsAction),
    /// The switches read.
    Switches,
    /// A switch changed, or a server reconnected.
    Switch(SwitchAction),
    /// The facts read.
    Facts,
    /// A Chrome change.
    Chrome(ChromeAction),
    /// The agent's provider read.
    AgentProvider,
    /// A provider checked, as sent.
    TestAgentProvider(ProviderRequest),
    /// A provider saved, as sent.
    SaveAgentProvider(ProviderRequest),
    /// The saved provider removed.
    ForgetAgentProvider,
    /// The setup checklist read.
    Setup,
    /// A task's page, by its id.
    Task(TaskId),
    /// A task search, by its query string.
    FindTasks(String),
    /// A message on a task: the task and the text.
    MessageTask(TaskId, String),
    /// An action on tasks.
    Act(TaskAction),
    /// A task renamed: the task and its new title.
    RenameTask(TaskId, String),
    /// A session's transcript: the session and the query string.
    Transcript(SessionId, String),
    /// A session's live stream followed: the session and the query string.
    FollowSession(SessionId, String),
    /// A message to a session: the session and the text.
    SendToSession(SessionId, String),
    /// A session stopped.
    StopSession(SessionId),
    /// A session's running turn stopped.
    InterruptSession(SessionId),
    /// A session's settings changed.
    ChangeSession(SessionId, SessionSettings),
    /// A session moved to a task.
    MoveSession(SessionId, TaskId),
    /// A session of the user's own opened.
    OpenSession(OpenSession),
    /// A machine's folders, by the query string.
    Folders(String),
    /// A repository's branches, by the query string.
    Branches(String),
    /// An external thread's steps, by the query string.
    Thread(String),
    /// The environments read.
    Environments,
    /// An environment's action.
    EnvironmentAction(EnvActionRequest),
    /// The accounts' use checked.
    CheckUsage,
    /// The assistant's account picked.
    UseAccount(UseAccountRequest),
    /// The jobs read, fresh or not.
    Jobs(bool),
    /// A job stopped or cleared.
    JobAction(JobAction, MachineJobId),
    /// The parties read.
    Parties,
    /// A new party, as sent.
    StartParty(NewParty),
    /// A control on a party: the party and the control.
    ActOnParty(PartyId, PartyAction),
    /// A party's stream followed.
    FollowParty(PartyId),
    /// The Dark room read.
    Dark,
    /// A message in the Dark room.
    SendDark(String),
    /// The Dark room cleared.
    ClearDark,
    /// The browser's live view asked for, by the device.
    BrowserSession(BrowserDeviceId),
    /// An input to the browser.
    BrowserInput(BrowserInput),
    /// A file for the browser's page: its name and size.
    BrowserUpload(String, usize),
    /// What is scheduled read.
    Scheduled,
    /// The calendar read, by its query string.
    Calendar(String),
    /// The notes read.
    Notes,
    /// A note saved.
    SaveNote(NoteSave),
    /// The release feed, asked for in this format first.
    ReleaseFeed(u32),
    /// A release payload, by its path, with the cap asked for.
    DownloadRelease(String, u64),
    /// The approval requests read, by the query string.
    Approvals(String),
    /// One approval request read.
    Approval(ApprovalId),
    /// A decision on an approval request: the request, the answer and the payload hash sent.
    DecideApproval(ApprovalId, Decision, String),
    /// An approval request withdrawn.
    WithdrawApproval(ApprovalId),
    /// Stopped for another copy of the app.
    Stop,
    /// The user's Try again of the server, from the fallback server.
    TryPrimary,
}

/// The fake.
#[derive(Debug)]
pub(crate) struct FakeBackend {
    server: ServerUrl,
    states: Mutex<VecDeque<Result<StateReply, Error>>>,
    seen: Mutex<VecDeque<Result<(), Error>>>,
    talk: Mutex<VecDeque<Result<TalkReply, Error>>>,
    uploads: Mutex<VecDeque<Result<ChatFile, Error>>>,
    downloads: Mutex<VecDeque<Result<Vec<u8>, Error>>>,
    /// The answers to the next stops, recordings and transcriptions (then they go through).
    interrupts: Mutex<VecDeque<Result<(), Error>>>,
    recordings: Mutex<VecDeque<Result<(), Error>>>,
    transcripts: Mutex<VecDeque<Result<String, Error>>>,
    calls: Mutex<Vec<Call>>,
    /// What a live stream sends before it waits for `stop`.
    live: Vec<RoomEvent>,
    /// The files on the server, by path.
    files: HashMap<String, Vec<u8>>,
    catch_ups: Mutex<VecDeque<Result<ChatPage, Error>>>,
    /// The answers to the next pages of earlier messages (then empty pages).
    histories: Mutex<VecDeque<Result<ChatPage, Error>>>,
    /// The server's answers to the next answers sent (then they go through).
    decisions: Mutex<VecDeque<Result<(), Error>>>,
    device: Option<DeviceId>,
    /// The answers to the user's changes and the pages behind Settings.
    pub(crate) replies: SettingsReplies,
    /// The answers about where the assistant works and what it uses.
    pub(crate) working: WorkingReplies,
    /// The tasks' pages, by id (any other is not found).
    pages: Mutex<HashMap<TaskId, TaskPage>>,
    /// The answers to task searches, in turn (then none found).
    found: Mutex<VecDeque<Result<Vec<Issue>, Error>>>,
    /// The answers to messages on tasks, in turn (then: to the worker).
    task_messages: Mutex<VecDeque<Result<TaskMessageReply, Error>>>,
    /// The answers to project changes, device sign-outs and task actions, in turn (then they
    /// go through).
    actions: Mutex<VecDeque<Result<(), Error>>>,
    /// The answers to renames, in turn (by default each goes through).
    renames: Mutex<VecDeque<Result<(), Error>>>,
    /// The runner sessions' transcripts, streams and answers.
    pub(crate) sessions: SessionReplies,
    /// The answers about the assistant's browser.
    pub(crate) browser: BrowserReplies,
    /// Party's and the Dark room's answers.
    pub(crate) party: PartyReplies,
    /// The answers to the pages about the user's days.
    pub(crate) days: DaysReplies,
    /// The release feed and payloads.
    pub(crate) releases: ReleaseReplies,
    /// The account's answers: link statements, pairings from a link.
    pub(crate) account: AccountReplies,
    /// The approval requests and the answers on them.
    pub(crate) approvals: ApprovalReplies,
    /// Which server it stands for, and whether the server answers a Try again.
    failover: Mutex<(Serving, bool)>,
    /// The agent's provider: what is saved, checks and saves.
    pub(crate) provider: ProviderReplies,
    /// The setup checklist, and whether the server answers its health check.
    pub(crate) setup: SetupReplies,
    /// Whether this device has a session of its own: from the start, or once a code is traded.
    session: std::sync::atomic::AtomicBool,
}

impl FakeBackend {
    /// A fake that answers state polls with `states`, in turn (then "unchanged").
    pub(crate) fn new(states: impl IntoIterator<Item = Result<StateReply, Error>>) -> Self {
        Self {
            server: ServerUrl::parse("http://fake.example").expect("an address"),
            states: Mutex::new(states.into_iter().collect()),
            seen: Mutex::new(VecDeque::new()),
            talk: Mutex::new(VecDeque::new()),
            uploads: Mutex::new(VecDeque::new()),
            downloads: Mutex::new(VecDeque::new()),
            interrupts: Mutex::new(VecDeque::new()),
            recordings: Mutex::new(VecDeque::new()),
            transcripts: Mutex::new(VecDeque::new()),
            calls: Mutex::new(Vec::new()),
            live: Vec::new(),
            files: HashMap::new(),
            catch_ups: Mutex::new(VecDeque::new()),
            histories: Mutex::new(VecDeque::new()),
            decisions: Mutex::new(VecDeque::new()),
            device: None,
            replies: SettingsReplies::default(),
            working: WorkingReplies::default(),
            pages: Mutex::new(HashMap::new()),
            found: Mutex::new(VecDeque::new()),
            task_messages: Mutex::new(VecDeque::new()),
            actions: Mutex::new(VecDeque::new()),
            renames: Mutex::new(VecDeque::new()),
            sessions: SessionReplies::default(),
            browser: BrowserReplies::default(),
            party: PartyReplies::default(),
            days: DaysReplies::default(),
            releases: ReleaseReplies::default(),
            account: AccountReplies::default(),
            approvals: ApprovalReplies::default(),
            failover: Mutex::new((Serving::Primary, false)),
            provider: ProviderReplies::default(),
            setup: SetupReplies::default(),
            session: std::sync::atomic::AtomicBool::new(false),
        }
    }

    /// The same fake, with a session of its own already (as after a pairing).
    pub(crate) fn with_session(self) -> Self {
        self.session
            .store(true, std::sync::atomic::Ordering::SeqCst);
        self
    }

    /// The same fake, on the fallback server `fallback` since `primary` stopped answering.
    pub(crate) fn on_fallback(self, primary: &str, fallback: &str) -> Self {
        let serving = Serving::Fallback {
            primary: ServerUrl::parse(primary).expect("an address"),
            fallback: ServerUrl::parse(fallback).expect("an address"),
        };
        *self.failover.lock().unwrap_or_else(PoisonError::into_inner) = (serving, false);
        self
    }

    /// Whether the server answers the next Try again.
    pub(crate) fn primary_answers(&self, answers: bool) {
        self.failover
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .1 = answers;
    }

    /// The same fake, as the server at `address`.
    pub(crate) fn at(mut self, address: &str) -> Self {
        self.server = ServerUrl::parse(address).expect("an address");
        self
    }

    /// The same fake, signed in as `device`.
    pub(crate) fn signed_in_as(mut self, device: DeviceId) -> Self {
        self.device = Some(device);
        self
    }

    /// Queues the answer to the next catch-up (by default an empty page).
    pub(crate) fn push_catch_up(&self, reply: Result<ChatPage, Error>) {
        self.catch_ups
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push_back(reply);
    }

    /// Queues the answer to the next page of earlier messages (by default an empty page).
    pub(crate) fn push_history(&self, reply: Result<ChatPage, Error>) {
        self.histories
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push_back(reply);
    }

    /// Serves `page` as its task's page (replacing one served before).
    pub(crate) fn put_task(&self, page: TaskPage) {
        self.pages
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(page.task.id, page);
    }

    /// Queues the answer to the next task search.
    pub(crate) fn push_found(&self, reply: Result<Vec<Issue>, Error>) {
        self.found
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push_back(reply);
    }

    /// Queues the server's answer to the next answer sent (by default it goes through).
    pub(crate) fn push_decision(&self, reply: Result<(), Error>) {
        self.decisions
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push_back(reply);
    }

    /// Queues the answer to the next message on a task.
    pub(crate) fn push_task_message(&self, reply: Result<TaskMessageReply, Error>) {
        self.task_messages
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push_back(reply);
    }

    /// Queues the answer to the next rename (by default it goes through).
    pub(crate) fn push_rename(&self, reply: Result<(), Error>) {
        self.renames
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push_back(reply);
    }

    /// Queues the answer to the next project change, device sign-out or task action (by
    /// default they go through).
    pub(crate) fn push_action(&self, reply: Result<(), Error>) {
        self.actions
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push_back(reply);
    }

    fn next_action(&self) -> Result<(), Error> {
        self.actions
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or(Ok(()))
    }

    /// The same fake, its live streams sending `events` and then staying open until stopped.
    pub(crate) fn with_live(self, events: Vec<LiveEvent>) -> Self {
        self.with_room_events(events.into_iter().map(RoomEvent::Live).collect())
    }

    /// The same fake, its live streams sending `events` (lines and approvals' changes) and
    /// then staying open until stopped.
    pub(crate) fn with_room_events(mut self, events: Vec<RoomEvent>) -> Self {
        self.live = events;
        self
    }

    /// The same fake, serving `bytes` at `path`.
    pub(crate) fn with_file(mut self, path: &str, bytes: &[u8]) -> Self {
        self.files.insert(path.to_owned(), bytes.to_vec());
        self
    }

    /// Queues the reply to the next mark seen (then they go through).
    pub(crate) fn push_seen(&self, reply: Result<(), Error>) {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push_back(reply);
    }

    /// Queues the reply to the next message.
    pub(crate) fn push_talk(&self, reply: Result<TalkReply, Error>) {
        self.talk
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push_back(reply);
    }

    /// Queues the answer to the next upload (by default the file is stored as
    /// `/uploads/<name>`).
    pub(crate) fn push_upload(&self, reply: Result<ChatFile, Error>) {
        self.uploads
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push_back(reply);
    }

    /// Queues the answer to the next download: the file's bytes, or why it failed (by default
    /// it isn't there).
    pub(crate) fn push_download(&self, reply: Result<Vec<u8>, Error>) {
        self.downloads
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push_back(reply);
    }

    /// Queues the answer to the next stop (by default it goes through).
    pub(crate) fn push_interrupt(&self, reply: Result<(), Error>) {
        self.interrupts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push_back(reply);
    }

    /// Queues the answer to the next start or stop of a call's recording (by default it goes
    /// through).
    pub(crate) fn push_record_call(&self, reply: Result<(), Error>) {
        self.recordings
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push_back(reply);
    }

    /// Queues the answer to the next transcription (by default, "hello there").
    pub(crate) fn push_transcript(&self, reply: Result<String, Error>) {
        self.transcripts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push_back(reply);
    }

    /// The file an upload of `name` stores, unless another answer was queued.
    pub(crate) fn stored(name: &str) -> ChatFile {
        ChatFile {
            name: name.to_owned(),
            url: format!("/uploads/{name}"),
            kind: FileKind::File,
            path: Some(format!("/data/uploads/{name}")),
        }
    }

    /// The calls so far.
    pub(crate) fn calls(&self) -> Vec<Call> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn record(&self, call: Call) {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call);
    }

    fn unavailable<T>() -> Result<T, Error> {
        Err(Error::Refused {
            status: Some(404),
            message: Some("not in this test".into()),
        })
    }
}

impl Backend for FakeBackend {
    fn server(&self) -> &ServerUrl {
        &self.server
    }

    fn is_signed_in(&self) -> bool {
        self.session.load(std::sync::atomic::Ordering::SeqCst)
    }

    fn state(&self, room: RoomId, etag: Option<&ETag>, looking: bool) -> Result<StateReply, Error> {
        self.record(Call::State(
            room,
            etag.map(|tag| tag.as_str().to_owned()),
            looking,
        ));
        self.states
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or(Ok(StateReply::Unchanged))
    }

    fn seen(&self, room: RoomId) -> Result<(), Error> {
        self.record(Call::Seen(room));
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or(Ok(()))
    }

    fn history(&self, room: RoomId, before: MessageId) -> Result<ChatPage, Error> {
        self.record(Call::History(room, before));
        self.histories
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or_else(|| Ok(ChatPage::default()))
    }

    fn talk(&self, request: &TalkRequest) -> Result<TalkReply, Error> {
        self.record(Call::Talk(request.clone()));
        self.talk
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or(Ok(TalkReply {
                ok: true,
                started: true,
                id: None,
            }))
    }

    /// Says half of it went, then all of it, then answers; stops when told to.
    fn upload(
        &self,
        name: &str,
        source: &UploadSource,
        progress: &mut dyn FnMut(u64) -> ControlFlow<()>,
    ) -> Result<ChatFile, Error> {
        self.record(Call::Upload(name.to_owned()));
        let size = match source {
            UploadSource::File(path) => std::fs::metadata(path).map_or(0, |meta| meta.len()),
            UploadSource::Bytes(bytes) => {
                u64::try_from((**bytes).as_ref().len()).unwrap_or(u64::MAX)
            }
        };
        if progress(size / 2).is_break() || progress(size).is_break() {
            self.record(Call::UploadStopped(name.to_owned()));
            return Err(Error::Local("the upload was stopped".into()));
        }
        self.uploads
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or_else(|| Ok(Self::stored(name)))
    }

    /// Writes the queued file in two halves, telling each, or fails as queued.
    fn download(
        &self,
        path: &str,
        to: &mut dyn Write,
        progress: &mut dyn FnMut(u64, Option<u64>) -> ControlFlow<()>,
    ) -> Result<u64, Error> {
        self.record(Call::Download(path.to_owned()));
        let bytes = self
            .downloads
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or_else(Self::unavailable)?;
        let size = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        let (first, second) = bytes.split_at(bytes.len() / 2);
        for (part, got) in [(first, size / 2), (second, size)] {
            to.write_all(part)
                .map_err(|error| Error::Local(Box::new(error)))?;
            if progress(got, Some(size)).is_break() {
                return Err(Error::Local("the download was stopped".into()));
            }
        }
        Ok(size)
    }

    fn decide(&self, task: TaskId, choice: &str) -> Result<(), Error> {
        self.record(Call::Decide(task, choice.to_owned()));
        self.decisions
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or(Ok(()))
    }

    fn theme(&self) -> Result<ThemePage, Error> {
        Self::unavailable()
    }

    /// The built-in theme, with the change's preset, appearance and tint.
    fn change_theme(&self, change: &ThemeChange) -> Result<ThemePage, Error> {
        self.record(Call::ChangeTheme(change.clone()));
        let mut page = crate::theme::built_in();
        if let Some(preset) = &change.preset {
            page.choice.preset.clone_from(preset);
        }
        if let Some(appearance) = &change.appearance {
            page.choice.appearance = appearance.clone();
        }
        if let Some(tint) = &change.ember_tint {
            page.choice.ember_tint.clone_from(tint);
        }
        Ok(page)
    }

    fn rename_ember(&self, name: Option<&str>) -> Result<EmberName, Error> {
        self.record(Call::RenameEmber(name.map(str::to_owned)));
        self.replies.rename(name)
    }

    fn machines(&self) -> Result<MachinesPage, Error> {
        self.record(Call::Machines);
        serde_json::from_value(serde_json::json!({"at": "2026-01-01T00:00:00Z", "machines": []}))
            .map_err(|error| Error::Unreadable(Box::new(error)))
    }

    fn file(&self, path: &str) -> Result<Vec<u8>, Error> {
        self.record(Call::File(path.to_owned()));
        self.files
            .get(path)
            .cloned()
            .map_or_else(Self::unavailable, Ok)
    }

    fn pair(&self, code: &str) -> Result<(), Error> {
        self.record(Call::Pair(code.to_owned()));
        let paired = self.account.pairing();
        if paired.is_ok() {
            self.session
                .store(true, std::sync::atomic::Ordering::SeqCst);
        }
        paired
    }

    fn pair_with_machine(
        &self,
        code: &str,
        machine: &emberhut_identity::MachineId,
    ) -> Result<(), Error> {
        self.record(Call::PairWithMachine(code.to_owned(), *machine));
        self.account.pairing()
    }

    fn link_statement(
        &self,
        request: &emberhut_api::connect::LinkStatementRequest,
    ) -> Result<emberhut_api::connect::SignedLink, Error> {
        self.record(Call::LinkStatement(request.clone()));
        self.account.link_statement()
    }

    /// Sends the scripted events, then stays open (as a real stream does between steps)
    /// until `stop` says so, asking it every few milliseconds.
    fn follow_live(
        &self,
        _room: RoomId,
        stop: &dyn Fn() -> bool,
        on: &mut dyn FnMut(RoomEvent),
    ) -> Result<(), Error> {
        for event in &self.live {
            on(event.clone());
        }
        while !stop() {
            std::thread::sleep(Duration::from_millis(5));
        }
        Ok(())
    }

    fn catch_up(&self, room: RoomId, after: Option<MessageId>) -> Result<ChatPage, Error> {
        self.record(Call::CatchUp(room, after));
        self.catch_ups
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or_else(|| Ok(ChatPage::default()))
    }

    fn set_project(&self, change: &ProjectSetRequest) -> Result<(), Error> {
        self.record(Call::SetProject(change.clone()));
        self.next_action()
    }

    fn device(&self) -> Option<DeviceId> {
        self.device
    }

    fn revoke_device(&self, device: DeviceId) -> Result<(), Error> {
        self.record(Call::Revoke(device));
        self.next_action()
    }

    fn sign_out_here(&self) -> Result<(), Error> {
        self.record(Call::SignOutHere);
        Ok(())
    }

    /// A code that works for ten minutes from the fixtures' day.
    fn new_pairing_code(&self) -> Result<PairingCode, Error> {
        self.record(Call::NewPairingCode);
        Ok(PairingCode {
            ok: true,
            code: "ABCD-EFGH".to_owned(),
            expires: Timestamp::new("2026-01-01T00:10:00.000Z"),
            link: None,
        })
    }

    fn interrupt(&self, room: RoomId) -> Result<(), Error> {
        self.record(Call::Interrupt(room));
        self.interrupts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or(Ok(()))
    }

    fn task(&self, id: TaskId) -> Result<TaskPage, Error> {
        self.record(Call::Task(id));
        self.pages
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&id)
            .cloned()
            .ok_or_else(|| Error::Refused {
                status: Some(404),
                message: Some("no such task".to_owned()),
            })
    }

    fn find_tasks(&self, query: &TaskQuery) -> Result<Vec<Issue>, Error> {
        self.record(Call::FindTasks(query.to_query()));
        self.found
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or_else(|| Ok(Vec::new()))
    }

    fn message_task(
        &self,
        id: TaskId,
        message: &TaskMessageRequest,
    ) -> Result<TaskMessageReply, Error> {
        self.record(Call::MessageTask(id, message.text.clone()));
        self.task_messages
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or_else(|| {
                Ok(TaskMessageReply {
                    ok: true,
                    to: MessageTo::Worker,
                    session: None,
                    queued: None,
                })
            })
    }

    fn act_on_tasks(&self, action: &TaskAction) -> Result<(), Error> {
        self.record(Call::Act(action.clone()));
        self.actions
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or(Ok(()))
    }

    fn rename_task(&self, id: TaskId, title: &str) -> Result<Issue, Error> {
        self.record(Call::RenameTask(id, title.to_owned()));
        self.renames
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or(Ok(()))
            .map(|()| crate::fixtures::issue(id.get(), "todo", serde_json::json!({"title": title})))
    }

    fn record_call(&self, request: &RecordCallRequest) -> Result<(), Error> {
        self.record(Call::Recording(*request));
        self.recordings
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or(Ok(()))
    }

    fn transcribe(&self, name: &str, words: &[String], audio: &[u8]) -> Result<String, Error> {
        self.record(Call::Transcribe(
            name.to_owned(),
            words.to_vec(),
            audio.len(),
        ));
        self.transcripts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or_else(|| Ok("hello there".to_owned()))
    }

    fn settings(&self, action: SettingsAction) -> Result<(), Error> {
        self.record(Call::Settings(action));
        self.replies.settings()
    }

    fn switches(&self) -> Result<SwitchPage, Error> {
        self.record(Call::Switches);
        self.replies.switches()
    }

    fn switch(&self, action: &SwitchAction) -> Result<SwitchPage, Error> {
        self.record(Call::Switch(action.clone()));
        self.replies.switches()
    }

    fn facts(&self) -> Result<FactsPage, Error> {
        self.record(Call::Facts);
        self.replies.facts()
    }

    fn chrome(&self, action: &ChromeAction) -> Result<ChromeReply, Error> {
        self.record(Call::Chrome(action.clone()));
        self.replies.chrome()
    }

    fn agent_provider(&self) -> Result<AgentProvider, Error> {
        self.record(Call::AgentProvider);
        self.provider.status()
    }

    fn test_agent_provider(
        &self,
        request: &ProviderRequest,
        _within: Duration,
    ) -> Result<ProviderTest, Error> {
        self.record(Call::TestAgentProvider(request.clone()));
        self.provider.test()
    }

    fn save_agent_provider(&self, request: &ProviderRequest) -> Result<AgentProvider, Error> {
        self.record(Call::SaveAgentProvider(request.clone()));
        self.provider.save(request)
    }

    fn forget_agent_provider(&self) -> Result<AgentProvider, Error> {
        self.record(Call::ForgetAgentProvider);
        self.provider.forget()
    }

    fn setup(&self) -> Result<emberhut_api::setup::SetupPage, Error> {
        self.record(Call::Setup);
        self.setup.page()
    }

    fn answers_within(&self, _within: Duration) -> bool {
        self.setup.answers()
    }

    fn session_transcript(&self, id: &SessionId, query: &StepsQuery) -> Result<Transcript, Error> {
        self.record(Call::Transcript(id.clone(), query.to_query()));
        self.sessions.transcript(id)
    }

    /// Sends the next scripted events, then stays open until `stop` says so.
    fn follow_session(
        &self,
        id: &SessionId,
        query: &StepsQuery,
        stop: &dyn Fn() -> bool,
        on: &mut dyn FnMut(SessionLive),
    ) -> Result<(), Error> {
        self.record(Call::FollowSession(id.clone(), query.to_query()));
        for event in self.sessions.stream()? {
            on(event);
        }
        while !stop() {
            std::thread::sleep(Duration::from_millis(5));
        }
        Ok(())
    }

    fn send_to_session(
        &self,
        id: &SessionId,
        message: &SendToSession,
    ) -> Result<SentToSession, Error> {
        self.record(Call::SendToSession(id.clone(), message.text.clone()));
        self.sessions.send()
    }

    fn stop_session(&self, id: &SessionId) -> Result<(), Error> {
        self.record(Call::StopSession(id.clone()));
        self.sessions.action()
    }

    fn interrupt_session(&self, id: &SessionId) -> Result<(), Error> {
        self.record(Call::InterruptSession(id.clone()));
        self.sessions.action()
    }

    fn change_session(&self, id: &SessionId, settings: &SessionSettings) -> Result<(), Error> {
        self.record(Call::ChangeSession(id.clone(), settings.clone()));
        self.sessions.action()
    }

    fn move_session(&self, id: &SessionId, task: TaskId) -> Result<TaskId, Error> {
        self.record(Call::MoveSession(id.clone(), task));
        self.sessions.moved(task)
    }

    fn open_session(&self, session: &OpenSession) -> Result<OpenedSession, Error> {
        self.record(Call::OpenSession(session.clone()));
        self.sessions.open()
    }

    fn runner_folders(&self, query: &FolderQuery) -> Result<FolderList, Error> {
        self.record(Call::Folders(query.to_query()));
        self.sessions.folders()
    }

    fn runner_branches(&self, query: &BranchQuery) -> Result<BranchList, Error> {
        self.record(Call::Branches(query.to_query()));
        self.sessions.branches()
    }

    fn t3_activity(&self, query: &ActivityQuery) -> Result<ThreadActivity, Error> {
        self.record(Call::Thread(query.to_query()));
        self.sessions.t3()
    }

    fn environments(&self) -> Result<EnvPage, Error> {
        self.record(Call::Environments);
        self.working.environments()
    }

    fn environment_action(&self, request: &EnvActionRequest) -> Result<EnvPage, Error> {
        self.record(Call::EnvironmentAction(request.clone()));
        self.working.environment_action()
    }

    fn check_usage(&self) -> Result<(), Error> {
        self.record(Call::CheckUsage);
        Ok(())
    }

    fn use_account(&self, request: &UseAccountRequest) -> Result<UseAccountReply, Error> {
        self.record(Call::UseAccount(request.clone()));
        self.working.use_account(request)
    }

    fn jobs(&self, fresh: bool) -> Result<JobsPage, Error> {
        self.record(Call::Jobs(fresh));
        self.working.jobs()
    }

    fn act_on_job(&self, action: JobAction, job: &MachineJobId) -> Result<(), Error> {
        self.record(Call::JobAction(action, job.clone()));
        self.working.job_action()
    }

    fn parties(&self) -> Result<PartiesPage, Error> {
        self.record(Call::Parties);
        self.party.parties()
    }

    fn start_party(&self, party: &NewParty) -> Result<Party, Error> {
        self.record(Call::StartParty(party.clone()));
        self.party.started()
    }

    fn act_on_party(&self, id: PartyId, action: &PartyAction) -> Result<Option<Party>, Error> {
        self.record(Call::ActOnParty(id, action.clone()));
        self.party.act()
    }

    /// Sends the next scripted events, then ends as scripted (by default as when the server
    /// closes the stream), at once.
    fn follow_party(
        &self,
        id: PartyId,
        stop: &dyn Fn() -> bool,
        on: &mut dyn FnMut(PartyEvent),
    ) -> Result<(), Error> {
        self.record(Call::FollowParty(id));
        for event in self.party.live() {
            if stop() {
                break;
            }
            on(event);
        }
        self.party.live_end()
    }

    fn dark(&self) -> Result<DarkRoom, Error> {
        self.record(Call::Dark);
        self.party.dark()
    }

    fn send_dark(&self, text: &str, on: &mut dyn FnMut(DarkEvent)) -> Result<(), Error> {
        self.record(Call::SendDark(text.to_owned()));
        self.party.wait_for_answer();
        let (events, end) = self.party.answer();
        for event in events {
            on(event);
        }
        end
    }

    fn clear_dark(&self) -> Result<(), Error> {
        self.record(Call::ClearDark);
        self.party.clear()
    }

    fn browser_session(&self, device: &BrowserDeviceId) -> Result<BrowserSession, Error> {
        self.record(Call::BrowserSession(device.clone()));
        self.browser.session()
    }

    fn browser_input(&self, input: &BrowserInput) -> Result<BrowserInfo, Error> {
        self.record(Call::BrowserInput(input.clone()));
        self.browser.input()
    }

    fn browser_upload(&self, name: &str, bytes: &[u8]) -> Result<BrowserInfo, Error> {
        self.record(Call::BrowserUpload(name.to_owned(), bytes.len()));
        self.browser.upload()
    }

    fn scheduled(&self) -> Result<ScheduledPage, Error> {
        self.record(Call::Scheduled);
        self.days.scheduled()
    }

    fn calendar(&self, query: &CalendarQuery) -> Result<CalendarPage, Error> {
        self.record(Call::Calendar(query.to_query()));
        self.days.calendar(query)
    }

    fn notes(&self) -> Result<NotesPage, Error> {
        self.record(Call::Notes);
        self.days.notes()
    }

    fn save_note(&self, note: &NoteSave) -> Result<NoteSaved, Error> {
        self.record(Call::SaveNote(note.clone()));
        self.days.save(note)
    }

    fn release_feed(
        &self,
        _platform: &emberhut_api::releases::Platform,
        format: u32,
    ) -> Result<Option<emberhut_api::releases::ReleaseFeed>, Error> {
        self.record(Call::ReleaseFeed(format));
        self.releases.feed()
    }

    fn download_release(
        &self,
        path: &str,
        to: &mut dyn Write,
        max_len: u64,
    ) -> Result<super::PayloadDigest, Error> {
        self.record(Call::DownloadRelease(path.to_owned(), max_len));
        self.releases.payload(to, max_len)
    }

    fn approvals(&self, query: &ApprovalsQuery) -> Result<ApprovalsPage, Error> {
        self.record(Call::Approvals(query.to_query()));
        self.approvals.page(query)
    }

    fn approval(&self, id: ApprovalId) -> Result<ApprovalReply, Error> {
        self.record(Call::Approval(id));
        self.approvals.one(id)
    }

    fn decide_approval(
        &self,
        id: ApprovalId,
        decision: &DecisionRequest,
    ) -> Result<ApprovalReply, Error> {
        self.record(Call::DecideApproval(
            id,
            decision.decision,
            decision.payload_hash.clone(),
        ));
        self.approvals.decide(id, decision)
    }

    fn withdraw_approval(
        &self,
        id: ApprovalId,
        _withdrawal: &WithdrawRequest,
    ) -> Result<ApprovalReply, Error> {
        self.record(Call::WithdrawApproval(id));
        self.approvals.withdraw(id)
    }

    fn stop(&self) -> Result<(), Error> {
        self.record(Call::Stop);
        Ok(())
    }

    fn serving(&self) -> Serving {
        self.failover
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .0
            .clone()
    }

    fn try_primary(&self) -> bool {
        self.record(Call::TryPrimary);
        let mut failover = self.failover.lock().unwrap_or_else(PoisonError::into_inner);
        if failover.1 {
            failover.0 = Serving::Primary;
        }
        failover.0 == Serving::Primary
    }
}
