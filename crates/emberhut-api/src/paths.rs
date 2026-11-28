//! The routes of the contract, relative to the server's base URL.

/// `POST`: trade a one-time pairing code for a device session.
pub const PAIR: &str = "/api/auth/pair";
/// `POST`: make a one-time pairing code (a signed-in device or the server's key only).
pub const PAIR_NEW: &str = "/api/auth/pair/new";
/// `POST`: a new access token for a refresh token (which turns over).
pub const REFRESH: &str = "/api/auth/refresh";
/// `GET`: the sign-in mode, and whether this request's session is a signed-in device's.
pub const AUTH_STATUS: &str = "/api/auth/status";
/// `GET`: what the server has set up, item by item ([`crate::setup::SetupPage`]), for a
/// signed-in device.
pub const SETUP: &str = "/api/setup";
/// `GET`: the model provider saved for the agent ([`crate::agent_provider::AgentProvider`]);
/// `PUT`: check one and save it ([`crate::agent_provider::ProviderRequest`]); `DELETE`: remove
/// it. A signed-in device only.
pub const AGENT_PROVIDER: &str = "/api/agent/provider";
/// `POST`: check a provider ([`crate::agent_provider::ProviderRequest`]) and list its models
/// ([`crate::agent_provider::ProviderTest`]), saving nothing. A signed-in device only.
pub const AGENT_PROVIDER_TEST: &str = "/api/agent/provider/test";
/// `GET`: the agent CLI accounts the sessions run on and whether each can take work
/// ([`crate::agent_accounts::AgentAccounts`]).
pub const AGENT_CLI_ACCOUNTS: &str = "/api/agent/accounts";
/// `GET`: the waitlists and whether the owner is on each ([`crate::waitlist::Waitlists`]).
pub const WAITLISTS: &str = "/api/waitlist";
/// `PUT`: join a waitlist ([`crate::waitlist::Waitlist`]); `{list}` is its id. A signed-in
/// device only.
pub const WAITLIST: &str = "/api/waitlist/{list}";
/// `POST`: where the server pushes this phone's alerts ([`crate::push::PushRegistration`]).
pub const DEVICES: &str = "/api/devices";
/// `GET`: a one-time secret link's page; `POST`: its form. `{token}` is the link's token.
pub const SECRET_PAGE: &str = "/s/{token}";
/// `GET ?room=`: everything the app shows for a room (with `ETag`).
pub const STATE: &str = "/api/state";
/// `GET ?room=&before=&after=`: a page of a room's chat.
pub const HISTORY: &str = "/api/history";
/// `GET ?room=`: the reply as it is being written, as server-sent events.
pub const LIVE: &str = "/api/live";
/// `POST`: say something in a room.
pub const TALK: &str = "/api/talk";
/// `POST ?name=`: store a file for the chat, the raw file as the body
/// ([`crate::uploads::UploadQuery`]).
pub const UPLOADS: &str = "/api/uploads";
/// `GET`: a stored file, by the `url` of its [`crate::chat::ChatFile`] (this prefix and the stored
/// name), with byte ranges.
pub const FILES: &str = "/uploads/";
/// `GET`: whether the server answers, and how late its runtime is (no sign-in).
pub const HEALTH: &str = "/health";
/// `GET`: how late the server's runtime is, and the work that held it.
pub const LOOP: &str = "/api/loop";
/// `POST`: the room's messages have been seen.
pub const SEEN: &str = "/api/seen";
/// `GET`: the machines that run sessions, their capacity and warnings.
pub const MACHINES: &str = "/api/machines";
/// `GET`: the theme; `POST`: change it.
pub const THEME: &str = "/api/theme";
/// `POST`: the person's changes, one route for all of them
/// ([`crate::settings::SettingsAction`]).
pub const SETTINGS: &str = "/api/settings";
/// `POST`: the MCP endpoint a room's agent session calls, with the room's agent token. The
/// path is in [`crate::wire`].
pub const MCP_ROOM: &str = crate::wire::mcp::ROOM_PATH;

/// The MCP endpoint a worker's session calls, with its task's token: its tools act on its own
/// task.
pub const MCP_WORKER: &str = "/mcp/worker";

