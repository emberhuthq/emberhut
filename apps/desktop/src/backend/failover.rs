//! The fallback server: when the server stops answering, the app turns to another by itself.
//!
//! [`Failover`] is a [`Backend`] over two: the server (the primary) and the fallback server,
//! each with its own client and its own session. Every call goes to the one being served. While
//! that is the primary, a watcher asks the primary's `GET /health` every
//! [`Timing::every`]; after [`Timing::failures`] failed checks in a row, or at start-up a first
//! check and [`Timing::start_retries`] quick retries that all fail, every call goes to the
//! fallback from then on.
//!
//! It never turns back by itself: what the user does on the fallback stays there, and turning
//! back unasked would split their work between two servers without their knowing. The banner's
//! Try again ([`Backend::try_primary`]) checks the primary once and turns back only when it
//! answers. Being on the fallback is kept in a file naming the primary it stands in for, so a
//! relaunch stays on the fallback too rather than turning back unasked.

use std::io::Write;
use std::ops::ControlFlow;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, PoisonError, Weak};
use std::time::Duration;

use emberhut_api::agent_provider::{AgentProvider, ProviderRequest, ProviderTest};
use emberhut_api::approvals::{
    ApprovalReply, ApprovalsPage, ApprovalsQuery, DecisionRequest, WithdrawRequest,
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
use serde::{Deserialize, Serialize};

use super::{Backend, Error, PayloadDigest, RoomEvent, Serving, StateReply, UploadSource};
use crate::config::ServerUrl;

/// How often the primary is checked, and how many failures turn to the fallback.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Timing {
    /// Between two checks while the primary answers.
    pub(crate) every: Duration,
    /// Failed checks in a row, after the primary has answered once, that turn to the fallback.
    pub(crate) failures: u32,
    /// Quick retries at start-up, after a first check that failed, before turning to the
    /// fallback.
    pub(crate) start_retries: u32,
    /// Between two checks at start-up.
    pub(crate) retry_after: Duration,
    /// How long one check waits for the answer.
    pub(crate) probe_within: Duration,
}

impl Timing {
    /// The app's: every 15 s, three failures in a row; at start-up two retries 2 s apart; 5 s
    /// for an answer.
    pub(crate) const DEFAULT: Self = Self {
        every: Duration::from_secs(15),
        failures: 3,
        start_retries: 2,
        retry_after: Duration::from_secs(2),
        probe_within: Duration::from_secs(5),
    };
}

/// What the watcher does after a check.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Next {
    /// Checks again after this long.
    CheckIn(Duration),
    /// Turns to the fallback (and checks no more).
    TurnToFallback,
}

/// The watcher's count: whether the primary has answered since the watch began, and how many
/// checks have failed since it last answered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Watch {
    timing: Timing,
    answered: bool,
    failed: u32,
}

impl Watch {
    /// A watch at start-up: the primary hasn't answered yet.
    pub(crate) fn starting(timing: Timing) -> Self {
        Self {
            timing,
            answered: false,
            failed: 0,
        }
    }

    /// A watch of a primary that just answered (turned back to).
    pub(crate) fn answered(timing: Timing) -> Self {
        Self {
            timing,
            answered: true,
            failed: 0,
        }
    }

    /// What to do after a check the primary `answered` (or not).
    pub(crate) fn heard(&mut self, answered: bool) -> Next {
        let timing = self.timing;
        if answered {
            self.answered = true;
            self.failed = 0;
            return Next::CheckIn(timing.every);
        }
        self.failed = self.failed.saturating_add(1);
        if self.answered {
            if self.failed >= timing.failures {
                Next::TurnToFallback
            } else {
                Next::CheckIn(timing.every)
            }
        } else if self.failed > timing.start_retries {
            Next::TurnToFallback
        } else {
            Next::CheckIn(timing.retry_after)
        }
    }
}

/// Asks the primary whether it answers (blocking, within the timing's wait).
pub(crate) type Probe = Box<dyn Fn() -> bool + Send + Sync>;

/// `fallback.json`: the primary the app turned away from.
#[derive(Debug, Serialize, Deserialize)]
struct Mark {
    /// The primary's address.
    primary: String,
}

/// What the watcher and the backend share.
struct Shared {
    /// Whether every call goes to the fallback.
    on_fallback: AtomicBool,
    probe: Probe,
    /// Where being on the fallback is kept (nowhere without the app's files).
    mark: Option<PathBuf>,
    primary: ServerUrl,
    wake: Mutex<Wake>,
    woken: Condvar,
}

