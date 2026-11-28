//! The one seam between the app and the server. Everything else reaches the server through
//! [`Backend`], whose replies and errors are the app's own ([`StateReply`], [`Error`]): they say
//! what the store tells apart, not how a client library models HTTP. The data itself is the
//! contract's (`emberhut_api`), used as it is.
//!
//! [`EmberhutBackend`](client::EmberhutBackend) adapts the shared `emberhut-client`; the tests use
//! `fake::FakeBackend`. With a fallback server, [`failover::Failover`] stands over two of them
//! and turns to the fallback when the server stops answering.

mod client;
pub(crate) mod failover;
#[cfg(any(target_os = "macos", test))]
mod keychain;

#[cfg(test)]
pub(crate) mod fake;
#[cfg(test)]
pub(crate) mod test_server;

use std::fmt;
use std::io::Write;
use std::ops::ControlFlow;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use emberhut_api::agent_provider::{AgentProvider, ProviderRequest, ProviderTest};
use emberhut_api::approvals::{
    ApprovalReply, ApprovalsEvent, ApprovalsPage, ApprovalsQuery, DecisionRequest, WithdrawRequest,
};
use emberhut_api::auth::PairingCode;
use emberhut_api::browser::{BrowserInfo, BrowserInput, BrowserSession};
use emberhut_api::calendar::{CalendarPage, CalendarQuery};
use emberhut_api::calls::RecordCallRequest;
use emberhut_api::chat::{ChatFile, ChatPage};
use emberhut_api::chrome::{ChromeAction, ChromeReply};
use emberhut_api::connect::{LinkStatementRequest, SignedLink};
use emberhut_api::dark::{DarkEvent, DarkRoom};
use emberhut_api::environments::{EnvActionRequest, EnvPage};
use emberhut_api::facts::FactsPage;
use emberhut_api::headers::ETag;
use emberhut_api::ids::SessionId;
use emberhut_api::ids::{ApprovalId, MachineJobId};
use emberhut_api::ids::{BrowserDeviceId, PartyId};
use emberhut_api::ids::{DeviceId, MessageId, RoomId, TaskId};
use emberhut_api::jobs::{JobAction, JobsPage};
use emberhut_api::live::LiveEvent;
use emberhut_api::machines::MachinesPage;
use emberhut_api::notes::{NoteSave, NoteSaved, NotesPage};
use emberhut_api::party::{NewParty, PartiesPage, Party, PartyAction, PartyEvent};
use emberhut_api::projects::ProjectSetRequest;
use emberhut_api::runner::actions::{SendToSession, SentToSession, SessionSettings};
use emberhut_api::runner::new_session::{
    BranchList, BranchQuery, FolderList, FolderQuery, OpenSession, OpenedSession,
};
use emberhut_api::runner::transcript::{SessionLive, StepsQuery, Transcript};
use emberhut_api::scheduled::ScheduledPage;
use emberhut_api::settings::SettingsAction;
use emberhut_api::setup::SetupPage;
use emberhut_api::state::State;
use emberhut_api::switches::{SwitchAction, SwitchPage};
use emberhut_api::talk::{TalkReply, TalkRequest};
use emberhut_api::task_actions::TaskAction;
use emberhut_api::task_message::{TaskMessageReply, TaskMessageRequest};
use emberhut_api::task_page::TaskPage;
use emberhut_api::task_search::TaskQuery;
use emberhut_api::tasks::Issue;
use emberhut_api::theme::{ThemeChange, ThemePage};
use emberhut_api::threads::{ActivityQuery, ThreadActivity};
use emberhut_api::usage::{UseAccountReply, UseAccountRequest};
use emberhut_identity::MachineId;

use crate::config::{AppFiles, Server, ServerUrl, Servers};

/// What the app asks of the server. Every method blocks: call them on a background thread.
pub(crate) trait Backend: Send + Sync {
    /// The server's address.
    fn server(&self) -> &ServerUrl;

    /// Whether this device has a session of its own.
    fn is_signed_in(&self) -> bool;

    /// Which server answers: the server, or the fallback the app turned to when the server
    /// stopped answering. A backend without a fallback is always on the server.
    fn serving(&self) -> Serving {
        Serving::Primary
    }

    /// Asks whether the server answers again and, only when it does, turns back to it from the
    /// fallback; whether the app is on the server after. It blocks while it asks.
    fn try_primary(&self) -> bool {
        true
    }