/// `POST`: the MCP endpoint the runners' sessions call, with the runner sessions' token: a
/// task's plan and the task. The path is in [`crate::wire`].
pub const MCP_RUNNER: &str = crate::wire::mcp::RUNNER_PATH;

/// The ultra gate an ultra run's lead's hook asks before each new sub-agent and shell
/// command, with its task's token.
pub const ULTRA_GATE: &str = "/ultra/gate";

/// `GET`: the owner's Mac, as its row shows it: online, locked, idle ([`crate::mac::MacStatus`]).
pub const MAC: &str = "/api/mac";
/// `POST`: lock the Mac's screen now.
pub const MAC_LOCK: &str = "/api/mac/lock";
/// `GET`: the server's backup, a `.tar.gz` of a consistent snapshot of its database and the
/// files it keeps.
pub const BACKUP: &str = "/api/backup";
/// `POST`: start or stop recording a call on the Mac ([`crate::calls::RecordCallRequest`]).
pub const CALLS_RECORD: &str = "/api/calls/record";
/// `POST ?name=&words=`: speech to text, the raw recording as the body
/// ([`crate::transcribe::TranscribeQuery`]).
pub const TRANSCRIBE: &str = "/api/transcribe";
/// `GET`: the MCP servers and skills with their switches; `POST`: switch one, or reconnect a
/// server ([`crate::switches::SwitchAction`]).
pub const SWITCHES: &str = "/api/switches";
/// `GET`: what the assistant knows about the person, and its rules on trial.
pub const FACTS: &str = "/api/facts";
/// `POST`: pair, rename, unpair or pause a Chrome profile ([`crate::chrome::ChromeAction`]).
pub const CHROME: &str = "/api/chrome";
/// `GET ?q=&limit=&...`: tasks from the whole list ([`crate::task_search::TaskQuery`]);
/// `GET /{id}`: one task's page ([`crate::task_page::TaskPage`]); `PATCH /{id}`: the person
/// renames it ([`crate::task_rename::RenameTask`]); `POST /{id}/message`: the person writes on
/// it ([`crate::task_message::TaskMessageRequest`]).
pub const TASKS: &str = "/api/tasks";
/// `GET ?device=`: the live view of the assistant's browser, its address and sign-in
/// ([`crate::browser::BrowserSessionQuery`]).
pub const BROWSER_SESSION: &str = "/api/browser/session";
/// `POST`: take the assistant's browser over, give it back, or drive its toolbar
/// ([`crate::browser::BrowserInput`]).
pub const BROWSER_INPUT: &str = "/api/browser/input";
/// `POST ?name=`: a file for the page's file field, the raw file as the body
/// ([`crate::uploads::UploadQuery`]).
pub const BROWSER_UPLOAD: &str = "/api/browser/upload";
/// `GET`: the latest picture of the browser's page (JPEG); 404 while the person drives.
pub const BROWSER_FRAME: &str = "/api/browser/frame.jpg";

/// `GET`: the page of task `id`; `PATCH`: rename it.
#[must_use]
pub fn task(id: crate::ids::TaskId) -> String {
    format!("{TASKS}/{id}")
}

/// `POST`: a message on task `id`.
#[must_use]
pub fn task_message(id: crate::ids::TaskId) -> String {
    format!("{TASKS}/{id}/message")
}

/// `GET ?task=&room=&direct`: the sessions on the runners; `POST`: open one of the person's own
/// ([`crate::runner::new_session::OpenSession`]). One session's routes are under it
/// ([`session`]).
pub const RUNNER_SESSIONS: &str = "/api/runner/sessions";
/// `GET ?machine=&path=`: a runner machine's folders, for a new session
/// ([`crate::runner::new_session::FolderQuery`]).
pub const RUNNER_FOLDERS: &str = "/api/runner/folders";
/// `GET ?machine=&cwd=`: a repository's branches on a runner machine, for a new session's
/// worktree ([`crate::runner::new_session::BranchQuery`]).
pub const RUNNER_BRANCHES: &str = "/api/runner/branches";

