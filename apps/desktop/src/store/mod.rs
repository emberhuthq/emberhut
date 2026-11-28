//! The app's data on this device, as one GPUI entity the views observe: the server's state for
//! the open room, each room's chat as held, the live step, the machines, the theme, and what
//! the user did that the server hasn't seen yet.
//!
//! Each submodule hides one decision; the logic with no GPUI in it ([`chats`], [`rooms`],
//! [`needs`], [`tasks`], [`answers`], [`attachments`], `kept`, `pasted`, [`in_flight`],
//! [`fetched`], [`retention`], [`status`], [`reading`], [`presence`]) is tested on its own.

pub(crate) mod answers;
pub(crate) mod approvals;
pub(crate) mod ask_text;
pub(crate) mod asks;
pub(crate) mod attachments;
pub(crate) mod browser;
pub(crate) mod calendar;
mod catch_up;
pub(crate) mod chats;
mod column;
pub(crate) mod composer;
#[cfg(test)]
mod composer_tests;
pub(crate) mod credentials;
pub(crate) mod dark;
pub(crate) mod decision;
pub(crate) mod devices;
pub(crate) mod diary;
mod disk;
mod fallback;
mod fetched;
pub(crate) mod files;
mod flaky;
pub(crate) mod gate;
pub(crate) mod goals;
pub(crate) mod huts_add;
pub(crate) mod images;
mod in_flight;
mod kept;
mod live;
pub(crate) mod lookup;
mod machines;
mod metrics;
pub(crate) mod need_card;
pub(crate) mod needs;
pub(crate) mod needs_view;
mod on_disk;
pub(crate) mod party;
mod pasted;
pub(crate) mod places;
#[cfg(test)]
mod places_tests;
mod poll;
pub(crate) mod presence;
pub(crate) mod projects;
pub(crate) mod put_away;
mod read_marks;
pub(crate) mod reading;
mod reconnect;
pub(crate) mod replying;
mod reports;
mod retention;
pub(crate) mod room_actions;
pub(crate) mod room_line;
pub(crate) mod rooms;
pub(crate) mod scheduled;
mod sending;
pub(crate) mod sessions;
pub(crate) mod settings;
pub(crate) mod setup;
mod sign_in;
pub(crate) mod spotlight;
pub(crate) mod status;
pub(crate) mod stills;
mod talk;
pub(crate) mod tasks;
#[cfg(test)]
mod tests;
mod theme;
pub(crate) mod updates;
mod uploads;
mod view_file;
pub(crate) mod working;
pub(crate) mod x_posts;

use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::AppContext as _;
use gpui_kit::{AsyncApp, Context, Task, WeakEntity};

pub(crate) use self::disk::Kept;
pub(crate) use self::fetched::Fetched;
pub(crate) use self::files::Opening;
pub(crate) use self::machines::MachinesFeed;
pub(crate) use self::metrics::Metrics;
pub(crate) use self::on_disk::DiskCache;
pub(crate) use self::reconnect::{Connected, Connector, Reconnect};
pub(crate) use self::reports::ReportPage;
pub(crate) use self::room_line::RoomLine;
pub(crate) use self::talk::PendingMessage;
pub(crate) use self::theme::ThemeState;
pub(crate) use self::view_file::{AgentsPlace, ViewFile};

use self::answers::Answers;
use self::attachments::Attachments;
use self::chats::Chats;
use self::fetched::FetchCache;
use self::images::ImageCache;
use self::in_flight::InFlight;
use self::live::LiveFeed;
use self::needs::Needs;
use self::poll::{Kicked, Kicker};
use self::presence::WindowState;
use self::reading::{LastInput, SeenMarks};
use self::retention::Limits;
use self::rooms::Switcher;
use self::status::StatusLine;
use self::tasks::TaskWork;
use emberhut_api::headers::ETag;
use emberhut_api::ids::{ApprovalId, GoalId, ProjectId, RoomId, TaskId};
use emberhut_api::live::Live;
use emberhut_api::state::State;

