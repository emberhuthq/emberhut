//! The sessions on the runners: a session's transcript and live stream, the person's message,
//! Stop and settings, a route the contract doesn't name yet, a new session of their own with its
//! folders and branches, and an external thread's latest steps. Each route is one method taking its request type from
//! [`emberhut_api::runner`].

use std::io::BufReader;
use std::time::Duration;

use emberhut_api::error::OkReply;
use emberhut_api::ids::SessionId;
use emberhut_api::paths::{self, SessionRoute};
use emberhut_api::runner::actions::{
    MoveSession, MovedSession, SendToSession, SentToSession, SessionSettings, Stopped,
};
use emberhut_api::runner::new_session::{
    BranchList, BranchQuery, FolderList, FolderQuery, OpenSession, OpenedSession,
};
use emberhut_api::runner::transcript::{SessionLive, StepsQuery, Transcript};
use emberhut_api::threads::{self, ActivityQuery, ThreadActivity};

use crate::live::{self, LiveEnd};
use crate::{Client, Error, exchange};

/// How long a message to a session or a new session waits for its answer: the server hands it
/// to the runner, which may first start the session's process (the reference client waits
/// 120 s too).
const RUNNER_TIMEOUT: Duration = Duration::from_secs(120);

impl Client {
    /// A session's transcript (`GET /api/runner/sessions/{id}/transcript`): its head and its
    /// newest steps, or those after `query.after`.
    ///
    /// # Errors
    ///
    /// [`Error::Status`] 404 for a worker's task the server doesn't have, 502 when the runner
    /// didn't answer; [`Error::SignInRequired`], the transport's and decoding errors.
    pub fn session_transcript(
        &self,
        id: &SessionId,
        query: &StepsQuery,
    ) -> Result<Transcript, Error> {
        exchange::json(self.get(
            &paths::session(id, SessionRoute::Transcript),
            &query.to_query(),
        )?)
    }

    /// Follows a session's live stream (`GET /api/runner/sessions/{id}/live?after=`) from the
    /// step after `query.after`: each event to `on_event`, until the stream ends or `stop` says
    /// so. `stop` is asked as each event or ping arrives (a ping every 15 s at most). An event
    /// this client can't read is skipped.
    ///
    /// # Errors
    ///
    /// [`Error::Status`] 502 when the runner didn't answer; [`Error::SignInRequired`],
    /// [`Error::Protocol`] for a stream that isn't an event stream, and the transport's errors,
    /// including a stream quiet for longer than the live read timeout.
    pub fn follow_session(
        &self,
        id: &SessionId,
        query: &StepsQuery,
        stop: impl FnMut() -> bool,
        on_event: impl FnMut(SessionLive),
    ) -> Result<LiveEnd, Error> {
        let response =
            self.get_stream(&paths::session(id, SessionRoute::Live), &query.to_query())?;
        live::follow_events(
            BufReader::new(response.into_body()),
            self.or_stopped(stop),
            on_event,
        )
    }

    /// The person's message to a session (`POST .../send`); sent while a turn runs, it waits
    /// behind it. Waits up to two minutes for the answer.
    ///
    /// # Errors
    ///
    /// [`Error::Status`] 400 `nothing to send`, 409 or 502 with the server's reason;
    /// [`Error::SignInRequired`], the transport's and decoding errors.
    pub fn send_to_session(
        &self,
        id: &SessionId,
        message: &SendToSession,
    ) -> Result<SentToSession, Error> {
        exchange::json(self.post_within(
            &paths::session(id, SessionRoute::Send),
            message,
            RUNNER_TIMEOUT,
        )?)
    }

    /// Stops a session (`POST .../stop`): the step it's on ends and nothing queued runs.
    ///
    /// # Errors
    ///
    /// [`Error::Status`] 502 with the server's reason; [`Error::SignInRequired`], the
    /// transport's and decoding errors.
    pub fn stop_session(&self, id: &SessionId) -> Result<Stopped, Error> {
        exchange::json(self.post(&paths::session(id, SessionRoute::Stop), &Empty {})?)
    }