    /// The state for `room`, or [`StateReply::Unchanged`] if it still matches `etag`;
    /// `looking` says the room is in front of the user (so the server holds its pushes).
    ///
    /// # Errors
    ///
    /// The server couldn't be reached, refused, or sent something unreadable.
    fn state(&self, room: RoomId, etag: Option<&ETag>, looking: bool) -> Result<StateReply, Error>;

    /// Marks `room`'s messages seen.
    ///
    /// # Errors
    ///
    /// As [`Backend::state`].
    fn seen(&self, room: RoomId) -> Result<(), Error>;

    /// The page of `room`'s chat before `before`.
    ///
    /// # Errors
    ///
    /// As [`Backend::state`].
    fn history(&self, room: RoomId, before: MessageId) -> Result<ChatPage, Error>;

    /// Sends a message to the assistant (its room, text and files are the request's).
    ///
    /// # Errors
    ///
    /// As [`Backend::state`].
    fn talk(&self, request: &TalkRequest) -> Result<TalkReply, Error>;

    /// Stores a file for the chat under `name`, telling `progress` the bytes sent so far as
    /// they go out; the stored file is what [`Backend::talk`] sends. When `progress` answers
    /// [`ControlFlow::Break`], the upload stops with [`Error::Local`] (the caller knows it
    /// asked).
    ///
    /// # Errors
    ///
    /// As [`Backend::state`]; [`Error::Local`] when the file couldn't be read or is larger than
    /// the server takes, or the upload was stopped.
    fn upload(
        &self,
        name: &str,
        source: &UploadSource,
        progress: &mut dyn FnMut(u64) -> ControlFlow<()>,
    ) -> Result<ChatFile, Error>;

    /// Streams the server's file at `path` into `to`, telling `progress` the bytes received and
    /// the size when the server gives it; gives the size. When `progress` answers
    /// [`ControlFlow::Break`], the download stops with [`Error::Local`].
    ///
    /// # Errors
    ///
    /// As [`Backend::state`]; [`Error::Local`] when `to` fails, the file is too large, or the
    /// download was stopped.
    fn download(
        &self,
        path: &str,
        to: &mut dyn Write,
        progress: &mut dyn FnMut(u64, Option<u64>) -> ControlFlow<()>,
    ) -> Result<u64, Error>;

    /// Sends the user's answer to a task's ask.
    ///
    /// # Errors
    ///
    /// As [`Backend::state`].
    fn decide(&self, task: TaskId, choice: &str) -> Result<(), Error>;

    /// The theme.
    ///
    /// # Errors
    ///
    /// As [`Backend::state`].
    fn theme(&self) -> Result<ThemePage, Error>;

    /// Changes the theme (or, with `preview`, says what it would be).
    ///
    /// # Errors
    ///
    /// As [`Backend::state`].
    fn change_theme(&self, change: &ThemeChange) -> Result<ThemePage, Error>;

    /// The machines page.
    ///
    /// # Errors
    ///
    /// As [`Backend::state`].
    fn machines(&self) -> Result<MachinesPage, Error>;

    /// A file on the server, by its path.
    ///
    /// # Errors
    ///
    /// As [`Backend::state`].
    fn file(&self, path: &str) -> Result<Vec<u8>, Error>;

    /// Trades a one-time pairing code for this device's own session (the device's name was
    /// given when the backend was made).
    ///
    /// # Errors
    ///
    /// The server couldn't be reached or refused the code ([`Error::Refused`], with its words).
    fn pair(&self, code: &str) -> Result<(), Error>;

    /// Trades the code of a pairing link for this device's own session, as [`Backend::pair`]
    /// does, keeping it only when the answer proves it comes from `machine`, the link's id.
    ///
    /// # Errors
    ///
    /// As [`Backend::pair`]; [`Error::WrongMachine`] when another machine answered, or one that
    /// didn't say which it is (nothing is kept then).
    fn pair_with_machine(&self, code: &str, machine: &MachineId) -> Result<(), Error>;

    /// The machine's signed link to the account and relay of `request`
    /// (`POST /api/connect/link-statement`), which the server answers on loopback only.
    ///
    /// # Errors
    ///
    /// As [`Backend::state`]; [`Error::Refused`] 403 when the connection isn't from loopback,
    /// 404 from a server without the route.
    fn link_statement(&self, request: &LinkStatementRequest) -> Result<SignedLink, Error>;

    /// Follows `room`'s live stream until the server closes it or `stop` says so (asked as
    /// each event or ping arrives): its lines, and each approval request's change.
    ///
    /// # Errors
    ///
    /// The stream couldn't be opened, or broke off.
    fn follow_live(
        &self,
        room: RoomId,
        stop: &dyn Fn() -> bool,
        on: &mut dyn FnMut(RoomEvent),
    ) -> Result<(), Error>;