use crate::backend::Backend;
use crate::config::{AppFiles, ServerUrl};

/// A place a main window shows: a room's chat, or a page in the column beside it
/// (`ui::column`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Page {
    /// A room's chat (not a page of the column: the chat's room).
    Room(RoomId),
    /// The asks waiting on the user.
    Needs,
    /// The open tasks.
    Tasks,
    /// The machines.
    Machines,
    /// Settings.
    Settings,
    /// A task's page.
    Task(TaskId),
    /// A project's page.
    Project(ProjectId),
    /// A goal's page.
    Goal(GoalId),
    /// A person the assistant knows, by the key the store gave them.
    Person(spotlight::PersonKey),
    /// The weekly report.
    Report,
    /// The sessions, every room's.
    Sessions,
    /// A session's page: a session, a helper agent's steps, an external thread or a new session.
    Session(sessions::places::SessionKey),
    /// A place the assistant works, or a connection it reads through.
    Environment(working::environments::EnvKey),
    /// The background jobs on the machines.
    Jobs,
    /// A background job's page.
    Job(working::jobs::JobKey),
    /// Everything the assistant has scheduled.
    Scheduled,
    /// One of Settings' pages, over Settings (Back returns to it).
    SettingsPane(settings::SettingsPane),
    /// The pending approval requests: every agent's, or one agent's alone.
    Approvals(Option<approvals::AgentKey>),
    /// An approval request's page, with its exact payload.
    Approval(ApprovalId),
}

/// A report open in the panel beside the page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct OpenReport {
    /// Its name, as the panel's title.
    pub(crate) name: String,
    /// Its path on the server.
    pub(crate) path: String,
}