/// Why the watcher wakes early.
#[derive(Debug, Default)]
struct Wake {
    /// The backend stopped: the watcher ends.
    stopped: bool,
    /// The app turned back to the primary: the watcher watches it again.
    turned_back: bool,
}

impl Shared {
    fn on_fallback(&self) -> bool {
        self.on_fallback.load(Ordering::SeqCst)
    }

    fn wake(&self) -> std::sync::MutexGuard<'_, Wake> {
        self.wake.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Turns every call to the fallback, and keeps that for the next launch.
    fn turn_to_fallback(&self) {
        if self.on_fallback.swap(true, Ordering::SeqCst) {
            return;
        }
        log::warn!(
            "{} isn't answering: turning to the fallback server",
            self.primary
        );
        let Some(path) = &self.mark else { return };
        let mark = Mark {
            primary: self.primary.as_str().to_owned(),
        };
        let written = serde_json::to_vec(&mark)
            .map_err(std::io::Error::other)
            .and_then(|bytes| std::fs::write(path, bytes));
        if let Err(error) = written {
            log::warn!("can't keep {}: {error}", path.display());
        }
    }

    /// Turns every call back to the primary, forgets the mark, and wakes the watcher.
    fn turn_back(&self) {
        self.on_fallback.store(false, Ordering::SeqCst);
        if let Some(path) = &self.mark {
            match std::fs::remove_file(path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => log::warn!("can't remove {}: {error}", path.display()),
            }
        }
        self.wake().turned_back = true;
        self.woken.notify_all();
        log::info!("{} answers again: back on it", self.primary);
    }

    /// Whether the mark says the app was on the fallback for this primary.
    fn marked(&self) -> bool {
        let Some(path) = &self.mark else { return false };
        let Ok(bytes) = std::fs::read(path) else {
            return false;
        };
        serde_json::from_slice::<Mark>(&bytes)
            .is_ok_and(|mark| mark.primary == self.primary.as_str())
    }

    /// One check of the primary, and what `watch` makes of it: on [`Next::TurnToFallback`]
    /// the app is on the fallback when this returns.
    fn check(&self, watch: &mut Watch) -> Next {
        let next = watch.heard((self.probe)());
        if next == Next::TurnToFallback {
            self.turn_to_fallback();
        }
        next
    }

    /// Waits `wait`, then, while the app is on the fallback, until it turns back; whether the
    /// watcher goes on (it ends when the backend stops) and whether it turned back meanwhile.
    fn pause(&self, wait: Duration) -> Option<bool> {
        let wake = self.wake();
        let (wake, _) = self
            .woken
            .wait_timeout_while(wake, wait, |wake| !wake.stopped && !wake.turned_back)
            .unwrap_or_else(PoisonError::into_inner);
        let mut wake = self
            .woken
            .wait_while(wake, |wake| {
                !wake.stopped && !wake.turned_back && self.on_fallback()
            })
            .unwrap_or_else(PoisonError::into_inner);
        if wake.stopped {
            return None;
        }
        Some(std::mem::take(&mut wake.turned_back))
    }

    fn stop(&self) {
        self.wake().stopped = true;
        self.woken.notify_all();
    }
}

/// The watcher: checks the primary while the app is on it, until the backend stops or goes.
fn watch(shared: &Weak<Shared>, timing: Timing) {
    let mut watch = Watch::starting(timing);
    let mut wait = Duration::ZERO;
    loop {
        let Some(shared) = shared.upgrade() else {
            return;
        };
        match shared.pause(wait) {
            None => return,
            Some(true) => {
                watch = Watch::answered(timing);
                wait = timing.every;
                continue;
            }
            Some(false) => {}
        }
        wait = match shared.check(&mut watch) {
            Next::CheckIn(after) => after,
            Next::TurnToFallback => Duration::ZERO,
        };
    }
}

/// The server and the fallback server, as one backend that turns to the fallback when the
/// server stops answering.
pub(crate) struct Failover {
    primary: Arc<dyn Backend>,
    fallback: Arc<dyn Backend>,
    shared: Arc<Shared>,
}

impl Failover {
    /// Both, on the primary (or on the fallback, when `mark` says the app was on it for this
    /// primary), without a watcher: the tests check by hand.
    fn new(
        primary: Arc<dyn Backend>,
        fallback: Arc<dyn Backend>,
        probe: Probe,
        mark: Option<PathBuf>,
    ) -> Self {
        let shared = Arc::new(Shared {
            on_fallback: AtomicBool::new(false),
            probe,
            mark,
            primary: primary.server().clone(),
            wake: Mutex::new(Wake::default()),
            woken: Condvar::new(),
        });
        if shared.marked() {
            log::warn!(
                "on the fallback server since {} stopped answering; Try again turns back",
                shared.primary
            );
            shared.on_fallback.store(true, Ordering::SeqCst);
        }
        Self {
            primary,
            fallback,
            shared,
        }
    }