    /// Makes one of the user's changes (`POST /api/settings`): pause or resume the assistant,
    /// correct a fact, drop a rule... Every action of that route goes through here; the next
    /// poll brings the state it changed.
    ///
    /// # Errors
    ///
    /// As [`Backend::state`]; [`Error::Refused`] with the server's words when the change can't
    /// be made.
    fn settings(&self, action: SettingsAction) -> Result<(), Error>;

    /// The MCP servers and skills with their switches.
    ///
    /// # Errors
    ///
    /// As [`Backend::state`]; [`Error::Refused`] 404 from a server without them.
    fn switches(&self) -> Result<SwitchPage, Error>;

    /// Turns a server or skill on or off, or reconnects a server; the page as it is after.
    ///
    /// # Errors
    ///
    /// As [`Backend::switches`]; [`Error::Refused`] with the server's words.
    fn switch(&self, action: &SwitchAction) -> Result<SwitchPage, Error>;

    /// What the assistant knows about the user, and the rules it is trying out.
    ///
    /// # Errors
    ///
    /// As [`Backend::state`].
    fn facts(&self) -> Result<FactsPage, Error>;

    /// Pairs, renames, unpairs or pauses a Chrome profile, or cancels a link; the profiles as
    /// they are after, with the new link when one was asked for.
    ///
    /// # Errors
    ///
    /// As [`Backend::state`]; [`Error::Refused`] with the server's words.
    fn chrome(&self, action: &ChromeAction) -> Result<ChromeReply, Error>;

    /// What the server's agent thinks with: the provider saved, without its key.
    ///
    /// # Errors
    ///
    /// As [`Backend::state`]; [`Error::Refused`] 404 from a server without the route.
    fn agent_provider(&self) -> Result<AgentProvider, Error>;

    /// Checks a provider with what `request` gives it and lists its models, saving nothing;
    /// the answer has `within` to come.
    ///
    /// # Errors
    ///
    /// As [`Backend::agent_provider`]; [`Error::Refused`] with the server's sentence when the
    /// provider doesn't take what was sent (400), refused the key (422) or didn't answer (502);
    /// [`Error::Unreachable`] once `within` is out.
    fn test_agent_provider(
        &self,
        request: &ProviderRequest,
        within: Duration,
    ) -> Result<ProviderTest, Error>;

    /// Checks a provider and saves it in place of whatever was saved; what is saved now.
    ///
    /// # Errors
    ///
    /// As [`Backend::test_agent_provider`] (nothing is saved then).
    fn save_agent_provider(&self, request: &ProviderRequest) -> Result<AgentProvider, Error>;

    /// Removes the saved provider and its key; what is saved now (nothing).
    ///
    /// # Errors
    ///
    /// As [`Backend::agent_provider`].
    fn forget_agent_provider(&self) -> Result<AgentProvider, Error>;

    /// What the server has set up, item by item (`GET /api/setup`): whether the ember is lit,
    /// and which of its connections are.
    ///
    /// # Errors
    ///
    /// As [`Backend::state`]; [`Error::Refused`] 404 from a server without the route.
    fn setup(&self) -> Result<SetupPage, Error>;

    /// Whether the server answers its health check within `within` (it blocks that long at
    /// most): whether an ember lives at its address at all.
    fn answers_within(&self, within: Duration) -> bool;

    /// Stops the answer the assistant is writing in `room` now (its background work goes on).
    ///
    /// # Errors
    ///
    /// As [`Backend::state`].
    fn interrupt(&self, room: RoomId) -> Result<(), Error>;

    /// Starts or stops recording a call on the Mac the server works with.
    ///
    /// # Errors
    ///
    /// As [`Backend::state`]; [`Error::Refused`] with the server's words (and no status) when
    /// the server couldn't tell the Mac.
    fn record_call(&self, request: &RecordCallRequest) -> Result<(), Error>;

    /// What was said in `audio`, a recording called `name` (its extension says the format),
    /// the server told to expect `words`.
    ///
    /// # Errors
    ///
    /// As [`Backend::state`]; [`Error::Local`] for a recording larger than the server takes.
    fn transcribe(&self, name: &str, words: &[String], audio: &[u8]) -> Result<String, Error>;