/// The store.
pub(crate) struct Store {
    backend: Arc<dyn Backend>,
    files: Option<AppFiles>,
    state: Option<State>,
    etag: Option<ETag>,
    room: RoomId,
    chats: Chats,
    /// The assistant's step from the live stream, newer than the state's.
    live: Option<Live>,
    /// Why the last poll failed, worded for the user.
    problem: Option<String>,
    needs_sign_in: bool,
    /// Show the sign-in screen whatever the server says (screenshots).
    preview_sign_in: bool,
    machines: MachinesFeed,
    pending: Option<PendingMessage>,
    /// The files attached in the composer.
    attachments: Attachments,
    /// The chat files kept and being opened in their default app.
    kept: files::KeptFiles,
    images: ImageCache,
    /// How many pictures have come since the store began: the views compare it to know when
    /// to look for rows to measure again.
    pictures_come: u64,
    reports: FetchCache<ReportPage>,
    answers: Answers,
    /// The answers on their way to the server.
    in_flight: InFlight,
    report: Option<OpenReport>,
    /// Whether Later is open on Needs you: it starts folded and stays as the user leaves it for
    /// the session.
    later_open: bool,
    metrics: Metrics,
    theme: ThemeState,
    /// Wakes the poll; the poll's end of it waits here until the poll starts.
    kicker: Kicker,
    kicked: Option<Kicked>,
    /// The state poll, once started: dropping it stops it.
    poll: Option<Task<()>>,
    /// How the store moves to another server while it runs.
    reconnect: Reconnect,
    live_feed: LiveFeed,
    /// What the platform says of the window: whether it is the active one (so a room on screen
    /// is being read) and whether it is shown (out of sight, nothing is polled).
    window: WindowState,
    /// When the user last used the window, so a window left in front of no one isn't read.
    last_input: LastInput,
    /// What the server was told was seen.
    seen_marks: SeenMarks,
    /// What each main window's column holds.
    columns: column::Columns,
    /// What is kept on disk to show a room at once (none in the tests).
    disk: Option<Arc<DiskCache>>,
    /// How the window was left, kept across launches.
    view: ViewFile,
    /// The writes of the view to its file.
    view_writes: crate::json_file::Writes,
    /// The sidebar's width as last kept (or read), to tell whether a drag changed it.
    sidebar_kept: Option<f32>,
    /// How far the server lets this device in.
    door: gate::Door,
    /// How the polls have been failing, for the offline line.
    failing: gate::Failing,
    /// The last action the server refused, worded for the user.
    action_problem: Option<String>,
    /// The room whose pause day is being picked.
    pausing: Option<room_actions::Pausing>,
    /// The folds open for the session: the sidebar's Paused, Settings' archived rooms.
    folds: view_file::Folds,
    /// A pairing code made here for another device.
    new_code: Option<devices::NewCode>,
    /// The device whose sign-out waits to be confirmed.
    sign_out_asked: Option<emberhut_api::ids::DeviceId>,
    /// The flaky-network test mode: the poll fails 6 s of every 20.
    flaky: flaky::Mode,
    /// The composer's drafts, reply and failed actions.
    composer: composer::Composer,
    /// The messages sent so far, numbering each send.
    sends: u64,
    /// What the user put away over the composer (ask cards, Main's line), kept across launches.
    put_away: put_away::PutAway,
    /// The numbers of the writes of what is put away, so the newest is the one kept.
    put_away_writes: crate::json_file::Writes,
    /// The message quoted above the composer, which the next one replies to.
    replying: Option<emberhut_api::chat::Quote>,
    /// Settings' pages and what they show.
    settings: settings::SettingsData,
    /// The Tasks list and the pages of the tasks the windows' columns hold.
    tasks: TaskWork,
    /// How the user looks at Needs you, kept on this device.
    needs_view: needs_view::NeedsView,
    /// The spotlight's field and what it found.
    spotlight: spotlight::Spotlight,
    /// What projects' and people's pages fetched.
    places: places::Places,
    /// The keys people's pages go by.
    person_keys: spotlight::PersonKeys,
    /// The sessions: the Sessions screen's filter, and the ones followed live.
    sessions: sessions::SessionWork,
    /// Where the assistant works and what it uses: environments, usage, jobs.
    working: working::WorkingData,
    /// Party: the list, a new party's form and the party open.
    party: party::PartyData,
    /// The Dark room.
    dark: dark::DarkData,
    /// The assistant's browser on this device.
    browser: browser::BrowserWork,
    /// What the assistant has scheduled.
    scheduled: scheduled::ScheduledFeed,
    /// The diary and the quick note.
    diary: diary::DiaryData,
    /// The calendar's ranges.
    calendar: calendar::CalendarFeed,
    /// The app's own updates.
    updates: updates::Updates,
    /// The approval requests and the user's answers on them.
    approvals: approvals::ApprovalsFeed,
    /// The user's last Try again of the server, while on the fallback server.
    primary_retry: fallback::Retry,
    /// What the ember thinks with, and the picker that chooses it.
    credentials: credentials::Credentials,
    /// The first-run setup.
    setup: setup::Setup,
    /// "Add a hut": another computer joining as a hut.
    adding_hut: huts_add::AddingHut,
}

impl Store {
    /// The store for `backend`, opening in `room`, with its loops started: the poll, the live
    /// stream and the machines. `home` is Emberhut on this computer, for the setup, and
    /// `reconnect` how the store moves to another server ([`Store::switch_server`]).
    pub(crate) fn new(
        backend: Arc<dyn Backend>,
        files: Option<AppFiles>,
        theme: ThemeState,
        room: RoomId,
        home: Rc<dyn setup::LocalHome>,
        reconnect: Reconnect,
        cx: &mut Context<'_, Self>,
    ) -> Self {
        let mut store = Self::idle(backend, files, theme, room);
        store.setup.set_home(home);
        store.reconnect = reconnect;
        store.start_poll(cx);
        store.start_live(cx);
        store.start_machines(cx);
        store.kept = files::KeptFiles::in_cache();
        store.sweep_kept_files(cx);
        store.start_switches(cx);
        store.start_working(cx);
        store.start_scheduled(cx);
        store.start_diary(cx);
        store.start_updates(cx);
        store.start_approvals(cx);
        store
    }