/// One route of session `id`: `transcript`, `live`, `send`, `stop`, `interrupt`, `settings` or
/// `task` (see [`crate::runner`]).
#[must_use]
pub fn session(id: &crate::ids::SessionId, route: SessionRoute) -> String {
    format!("{RUNNER_SESSIONS}/{id}/{}", route.as_str())
}

/// A route of one session ([`session`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionRoute {
    /// `GET`: its transcript.
    Transcript,
    /// `GET`: its live stream.
    Live,
    /// `POST`: the person's message.
    Send,
    /// `POST`: stop it, what is queued too.
    Stop,
    /// `POST`: stop the running turn only.
    Interrupt,
    /// `POST`: its model and effort.
    Settings,
    /// `POST`: move it to another task ([`crate::runner::actions::MoveSession`]).
    Task,
}

impl SessionRoute {
    /// The route's last part.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Transcript => "transcript",
            Self::Live => "live",
            Self::Send => "send",
            Self::Stop => "stop",
            Self::Interrupt => "interrupt",
            Self::Settings => "settings",
            Self::Task => "task",
        }
    }
}

/// `GET`: every place the assistant works and connection it reads through, live
/// ([`crate::environments::EnvPage`]).
pub const ENVIRONMENTS: &str = "/api/environments";
/// `POST`: one of those rows' actions: look again, a re-pair or sign-in link, start, unlink
/// ([`crate::environments::EnvActionRequest`]).
pub const ENVIRONMENTS_CHECK: &str = "/api/environments/check";
/// `POST`: check every Claude account's use now ([`crate::usage::UsageCheckReply`]).
pub const USAGE_CHECK: &str = "/api/usage/check";
/// `POST`: pin the assistant to an account, or set it to Auto
/// ([`crate::usage::UseAccountRequest`]).
pub const ACCOUNTS_USE: &str = "/api/accounts/use";
/// `GET ?fresh=1`: the background jobs on the machines ([`crate::jobs::JobsPage`]).
pub const JOBS: &str = "/api/jobs";

/// `POST`: stop a job, or clear a failed one ([`crate::jobs::JobActionRequest`]).
#[must_use]
pub fn job_action(action: crate::jobs::JobAction) -> String {
    format!("{JOBS}/{}", action.as_str())
}

/// `GET`: the parties, the ready-made personas and the models; `POST`: a new party
/// ([`crate::party::NewParty`]).
pub const PARTIES: &str = "/api/parties";

/// `GET`: party `id`'s live stream, as server-sent events ([`crate::party::PartyEvent`]).
#[must_use]
pub fn party_live(id: crate::ids::PartyId) -> String {
    format!("{PARTIES}/{id}/live")
}

/// `POST`: one of the person's controls on party `id` ([`crate::party::PartyAction`]).
#[must_use]
pub fn party_action(id: crate::ids::PartyId, action: &crate::party::PartyAction) -> String {
    format!("{PARTIES}/{id}/{}", action.route())
}

/// `GET`: the Dark room; `POST`: a message, answered as server-sent events; `DELETE`: clear it.
pub const DARK: &str = "/api/dark";

/// `GET`: everything with a next run ([`crate::scheduled::ScheduledPage`]).
pub const SCHEDULED: &str = "/api/scheduled";

/// `GET ?from=&to=`: the calendar for a range of days ([`crate::calendar::CalendarQuery`],
/// [`crate::calendar::CalendarPage`]).
pub const CALENDAR: &str = "/api/calendar";

/// `GET`: every quick note ([`crate::notes::NotesPage`]); `POST`: one saved
/// ([`crate::notes::NoteSave`]).
pub const NOTES: &str = "/api/notes";

/// The release feeds and their payloads: `GET /v{format}/{platform}` a feed
/// ([`release_feed`]), `GET /files/{name}` a payload, sent as it is (never compressed).
pub const RELEASES: &str = "/api/releases";

