//! The adapter for the shared `emberhut-client`: its replies and errors mapped into the seam's.

use std::io::Write;
use std::ops::ControlFlow;
use std::sync::Arc;

use emberhut_api::answers::AnswerRequest;
use emberhut_api::approvals::{
    ApprovalReply, ApprovalsPage, ApprovalsQuery, DecisionRequest, WithdrawRequest,
};
use emberhut_api::auth::PairingCode;
use emberhut_api::browser::{BrowserInfo, BrowserInput, BrowserSession, BrowserSessionQuery};
use emberhut_api::calendar::{CalendarPage, CalendarQuery};
use emberhut_api::calls::RecordCallRequest;
use emberhut_api::chat::{ChatFile, ChatPage, HistoryQuery};
use emberhut_api::chrome::{ChromeAction, ChromeReply};
use emberhut_api::connect::{LinkStatementRequest, SignedLink};
use emberhut_api::dark::{DarkEvent, DarkRequest, DarkRoom};
use emberhut_api::environments::{EnvActionRequest, EnvPage};
use emberhut_api::facts::FactsPage;
use emberhut_api::headers::ETag;
use emberhut_api::ids::SessionId;
use emberhut_api::ids::{ApprovalId, MachineJobId};
use emberhut_api::ids::{BrowserDeviceId, PartyId};
use emberhut_api::ids::{DeviceId, MessageId, RoomId, TaskId};
use emberhut_api::jobs::{JobAction, JobActionRequest, JobsPage};
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
use emberhut_api::switches::{SwitchAction, SwitchPage};
use emberhut_api::talk::{TalkReply, TalkRequest};
use emberhut_api::task_actions::TaskAction;
use emberhut_api::task_message::{TaskMessageReply, TaskMessageRequest};
use emberhut_api::task_page::TaskPage;
use emberhut_api::task_search::TaskQuery;
use emberhut_api::tasks::Issue;
use emberhut_api::theme::{ThemeChange, ThemePage};
use emberhut_api::threads::{ActivityQuery, ThreadActivity};
use emberhut_api::transcribe::TranscribeQuery;
use emberhut_api::uploads::UploadQuery;
use emberhut_api::usage::{UseAccountReply, UseAccountRequest};
use emberhut_client::{Client, ClientConfig, MemorySessionStore, SessionStore, StatePoll, Upload};
use emberhut_identity::MachineId;

use super::{
    Backend, Error, FreshState, RoomEvent, SharedBytes, StateReply, TooLarge, UploadSource,
};
use crate::config::{AppFiles, ServerUrl};

/// `emberhut-client`, as the app's backend.
#[derive(Debug)]
pub(super) struct EmberhutBackend {
    server: ServerUrl,
    client: Client,
}

impl EmberhutBackend {
    /// The client for `server`, its session kept in `files` (in memory without them): the
    /// Keychain on macOS, the private session file elsewhere ([`session_store`]).
    pub(super) fn open(
        server: ServerUrl,
        files: Option<&AppFiles>,
        device_name: &str,
    ) -> Result<Self, Error> {
        let store = files.map(|files| session_store(&server, files));
        Self::with_store(server, store, device_name)
    }

    /// The client for `server`, its session kept in `store` (in memory without one).
    /// emberhut-client starts a new install over a session that doesn't read; one it can't use
    /// otherwise (written by a newer app, or not readable at all) is left as it is, and this
    /// launch keeps its session in memory.
    fn with_store(
        server: ServerUrl,
        store: Option<impl SessionStore + 'static>,
        device_name: &str,
    ) -> Result<Self, Error> {
        let config = ClientConfig::new(server.as_str())
            .map_err(|error| Error::Local(Box::new(error)))?
            .with_device(device_name, std::env::consts::OS);
        let client = match store {
            None => Client::new(&config, MemorySessionStore::new()),
            Some(store) => Client::new(&config, store).or_else(|error| {
                log::warn!("can't use the kept session ({error}); this launch keeps one in memory");
                Client::new(&config, MemorySessionStore::new())
            }),
        }
        .map_err(error)?;
        Ok(Self { server, client })
    }
}

