//! The client: a session, and a method per route of the contract.

use std::borrow::Cow;
use std::io::{BufReader, Read as _, Write};
use std::ops::ControlFlow;
use std::time::{Duration, SystemTime};

use emberhut_api::answers::AnswerRequest;
use emberhut_api::auth::{
    AccessToken, AuthReply, AuthStatus, PairRequest, PairingCode, RefreshRequest, RevokeRequest,
    SessionGrant,
};
use emberhut_api::calls::{RecordCallReply, RecordCallRequest};
use emberhut_api::chat::{ChatFile, ChatPage, HistoryQuery};
use emberhut_api::connect::{LINK_STATEMENT, LinkStatementRequest, MachineIdentity, SignedLink};
use emberhut_api::error::OkReply;
use emberhut_api::headers::{self, ETag};
use emberhut_api::health::Health;
use emberhut_api::ids::{DeviceId, InstallId, RoomId, TaskId};
use emberhut_api::interrupt::InterruptRequest;
use emberhut_api::live::{LiveEvent, RoomQuery};
use emberhut_api::machines::MachinesPage;
use emberhut_api::paths;
use emberhut_api::projects::ProjectSetRequest;
use emberhut_api::seen::SeenRequest;
use emberhut_api::state::State;
use emberhut_api::talk::{TalkReply, TalkRequest};
use emberhut_api::task_actions::TaskAction;
use emberhut_api::task_message::{TaskMessageReply, TaskMessageRequest};
use emberhut_api::task_page::TaskPage;
use emberhut_api::task_search::{FoundTasks, TaskQuery};
use emberhut_api::tasks::Issue;
use emberhut_api::theme::{ThemeChange, ThemePage, ThemeSetReply};
use emberhut_api::transcribe::{self, TranscribeQuery, TranscribeReply};
use emberhut_api::uploads::{UploadQuery, UploadReply};
use emberhut_identity::MachineId;
use http::StatusCode;
use http::uri::PathAndQuery;
use serde::Serialize;

use crate::dialect::Dialect;
use crate::exchange::{self, Http};
use crate::live::{self, LiveEnd, RoomEvent};
use crate::renewal::{Renewal, Sessions};
use crate::session::SessionStore;
use crate::transport::{Answer, Direct, Transport, TransportError};
use crate::upload::{Progress, Reporting, Stopped, Upload};
use crate::{ClientConfig, Error};

/// How long [`Client::record_call`] waits for its answer: the server waits up to 20 s for the
/// Mac to start a recording and up to 30 s for it to stop one before it answers, longer than
/// the request timeout (the reference client waits 40 s too).
const RECORD_CALL_TIMEOUT: Duration = Duration::from_secs(40);

/// A blocking client of a server that speaks the emberhut contract.
///
/// Its requests go through a [`Transport`]: [`Client::new`] makes a [`Direct`] to the
/// configured address, and [`Client::with_transport`] takes any other.
///
/// Every method blocks until the server answers (or the timeout); call them from background
/// threads. A `Client` is `Send + Sync`: share one (in an `Arc`) between the poll, the live
/// stream and the actions, so that they share one session and renew it one at a time.
#[derive(Debug)]
pub struct Client {
    pub(crate) http: Http,
    sessions: Sessions,
    device_name: String,
    device_model: String,
    download_limit: u64,
}

/// What a poll of the state found.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum StateReply {
    /// The state is still the one with the `ETag` sent (304).
    Unchanged,
    /// A new state, with its `ETag` to send next time.
    Fresh {
        /// The state.
        state: Box<State>,
        /// Its fingerprint.
        etag: Option<ETag>,
    },
    /// The server enforces sign-in and this device has no valid session: pair it first.
    SignInRequired,
}

/// A poll of the state: the room, the state held (by its `ETag`), and whether the room is on
/// screen in front of the person.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatePoll {
    room: RoomId,
    held: Option<ETag>,
    looking: bool,
}

impl StatePoll {
    /// A poll of a room's state.
    #[must_use]
    pub fn new(room: RoomId) -> Self {
        Self {
            room,
            held: None,
            looking: false,
        }
    }

    /// The state held is the one with this `ETag`: an unchanged state comes back as
    /// [`StateReply::Unchanged`], with nothing to download. Send it only for this same room.
    #[must_use]
    pub fn held(mut self, etag: Option<ETag>) -> Self {
        self.held = etag;
        self
    }

    /// The room is on screen and the person is at the device (the app is in front; on a
    /// desktop, input in the last few minutes). The server then doesn't push the room's
    /// replies to the phone, since they are being read.
    #[must_use]
    pub fn looking(mut self, looking: bool) -> Self {
        self.looking = looking;
        self
    }
}

/// An accepted pairing, before its session is kept.
struct Paired {
    /// The session.
    grant: SessionGrant,
    /// The server's date, for the token's expiry.
    date: Option<SystemTime>,
    /// The machine the answer names, from a server that has one; an error when its key
    /// doesn't have its id.
    machine: Option<Result<MachineIdentity, serde_json::Error>>,
}