    /// What came to `room` after `after` (none: its newest page), so a room switched to shows
    /// its new messages before its (much larger) state arrives.
    ///
    /// # Errors
    ///
    /// As [`Backend::state`].
    fn catch_up(&self, room: RoomId, after: Option<MessageId>) -> Result<ChatPage, Error>;

    /// Sets a project's status: paused until a day, archived, or active again.
    ///
    /// # Errors
    ///
    /// As [`Backend::state`]; [`Error::Refused`] with the server's reason.
    fn set_project(&self, change: &ProjectSetRequest) -> Result<(), Error>;

    /// This device's row among the signed-in devices, once it has signed in.
    fn device(&self) -> Option<DeviceId>;

    /// Signs `device` out on the server.
    ///
    /// # Errors
    ///
    /// As [`Backend::state`]; [`Error::Refused`] with the server's reason.
    fn revoke_device(&self, device: DeviceId) -> Result<(), Error>;

    /// Forgets this device's session here (it was signed out on the server).
    ///
    /// # Errors
    ///
    /// [`Error::Local`]: the session store couldn't be written. The device is signed out here
    /// all the same.
    fn sign_out_here(&self) -> Result<(), Error>;

    /// A one-time pairing code for another device (this one must be signed in).
    ///
    /// # Errors
    ///
    /// As [`Backend::state`]; [`Error::Refused`] when this device can't make one.
    fn new_pairing_code(&self) -> Result<PairingCode, Error>;
    /// One task's page, done and archived ones too.
    ///
    /// # Errors
    ///
    /// As [`Backend::state`]; [`Error::Refused`] with status 404 when the server has no such
    /// task.
    fn task(&self, id: TaskId) -> Result<TaskPage, Error>;

    /// Tasks from the whole list, as `query` narrows them.
    ///
    /// # Errors
    ///
    /// As [`Backend::state`].
    fn find_tasks(&self, query: &TaskQuery) -> Result<Vec<Issue>, Error>;

    /// The user writes on task `id`'s page; where it went.
    ///
    /// # Errors
    ///
    /// As [`Backend::state`]; [`Error::Refused`] with the server's reason.
    fn message_task(
        &self,
        id: TaskId,
        message: &TaskMessageRequest,
    ) -> Result<TaskMessageReply, Error>;

    /// An action on tasks: done, put off, archived, back, a worker stopped, added, changed.
    ///
    /// # Errors
    ///
    /// As [`Backend::state`]; [`Error::Refused`] with the server's reason.
    fn act_on_tasks(&self, action: &TaskAction) -> Result<(), Error>;

    /// Renames task `id` to `title`: the task as it is now.
    ///
    /// # Errors
    ///
    /// As [`Backend::state`]; [`Error::Refused`] with the server's reason: 400 for an empty or
    /// too long title, 404 for a task it doesn't have.
    fn rename_task(&self, id: TaskId, title: &str) -> Result<Issue, Error>;

    /// A runner session's transcript: its head and newest steps, or those after one.
    ///
    /// # Errors
    ///
    /// As [`Backend::state`]; [`Error::Refused`] 502 when its runner didn't answer.
    fn session_transcript(&self, id: &SessionId, query: &StepsQuery) -> Result<Transcript, Error>;

    /// Follows a runner session's live stream from the step after `query.after`, until the
    /// server closes it or `stop` says so (asked as each event or ping arrives).
    ///
    /// # Errors
    ///
    /// The stream couldn't be opened, or broke off.
    fn follow_session(
        &self,
        id: &SessionId,
        query: &StepsQuery,
        stop: &dyn Fn() -> bool,
        on: &mut dyn FnMut(SessionLive),
    ) -> Result<(), Error>;

    /// The user's message to a session (queued behind a running turn).
    ///
    /// # Errors
    ///
    /// As [`Backend::state`]; [`Error::Refused`] with the server's reason.
    fn send_to_session(
        &self,
        id: &SessionId,
        message: &SendToSession,
    ) -> Result<SentToSession, Error>;

    /// Stops a session: the step it's on ends and nothing queued runs.
    ///
    /// # Errors
    ///
    /// As [`Backend::send_to_session`].
    fn stop_session(&self, id: &SessionId) -> Result<(), Error>;

    /// Stops a session's running turn; what is queued runs next.
    ///
    /// # Errors
    ///
    /// As [`Backend::send_to_session`].
    fn interrupt_session(&self, id: &SessionId) -> Result<(), Error>;

    /// Changes a session's model or effort for its next turn.
    ///
    /// # Errors
    ///
    /// As [`Backend::send_to_session`].
    fn change_session(&self, id: &SessionId, settings: &SessionSettings) -> Result<(), Error>;