    /// The store with no loop running yet (the tests start the ones they need).
    fn idle(
        backend: Arc<dyn Backend>,
        files: Option<AppFiles>,
        theme: ThemeState,
        room: RoomId,
    ) -> Self {
        let (kicker, waiting) = poll::kicker();
        let live_feed = LiveFeed::new(Arc::clone(&backend));
        let put_away = Self::load_put_away(files.as_ref());
        let tasks = TaskWork::new(files.as_ref());
        let needs_view = needs_view::NeedsView::load(files.as_ref());
        let sessions = sessions::SessionWork::new(files.as_ref());
        let working = working::WorkingData::default();
        let browser = browser::BrowserWork::new(files.as_ref());
        let diary = diary::DiaryData::load(files.as_ref());
        let setup = setup::Setup::load(files.as_ref());
        Self {
            backend,
            files,
            state: None,
            etag: None,
            room,
            chats: Chats::default(),
            live: None,
            problem: None,
            needs_sign_in: false,
            preview_sign_in: false,
            machines: MachinesFeed::default(),
            pending: None,
            attachments: Attachments::default(),
            kept: files::KeptFiles::in_folder(None),
            images: ImageCache::new(room, Limits::PICTURES),
            pictures_come: 0,
            reports: FetchCache::default(),
            answers: Answers::default(),
            in_flight: InFlight::default(),
            report: None,
            later_open: false,
            metrics: Metrics::default(),
            theme,
            kicker,
            kicked: Some(waiting),
            poll: None,
            reconnect: Reconnect::none(),
            live_feed,
            window: WindowState::default(),
            last_input: LastInput::default(),
            seen_marks: SeenMarks::default(),
            columns: column::Columns::default(),
            disk: None,
            view: ViewFile::default(),
            view_writes: crate::json_file::Writes::default(),
            sidebar_kept: None,
            door: gate::Door::default(),
            failing: gate::Failing::default(),
            action_problem: None,
            pausing: None,
            folds: view_file::Folds::default(),
            new_code: None,
            sign_out_asked: None,
            flaky: flaky::Mode::Off,
            composer: composer::Composer::default(),
            sends: 0,
            put_away,
            put_away_writes: crate::json_file::Writes::default(),
            replying: None,
            settings: settings::SettingsData::default(),
            tasks,
            needs_view,
            spotlight: spotlight::Spotlight::default(),
            places: places::Places::default(),
            person_keys: spotlight::PersonKeys::default(),
            sessions,
            working,
            party: party::PartyData::default(),
            dark: dark::DarkData::default(),
            browser,
            scheduled: scheduled::ScheduledFeed::default(),
            diary,
            calendar: calendar::CalendarFeed::default(),
            updates: updates::Updates::default(),
            approvals: approvals::ApprovalsFeed::default(),
            primary_retry: fallback::Retry::default(),
            credentials: credentials::Credentials::default(),
            setup,
            adding_hut: huts_add::AddingHut::default(),
        }
    }

    /// A store over `backend` with nothing running, for the views' tests.
    #[cfg(test)]
    pub(crate) fn for_tests(backend: Arc<dyn Backend>) -> Self {
        Self::idle(backend, None, ThemeState::load(None), RoomId::MAIN)
    }

    /// A store over `backend` with nothing running, its files in `files`, `home` as Emberhut
    /// on this computer and `reconnect` its moves to another server, for the setup's tests.
    #[cfg(test)]
    pub(crate) fn for_setup_tests(
        backend: Arc<dyn Backend>,
        files: Option<AppFiles>,
        home: Rc<dyn setup::LocalHome>,
        reconnect: Reconnect,
    ) -> Self {
        let mut store = Self::idle(backend, files, ThemeState::load(None), RoomId::MAIN);
        store.setup.set_home(home);
        store.reconnect = reconnect;
        store
    }