impl Client {
    /// A client of the server in `config`, with the session in `store` (a new install is made
    /// and saved when the store has none).
    ///
    /// # Errors
    ///
    /// The store couldn't be read or written, or there was no randomness for a new install id.
    pub fn new(config: &ClientConfig, store: impl SessionStore + 'static) -> Result<Self, Error> {
        Self::with_transport(config, Direct::for_config(config), store)
    }

    /// A client whose requests go through `transport`, with the waits, device and limits of
    /// `config` (whose base URL isn't used: the transport knows where the server is) and the
    /// session in `store`, as [`Client::new`] has.
    ///
    /// # Errors
    ///
    /// The store couldn't be read or written, or there was no randomness for a new install id.
    pub fn with_transport(
        config: &ClientConfig,
        transport: impl Transport + 'static,
        store: impl SessionStore + 'static,
    ) -> Result<Self, Error> {
        Ok(Self {
            http: Http::new(Box::new(transport), config),
            sessions: Sessions::open(Box::new(store))?,
            device_name: config.device_name().to_owned(),
            device_model: config.device_model().to_owned(),
            download_limit: config.download_limit(),
        })
    }

    /// The base URL of the route in use (it may change as the route does), for display and
    /// logs only: requests are built from paths, and the transport puts them where they go.
    #[must_use]
    pub fn base_url(&self) -> Cow<'_, str> {
        self.http.base_url()
    }

    /// The dialect this client speaks to its server in (see [`crate::dialect`]): what a link or
    /// a value passed to or taken from the server outside a request goes through.
    #[must_use]
    pub fn dialect(&self) -> Dialect {
        self.http.dialect()
    }

    /// Whether the server answers (`GET /health`, unsigned), within `within`: a check to make
    /// before trusting the server with the user's work, which a server that is down, or not a
    /// server at all, fails.
    ///
    /// # Errors
    ///
    /// The transport's errors (no answer within `within` among them); [`Error::Status`] for an
    /// answer that isn't a success; [`Error::Decode`] for one that isn't the health's.
    pub fn health(&self, within: Duration) -> Result<Health, Error> {
        let health: Health = exchange::json(self.http.get_within(paths::HEALTH, within)?)?;
        if health.ok {
            Ok(health)
        } else {
            Err(Error::Status {
                status: StatusCode::SERVICE_UNAVAILABLE.as_u16(),
                message: Some("the server says it isn't well".to_owned()),
            })
        }
    }

    /// Whether this device holds a session.
    #[must_use]
    pub fn is_signed_in(&self) -> bool {
        self.sessions.is_signed_in()
    }

    /// This device's row on the server, while signed in (to mark "this one" in the device list).
    #[must_use]
    pub fn device(&self) -> Option<DeviceId> {
        self.sessions.device()
    }

    /// This installation's id.
    #[must_use]
    pub fn install(&self) -> InstallId {
        self.sessions.install()
    }

    /// Forgets this device's session here (the server's row stays until it is signed out
    /// there or goes stale).
    ///
    /// # Errors
    ///
    /// The store couldn't be written. The device is signed out here all the same; the save is
    /// tried again on the next call.
    pub fn sign_out_here(&self) -> Result<(), Error> {
        self.sessions.sign_out()
    }

    /// Stops this client for good, for another process to take its session over (an update's
    /// new copy): a renewal in flight is waited for, the session is saved, and from then on
    /// every request fails at once with [`Error::Stopped`], nothing is renewed, and the live
    /// streams end at their next event or ping.
    ///
    /// # Errors
    ///
    /// The session couldn't be saved. The client is stopped all the same.
    pub fn stop(&self) -> Result<(), Error> {
        self.sessions.stop()
    }

    /// Whether [`Client::stop`] was called.
    #[must_use]
    pub fn is_stopped(&self) -> bool {
        self.sessions.is_stopped()
    }

    /// `stop`, or this client stopped: what a stream's follower asks before each item.
    pub(crate) fn or_stopped(&self, mut stop: impl FnMut() -> bool) -> impl FnMut() -> bool {
        move || self.is_stopped() || stop()
    }

    /// Sends a request with the session's access token, renewed first when it is nearly out.
    ///
    /// A 401 asking for sign-in on a token this device thinks is good (the server's clock is
    /// ahead, the device was signed out there, the server's signing key changed) renews it once
    /// and sends the request again. Any other answer, or a second 401, comes back as it is; a
    /// 401 as its [`Error`].
    pub(crate) fn send(
        &self,
        mut request: impl FnMut(Option<&AccessToken>) -> Result<Answer, Error>,
    ) -> Result<Answer, Error> {
        let bearer = self.sessions.bearer(None, |r| self.renew(r))?;
        let response = request(bearer.as_ref())?;
        if response.status() != StatusCode::UNAUTHORIZED {
            return Ok(response);
        }
        match (exchange::refusal(response), bearer) {
            (Error::SignInRequired, Some(rejected)) => {
                let bearer = self.sessions.bearer(Some(&rejected), |r| self.renew(r))?;
                let response = request(bearer.as_ref())?;
                if response.status() == StatusCode::UNAUTHORIZED {
                    return Err(exchange::refusal(response));
                }
                Ok(response)
            }
            (error, _) => Err(error),
        }
    }

    pub(crate) fn get(&self, path: &str, query: &str) -> Result<Answer, Error> {
        self.send(|bearer| self.http.get(path, query, bearer, &[]))
    }

    /// A `GET` of a file to stream to disk, signed and renewed as every request.
    pub(crate) fn get_file(&self, path: &str, query: &str) -> Result<Answer, Error> {
        self.send(|bearer| self.http.get_file(path, query, bearer))
    }

    pub(crate) fn post<B: Serialize>(&self, path: &str, body: &B) -> Result<Answer, Error> {
        let body = exchange::encode(body)?;
        self.send(|bearer| self.http.post_json(path, &body, bearer))
    }

    /// A `POST` as [`Client::post`], given `within` for the whole answer in place of the
    /// request timeout (a route the server answers slowly on purpose).
    pub(crate) fn post_within<B: Serialize>(
        &self,
        path: &str,
        body: &B,
        within: Duration,
    ) -> Result<Answer, Error> {
        let body = exchange::encode(body)?;
        self.send(|bearer| self.http.post_json_within(path, &body, within, bearer))
    }

    /// A `GET` of a server-sent event stream: the answer, once it is a success.
    pub(crate) fn get_stream(&self, path: &str, query: &str) -> Result<Answer, Error> {
        let answer = self.send(|bearer| self.http.get_stream(path, query, bearer))?;
        if !answer.status().is_success() {
            return Err(exchange::refusal(answer));
        }
        Ok(answer)
    }

    /// A `PUT` of a JSON body, given `within` for the whole answer in place of the request
    /// timeout.
    pub(crate) fn put_within<B: Serialize>(
        &self,
        path: &str,
        body: &B,
        within: Duration,
    ) -> Result<Answer, Error> {
        let body = exchange::encode(body)?;
        self.send(|bearer| self.http.put_json_within(path, &body, within, bearer))
    }

    /// A `PATCH` of a JSON body: a change to what the route names.
    pub(crate) fn patch<B: Serialize>(&self, path: &str, body: &B) -> Result<Answer, Error> {
        let body = exchange::encode(body)?;
        self.send(|bearer| self.http.patch_json(path, &body, bearer))
    }

    pub(crate) fn delete(&self, path: &str) -> Result<Answer, Error> {
        self.send(|bearer| self.http.delete(path, bearer))
    }

    /// A `POST` whose answer is an event stream, refused unless it answers 2xx.
    pub(crate) fn post_events<B: Serialize>(&self, path: &str, body: &B) -> Result<Answer, Error> {
        let body = exchange::encode(body)?;
        let response = self.send(|bearer| self.http.post_stream(path, &body, bearer))?;
        if !response.status().is_success() {
            return Err(exchange::refusal(response));
        }
        Ok(response)
    }

    /// `POST /api/auth/refresh`. A refusal is an [`AuthReply`] with `ok: false`; a 401 without
    /// one (a network gate in front of the server) is not about the session.
    fn renew(&self, request: &RefreshRequest) -> Result<Renewal, Error> {
        let mut response =
            self.http
                .post_json(paths::REFRESH, &exchange::encode(request)?, None)?;
        let status = response.status();
        let date = exchange::server_date(&response);
        let bytes = exchange::body_bytes(&mut response)?;
        match serde_json::from_slice::<AuthReply>(&bytes) {
            Ok(reply) => match reply.into_grant() {
                Ok(grant) if status.is_success() => Ok(Renewal::Granted(grant, date)),
                Ok(_) => Err(Error::Status {
                    status: status.as_u16(),
                    message: None,
                }),
                Err(refusal) if status == StatusCode::UNAUTHORIZED => Ok(Renewal::Refused(refusal)),
                Err(refusal) => Err(Error::Status {
                    status: status.as_u16(),
                    message: Some(refusal.error),
                }),
            },
            Err(_) if !status.is_success() => Err(exchange::refusal_of(status, &bytes)),
            Err(error) => Err(Error::Decode(error)),
        }
    }

    /// Trades a one-time pairing code (made on a signed-in device) for a session of this
    /// device's own, and keeps it.
    ///
    /// # Errors
    ///
    /// [`Error::Refused`] for a wrong, used or expired code, or too many wrong codes; the
    /// transport's errors; [`Error::Store`] when the session couldn't be saved (it is in use
    /// all the same, and saved again on the next call).
    pub fn pair(&self, code: &str) -> Result<(), Error> {
        let paired = self.pair_reply(code)?;
        if let Some(Err(unreadable)) = paired.machine {
            return Err(Error::Decode(unreadable));
        }
        self.sessions.keep(paired.grant, paired.date)
    }

    /// Pairs as [`Client::pair`] does, for the machine a pairing link names: the answer must
    /// carry that machine's identity (its key, which has the id). An answer without one, or
    /// with another machine's, is refused and its session isn't kept, so neither stripping the
    /// field nor another server answering can stand in for the machine the link named.
    ///
    /// # Errors
    ///
    /// [`Error::WrongMachine`] for an answer from another machine, one without a machine, or
    /// one whose key doesn't have the id it names; otherwise as [`Client::pair`].
    pub fn pair_with_machine(
        &self,
        code: &str,
        machine: &MachineId,
    ) -> Result<MachineIdentity, Error> {
        let paired = self.pair_reply(code)?;
        let identity = paired
            .machine
            .and_then(Result::ok)
            .filter(|identity| identity.id() == machine)
            .ok_or(Error::WrongMachine)?;
        self.sessions.keep(paired.grant, paired.date)?;
        Ok(identity)
    }

    /// `POST /api/auth/pair` for `code`: what it granted, not kept yet.
    fn pair_reply(&self, code: &str) -> Result<Paired, Error> {
        if self.is_stopped() {
            return Err(Error::Stopped);
        }
        let request = PairRequest {
            code: code.trim().to_owned(),
            install: Some(self.install()),
            name: self.device_name.clone(),
            model: self.device_model.clone(),
            public_key: None,
        };
        let mut response = self
            .http
            .post_json(paths::PAIR, &exchange::encode(&request)?, None)?;
        let status = response.status();
        let date = exchange::server_date(&response);
        let bytes = exchange::body_bytes(&mut response)?;
        let read = serde_json::from_slice::<serde_json::Value>(&bytes).and_then(|mut answer| {
            // The machine is read on its own, so an answer whose key doesn't have its id is
            // told apart from one that doesn't read at all.
            let machine = answer
                .as_object_mut()
                .and_then(|fields| fields.remove("machine"))
                .filter(|machine| !machine.is_null());
            Ok((serde_json::from_value::<AuthReply>(answer)?, machine))
        });
        let (reply, machine) = match read {
            Ok(read) => read,
            Err(_) if !status.is_success() => return Err(exchange::refusal_of(status, &bytes)),
            Err(error) => return Err(Error::Decode(error)),
        };
        let grant = reply.into_grant().map_err(Error::Refused)?;
        let machine = machine.map(serde_json::from_value::<MachineIdentity>);
        Ok(Paired {
            grant,
            date,
            machine,
        })
    }

    /// The server's sign-in state, and whether it takes this device's session as a signed-in
    /// device's (`GET /api/auth/status`, signed when there is a session): the check that a
    /// pairing took.
    ///
    /// # Errors
    ///
    /// The transport's and decoding errors; [`Error::Status`] when the server fails.
    pub fn auth_status(&self) -> Result<AuthStatus, Error> {
        exchange::json(self.get(paths::AUTH_STATUS, "")?)
    }

    /// Makes a one-time pairing code for another device (this one must be signed in).
    ///
    /// # Errors
    ///
    /// [`Error::Status`] 401 when this device isn't signed in; the transport's errors.
    pub fn new_pairing_code(&self) -> Result<PairingCode, Error> {
        exchange::json(self.post(paths::PAIR_NEW, &serde_json::json!({}))?)
    }

    /// Asks the machine to sign a link statement for `request`'s account and relay
    /// (`POST /api/connect/link-statement`), for the account directory. The server answers only
    /// on a connection from loopback, so only a device on the same computer gets one.
    ///
    /// # Errors
    ///
    /// [`Error::Status`] 403 when the connection isn't from loopback, 401 when this device
    /// isn't signed in, 404 from a server without the route; the transport's errors.
    pub fn link_statement(&self, request: &LinkStatementRequest) -> Result<SignedLink, Error> {
        exchange::json(self.post(LINK_STATEMENT, request)?)
    }

    /// Polls a room's state (`GET /api/state`), with the held state's `ETag` and the looking
    /// header when the poll says so.
    ///
    /// # Errors
    ///
    /// [`Error::Status`] when the server refuses or fails (a 401 without `signIn` means this
    /// device isn't let in at all); the transport's and decoding errors.
    pub fn state(&self, poll: &StatePoll) -> Result<StateReply, Error> {
        let mut extra = Vec::new();
        if let Some(etag) = &poll.held {
            extra.push((headers::IF_NONE_MATCH, etag.as_str().to_owned()));
        }
        if poll.looking {
            extra.push((headers::LOOKING, poll.room.to_string()));
        }
        let query = RoomQuery::new(poll.room).to_query();
        let mut response =
            match self.send(|bearer| self.http.get(paths::STATE, &query, bearer, &extra)) {
                Ok(response) => response,
                Err(Error::SignInRequired) => return Ok(StateReply::SignInRequired),
                Err(error) => return Err(error),
            };
        let status = response.status();
        if status == StatusCode::NOT_MODIFIED {
            return Ok(StateReply::Unchanged);
        }
        if !status.is_success() {
            return Err(exchange::refusal(response));
        }
        let etag = response
            .headers()
            .get(headers::ETAG)
            .and_then(|v| v.to_str().ok())
            .map(ETag::new);
        let bytes = exchange::body_bytes(&mut response)?;
        let state = serde_json::from_slice(&bytes).map_err(Error::Decode)?;
        Ok(StateReply::Fresh {
            state: Box::new(state),
            etag,
        })
    }

    /// A page of a room's chat (`GET /api/history`).
    ///
    /// # Errors
    ///
    /// [`Error::SignInRequired`], [`Error::Status`], the transport's and decoding errors.
    pub fn history(&self, query: &HistoryQuery) -> Result<ChatPage, Error> {
        exchange::json(self.get(paths::HISTORY, &query.to_query())?)
    }

    /// Follows a room's live stream (`GET /api/live`): each line to `on_event` until `stop`
    /// says so or the server closes the stream. Call it again after a pause to keep following.
    /// A named event (an approval request's change) is left out; [`Client::follow_room`] hands
    /// those on too.
    ///
    /// `stop` is asked only when something arrives (an event, or the server's ping every 15 s),
    /// so after it turns true the call returns within 15 s, or within the live read timeout if
    /// the pings stop. Until then the stream holds its thread and its connection: a client that
    /// switches rooms often has one such stream winding down per switch for those seconds.
    ///
    /// # Errors
    ///
    /// [`Error::SignInRequired`], [`Error::Status`], [`Error::Protocol`] for a stream that isn't
    /// an event stream, and the transport's errors, including a stream quiet for longer than the
    /// live read timeout.
    pub fn follow_live(
        &self,
        room: RoomId,
        stop: impl FnMut() -> bool,
        on_event: impl FnMut(LiveEvent),
    ) -> Result<LiveEnd, Error> {
        let response = self.get_stream(paths::LIVE, &RoomQuery::new(room).to_query())?;
        live::follow(
            BufReader::new(response.into_body()),
            self.or_stopped(stop),
            on_event,
        )
    }

    /// Follows a room's live stream as [`Client::follow_live`] does, handing on its named
    /// events too: each line, and each approval request's change ([`RoomEvent::Approvals`]),
    /// to `on_event`.
    ///
    /// # Errors
    ///
    /// As [`Client::follow_live`].
    pub fn follow_room(
        &self,
        room: RoomId,
        stop: impl FnMut() -> bool,
        on_event: impl FnMut(RoomEvent),
    ) -> Result<LiveEnd, Error> {
        let response = self.get_stream(paths::LIVE, &RoomQuery::new(room).to_query())?;
        live::follow_room(
            BufReader::new(response.into_body()),
            self.or_stopped(stop),
            on_event,
        )
    }

    /// Says something in a room (`POST /api/talk`); the reply comes with the next polls.
    ///
    /// # Errors
    ///
    /// [`Error::SignInRequired`], [`Error::Status`], the transport's and decoding errors.
    pub fn talk(&self, request: &TalkRequest) -> Result<TalkReply, Error> {
        exchange::json(self.post(paths::TALK, request)?)
    }

    /// Stores a file for the chat (`POST /api/uploads?name=`), streamed from disk or memory as
    /// the raw body (`application/octet-stream`, with its `Content-Length`), and gives back the
    /// stored file to send with [`Client::talk`] ([`TalkRequest::with_files`]).
    ///
    /// `progress` is told after each chunk goes out; when it answers [`ControlFlow::Break`] the
    /// upload stops ([`Error::Cancelled`]). There is no overall timeout: only a pause longer
    /// than the upload timeout fails it ([`ClientConfig::with_upload_timeout`]).
    ///
    /// A token the server refuses although it looks good is renewed and the file sent again
    /// from its start (`progress` starts over from 0). That holds for a small file; for a large
    /// one the server answers its 401 without reading the body and may close the connection
    /// while the file still goes out, which reads as [`Error::Network`]. The token is renewed
    /// before it runs out, so that takes a session the server ended early; the next call
    /// renews it.
    ///
    /// # Errors
    ///
    /// [`Error::TooLarge`] for a file over the server's limit ([`emberhut_api::uploads::MAX_SIZE`])
    /// and [`Error::Read`] for one that couldn't be read, both before anything is sent
    /// (`Read` also when the file fails mid-way); [`Error::Cancelled`]; [`Error::Status`] 400
    /// with the server's reason; [`Error::SignInRequired`], the transport's and decoding errors.
    pub fn upload(
        &self,
        upload: &Upload,
        mut progress: impl FnMut(Progress) -> ControlFlow<()>,
    ) -> Result<ChatFile, Error> {
        let size = upload.size()?;
        let query = UploadQuery::new(upload.name()).to_query();
        let mut stopped = Stopped::No;
        let sent = self.send(|bearer| {
            let mut body = Reporting::new(upload.open(size)?, size, &mut progress, &mut stopped);
            self.http
                .post_raw(paths::UPLOADS, &query, size, &mut body, bearer)
        });
        let response = match sent {
            Err(Error::Network(network)) => {
                return Err(stopped
                    .into_error(Error::Read)
                    .unwrap_or(Error::Network(network)));
            }
            sent => sent?,
        };
        let reply: UploadReply = exchange::json(response)?;
        Ok(reply.file)
    }

    /// Streams a file the server serves behind the same sign-in, by its server path (as
    /// [`Client::file`] takes it), into `to`, and gives its size. Nothing is held in memory but
    /// a chunk, so it suits a file of any size up to the download limit
    /// ([`crate::MAX_DOWNLOAD`] unless [`ClientConfig::with_download_limit`] says less).
    ///
    /// `progress` is told after each chunk is written, with the size when the server gives it;
    /// when it answers [`ControlFlow::Break`] the download stops ([`Error::Cancelled`]). As with
    /// uploads there is no overall timeout, only the upload timeout's wait for each read. What
    /// `to` holds after an error is a part of the file: the caller drops it.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidPath`] for anything but a server path; [`Error::TooLarge`] for a file
    /// over the download limit (refused by its size, or stopped there); [`Error::Write`] when `to`
    /// fails; [`Error::Cancelled`]; [`Error::SignInRequired`], [`Error::Status`] (404 for a file
    /// that isn't there), the transport's errors.
    pub fn download(
        &self,
        path: &str,
        to: &mut (impl Write + ?Sized),
        mut progress: impl FnMut(Progress) -> ControlFlow<()>,
    ) -> Result<u64, Error> {
        let (path, query) = server_path(path)?;
        let response = self.get_file(path, query)?;
        if !response.status().is_success() {
            return Err(exchange::refusal(response));
        }
        let total = response
            .headers()
            .get("content-length")
            .and_then(|length| length.to_str().ok()?.parse::<u64>().ok());
        let limit = self.download_limit;
        if let Some(size) = total.filter(|&size| size > limit) {
            return Err(Error::TooLarge { size, limit });
        }
        let mut body = response.into_body();
        let mut buffer = vec![0; 64 << 10];
        let mut done = 0_u64;
        loop {
            let read = match body.read(&mut buffer) {
                Ok(0) => return Ok(done),
                Ok(read) => read,
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(TransportError::from(error).into()),
            };
            done += u64::try_from(read).unwrap_or(u64::MAX);
            if done > limit {
                return Err(Error::TooLarge { size: done, limit });
            }
            to.write_all(&buffer[..read]).map_err(Error::Write)?;
            if progress(Progress { done, total }).is_break() {
                return Err(Error::Cancelled);
            }
        }
    }

    /// Marks a room's messages seen (`POST /api/seen`).
    ///
    /// # Errors
    ///
    /// [`Error::SignInRequired`], [`Error::Status`], the transport's and decoding errors.
    pub fn seen(&self, room: RoomId) -> Result<(), Error> {
        let _: OkReply = exchange::json(self.post(paths::SEEN, &SeenRequest { room })?)?;
        Ok(())
    }

    /// Answers an ask (`POST /api/settings` with `taskDecide`). It can't be taken back.
    ///
    /// # Errors
    ///
    /// [`Error::Status`] 400 with the server's reason (no such task); [`Error::SignInRequired`],
    /// the transport's and decoding errors.
    pub fn answer(&self, answer: &AnswerRequest) -> Result<(), Error> {
        let _: OkReply = exchange::json(self.post(paths::SETTINGS, answer)?)?;
        Ok(())
    }

    /// A file the server serves behind the same sign-in, by its server path (`url` of a
    /// [`emberhut_api::chat::ChatFile`], e.g. `/uploads/<name>`, or a report's path): its bytes, up
    /// to the client's body limit (64 MiB).
    ///
    /// Only a path on this server is fetched, as [`is_server_path`] says. The path comes from
    /// server data (a message's file or reference), and the request carries this device's
    /// token, so anything else is refused before it is sent.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidPath`] for anything but a server path; [`Error::SignInRequired`],
    /// [`Error::Status`] (404 for a file that isn't there), the transport's errors.
    pub fn file(&self, path: &str) -> Result<Vec<u8>, Error> {
        let (path, query) = server_path(path)?;
        let mut response = self.get(path, query)?;
        if !response.status().is_success() {
            return Err(exchange::refusal(response));
        }
        exchange::body_bytes(&mut response)
    }

    /// The machines page (`GET /api/machines`), with each machine's last day.
    ///
    /// # Errors
    ///
    /// [`Error::SignInRequired`], [`Error::Status`], the transport's and decoding errors.
    pub fn machines(&self) -> Result<MachinesPage, Error> {
        exchange::json(self.get(paths::MACHINES, "")?)
    }

    /// The theme page (`GET /api/theme`).
    ///
    /// # Errors
    ///
    /// [`Error::SignInRequired`], [`Error::Status`], the transport's and decoding errors.
    pub fn theme(&self) -> Result<ThemePage, Error> {
        exchange::json(self.get(paths::THEME, "")?)
    }

    /// Changes the theme (`POST /api/theme`), or with `preview` works the page out without
    /// saving; the page as it is (or would be).
    ///
    /// # Errors
    ///
    /// [`Error::Status`] 400 with the server's reason for a bad value; [`Error::SignInRequired`],
    /// the transport's and decoding errors.
    pub fn set_theme(&self, change: &ThemeChange) -> Result<ThemePage, Error> {
        let reply: ThemeSetReply = exchange::json(self.post(paths::THEME, change)?)?;
        Ok(reply.page)
    }

    /// Sets a project's status (`POST /api/settings` with `projectSet`): pauses it until a
    /// day, archives it, or makes it active again.
    ///
    /// # Errors
    ///
    /// [`Error::Status`] 400 with the server's reason (no such project); [`Error::SignInRequired`],
    /// the transport's and decoding errors.
    pub fn set_project(&self, change: &ProjectSetRequest) -> Result<(), Error> {
        let _: OkReply = exchange::json(self.post(paths::SETTINGS, change)?)?;
        Ok(())
    }

    /// Signs a device out on the server (`POST /api/settings` with `authRevoke`): its refresh
    /// token stops working. Signing out this device leaves its session here until the next
    /// renewal is refused.
    ///
    /// # Errors
    ///
    /// [`Error::Status`] with the server's reason; [`Error::SignInRequired`], the transport's
    /// and decoding errors.
    pub fn revoke_device(&self, device: DeviceId) -> Result<(), Error> {
        let request = RevokeRequest {
            auth_revoke: device,
        };
        let _: OkReply = exchange::json(self.post(paths::SETTINGS, &request)?)?;
        Ok(())
    }

    /// Stops the answer the assistant is writing in `room` now (`POST /api/settings` with
    /// `interrupt`); its background work keeps running.
    ///
    /// # Errors
    ///
    /// [`Error::SignInRequired`], [`Error::Status`], the transport's and decoding errors.
    pub fn interrupt(&self, room: RoomId) -> Result<(), Error> {
        let _: OkReply = exchange::json(self.post(paths::SETTINGS, &InterruptRequest::new(room))?)?;
        Ok(())
    }

    /// Starts or stops recording a call on the Mac (`POST /api/calls/record`). The server
    /// answers a Mac it couldn't tell with status 200 too: the reply's `ok` and `error` say so.
    /// The server waits for the Mac before it answers, so this call waits up to 40 s rather
    /// than the request timeout.
    ///
    /// # Errors
    ///
    /// [`Error::SignInRequired`], [`Error::Status`], the transport's and decoding errors.
    pub fn record_call(&self, request: &RecordCallRequest) -> Result<RecordCallReply, Error> {
        let body = exchange::encode(request)?;
        let answer = self.send(|bearer| {
            self.http
                .post_json_within(paths::CALLS_RECORD, &body, RECORD_CALL_TIMEOUT, bearer)
        })?;
        exchange::json(answer)
    }

    /// Turns a recording into text (`POST /api/transcribe?name=&words=`, the recording as the
    /// raw body): what was said, trimmed.
    ///
    /// # Errors
    ///
    /// [`Error::TooLarge`] for a recording over [`emberhut_api::transcribe::MAX_SIZE`], never sent;
    /// [`Error::Status`] 413 or 502 with the server's reason; [`Error::SignInRequired`], the
    /// transport's and decoding errors.
    pub fn transcribe(&self, query: &TranscribeQuery, audio: &[u8]) -> Result<String, Error> {
        let size = u64::try_from(audio.len()).unwrap_or(u64::MAX);
        if size > transcribe::MAX_SIZE {
            return Err(Error::TooLarge {
                size,
                limit: transcribe::MAX_SIZE,
            });
        }
        let query = query.to_query();
        let response = self.send(|bearer| {
            let mut body = audio;
            self.http
                .post_raw(paths::TRANSCRIBE, &query, size, &mut body, bearer)
        })?;
        let reply: TranscribeReply = exchange::json(response)?;
        Ok(reply.text.trim().to_owned())
    }

    /// One task's page (`GET /api/tasks/{id}`): the task, done and archived ones too, with its
    /// history, subtasks, findings, links, runs, sessions and agents.
    ///
    /// # Errors
    ///
    /// [`Error::Status`] 404 for a task the server doesn't have; [`Error::SignInRequired`], the
    /// transport's and decoding errors.
    pub fn task(&self, id: TaskId) -> Result<TaskPage, Error> {
        exchange::json(self.get(&paths::task(id), "")?)
    }

    /// Tasks from the whole list (`GET /api/tasks`), done and archived ones too, newest first,
    /// narrowed by the server as `query` says.
    ///
    /// # Errors
    ///
    /// [`Error::SignInRequired`], [`Error::Status`], the transport's and decoding errors.
    pub fn find_tasks(&self, query: &TaskQuery) -> Result<FoundTasks, Error> {
        exchange::json(self.get(paths::TASKS, &query.to_query())?)
    }

    /// Changes task `id` (`PATCH /api/tasks/{id}`) with `change`, a body the contract doesn't
    /// name yet, spelled by the app that calls it (its title): the task as it is now.
    ///
    /// # Errors
    ///
    /// [`Error::Status`] 400 with the server's reason (a change it refuses), 404 for a task it
    /// doesn't have; [`Error::SignInRequired`], the transport's and decoding errors.
    pub fn change_task<B: Serialize>(&self, id: TaskId, change: &B) -> Result<Issue, Error> {
        exchange::json(self.patch(&paths::task(id), change)?)
    }

    /// The person writes on a task's page (`POST /api/tasks/{id}/message`): it goes to the
    /// task's worker, its session or the assistant in its room, as the answer says.
    ///
    /// # Errors
    ///
    /// [`Error::Status`] 400 with the server's reason (no such task, nothing to send);
    /// [`Error::SignInRequired`], the transport's and decoding errors.
    pub fn message_task(
        &self,
        id: TaskId,
        message: &TaskMessageRequest,
    ) -> Result<TaskMessageReply, Error> {
        exchange::json(self.post(&paths::task_message(id), message)?)
    }

    /// An action on tasks (`POST /api/settings`): done, put off, archived, back, a worker
    /// stopped, a task added or changed.
    ///
    /// # Errors
    ///
    /// [`Error::Status`] 400 with the server's reason (no such task); [`Error::SignInRequired`],
    /// the transport's and decoding errors.
    pub fn act_on_tasks(&self, action: &TaskAction) -> Result<(), Error> {
        let _: OkReply = exchange::json(self.post(paths::SETTINGS, action)?)?;
        Ok(())
    }
}