    /// Moves a session to task `task` (on a server that keeps plans): its turns count there
    /// from now on. The task it is on now, as the server says.
    ///
    /// # Errors
    ///
    /// As [`Backend::send_to_session`]: 404 for a session or task the server doesn't have, 400
    /// for a task that is done or archived, each with the server's reason.
    fn move_session(&self, id: &SessionId, task: TaskId) -> Result<TaskId, Error>;

    /// Opens a session of the user's own with their first message.
    ///
    /// # Errors
    ///
    /// As [`Backend::send_to_session`].
    fn open_session(&self, session: &OpenSession) -> Result<OpenedSession, Error>;

    /// A runner machine's folders: where to start, or those inside one.
    ///
    /// # Errors
    ///
    /// As [`Backend::send_to_session`].
    fn runner_folders(&self, query: &FolderQuery) -> Result<FolderList, Error>;

    /// A repository's branches on a runner machine.
    ///
    /// # Errors
    ///
    /// As [`Backend::send_to_session`].
    fn runner_branches(&self, query: &BranchQuery) -> Result<BranchList, Error>;

    /// An external thread's latest steps.
    ///
    /// # Errors
    ///
    /// As [`Backend::send_to_session`].
    fn t3_activity(&self, query: &ActivityQuery) -> Result<ThreadActivity, Error>;
    /// Every place the assistant works and connection it reads through, with their states.
    ///
    /// # Errors
    ///
    /// As [`Backend::state`]; [`Error::Refused`] 404 from a server without the page.
    fn environments(&self) -> Result<EnvPage, Error>;

    /// Runs one of a row's actions (look again, a link, start, unlink); the page as it is
    /// after, with what the action made.
    ///
    /// # Errors
    ///
    /// As [`Backend::environments`]; [`Error::Refused`] with the server's words.
    fn environment_action(&self, request: &EnvActionRequest) -> Result<EnvPage, Error>;

    /// Asks the server to check every Claude account's use now (the poll brings the numbers).
    ///
    /// # Errors
    ///
    /// As [`Backend::state`].
    fn check_usage(&self) -> Result<(), Error>;

    /// Pins the assistant to an account, or sets it to Auto; the account as it is after.
    ///
    /// # Errors
    ///
    /// As [`Backend::state`]; [`Error::Refused`] with the server's words.
    fn use_account(&self, request: &UseAccountRequest) -> Result<UseAccountReply, Error>;

    /// The background jobs on the machines; `fresh` looks again on the server.
    ///
    /// # Errors
    ///
    /// As [`Backend::state`]; [`Error::Refused`] 404 from a server without the list.
    fn jobs(&self, fresh: bool) -> Result<JobsPage, Error>;

    /// Stops a job, or clears a failed one.
    ///
    /// # Errors
    ///
    /// As [`Backend::jobs`]; [`Error::Refused`] with the server's words.
    fn act_on_job(&self, action: JobAction, job: &MachineJobId) -> Result<(), Error>;

    /// The parties, the ready-made personas, the user's own from earlier parties and the
    /// models.
    ///
    /// # Errors
    ///
    /// As [`Backend::state`]; [`Error::Refused`] 404 from a server without Party.
    fn parties(&self) -> Result<PartiesPage, Error>;

    /// Starts a new party; the party as the server made it.
    ///
    /// # Errors
    ///
    /// As [`Backend::parties`]; [`Error::Refused`] with the server's reason.
    fn start_party(&self, party: &NewParty) -> Result<Party, Error>;

    /// One of the user's controls on party `id`; the party as it is after (none after a
    /// delete).
    ///
    /// # Errors
    ///
    /// As [`Backend::parties`]; [`Error::Refused`] with the server's reason.
    fn act_on_party(&self, id: PartyId, action: &PartyAction) -> Result<Option<Party>, Error>;

    /// Follows party `id`'s live stream until the server closes it or `stop` says so (asked
    /// as each event or ping arrives).
    ///
    /// # Errors
    ///
    /// The stream couldn't be opened, or broke off.
    fn follow_party(
        &self,
        id: PartyId,
        stop: &dyn Fn() -> bool,
        on: &mut dyn FnMut(PartyEvent),
    ) -> Result<(), Error>;

    /// The Dark room.
    ///
    /// # Errors
    ///
    /// As [`Backend::state`].
    fn dark(&self) -> Result<DarkRoom, Error>;