/// Where this device's session is kept: the Keychain on macOS (moving in a session file an
/// earlier launch kept), the private session file elsewhere.
#[cfg(target_os = "macos")]
fn session_store(
    server: &ServerUrl,
    files: &AppFiles,
) -> super::keychain::KeychainSessionStore<super::keychain::system::SystemItem> {
    super::keychain::KeychainSessionStore::system(server.as_str(), files.session())
}

/// Where this device's session is kept: the Keychain on macOS, the private session file
/// elsewhere (readable by the user only).
#[cfg(not(target_os = "macos"))]
fn session_store(_server: &ServerUrl, files: &AppFiles) -> emberhut_client::FileSessionStore {
    emberhut_client::FileSessionStore::new(files.session())
}

impl Backend for EmberhutBackend {
    fn server(&self) -> &ServerUrl {
        &self.server
    }

    fn is_signed_in(&self) -> bool {
        self.client.is_signed_in()
    }

    fn state(&self, room: RoomId, etag: Option<&ETag>, looking: bool) -> Result<StateReply, Error> {
        let poll = StatePoll::new(room).held(etag.cloned()).looking(looking);
        match self.client.state(&poll).map_err(error)? {
            emberhut_client::StateReply::Unchanged => Ok(StateReply::Unchanged),
            emberhut_client::StateReply::Fresh { state, etag } => {
                Ok(StateReply::Fresh(FreshState { state, etag }))
            }
            emberhut_client::StateReply::SignInRequired => Ok(StateReply::SignInRequired),
            other => Err(Error::Unreadable(
                format!("a state reply this app doesn't know: {other:?}").into(),
            )),
        }
    }

    fn seen(&self, room: RoomId) -> Result<(), Error> {
        self.client.seen(room).map_err(error)
    }

    fn history(&self, room: RoomId, before: MessageId) -> Result<ChatPage, Error> {
        self.client
            .history(&HistoryQuery::before(room, before))
            .map_err(error)
    }

    fn talk(&self, request: &TalkRequest) -> Result<TalkReply, Error> {
        self.client.talk(request).map_err(error)
    }

    fn upload(
        &self,
        name: &str,
        source: &UploadSource,
        progress: &mut dyn FnMut(u64) -> ControlFlow<()>,
    ) -> Result<ChatFile, Error> {
        let upload = match source {
            UploadSource::File(path) => Upload::file(path).named(name),
            UploadSource::Bytes(bytes) => Upload::bytes(name, Shared(Arc::clone(bytes))),
        };
        self.client
            .upload(&upload, |sent| progress(sent.done))
            .map_err(error)
    }

    fn download(
        &self,
        path: &str,
        to: &mut dyn Write,
        progress: &mut dyn FnMut(u64, Option<u64>) -> ControlFlow<()>,
    ) -> Result<u64, Error> {
        self.client
            .download(path, to, |got| progress(got.done, got.total))
            .map_err(error)
    }

    fn decide(&self, task: TaskId, choice: &str) -> Result<(), Error> {
        self.client
            .answer(&AnswerRequest::new(task, choice))
            .map_err(error)
    }

    fn theme(&self) -> Result<ThemePage, Error> {
        self.client.theme().map_err(error)
    }

    fn change_theme(&self, change: &ThemeChange) -> Result<ThemePage, Error> {
        self.client.set_theme(change).map_err(error)
    }

    fn machines(&self) -> Result<MachinesPage, Error> {
        self.client.machines().map_err(error)
    }

    fn file(&self, path: &str) -> Result<Vec<u8>, Error> {
        self.client.file(path).map_err(error)
    }

    fn pair(&self, code: &str) -> Result<(), Error> {
        self.client.pair(code).map_err(error)
    }

    fn pair_with_machine(&self, code: &str, machine: &MachineId) -> Result<(), Error> {
        self.client
            .pair_with_machine(code, machine)
            .map(drop)
            .map_err(error)
    }

    fn link_statement(&self, request: &LinkStatementRequest) -> Result<SignedLink, Error> {
        self.client.link_statement(request).map_err(error)
    }

    fn follow_live(
        &self,
        room: RoomId,
        stop: &dyn Fn() -> bool,
        on: &mut dyn FnMut(RoomEvent),
    ) -> Result<(), Error> {
        self.client
            .follow_room(room, stop, |event| match event {
                emberhut_client::RoomEvent::Live(live) => on(RoomEvent::Live(live)),
                emberhut_client::RoomEvent::Approvals(changed) => on(RoomEvent::Approvals(changed)),
                other => log::debug!("a room event this app doesn't know: {other:?}"),
            })
            .map(drop)
            .map_err(error)
    }