/// Whether `path` is a path on the server, the only kind [`Client::file`] fetches: it starts
/// with one `/`, and has no scheme, no backslash, no whitespace or control character, no
/// fragment, and nothing a URL would read as another host. An app that tells server paths
/// from other links asks this, so it takes for one exactly what the client fetches.
///
/// An `@` is fine (`/uploads/shot@2x.png`): the request URL is the base address followed by
/// `path`, so once `path` starts with a single `/` the host is settled, and an `@` after it
/// is part of the path, not a user before a host.
///
/// ```
/// use emberhut_client::is_server_path;
///
/// assert!(is_server_path("/uploads/shot@2x.png"));
/// assert!(is_server_path("/r/week.html?v=2"));
/// assert!(!is_server_path("//user@evil/x"));
/// assert!(!is_server_path("https://evil.example/x"));
/// assert!(!is_server_path("/uploads/a b.png"));
/// ```
#[must_use]
pub fn is_server_path(path: &str) -> bool {
    split_server_path(path).is_some()
}

/// `path` split into the path and its query, when it is a path on this server; else
/// [`Error::InvalidPath`].
pub(crate) fn server_path(path: &str) -> Result<(&str, &str), Error> {
    split_server_path(path).ok_or_else(|| Error::InvalidPath(path.to_owned()))
}