/// `GET`: the release feed of `platform` in `format` (`404` when nothing is published).
#[must_use]
pub fn release_feed(format: u32, platform: &crate::releases::Platform) -> String {
    format!("{RELEASES}/v{format}/{platform}")
}
/// `GET`: one task with its history, subtasks, findings, links and runs. `{id}` is its number.
pub const TASK: &str = "/api/tasks/{id}";
/// `POST`: the person's reply on a task's page ([`crate::task_message::TaskMessageRequest`]).
pub const TASK_MESSAGE: &str = "/api/tasks/{id}/message";
/// `GET`: a file a task produced, in byte ranges.
pub const ARTIFACT: &str = "/api/artifacts/{id}";
/// `GET`: a small JPEG of a picture or a video a task produced.
pub const ARTIFACT_THUMB: &str = "/api/artifacts/{id}/thumb";
/// `POST`: a program tells the assistant something ([`crate::wake::EventRequest`]).
pub const EVENTS: &str = "/api/events";

/// `POST ?call=&track=&last=`: one track of a recorded call, its audio as the body
/// ([`crate::calls::TrackQuery`]).
pub const CALLS_IN: &str = "/api/calls/in";
/// `POST ?call=&track=&t=`: a few seconds of a call being recorded, for its live transcript
/// ([`crate::calls::ChunkQuery`]).
pub const CALLS_LIVE: &str = "/api/calls/live";
/// `GET`: the latest calls.
pub const CALLS: &str = "/api/calls";
/// `GET`: a call's transcript, as Markdown. `{id}` is the call's id.
pub const CALL_TRANSCRIPT: &str = "/api/calls/{id}/transcript";
/// `GET`: a call's live lines and the copilot's hints. `{id}` is the call's id.
pub const CALL_LIVE: &str = "/api/calls/{id}/live";
/// `POST`: a key for one voice call with the assistant.
pub const VOICE_SESSION: &str = "/api/voice/session";
/// `POST`: the voice call asks the assistant ([`crate::voice::VoiceAsk`]).
pub const VOICE_ASK: &str = "/api/voice/ask";
/// `POST`: the voice call ended ([`crate::voice::VoiceDone`]).
pub const VOICE_DONE: &str = "/api/voice/done";
/// `POST`: the phone's daily health summaries ([`crate::health_data::HealthUpload`]). Not to be
/// confused with [`HEALTH`], the server's own health.
pub const HEALTH_DAYS: &str = "/api/health";
/// `POST`: the Mac helper's screen-time totals ([`crate::health_data::ScreenTimeUpload`]).
pub const SCREENTIME: &str = "/api/screentime";
/// `GET`, upgraded: the browser extension's WebSocket.
pub const CHROME_WS: &str = "/api/chrome/ws";
/// `GET`: the page a Chrome profile opens to pair (its code after `#`).
pub const CHROME_PAIR: &str = "/chrome/pair";
/// `GET ?code=`: how a pairing link stands ([`crate::browser::ChromePairStatus`]).
pub const CHROME_PAIR_STATUS: &str = "/chrome/pair/status";
/// `GET`: the browser extension as a zip, to load unpacked (the pairing page links to it).
pub const CHROME_EXTENSION_ZIP: &str = "/chrome/emberhut-extension.zip";
/// `GET`: every agent ([`crate::agents::AgentsPage`]); `POST`: a new one
/// ([`crate::agents::NewAgent`]).
pub const AGENTS: &str = "/api/agents";
/// `GET`: an agent with its lists ([`crate::agents::AgentPage`]); `PATCH`: change it
/// ([`crate::agents::AgentChange`]); `DELETE`: delete it. `{id}` is its id.
pub const AGENT: &str = "/api/agents/{id}";
/// `PUT`: an agent's whole list of channels ([`crate::agents::ChannelsRequest`]).
pub const AGENT_CHANNELS: &str = "/api/agents/{id}/channels";
/// `PUT`: an agent's whole list of tools ([`crate::agents::ToolsRequest`]).
pub const AGENT_TOOLS: &str = "/api/agents/{id}/tools";
/// `PUT`: an agent's whole list of accounts ([`crate::agents::AccountsRequest`]).
pub const AGENT_ACCOUNTS: &str = "/api/agents/{id}/accounts";
/// `PUT`: an agent's whole list of policies ([`crate::agents::PoliciesRequest`]).
pub const AGENT_POLICIES: &str = "/api/agents/{id}/policies";
/// `POST`: a message from one of an agent's channels, from its bridge, with the agent's
/// inbound bearer token ([`crate::agents::InboundMessage`]).
pub const AGENT_INBOUND: &str = "/api/agents/{id}/inbound";
/// `POST`: a note from an agent's bridge about the bridge itself, with the agent's inbound
/// bearer token ([`crate::agents::InboundNote`]).
pub const AGENT_NOTES: &str = "/api/agents/{id}/notes";
/// `POST`: a click on one of an agent's approval buttons in Slack, as its bridge's Socket Mode
/// connection received it (Slack's `block_actions` payload), with the agent's inbound bearer
/// token.
pub const AGENT_SLACK_ACTIONS: &str = "/api/agents/{id}/slack-actions";
/// `GET`: every external approver ([`crate::agents::ApproversPage`]); `POST`: a new one
/// ([`crate::agents::Approver`]).
pub const APPROVERS: &str = "/api/approvers";
/// `PATCH`: change an approver ([`crate::agents::ApproverChange`]); `DELETE`: delete it.
pub const APPROVER: &str = "/api/approvers/{id}";
/// `GET`: the trusted requesters; `PUT`: the whole list
/// ([`crate::agents::TrustedRequestersPage`]).
pub const TRUSTED_REQUESTERS: &str = "/api/trusted-requesters";
/// `GET ?state=&agent=`: approval requests ([`crate::approvals::ApprovalsQuery`],
/// [`crate::approvals::ApprovalsPage`]).
pub const APPROVALS: &str = "/api/approvals";
/// `GET`: one request ([`crate::approvals::ApprovalReply`]). `{id}` is its number.
pub const APPROVAL: &str = "/api/approvals/{id}";
/// `POST`: the owner's decision ([`crate::approvals::DecisionRequest`]).
pub const APPROVAL_DECISION: &str = "/api/approvals/{id}/decision";
/// `POST`: withdraw a request ([`crate::approvals::WithdrawRequest`]).
pub const APPROVAL_WITHDRAW: &str = "/api/approvals/{id}/withdraw";
/// `POST`: use an approval before a step, with the agent's room token
/// ([`crate::approvals::UseRequest`], [`crate::approvals::UseReply`]).
pub const APPROVAL_USE: &str = "/api/approvals/{id}/use";
/// `POST`: an approver's own app answers, signed with its secret
/// ([`crate::approvals::WebhookDecision`]).
pub const APPROVAL_WEBHOOK: &str = "/api/approvals/{id}/webhook";
/// `POST`: Slack's interactivity call for an approval DM's buttons, signed by Slack.
pub const APPROVALS_SLACK: &str = "/api/approvals/slack";
/// `GET`: a signed link's one-time page; `POST`: its decision. `{token}` is the link's token.
pub const APPROVE_PAGE: &str = "/approve/{token}";