    fn interrupt(&self, room: RoomId) -> Result<(), Error> {
        self.client.interrupt(room).map_err(error)
    }

    fn record_call(&self, request: &RecordCallRequest) -> Result<(), Error> {
        let reply = self.client.record_call(request).map_err(error)?;
        if reply.ok {
            Ok(())
        } else {
            Err(Error::Refused {
                status: None,
                message: reply.error,
            })
        }
    }

    fn transcribe(&self, name: &str, words: &[String], audio: &[u8]) -> Result<String, Error> {
        self.client
            .transcribe(&TranscribeQuery::new(name, words), audio)
            .map_err(error)
    }

    fn catch_up(&self, room: RoomId, after: Option<MessageId>) -> Result<ChatPage, Error> {
        let query = after.map_or_else(
            || HistoryQuery::latest(room),
            |after| HistoryQuery::after(room, after),
        );
        self.client.history(&query).map_err(error)
    }

    fn set_project(&self, change: &ProjectSetRequest) -> Result<(), Error> {
        self.client.set_project(change).map_err(error)
    }

    fn device(&self) -> Option<DeviceId> {
        self.client.device()
    }

    fn revoke_device(&self, device: DeviceId) -> Result<(), Error> {
        self.client.revoke_device(device).map_err(error)
    }

    fn sign_out_here(&self) -> Result<(), Error> {
        self.client.sign_out_here().map_err(error)
    }

    fn new_pairing_code(&self) -> Result<PairingCode, Error> {
        self.client.new_pairing_code().map_err(error)
    }

    fn settings(&self, action: SettingsAction) -> Result<(), Error> {
        self.client.settings(action).map_err(error)
    }

    fn switches(&self) -> Result<SwitchPage, Error> {
        self.client.switches().map_err(error)
    }

    fn switch(&self, action: &SwitchAction) -> Result<SwitchPage, Error> {
        self.client.switch(action).map_err(error)
    }

    fn facts(&self) -> Result<FactsPage, Error> {
        self.client.facts().map_err(error)
    }

    fn chrome(&self, action: &ChromeAction) -> Result<ChromeReply, Error> {
        self.client.chrome(action).map_err(error)
    }

    fn task(&self, id: TaskId) -> Result<TaskPage, Error> {
        self.client.task(id).map_err(error)
    }

    fn find_tasks(&self, query: &TaskQuery) -> Result<Vec<Issue>, Error> {
        self.client
            .find_tasks(query)
            .map(|found| found.tasks)
            .map_err(error)
    }

    fn message_task(
        &self,
        id: TaskId,
        message: &TaskMessageRequest,
    ) -> Result<TaskMessageReply, Error> {
        self.client.message_task(id, message).map_err(error)
    }

    fn act_on_tasks(&self, action: &TaskAction) -> Result<(), Error> {
        self.client.act_on_tasks(action).map_err(error)
    }

    fn session_transcript(&self, id: &SessionId, query: &StepsQuery) -> Result<Transcript, Error> {
        self.client.session_transcript(id, query).map_err(error)
    }

    fn follow_session(
        &self,
        id: &SessionId,
        query: &StepsQuery,
        stop: &dyn Fn() -> bool,
        on: &mut dyn FnMut(SessionLive),
    ) -> Result<(), Error> {
        self.client
            .follow_session(id, query, stop, on)
            .map(drop)
            .map_err(error)
    }

    fn send_to_session(
        &self,
        id: &SessionId,
        message: &SendToSession,
    ) -> Result<SentToSession, Error> {
        self.client.send_to_session(id, message).map_err(error)
    }

    fn stop_session(&self, id: &SessionId) -> Result<(), Error> {
        self.client.stop_session(id).map(drop).map_err(error)
    }

    fn interrupt_session(&self, id: &SessionId) -> Result<(), Error> {
        self.client.interrupt_session(id).map(drop).map_err(error)
    }

    fn change_session(&self, id: &SessionId, settings: &SessionSettings) -> Result<(), Error> {
        self.client.change_session(id, settings).map_err(error)
    }