/// `path` split into the path and its query, when [`is_server_path`] says it is a path on this
/// server.
fn split_server_path(path: &str) -> Option<(&str, &str)> {
    let plain = path.starts_with('/')
        && !path.starts_with("//")
        && !path.contains(['\\', '#'])
        && !path.chars().any(|c| c.is_whitespace() || c.is_control());
    if !plain {
        return None;
    }
    // What a URL parser reads it as must be the same path, with no scheme or authority.
    let parsed = PathAndQuery::try_from(path).ok()?;
    let (bare, query) = path.split_once('?').unwrap_or((path, ""));
    (parsed.path() == bare).then_some((bare, query))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_paths_are_split_from_their_query() {
        assert_eq!(
            server_path("/uploads/a.png").unwrap(),
            ("/uploads/a.png", "")
        );
        assert_eq!(
            server_path("/r/week.html?v=2").unwrap(),
            ("/r/week.html", "v=2")
        );
    }

    #[test]
    fn an_at_sign_after_the_leading_slash_is_part_of_the_path() {
        for path in ["/uploads/a@2x.png", "/@evil.example/x", "/x@example.com"] {
            assert_eq!(server_path(path).unwrap(), (path, ""), "{path:?}");
        }
        assert_eq!(
            server_path("/uploads/a@2x.png?by=me@home").unwrap(),
            ("/uploads/a@2x.png", "by=me@home")
        );
    }

    #[test]
    fn the_public_check_is_the_one_file_uses() {
        for path in ["/uploads/a@2x.png", "/r/week.html?v=2"] {
            assert!(is_server_path(path), "{path:?}");
        }
        for path in ["//evil/x", "/a b", "/a#b", "uploads/a.png"] {
            assert!(!is_server_path(path), "{path:?}");
        }
    }

    #[test]
    fn anything_that_could_reach_another_host_is_refused() {
        for path in [
            "@evil.example/x",
            ".evil.net/x",
            "//evil/x",
            "https://evil/x",
            "/\\evil/x",
            "/a\\b@evil",
            "//user@evil/x",
            "uploads/a.png",
            "",
            "/a b",
            "/a\nb",
            "/a\tb",
            "/a\u{7f}b",
            "http://user@evil/x",
            "/a#b",
            "/a?x#y",
        ] {
            assert!(
                matches!(server_path(path), Err(Error::InvalidPath(ref p)) if p == path),
                "{path:?}"
            );
        }
    }
}