    /// The server ends the session, as the state poll learns it (for the views' tests).
    #[cfg(test)]
    pub(crate) fn end_session(&mut self, cx: &mut Context<'_, Self>) {
        let failed = self.apply(
            self.room,
            Err(crate::backend::Error::SignInRequired),
            std::time::Instant::now(),
            cx,
        );
        assert!(failed, "a poll that needs sign-in failed");
    }

    /// The server stops letting this device in, as the state poll learns it (for the views'
    /// tests).
    #[cfg(test)]
    pub(crate) fn refuse_for_tests(&mut self, cx: &mut Context<'_, Self>) {
        let refused = Err(crate::backend::Error::Refused {
            status: Some(401),
            message: None,
        });
        let failed = self.apply(self.room, refused, std::time::Instant::now(), cx);
        assert!(failed, "a refused poll failed");
    }

    /// Starts the state poll (once).
    fn start_poll(&mut self, cx: &mut Context<'_, Self>) {
        if let Some(kicked) = self.kicked.take() {
            self.poll = Some(poll::start(kicked, cx));
        }
    }

    /// Starts following the open room's live stream.
    fn start_live(&mut self, cx: &mut Context<'_, Self>) {
        self.live_feed.start(cx);
        self.live_feed.follow(self.room);
    }

    /// The server's address.
    #[must_use]
    pub(crate) fn server(&self) -> &ServerUrl {
        self.backend.server()
    }

    /// Whether this device has a session of its own.
    #[must_use]
    pub(crate) fn is_signed_in(&self) -> bool {
        self.backend.is_signed_in()
    }

    /// The last state, if one came.
    #[must_use]
    pub(crate) fn state(&self) -> Option<&State> {
        self.state.as_ref()
    }

    /// The open room, the chat's in every main window.
    #[must_use]
    pub(crate) fn room(&self) -> RoomId {
        self.room
    }

    /// The chats held.
    #[must_use]
    pub(crate) fn chats(&self) -> &Chats {
        &self.chats
    }

    /// Whether the state on hand is the open room's.
    #[must_use]
    pub(crate) fn is_state_here(&self) -> bool {
        self.state
            .as_ref()
            .is_some_and(|state| state.room == self.room)
    }

    /// The assistant's current step in the open room: the live stream's, else the state's.
    #[must_use]
    pub(crate) fn live(&self) -> Option<&Live> {
        self.live
            .as_ref()
            .or_else(|| self.state.as_ref()?.live.as_ref())
    }

    /// Why the last poll failed, if it did.
    #[must_use]
    pub(crate) fn problem(&self) -> Option<&str> {
        self.problem.as_deref()
    }

    /// Whether to show the sign-in screen.
    #[must_use]
    pub(crate) fn shows_sign_in(&self) -> bool {
        self.needs_sign_in || self.preview_sign_in
    }

    /// This device signed in: the sign-in screen goes.
    pub(super) fn signed_in(&mut self) {
        self.needs_sign_in = false;
    }

    /// Turns the flaky-network test mode on: the state poll fails 6 s of every 20.
    #[cfg(debug_assertions)]
    pub(crate) fn make_flaky(&mut self) {
        self.flaky = flaky::Mode::Flaky;
    }

    /// Shows the sign-in screen whatever the server says (screenshots).
    pub(crate) fn preview_sign_in(&mut self) {
        self.preview_sign_in = true;
    }

    /// The message sent from here that the chat doesn't have yet.
    #[must_use]
    pub(crate) fn pending(&self) -> Option<&PendingMessage> {
        self.pending.as_ref()
    }

    /// The answers given here.
    #[must_use]
    pub(crate) fn answers(&self) -> &Answers {
        &self.answers
    }

    /// The report open beside the page.
    #[must_use]
    pub(crate) fn open_report(&self) -> Option<&OpenReport> {
        self.report.as_ref()
    }