    fn open_session(&self, session: &OpenSession) -> Result<OpenedSession, Error> {
        self.client.open_session(session).map_err(error)
    }

    fn runner_folders(&self, query: &FolderQuery) -> Result<FolderList, Error> {
        self.client.runner_folders(query).map_err(error)
    }

    fn runner_branches(&self, query: &BranchQuery) -> Result<BranchList, Error> {
        self.client.runner_branches(query).map_err(error)
    }

    fn t3_activity(&self, query: &ActivityQuery) -> Result<ThreadActivity, Error> {
        self.client.t3_activity(query).map_err(error)
    }

    fn environments(&self) -> Result<EnvPage, Error> {
        self.client.environments().map_err(error)
    }

    fn environment_action(&self, request: &EnvActionRequest) -> Result<EnvPage, Error> {
        self.client.environment_action(request).map_err(error)
    }

    fn check_usage(&self) -> Result<(), Error> {
        self.client.check_usage().map(drop).map_err(error)
    }

    fn use_account(&self, request: &UseAccountRequest) -> Result<UseAccountReply, Error> {
        self.client.use_account(request).map_err(error)
    }

    fn jobs(&self, fresh: bool) -> Result<JobsPage, Error> {
        self.client.jobs(fresh).map_err(error)
    }

    fn act_on_job(&self, action: JobAction, job: &MachineJobId) -> Result<(), Error> {
        let request = JobActionRequest { id: job.clone() };
        self.client.act_on_job(action, &request).map_err(error)
    }

    fn parties(&self) -> Result<PartiesPage, Error> {
        self.client.parties().map_err(error)
    }

    fn start_party(&self, party: &NewParty) -> Result<Party, Error> {
        self.client
            .start_party(party)
            .map_err(error)?
            .party
            .ok_or_else(|| Error::Unreadable("a new party's answer without its party".into()))
    }

    fn act_on_party(&self, id: PartyId, action: &PartyAction) -> Result<Option<Party>, Error> {
        self.client
            .act_on_party(id, action)
            .map(|reply| reply.party)
            .map_err(error)
    }

    fn follow_party(
        &self,
        id: PartyId,
        stop: &dyn Fn() -> bool,
        on: &mut dyn FnMut(PartyEvent),
    ) -> Result<(), Error> {
        self.client
            .follow_party(id, stop, on)
            .map(drop)
            .map_err(error)
    }

    fn dark(&self) -> Result<DarkRoom, Error> {
        self.client.dark().map_err(error)
    }

    fn send_dark(&self, text: &str, on: &mut dyn FnMut(DarkEvent)) -> Result<(), Error> {
        let request = DarkRequest {
            text: text.to_owned(),
        };
        self.client.send_dark(&request, on).map_err(error)
    }

    fn clear_dark(&self) -> Result<(), Error> {
        self.client.clear_dark().map_err(error)
    }

    fn browser_session(&self, device: &BrowserDeviceId) -> Result<BrowserSession, Error> {
        let query = BrowserSessionQuery {
            device: device.clone(),
        };
        self.client
            .browser_session(&query)
            .map(|reply| reply.session)
            .map_err(error)
    }

    fn browser_input(&self, input: &BrowserInput) -> Result<BrowserInfo, Error> {
        self.client
            .browser_input(input)
            .map(|reply| reply.browser)
            .map_err(error)
    }

    fn browser_upload(&self, name: &str, bytes: &[u8]) -> Result<BrowserInfo, Error> {
        self.client
            .browser_upload(&UploadQuery::new(name), bytes)
            .map(|reply| reply.browser)
            .map_err(error)
    }

    fn scheduled(&self) -> Result<ScheduledPage, Error> {
        self.client.scheduled().map_err(error)
    }

    fn calendar(&self, query: &CalendarQuery) -> Result<CalendarPage, Error> {
        self.client.calendar(query).map_err(error)
    }

    fn notes(&self) -> Result<NotesPage, Error> {
        self.client.notes().map_err(error)
    }

    fn save_note(&self, note: &NoteSave) -> Result<NoteSaved, Error> {
        self.client.save_note(note).map_err(error)
    }

    fn release_feed(
        &self,
        platform: &emberhut_api::releases::Platform,
        format: u32,
    ) -> Result<Option<emberhut_api::releases::ReleaseFeed>, Error> {
        self.client.release_feed(platform, format).map_err(error)
    }