    /// Both, with the watcher started: `probe` asks whether the primary answers, `mark` is
    /// where being on the fallback is kept across launches.
    pub(crate) fn start(
        primary: Arc<dyn Backend>,
        fallback: Arc<dyn Backend>,
        probe: Probe,
        mark: Option<PathBuf>,
        timing: Timing,
    ) -> Arc<dyn Backend> {
        let failover = Self::new(primary, fallback, probe, mark);
        let shared = Arc::downgrade(&failover.shared);
        let started = std::thread::Builder::new()
            .name("server-health".into())
            .spawn(move || watch(&shared, timing));
        if let Err(error) = started {
            log::warn!("can't watch the server's health ({error}); no fallback this launch");
        }
        Arc::new(failover)
    }

    /// The backend every call goes to now.
    fn active(&self) -> &dyn Backend {
        self.at(self.shared.on_fallback())
    }

    /// The fallback's backend when `on_fallback`, else the primary's.
    fn at(&self, on_fallback: bool) -> &dyn Backend {
        if on_fallback {
            &*self.fallback
        } else {
            &*self.primary
        }
    }
}

impl Drop for Failover {
    fn drop(&mut self) {
        self.shared.stop();
    }
}

/// A live stream follows the server it was opened on until the app turns to the other one:
/// then it ends at its next event or ping, and the app opens it again on the other.
impl Backend for Failover {
    fn server(&self) -> &ServerUrl {
        self.active().server()
    }

    fn serving(&self) -> Serving {
        if self.shared.on_fallback() {
            Serving::Fallback {
                primary: self.primary.server().clone(),
                fallback: self.fallback.server().clone(),
            }
        } else {
            Serving::Primary
        }
    }

    fn try_primary(&self) -> bool {
        if !self.shared.on_fallback() {
            return true;
        }
        if !(self.shared.probe)() {
            return false;
        }
        self.shared.turn_back();
        true
    }

    fn stop(&self) -> Result<(), Error> {
        self.shared.stop();
        let fallback = self.fallback.stop();
        self.primary.stop().and(fallback)
    }

    fn is_signed_in(&self) -> bool {
        self.active().is_signed_in()
    }

    fn state(&self, room: RoomId, etag: Option<&ETag>, looking: bool) -> Result<StateReply, Error> {
        self.active().state(room, etag, looking)
    }

    fn seen(&self, room: RoomId) -> Result<(), Error> {
        self.active().seen(room)
    }

    fn history(&self, room: RoomId, before: MessageId) -> Result<ChatPage, Error> {
        self.active().history(room, before)
    }

    fn talk(&self, request: &TalkRequest) -> Result<TalkReply, Error> {
        self.active().talk(request)
    }

    fn upload(
        &self,
        name: &str,
        source: &UploadSource,
        progress: &mut dyn FnMut(u64) -> ControlFlow<()>,
    ) -> Result<ChatFile, Error> {
        self.active().upload(name, source, progress)
    }

    fn download(
        &self,
        path: &str,
        to: &mut dyn Write,
        progress: &mut dyn FnMut(u64, Option<u64>) -> ControlFlow<()>,
    ) -> Result<u64, Error> {
        self.active().download(path, to, progress)
    }

    fn decide(&self, task: TaskId, choice: &str) -> Result<(), Error> {
        self.active().decide(task, choice)
    }

    fn theme(&self) -> Result<ThemePage, Error> {
        self.active().theme()
    }

    fn change_theme(&self, change: &ThemeChange) -> Result<ThemePage, Error> {
        self.active().change_theme(change)
    }

    fn machines(&self) -> Result<MachinesPage, Error> {
        self.active().machines()
    }

    fn file(&self, path: &str) -> Result<Vec<u8>, Error> {
        self.active().file(path)
    }

    fn pair(&self, code: &str) -> Result<(), Error> {
        self.active().pair(code)
    }

    fn pair_with_machine(&self, code: &str, machine: &MachineId) -> Result<(), Error> {
        self.active().pair_with_machine(code, machine)
    }

    fn link_statement(&self, request: &LinkStatementRequest) -> Result<SignedLink, Error> {
        self.active().link_statement(request)
    }