    /// Writes `text` in the Dark room: the answer's events to `on` as it is written.
    ///
    /// # Errors
    ///
    /// As [`Backend::state`]; [`Error::Refused`] with the server's words (a blank message, an
    /// answer still being written); [`Error::Unreachable`] when the stream broke off.
    fn send_dark(&self, text: &str, on: &mut dyn FnMut(DarkEvent)) -> Result<(), Error>;

    /// Deletes everything said in the Dark room.
    ///
    /// # Errors
    ///
    /// As [`Backend::state`]; [`Error::Refused`] while an answer is being written.
    fn clear_dark(&self) -> Result<(), Error>;

    /// The live view of the assistant's browser for `device`: its address and the sign-in
    /// for what the device may do (control while it drives, view only otherwise).
    ///
    /// # Errors
    ///
    /// As [`Backend::state`]; [`Error::Refused`] 503 with the server's reason when its machine
    /// has no live view.
    fn browser_session(&self, device: &BrowserDeviceId) -> Result<BrowserSession, Error>;

    /// Takes the assistant's browser over, gives it back, or drives its toolbar; the browser
    /// as it is after.
    ///
    /// # Errors
    ///
    /// As [`Backend::state`]; [`Error::Refused`] 409 with the server's reason.
    fn browser_input(&self, input: &BrowserInput) -> Result<BrowserInfo, Error>;

    /// Puts `bytes`, a file called `name`, into the field the browser's page asks a file for;
    /// the browser as it is after.
    ///
    /// # Errors
    ///
    /// As [`Backend::state`]; [`Error::Refused`] 409 when the page isn't asking for one.
    fn browser_upload(&self, name: &str, bytes: &[u8]) -> Result<BrowserInfo, Error>;

    /// Everything the assistant has with a next run, soonest first.
    ///
    /// # Errors
    ///
    /// As [`Backend::state`]; [`Error::Refused`] 404 from a server without the list.
    fn scheduled(&self) -> Result<ScheduledPage, Error>;

    /// The calendar for the days `query` names: the events and what the user has to work on.
    ///
    /// # Errors
    ///
    /// As [`Backend::state`]; [`Error::Refused`] with the server's reason for a bad range.
    fn calendar(&self, query: &CalendarQuery) -> Result<CalendarPage, Error>;

    /// Every quick note, newest first.
    ///
    /// # Errors
    ///
    /// As [`Backend::state`].
    fn notes(&self) -> Result<NotesPage, Error>;

    /// Saves a quick note: a new one, an edit, or (empty text) a deletion.
    ///
    /// # Errors
    ///
    /// As [`Backend::state`]; [`Error::Refused`] 400 when the server won't ever take it.
    fn save_note(&self, note: &NoteSave) -> Result<NoteSaved, Error>;

    /// The release feed of `platform`, asking for `format` first and each lower one after it;
    /// `None` when the server has published none.
    ///
    /// # Errors
    ///
    /// As [`Backend::state`].
    fn release_feed(
        &self,
        platform: &emberhut_api::releases::Platform,
        format: u32,
    ) -> Result<Option<emberhut_api::releases::ReleaseFeed>, Error>;

    /// Streams the release payload at the server path `path` into `to`, never more than
    /// `max_len` bytes; its size and SHA-256.
    ///
    /// # Errors
    ///
    /// As [`Backend::state`]; [`Error::Local`] with [`TooLarge`] past `max_len`, or when `to`
    /// fails.
    fn download_release(
        &self,
        path: &str,
        to: &mut dyn Write,
        max_len: u64,
    ) -> Result<PayloadDigest, Error>;

    /// The approval requests `query` names, newest first.
    ///
    /// # Errors
    ///
    /// As [`Backend::state`]; [`Error::Refused`] 404 from a server without agents.
    fn approvals(&self, query: &ApprovalsQuery) -> Result<ApprovalsPage, Error>;

    /// Approval request `id` as it is now, with its whole payload.
    ///
    /// # Errors
    ///
    /// As [`Backend::approvals`]; [`Error::Refused`] 404 for a request the server doesn't
    /// have.
    fn approval(&self, id: ApprovalId) -> Result<ApprovalReply, Error>;

    /// The user's answer on approval request `id`, on the payload hash they were shown; the
    /// request as it is after.
    ///
    /// # Errors
    ///
    /// As [`Backend::approval`]; [`Error::Refused`] 409 for a request no longer pending or
    /// whose payload hash isn't the one sent, with the server's words.
    fn decide_approval(
        &self,
        id: ApprovalId,
        decision: &DecisionRequest,
    ) -> Result<ApprovalReply, Error>;