    /// Opens a report beside the page (`None` closes it).
    pub(crate) fn show_report(&mut self, report: Option<OpenReport>, cx: &mut Context<'_, Self>) {
        if let Some(report) = &report {
            if !ServerUrl::is_path(&report.path) {
                log::warn!(
                    "not opening a report that isn't on the server: {}",
                    report.path
                );
                return;
            }
            self.fetch_report(report.path.clone(), cx);
        }
        self.report = report;
        cx.notify();
    }

    /// Whether Later is open on Needs you.
    #[must_use]
    pub(crate) fn is_later_open(&self) -> bool {
        self.later_open
    }

    /// Opens Later on Needs you, or folds it again.
    pub(crate) fn toggle_later(&mut self, cx: &mut Context<'_, Self>) {
        self.later_open = !self.later_open;
        cx.notify();
    }

    /// The numbers behind Settings' Server line and the benchmarks.
    #[must_use]
    pub(crate) fn metrics(&self) -> &Metrics {
        &self.metrics
    }

    /// The switcher over the state's rooms, the agents' own rooms left to the sidebar's
    /// Agents.
    #[must_use]
    pub(crate) fn switcher(&self) -> Switcher<'_> {
        let agents: Vec<RoomId> = self
            .agent_rooms()
            .iter()
            .map(|own| own.agent.room)
            .collect();
        Switcher::new(
            self.state
                .as_ref()
                .map_or(&[], |state| state.rooms.as_slice()),
        )
        .without(&agents)
    }

    /// What waits on the user, over the last state, as it stands today on this device's clock.
    #[must_use]
    pub(crate) fn needs(&self) -> Option<Needs<'_>> {
        let today = jiff::ZonedDateTime::now().date();
        Some(Needs::new(self.state.as_ref()?, self.answers.gone(), today))
    }

    /// The assistant's status line under its name.
    #[must_use]
    pub(crate) fn status(&self) -> StatusLine {
        let now = jiff::ZonedDateTime::now();
        status::line(self.standing(&now))
    }

    /// Whether the assistant is at work now (see [`status::Standing::is_working`]).
    #[must_use]
    pub(crate) fn is_working(&self) -> bool {
        self.standing(&jiff::ZonedDateTime::now()).is_working()
    }

    /// Where the assistant stands at `now`, for its status line.
    fn standing<'a>(&'a self, now: &'a jiff::ZonedDateTime) -> status::Standing<'a> {
        status::Standing {
            state: self.state.as_ref(),
            here: self.is_state_here(),
            offline: self.outage(),
            live: self.live.as_ref(),
            now: now.timestamp(),
            zone: now.time_zone(),
        }
    }

    /// Loads the page of the open room's chat before what is held.
    pub(crate) fn load_earlier(&mut self, cx: &mut Context<'_, Self>) {
        let room = self.room;
        let Some(before) = self.chats.begin_earlier(room) else {
            return;
        };
        let backend = Arc::clone(&self.backend);
        cx.spawn(async move |this, cx| {
            let page = cx
                .background_spawn(async move { backend.history(room, before) })
                .await;
            update_or_log(&this, cx, |store, cx| {
                let page = page
                    .inspect_err(|error| {
                        log::warn!("couldn't load room {room}'s earlier chat: {error}");
                    })
                    .ok()
                    .map(|page| page.chat);
                store.chats.finish_earlier(room, before, page);
                store.chat_changed(room, cx);
            });
        })
        .detach();
        cx.notify();
    }
}

/// Runs `update` on the store if it still exists; once the app is closing it may not, which is
/// fine for the background work that calls this.
fn update_or_log<R>(
    this: &WeakEntity<Store>,
    cx: &mut AsyncApp,
    update: impl FnOnce(&mut Store, &mut Context<'_, Store>) -> R,
) -> Option<R> {
    this.update(cx, update)
        .inspect_err(|error| log::debug!("the store is gone: {error}"))
        .ok()
}