/// [`AGENT`] for agent `id`.
#[must_use]
pub fn agent(id: &crate::ids::ConfiguredAgentId) -> String {
    format!("{AGENTS}/{id}")
}

/// One of an agent's lists: `/api/agents/{id}/{list}` (`channels`, `tools`, `accounts`,
/// `policies`).
#[must_use]
pub fn agent_list(id: &crate::ids::ConfiguredAgentId, list: crate::agents::AgentList) -> String {
    format!("{AGENTS}/{id}/{}", list.as_str())
}

/// [`APPROVER`] for approver `id`.
#[must_use]
pub fn approver(id: &crate::ids::ApproverId) -> String {
    format!("{APPROVERS}/{id}")
}

/// [`APPROVAL`] for request `id`.
#[must_use]
pub fn approval(id: crate::ids::ApprovalId) -> String {
    format!("{APPROVALS}/{id}")
}

/// [`APPROVAL_DECISION`] for request `id`.
#[must_use]
pub fn approval_decision(id: crate::ids::ApprovalId) -> String {
    format!("{APPROVALS}/{id}/decision")
}

/// [`APPROVAL_WITHDRAW`] for request `id`.
#[must_use]
pub fn approval_withdraw(id: crate::ids::ApprovalId) -> String {
    format!("{APPROVALS}/{id}/withdraw")
}

/// [`APPROVAL_USE`] for request `id`.
#[must_use]
pub fn approval_use(id: crate::ids::ApprovalId) -> String {
    format!("{APPROVALS}/{id}/use")
}