    /// Withdraws approval request `id`; the request as it is after.
    ///
    /// # Errors
    ///
    /// As [`Backend::decide_approval`].
    fn withdraw_approval(
        &self,
        id: ApprovalId,
        withdrawal: &WithdrawRequest,
    ) -> Result<ApprovalReply, Error>;

    /// Stops for good, for another copy of the app to take the session over (an update's new
    /// copy): waits for a renewal in flight, saves the session, then sends and renews nothing
    /// more; every later call fails and the live streams end at their next event or ping. It
    /// may wait: call it off the UI thread.
    ///
    /// # Errors
    ///
    /// [`Error::Local`]: the session couldn't be saved (the backend is stopped all the same).
    fn stop(&self) -> Result<(), Error>;
}

/// Whether this launch keeps its secrets (the device sessions, the account's tokens) in the
/// login Keychain: not in a test run ([`crate::config::is_test_run`]), and only when this user
/// has a default keychain, so the Keychain never asks for one in a dialog.
#[cfg(target_os = "macos")]
#[must_use]
pub(crate) fn has_keychain() -> bool {
    !crate::config::is_test_run() && keychain::system::has_default_keychain()
}

/// The backend for the server at `url`: its session kept in `files` (in memory without them),
/// the device named `device_name` when it pairs. With a fallback server in `url`, a
/// [`failover::Failover`] over both, each with its own session, watching the server's health.
///
/// # Errors
///
/// A client couldn't be made: [`Error::Local`] (no randomness for a new install id).
pub(crate) fn connect(
    url: Servers,
    files: Option<&AppFiles>,
    device_name: &str,
) -> Result<Arc<dyn Backend>, Error> {
    let primary = Arc::new(client::EmberhutBackend::open(
        url.primary,
        files,
        device_name,
    )?);
    let Some(fallback) = url.fallback else {
        return Ok(primary);
    };
    // The fallback's session is its own: on macOS its server's Keychain item, else a file in
    // a folder of its own.
    let fallback_files = files.map(AppFiles::fallback);
    let fallback = Arc::new(client::EmberhutBackend::open(
        fallback,
        fallback_files.as_ref(),
        device_name,
    )?);
    let probe = Arc::clone(&primary);
    Ok(failover::Failover::start(
        primary,
        fallback,
        Box::new(move || probe.answers(failover::Timing::DEFAULT.probe_within)),
        files.map(AppFiles::fallback_mark),
        failover::Timing::DEFAULT,
    ))
}

/// Trades `code` for this device's session with `server`, kept in `files` as the app keeps it
/// (the Keychain on macOS), without the app's window: what the server says after.
///
/// # Errors
///
/// As [`Backend::pair`]; then as [`Backend::state`] when the server can't be asked whether it
/// takes the session.
pub(crate) fn pair_here(
    server: Server,
    files: &AppFiles,
    device_name: &str,
    code: &str,
) -> Result<PairedHere, Error> {
    let backend = client::EmberhutBackend::open(server, Some(files), device_name)?;
    backend.pair(code)?;
    Ok(PairedHere {
        device: backend.device(),
        signed_in: backend.signed_in_on_server()?,
    })
}

/// What [`pair_here`] found once the code was traded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PairedHere {
    /// This device's row on the server.
    pub(crate) device: Option<DeviceId>,
    /// Whether the server takes the session as a signed-in device's.
    pub(crate) signed_in: bool,
}

/// Removes this device's session with `server` from `files` (on macOS, its Keychain item),
/// install id and all, without the app's window.
///
/// # Errors
///
/// [`Error::Local`]: the session store couldn't be written.
pub(crate) fn forget_here(server: &Server, files: &AppFiles) -> Result<(), Error> {
    client::EmberhutBackend::forget(server, files)
}

/// Bytes an upload can read from memory, shared rather than copied.
pub(crate) type SharedBytes = Arc<dyn AsRef<[u8]> + Send + Sync>;

/// What an upload reads: a file on this device (streamed from disk), or bytes in memory (a
/// pasted picture, shared with what shows it).
#[derive(Clone)]
pub(crate) enum UploadSource {
    /// A file, by its path.
    File(PathBuf),
    /// Bytes.
    Bytes(SharedBytes),
}

impl fmt::Debug for UploadSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::File(path) => f.debug_tuple("File").field(path).finish(),
            Self::Bytes(bytes) => write!(f, "Bytes({} bytes)", (**bytes).as_ref().len()),
        }
    }
}