    fn follow_live(
        &self,
        room: RoomId,
        stop: &dyn Fn() -> bool,
        on: &mut dyn FnMut(RoomEvent),
    ) -> Result<(), Error> {
        let (on_fallback, shared) = (self.shared.on_fallback(), &self.shared);
        let moved = || stop() || shared.on_fallback() != on_fallback;
        self.at(on_fallback).follow_live(room, &moved, on)
    }

    fn settings(&self, action: SettingsAction) -> Result<(), Error> {
        self.active().settings(action)
    }

    fn switches(&self) -> Result<SwitchPage, Error> {
        self.active().switches()
    }

    fn switch(&self, action: &SwitchAction) -> Result<SwitchPage, Error> {
        self.active().switch(action)
    }

    fn facts(&self) -> Result<FactsPage, Error> {
        self.active().facts()
    }

    fn chrome(&self, action: &ChromeAction) -> Result<ChromeReply, Error> {
        self.active().chrome(action)
    }

    fn agent_provider(&self) -> Result<AgentProvider, Error> {
        self.active().agent_provider()
    }

    fn test_agent_provider(
        &self,
        request: &ProviderRequest,
        within: Duration,
    ) -> Result<ProviderTest, Error> {
        self.active().test_agent_provider(request, within)
    }

    fn save_agent_provider(&self, request: &ProviderRequest) -> Result<AgentProvider, Error> {
        self.active().save_agent_provider(request)
    }

    fn forget_agent_provider(&self) -> Result<AgentProvider, Error> {
        self.active().forget_agent_provider()
    }

    fn setup(&self) -> Result<SetupPage, Error> {
        self.active().setup()
    }

    fn answers_within(&self, within: Duration) -> bool {
        self.active().answers_within(within)
    }

    fn interrupt(&self, room: RoomId) -> Result<(), Error> {
        self.active().interrupt(room)
    }

    fn record_call(&self, request: &RecordCallRequest) -> Result<(), Error> {
        self.active().record_call(request)
    }

    fn transcribe(&self, name: &str, words: &[String], audio: &[u8]) -> Result<String, Error> {
        self.active().transcribe(name, words, audio)
    }

    fn catch_up(&self, room: RoomId, after: Option<MessageId>) -> Result<ChatPage, Error> {
        self.active().catch_up(room, after)
    }

    fn set_project(&self, change: &ProjectSetRequest) -> Result<(), Error> {
        self.active().set_project(change)
    }

    fn device(&self) -> Option<DeviceId> {
        self.active().device()
    }

    fn revoke_device(&self, device: DeviceId) -> Result<(), Error> {
        self.active().revoke_device(device)
    }

    fn sign_out_here(&self) -> Result<(), Error> {
        self.active().sign_out_here()
    }

    fn new_pairing_code(&self) -> Result<PairingCode, Error> {
        self.active().new_pairing_code()
    }

    fn task(&self, id: TaskId) -> Result<TaskPage, Error> {
        self.active().task(id)
    }

    fn find_tasks(&self, query: &TaskQuery) -> Result<Vec<Issue>, Error> {
        self.active().find_tasks(query)
    }

    fn message_task(
        &self,
        id: TaskId,
        message: &TaskMessageRequest,
    ) -> Result<TaskMessageReply, Error> {
        self.active().message_task(id, message)
    }

    fn act_on_tasks(&self, action: &TaskAction) -> Result<(), Error> {
        self.active().act_on_tasks(action)
    }

    fn session_transcript(&self, id: &SessionId, query: &StepsQuery) -> Result<Transcript, Error> {
        self.active().session_transcript(id, query)
    }

    fn follow_session(
        &self,
        id: &SessionId,
        query: &StepsQuery,
        stop: &dyn Fn() -> bool,
        on: &mut dyn FnMut(SessionLive),
    ) -> Result<(), Error> {
        let (on_fallback, shared) = (self.shared.on_fallback(), &self.shared);
        let moved = || stop() || shared.on_fallback() != on_fallback;
        self.at(on_fallback).follow_session(id, query, &moved, on)
    }

    fn send_to_session(
        &self,
        id: &SessionId,
        message: &SendToSession,
    ) -> Result<SentToSession, Error> {
        self.active().send_to_session(id, message)
    }

    fn stop_session(&self, id: &SessionId) -> Result<(), Error> {
        self.active().stop_session(id)
    }

    fn interrupt_session(&self, id: &SessionId) -> Result<(), Error> {
        self.active().interrupt_session(id)
    }

    fn change_session(&self, id: &SessionId, settings: &SessionSettings) -> Result<(), Error> {
        self.active().change_session(id, settings)
    }

    fn move_session(&self, id: &SessionId, task: TaskId) -> Result<TaskId, Error> {
        self.active().move_session(id, task)
    }