    fn download_release(
        &self,
        path: &str,
        to: &mut dyn Write,
        max_len: u64,
    ) -> Result<super::PayloadDigest, Error> {
        self.client
            .download_release(path, to, max_len)
            .map(|got| super::PayloadDigest {
                size: got.size,
                sha256: got.sha256,
            })
            .map_err(error)
    }

    fn approvals(&self, query: &ApprovalsQuery) -> Result<ApprovalsPage, Error> {
        self.client.approvals(query).map_err(error)
    }

    fn approval(&self, id: ApprovalId) -> Result<ApprovalReply, Error> {
        self.client.approval(id).map_err(error)
    }

    fn decide_approval(
        &self,
        id: ApprovalId,
        decision: &DecisionRequest,
    ) -> Result<ApprovalReply, Error> {
        self.client.decide(id, decision).map_err(error)
    }

    fn withdraw_approval(
        &self,
        id: ApprovalId,
        withdrawal: &WithdrawRequest,
    ) -> Result<ApprovalReply, Error> {
        self.client.withdraw(id, withdrawal).map_err(error)
    }

    fn stop(&self) -> Result<(), Error> {
        self.client.stop().map_err(error)
    }
}

/// Shared bytes as `emberhut-client` uploads them, without a copy.
struct Shared(SharedBytes);

impl AsRef<[u8]> for Shared {
    fn as_ref(&self) -> &[u8] {
        (*self.0).as_ref()
    }
}

/// `emberhut-client`'s error as the app tells errors apart.
fn error(error: emberhut_client::Error) -> Error {
    use emberhut_client::Error as Client;
    match error {
        Client::Network(network) => Error::Unreachable(Box::new(network)),
        Client::SignInRequired => Error::SignInRequired,
        Client::Status { status, message } => Error::Refused {
            status: Some(status),
            message,
        },
        Client::Refused(refusal) => Error::Refused {
            status: Some(refusal.status),
            message: Some(refusal.error),
        },
        Client::Decode(unreadable) => Error::Unreadable(Box::new(unreadable)),
        Client::Protocol(broken) => Error::Unreadable(Box::new(broken)),
        Client::TooLarge { size, limit } => Error::Local(Box::new(TooLarge { size, limit })),
        Client::WrongMachine => Error::WrongMachine,
        other => Error::Local(Box::new(other)),
    }
}

#[cfg(test)]
mod tests {
    use emberhut_api::headers::LOOKING;
    use serde_json::json;

    use super::*;
    use crate::backend::test_server::{Reply, TestServer};

    fn backend(server: &TestServer) -> EmberhutBackend {
        EmberhutBackend::open(server.url(), None, "Emberhut · test").unwrap()
    }

    #[test]
    fn the_clients_errors_map_to_what_the_app_tells_apart() {
        let refused = error(emberhut_client::Error::Status {
            status: 401,
            message: Some("why".into()),
        });
        assert!(
            matches!(refused, Error::Refused { status: Some(401), message: Some(ref why) } if why == "why")
        );
        assert!(matches!(
            error(emberhut_client::Error::SignInRequired),
            Error::SignInRequired
        ));
        let large = error(emberhut_client::Error::TooLarge { size: 9, limit: 4 });
        assert!(matches!(large, Error::Local(ref cause) if cause.is::<TooLarge>()));
        let garbage = serde_json::from_str::<u8>("x").unwrap_err();
        assert!(matches!(
            error(emberhut_client::Error::Decode(garbage)),
            Error::Unreadable(_)
        ));
    }

    #[test]
    fn a_state_poll_maps_to_the_seams_reply() {
        let state = json!({"room": 2, "rooms": [], "on": true, "busy": false, "live": null,
            "unread": false, "pending": 0, "chat": [], "earlier": false});
        let server = TestServer::start(3, move |request| {
            if request.headers.contains_key("if-none-match") {
                Reply::text(304, "")
            } else if request.target.contains("room=2") {
                Reply::json(200, &state).header("ETag", "\"v1\"")
            } else {
                Reply::json(401, &json!({"error": "Sign in", "signIn": true}))
            }
        });
        let backend = backend(&server);
        let StateReply::Fresh(fresh) = backend.state(RoomId::new(2), None, true).unwrap() else {
            panic!("a fresh state")
        };
        assert_eq!(fresh.state.room, RoomId::new(2));
        let etag = fresh.etag.expect("its fingerprint");
        assert_eq!(
            backend.state(RoomId::new(2), Some(&etag), false).unwrap(),
            StateReply::Unchanged
        );
        assert_eq!(
            backend.state(RoomId::new(3), None, false).unwrap(),
            StateReply::SignInRequired
        );
        let requests = server.finish();
        assert_eq!(requests[0].method, "GET");
        assert_eq!(requests[0].target, "/api/state?room=2");
        assert_eq!(
            requests[0].headers.get(LOOKING).map(String::as_str),
            Some("2")
        );
        assert!(!requests[1].headers.contains_key(LOOKING));
    }