/// Which server the app talks to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Serving {
    /// The server.
    Primary,
    /// The fallback server, since the server stopped answering.
    Fallback {
        /// The server that stopped answering.
        primary: ServerUrl,
        /// The fallback server, which answers now.
        fallback: ServerUrl,
    },
}

/// One event of a room's live stream.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RoomEvent {
    /// The assistant's line in the room: what it is doing, or that its turn is over.
    Live(LiveEvent),
    /// An approval request changed (made, decided, used, withdrawn or expired).
    Approvals(ApprovalsEvent),
}

/// A state that changed since the last poll.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct FreshState {
    /// The state.
    pub(crate) state: Box<State>,
    /// Its fingerprint, to send with the next poll.
    pub(crate) etag: Option<ETag>,
}

/// The answer to a state poll.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum StateReply {
    /// Nothing changed since the fingerprint sent.
    Unchanged,
    /// A new state.
    Fresh(FreshState),
    /// The server wants this device signed in first.
    SignInRequired,
}

/// A file larger than a transfer takes, as the cause of an [`Error::Local`] (the app words it
/// on its own).
#[derive(Debug, thiserror::Error)]
#[error("the file is {size} bytes, over the limit of {limit}")]
pub(crate) struct TooLarge {
    /// Its size in bytes (or how far a download without a size got).
    pub(crate) size: u64,
    /// The limit.
    pub(crate) limit: u64,
}

/// A release payload as it reached the disk: its size and SHA-256.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PayloadDigest {
    /// The bytes written.
    pub(crate) size: u64,
    /// Their SHA-256.
    pub(crate) sha256: [u8; 32],
}

/// The cause behind an [`Error`], as the client reported it.
pub(crate) type Cause = Box<dyn std::error::Error + Send + Sync>;

/// What can go wrong talking to the server, as far as the app tells cases apart.
#[derive(Debug, thiserror::Error)]
pub(crate) enum Error {
    /// The server couldn't be reached, or its answer didn't arrive whole; trying again later
    /// may work.
    #[error("can't reach the server")]
    Unreachable(#[source] Cause),
    /// The server wants this device signed in first.
    #[error("this device has to sign in first")]
    SignInRequired,
    /// The server said no, with its HTTP status and its own words when it gave them.
    #[error("the server said no{}", refusal_tail(*.status, .message.as_deref()))]
    Refused {
        /// The HTTP status.
        status: Option<u16>,
        /// The server's reason, worded for the user.
        message: Option<String>,
    },
    /// The answer wasn't what this app expects.
    #[error("the server sent an answer this app can't read")]
    Unreadable(#[source] Cause),
    /// Something on this device failed (keeping the session, making a request).
    #[error("something went wrong on this device")]
    Local(#[source] Cause),
    /// A pairing from a link was answered by another machine than the link names, or by one
    /// that didn't say which it is.
    #[error("This server didn't prove it is the machine you meant.")]
    WrongMachine,
}

impl Error {
    /// Whether the server was never reached, so trying again later may work.
    #[must_use]
    pub(crate) fn is_unreachable(&self) -> bool {
        matches!(self, Self::Unreachable(_))
    }

    /// The server's own words for the user, if it gave any.
    #[must_use]
    pub(crate) fn server_message(&self) -> Option<&str> {
        match self {
            Self::Refused { message, .. } => message.as_deref(),
            Self::Unreachable(_)
            | Self::SignInRequired
            | Self::Unreadable(_)
            | Self::Local(_)
            | Self::WrongMachine => None,
        }
    }
}

/// The tail of [`Error::Refused`]'s message: ` (401): its words`.
fn refusal_tail(status: Option<u16>, message: Option<&str>) -> String {
    let status = status.map_or_default(|status| format!(" ({status})"));
    let message = message.map_or_default(|message| format!(": {message}"));
    status + &message
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_refusal_reads_with_its_status_and_words() {
        let refused = Error::Refused {
            status: Some(422),
            message: Some("Too light".into()),
        };
        assert_eq!(refused.to_string(), "the server said no (422): Too light");
        assert_eq!(refused.server_message(), Some("Too light"));
        let bare = Error::Refused {
            status: None,
            message: None,
        };
        assert_eq!(bare.to_string(), "the server said no");
        assert!(!bare.is_unreachable());
    }

    #[test]
    fn an_unreachable_server_has_no_words() {
        let error = Error::Unreachable("connection refused".into());
        assert!(error.is_unreachable());
        assert_eq!(error.server_message(), None);
    }
}