    fn open_session(&self, session: &OpenSession) -> Result<OpenedSession, Error> {
        self.active().open_session(session)
    }

    fn runner_folders(&self, query: &FolderQuery) -> Result<FolderList, Error> {
        self.active().runner_folders(query)
    }

    fn runner_branches(&self, query: &BranchQuery) -> Result<BranchList, Error> {
        self.active().runner_branches(query)
    }

    fn t3_activity(&self, query: &ActivityQuery) -> Result<ThreadActivity, Error> {
        self.active().t3_activity(query)
    }

    fn environments(&self) -> Result<EnvPage, Error> {
        self.active().environments()
    }

    fn environment_action(&self, request: &EnvActionRequest) -> Result<EnvPage, Error> {
        self.active().environment_action(request)
    }

    fn check_usage(&self) -> Result<(), Error> {
        self.active().check_usage()
    }

    fn use_account(&self, request: &UseAccountRequest) -> Result<UseAccountReply, Error> {
        self.active().use_account(request)
    }

    fn jobs(&self, fresh: bool) -> Result<JobsPage, Error> {
        self.active().jobs(fresh)
    }

    fn act_on_job(&self, action: JobAction, job: &MachineJobId) -> Result<(), Error> {
        self.active().act_on_job(action, job)
    }

    fn parties(&self) -> Result<PartiesPage, Error> {
        self.active().parties()
    }

    fn start_party(&self, party: &NewParty) -> Result<Party, Error> {
        self.active().start_party(party)
    }

    fn act_on_party(&self, id: PartyId, action: &PartyAction) -> Result<Option<Party>, Error> {
        self.active().act_on_party(id, action)
    }

    fn follow_party(
        &self,
        id: PartyId,
        stop: &dyn Fn() -> bool,
        on: &mut dyn FnMut(PartyEvent),
    ) -> Result<(), Error> {
        let (on_fallback, shared) = (self.shared.on_fallback(), &self.shared);
        let moved = || stop() || shared.on_fallback() != on_fallback;
        self.at(on_fallback).follow_party(id, &moved, on)
    }

    fn dark(&self) -> Result<DarkRoom, Error> {
        self.active().dark()
    }

    fn send_dark(&self, text: &str, on: &mut dyn FnMut(DarkEvent)) -> Result<(), Error> {
        self.active().send_dark(text, on)
    }

    fn clear_dark(&self) -> Result<(), Error> {
        self.active().clear_dark()
    }

    fn browser_session(&self, device: &BrowserDeviceId) -> Result<BrowserSession, Error> {
        self.active().browser_session(device)
    }

    fn browser_input(&self, input: &BrowserInput) -> Result<BrowserInfo, Error> {
        self.active().browser_input(input)
    }

    fn browser_upload(&self, name: &str, bytes: &[u8]) -> Result<BrowserInfo, Error> {
        self.active().browser_upload(name, bytes)
    }

    fn scheduled(&self) -> Result<ScheduledPage, Error> {
        self.active().scheduled()
    }

    fn calendar(&self, query: &CalendarQuery) -> Result<CalendarPage, Error> {
        self.active().calendar(query)
    }

    fn notes(&self) -> Result<NotesPage, Error> {
        self.active().notes()
    }

    fn save_note(&self, note: &NoteSave) -> Result<NoteSaved, Error> {
        self.active().save_note(note)
    }

    fn release_feed(
        &self,
        platform: &emberhut_api::releases::Platform,
        format: u32,
    ) -> Result<Option<emberhut_api::releases::ReleaseFeed>, Error> {
        self.active().release_feed(platform, format)
    }

    fn download_release(
        &self,
        path: &str,
        to: &mut dyn Write,
        max_len: u64,
    ) -> Result<PayloadDigest, Error> {
        self.active().download_release(path, to, max_len)
    }

    fn approvals(&self, query: &ApprovalsQuery) -> Result<ApprovalsPage, Error> {
        self.active().approvals(query)
    }

    fn approval(&self, id: ApprovalId) -> Result<ApprovalReply, Error> {
        self.active().approval(id)
    }

    fn decide_approval(
        &self,
        id: ApprovalId,
        decision: &DecisionRequest,
    ) -> Result<ApprovalReply, Error> {
        self.active().decide_approval(id, decision)
    }

    fn withdraw_approval(
        &self,
        id: ApprovalId,
        withdrawal: &WithdrawRequest,
    ) -> Result<ApprovalReply, Error> {
        self.active().withdraw_approval(id, withdrawal)
    }
}

#[cfg(test)]
mod tests;