    #[test]
    fn approvals_are_read_decided_and_followed_on_the_live_stream() {
        let approval = json!({
            "id": 7, "agent": "client-a", "kind": "post", "target": "example blog",
            "summary": "The launch post", "files": [], "payloadHash": "ab".repeat(32),
            "approver": "owner", "waitsOnOwner": true, "state": "pending",
            "createdAt": "2026-11-27T10:00:00Z"
        });
        let stream = "event: approvals\ndata: {\"id\":7,\"state\":\"pending\",\"pending\":1}\n\n\
                      data: null\n\n";
        let server = TestServer::start(4, move |request| {
            match (request.method.as_str(), request.target.as_str()) {
                ("GET", "/api/approvals?state=pending") => {
                    Reply::json(200, &json!({"approvals": [approval.clone()]}))
                }
                ("GET", "/api/approvals/7") => {
                    Reply::json(200, &json!({"approval": approval.clone()}))
                }
                ("POST", "/api/approvals/7/decision") => {
                    Reply::json(409, &json!({"error": "This request changed."}))
                }
                _ => Reply::text(200, stream).header("Content-Type", "text/event-stream"),
            }
        });
        let backend = backend(&server);
        let query = ApprovalsQuery {
            state: Some(emberhut_api::approvals::ApprovalState::Pending),
            agent: None,
        };
        let page = backend.approvals(&query).unwrap();
        assert_eq!(page.approvals[0].id, ApprovalId::new(7));
        let decision = DecisionRequest {
            decision: emberhut_api::approvals::Decision::Approve,
            payload_hash: "cd".repeat(32),
            reason: None,
        };
        let refused = backend
            .decide_approval(ApprovalId::new(7), &decision)
            .unwrap_err();
        assert!(
            matches!(
                refused,
                Error::Refused {
                    status: Some(409),
                    ..
                }
            ),
            "{refused:?}"
        );
        assert_eq!(refused.server_message(), Some("This request changed."));
        assert!(
            backend
                .approval(ApprovalId::new(7))
                .unwrap()
                .approval
                .waits_on_owner
        );
        let mut events = Vec::new();
        backend
            .follow_live(RoomId::MAIN, &|| false, &mut |event| events.push(event))
            .unwrap();
        let changed = emberhut_api::approvals::ApprovalsEvent {
            id: ApprovalId::new(7),
            state: emberhut_api::approvals::ApprovalState::Pending,
            pending: 1,
        };
        assert_eq!(
            events,
            [
                RoomEvent::Approvals(changed),
                RoomEvent::Live(emberhut_api::live::LiveEvent::Idle)
            ]
        );
        let requests = server.finish();
        assert_eq!(
            requests[1].json(),
            json!({"decision": "approve", "payloadHash": "cd".repeat(32)})
        );
    }

    #[test]
    fn messages_answers_and_pairing_post_their_bodies() {
        let server = TestServer::start(3, |request| match request.target.as_str() {
            "/api/talk" => Reply::json(200, &json!({"ok": true, "started": true})),
            "/api/auth/pair" => Reply::json(
                401,
                &json!({"ok": false, "error": "That code was already used."}),
            ),
            _ => Reply::json(200, &json!({"ok": true})),
        });
        let backend = backend(&server);
        let talk = TalkRequest::text(RoomId::new(2), "Hello");
        assert!(backend.talk(&talk).unwrap().started);
        backend.decide(TaskId::new(7), "Yes").unwrap();
        let refused = backend.pair("ABCD-EFGH").unwrap_err();
        assert_eq!(
            refused.server_message(),
            Some("That code was already used.")
        );
        let requests = server.finish();
        assert_eq!(requests[0].method, "POST");
        assert_eq!(
            requests[0].json(),
            json!({"text": "Hello", "files": [], "room": 2})
        );
        assert_eq!(
            requests[1].json(),
            json!({"taskDecide": 7, "choice": "Yes"})
        );
        assert_eq!(requests[2].json()["name"], "Emberhut · test");
    }