    /// Stops a session's running turn (`POST .../interrupt`); what is queued runs next.
    ///
    /// # Errors
    ///
    /// As [`Client::stop_session`].
    pub fn interrupt_session(&self, id: &SessionId) -> Result<Stopped, Error> {
        exchange::json(self.post(&paths::session(id, SessionRoute::Interrupt), &Empty {})?)
    }

    /// Changes a session's model or effort for its next turn (`POST .../settings`).
    ///
    /// # Errors
    ///
    /// [`Error::Status`] 502 with the runner's reason (an effort it doesn't know);
    /// [`Error::SignInRequired`], the transport's and decoding errors.
    pub fn change_session(&self, id: &SessionId, settings: &SessionSettings) -> Result<(), Error> {
        let _: OkReply =
            exchange::json(self.post(&paths::session(id, SessionRoute::Settings), settings)?)?;
        Ok(())
    }

    /// Moves a session to another task (`POST .../task`), on a server that keeps plans.
    ///
    /// # Errors
    ///
    /// [`Error::Status`] 404 for a session or task the server doesn't know (or a server that
    /// doesn't keep plans), 400 for a task that is done or archived; [`Error::SignInRequired`],
    /// the transport's and decoding errors.
    pub fn move_session(&self, id: &SessionId, to: &MoveSession) -> Result<MovedSession, Error> {
        exchange::json(self.post(&paths::session(id, SessionRoute::Task), to)?)
    }

    /// Posts `body` to route `route` of session `id` (`POST .../{route}`) and reads the answer:
    /// a session route the contract doesn't name yet, spelled by the app that calls it.
    ///
    /// # Errors
    ///
    /// [`Error::Status`] with the server's reason when it refused; [`Error::SignInRequired`],
    /// the transport's and decoding errors.
    pub fn post_to_session<B: serde::Serialize, R: serde::de::DeserializeOwned>(
        &self,
        id: &SessionId,
        route: &str,
        body: &B,
    ) -> Result<R, Error> {
        exchange::json(self.post(&format!("{}/{id}/{route}", paths::RUNNER_SESSIONS), body)?)
    }

    /// Opens a session of the person's own with their first message
    /// (`POST /api/runner/sessions`). Waits up to two minutes for the answer.
    ///
    /// # Errors
    ///
    /// [`Error::Status`] 409 with the server's reason when it couldn't be opened;
    /// [`Error::SignInRequired`], the transport's and decoding errors.
    pub fn open_session(&self, session: &OpenSession) -> Result<OpenedSession, Error> {
        exchange::json(self.post_within(paths::RUNNER_SESSIONS, session, RUNNER_TIMEOUT)?)
    }

    /// A runner machine's folders (`GET /api/runner/folders`): where to start, or the folders
    /// inside one.
    ///
    /// # Errors
    ///
    /// [`Error::Status`] 400 for a machine that isn't a runner, 502 when its runner didn't
    /// answer; [`Error::SignInRequired`], the transport's and decoding errors.
    pub fn runner_folders(&self, query: &FolderQuery) -> Result<FolderList, Error> {
        exchange::json(self.get(paths::RUNNER_FOLDERS, &query.to_query())?)
    }

    /// A repository's branches on a runner machine (`GET /api/runner/branches`).
    ///
    /// # Errors
    ///
    /// As [`Client::runner_folders`].
    pub fn runner_branches(&self, query: &BranchQuery) -> Result<BranchList, Error> {
        exchange::json(self.get(paths::RUNNER_BRANCHES, &query.to_query())?)
    }

    /// An external thread's latest steps ([`threads::ACTIVITY`]).
    ///
    /// # Errors
    ///
    /// [`Error::Status`] 400 for a machine without the coding-thread app, 502 when it didn't answer;
    /// [`Error::SignInRequired`], the transport's and decoding errors.
    pub fn t3_activity(&self, query: &ActivityQuery) -> Result<ThreadActivity, Error> {
        exchange::json(self.get(threads::ACTIVITY, &query.to_query())?)
    }
}

/// An empty JSON object, the body of a `POST` that carries nothing.
#[derive(serde::Serialize)]
struct Empty {}