    #[test]
    fn a_recording_the_mac_wasnt_told_of_is_refused_with_the_servers_words() {
        let refused: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../crates/emberhut-api/tests/fixtures/record_call_refused.json"
        ))
        .unwrap();
        let server = TestServer::start(2, move |request| {
            if request.json()["action"] == "start" {
                Reply::json(200, &refused)
            } else {
                Reply::json(200, &json!({"ok": true, "text": "Stopped."}))
            }
        });
        let backend = backend(&server);
        let error = backend
            .record_call(&RecordCallRequest::start(RoomId::new(2)))
            .unwrap_err();
        assert!(
            matches!(error, Error::Refused { status: None, .. }),
            "{error:?}"
        );
        assert_eq!(error.server_message(), Some("the Mac isn't reachable"));
        backend
            .record_call(&RecordCallRequest::stop(RoomId::new(2)))
            .unwrap();
        let requests = server.finish();
        assert_eq!(requests[0].target, "/api/calls/record");
        assert_eq!(requests[0].json(), json!({"action": "start", "room": 2}));
    }

    #[test]
    fn an_upload_sends_the_bytes_and_reads_the_stored_file() {
        let stored = json!({"ok": true, "file": {"name": "a b.png", "url": "/uploads/1-a%20b.png",
            "kind": "image", "path": "/data/uploads/1-a b.png"}});
        let server = TestServer::start(1, move |_| Reply::json(200, &stored));
        let mut sent = Vec::new();
        let bytes: SharedBytes = Arc::new(b"picture".to_vec());
        let file = backend(&server)
            .upload("a b.png", &UploadSource::Bytes(bytes), &mut |n| {
                sent.push(n);
                ControlFlow::Continue(())
            })
            .unwrap();
        assert_eq!(file.url, "/uploads/1-a%20b.png");
        assert_eq!(sent.last(), Some(&7));
        let requests = server.finish();
        assert_eq!(requests[0].target, "/api/uploads?name=a%20b.png");
        assert_eq!(requests[0].body, "picture");
    }

    #[test]
    fn a_file_that_cant_be_read_is_local() {
        let server = TestServer::start(0, |_| Reply::text(500, ""));
        let missing = UploadSource::File("/nonexistent/emberhut/a.pdf".into());
        let error = backend(&server)
            .upload("a.pdf", &missing, &mut |_| ControlFlow::Continue(()))
            .unwrap_err();
        assert!(matches!(error, Error::Local(_)), "{error:?}");
        assert!(server.finish().is_empty());
    }

    #[test]
    fn a_download_streams_into_the_writer() {
        let server = TestServer::start(1, |_| Reply::text(200, "a file"));
        let mut kept = Vec::new();
        let mut told = Vec::new();
        let size = backend(&server)
            .download("/uploads/a.txt", &mut kept, &mut |got, total| {
                told.push((got, total));
                ControlFlow::Continue(())
            })
            .unwrap();
        assert_eq!((size, kept.as_slice()), (6, &b"a file"[..]));
        assert_eq!(told, [(6, Some(6))]);
        assert_eq!(server.finish()[0].target, "/uploads/a.txt");
    }

    #[test]
    fn a_stopped_download_is_cancelled() {
        let server = TestServer::start(1, |_| Reply::text(200, "a file"));
        let error = backend(&server)
            .download("/uploads/a.txt", &mut Vec::new(), &mut |_, _| {
                ControlFlow::Break(())
            })
            .unwrap_err();
        assert!(matches!(error, Error::Local(_)), "{error:?}");
    }

    #[test]
    fn a_release_payload_is_streamed_with_its_hash_and_the_feed_read() {
        let server = TestServer::start(2, |request| {
            if request.target.starts_with("/api/releases/v") {
                Reply::json(404, &json!({"error": "not found"}))
            } else {
                Reply::text(200, "a disk image")
            }
        });
        let backend = backend(&server);
        let mut kept = Vec::new();
        let got = backend
            .download_release("/api/releases/files/Emberhut.dmg", &mut kept, 100)
            .unwrap();
        assert_eq!((got.size, kept.as_slice()), (12, &b"a disk image"[..]));
        assert_eq!(
            backend
                .release_feed(&emberhut_api::releases::Platform::Macos, 1)
                .unwrap(),
            None
        );
        let targets: Vec<_> = server.finish().into_iter().map(|r| r.target).collect();
        assert_eq!(
            targets,
            ["/api/releases/files/Emberhut.dmg", "/api/releases/v1/macos"]
        );
    }

    #[test]
    fn a_settings_change_posts_its_keys() {
        let server = TestServer::start(1, |_| Reply::json(200, &json!({"ok": true})));
        backend(&server)
            .settings(SettingsAction::SetOn(true))
            .unwrap();
        let requests = server.finish();
        assert_eq!(requests[0].target, "/api/settings");
        assert_eq!(requests[0].json(), json!({"on": true}));
    }

    #[test]
    fn seen_posts_the_room() {
        let server = TestServer::start(1, |_| Reply::json(200, &json!({"ok": true})));
        backend(&server).seen(RoomId::new(4)).unwrap();
        let requests = server.finish();
        assert_eq!(requests[0].target, "/api/seen");
        assert_eq!(requests[0].json(), json!({"room": 4}));
    }

    #[test]
    fn a_closed_port_is_unreachable() {
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let url = ServerUrl::parse(&format!("http://127.0.0.1:{port}")).unwrap();
        let error = EmberhutBackend::open(url, None, "Emberhut")
            .unwrap()
            .machines()
            .unwrap_err();
        assert!(error.is_unreachable(), "{error:?}");
    }

    /// A backend for a server nothing listens on, its session in `store`.
    fn backend_over(store: impl SessionStore + 'static) -> EmberhutBackend {
        let url = ServerUrl::parse("http://127.0.0.1:9").unwrap();
        EmberhutBackend::with_store(url, Some(store), "Emberhut").unwrap()
    }

    #[test]
    fn a_session_file_that_doesnt_read_starts_a_new_install() {
        let dir = tempfile::tempdir().unwrap();
        let files = AppFiles::in_dir(dir.path());
        std::fs::write(files.session(), "not a session").unwrap();
        let backend = backend_over(emberhut_client::FileSessionStore::new(files.session()));
        assert!(!backend.is_signed_in());
        let kept = std::fs::read_to_string(files.session()).unwrap();
        assert_ne!(kept, "not a session", "a new install was kept in its place");
    }

    /// The Keychain's store as macOS uses it, over a fake item, so it runs on every system.
    #[test]
    fn a_keychain_item_that_doesnt_read_starts_a_new_install() {
        use super::super::keychain::KeychainSessionStore;
        use super::super::keychain::tests::FakeItem;

        let dir = tempfile::tempdir().unwrap();
        let files = AppFiles::in_dir(dir.path());
        let item = FakeItem::holding(b"not a session");
        let store = KeychainSessionStore::new(
            item.clone(),
            emberhut_client::FileSessionStore::new(files.session()),
        );
        let backend = backend_over(store);
        assert!(!backend.is_signed_in());
        let kept = item.bytes().unwrap();
        assert_ne!(
            kept, b"not a session",
            "a new install was kept in its place"
        );
        assert!(
            emberhut_client::stored_version(&kept).is_some(),
            "a session's form"
        );
    }

    #[test]
    fn a_session_file_moves_into_the_keychain_when_the_client_starts() {
        use super::super::keychain::KeychainSessionStore;
        use super::super::keychain::tests::FakeItem;

        let dir = tempfile::tempdir().unwrap();
        let files = AppFiles::in_dir(dir.path());
        let file = emberhut_client::FileSessionStore::new(files.session());
        let session = emberhut_client::Session::new_install().unwrap();
        file.save(&session).unwrap();
        let item = FakeItem::default();
        let backend = backend_over(KeychainSessionStore::new(item.clone(), file));
        assert_eq!(
            backend.client.install(),
            session.install,
            "the same install"
        );
        assert!(item.bytes().is_some(), "in the Keychain");
        assert!(!files.session().exists(), "the file went");
    }
}
