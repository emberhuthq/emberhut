//! The first-run setup, "Set up your ember": five steps in the ember's voice, the first two
//! needed (where it lives, what it thinks with) and the other three optional (a phone, the
//! user's world, sharing), then "Your ember is lit."
//!
//! The setup shows on a fresh install (no server named and no session), and when the ember
//! this device is signed in to isn't lit (`GET /api/setup`'s `provider` is `missing` or
//! `off`). Once it shows it stays until it is finished or put off, and that is kept in
//! `setup.json`, so it never comes back: an ember that goes out later gets a quiet line over
//! the chat instead.
//!
//! The ember beside the steps glows a little more with each one done ([`Setup::glow`]). Where
//! the setup begins is one choice ([`DIRECTION`]); Emberhut on this computer is a port
//! ([`LocalHome`]) the app hands the store at launch.
//!
//! "Here, on this computer" starts Emberhut here; when it answers at another address than the
//! one the app talks to (its port was taken), the app moves there as it runs
//! ([`Store::switch_server`]). Then it is kept lit while the window is closed and from login
//! ([`KEEP_LIT`], which step 1's Change sheet turns back), and the app pairs with a code it
//! makes, through the same sign-in as a typed code. At a later launch, Emberhut here is started
//! again when the app talks to it and it is out ([`Store::relight_home`]).

mod home;
mod plans;
mod relight;
#[cfg(test)]
pub(crate) mod tests;

use std::collections::BTreeSet;
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::AppContext as _;
use gpui_kit::Context;
use serde::{Deserialize, Serialize};

pub(crate) use self::home::{
    CANT_HERE, DevHome, HutHome, LocalHome, NoLocalHome, Relit, on_this_computer,
};
pub(crate) use self::plans::Plans;
pub(crate) use self::relight::Relighting;

use super::{Store, update_or_log};
use crate::backend::Error;
use crate::config::{AppFiles, ServerUrl};
use crate::hut::KeepLit;
use crate::json_file::{self, Writes};
use crate::store::credentials;
use emberhut_api::ids::DeviceId;
use emberhut_api::setup::{SetupItemId, SetupPage, SetupState};

/// How the setup begins.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[expect(
    clippy::enum_variant_names,
    reason = "each direction is named for what comes first"
)]
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the directions not built yet stay named, so taking one is a one-line change"
    )
)]
pub(crate) enum SetupDirection {
    /// Where the ember lives comes first, with every step in sight under it.
    HomeFirst,
    /// An account's sign-in comes first. Not built: it begins as [`Self::HomeFirst`] does.
    AccountFirst,
    /// A sample day comes first. Not built: it begins as [`Self::HomeFirst`] does.
    ShowFirst,
}

/// The direction this build's setup takes.
pub(crate) const DIRECTION: SetupDirection = SetupDirection::HomeFirst;

/// The step the setup opens on, going `direction`.
#[must_use]
pub(crate) fn first_screen(direction: SetupDirection) -> SetupStep {
    match direction {
        SetupDirection::HomeFirst | SetupDirection::AccountFirst | SetupDirection::ShowFirst => {
            SetupStep::Home
        }
    }
}

/// A step of the setup.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum SetupStep {
    /// Give your ember a home: the computer it lives on.
    Home,
    /// Light it: the model it thinks with.
    Light,
    /// Let it reach you: a phone paired with a QR code.
    Reach,
    /// Show it your world: Calendar, Gmail, Slack and Chrome.
    World,
    /// Share your ember: someone else talks to it in Slack.
    Share,
}

impl SetupStep {
    /// Every step, in order.
    pub(crate) const ALL: [Self; 5] = [
        Self::Home,
        Self::Light,
        Self::Reach,
        Self::World,
        Self::Share,
    ];

    /// Its place in the order, from 0.
    #[must_use]
    pub(crate) fn index(self) -> usize {
        match self {
            Self::Home => 0,
            Self::Light => 1,
            Self::Reach => 2,
            Self::World => 3,
            Self::Share => 4,
        }
    }

    /// The step after it, if one is.
    #[must_use]
    pub(crate) fn next(self) -> Option<Self> {
        Self::ALL.get(self.index() + 1).copied()
    }

    /// Its name, in the ember's voice.
    #[must_use]
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Home => "Give your ember a home",
            Self::Light => "Light it",
            Self::Reach => "Let it reach you",
            Self::World => "Show it your world",
            Self::Share => "Share your ember",
        }
    }

    /// What actually happens, in one plain line.
    #[must_use]
    pub(crate) fn line(self) -> &'static str {
        match self {
            Self::Home => "Pick the computer it lives on. It runs there in the background.",
            Self::Light => "Choose the model your ember thinks with.",
            Self::Reach => "Pair your phone with one QR code.",
            Self::World => "Connect Slack, Gmail, Calendar and Chrome, one at a time.",
            Self::Share => "Let someone else talk to it in Slack.",
        }
    }

    /// What it unlocks, for the rail beside the step.
    #[must_use]
    pub(crate) fn unlocks(self) -> &'static str {
        match self {
            Self::Home => "Works while you're away",
            Self::Light => "It can think",
            Self::Reach => "Reaches your phone",
            Self::World => "Sees your mail and calendar",
            Self::Share => "Others can talk to it",
        }
    }

    /// Whether the ember gets going without it.
    #[must_use]
    pub(crate) fn optional(self) -> bool {
        matches!(self, Self::Reach | Self::World | Self::Share)
    }

    /// The headline while it is open: the words before the ember's word, and its word.
    #[must_use]
    pub(crate) fn headline(self) -> (&'static str, &'static str) {
        match self {
            Self::Home => ("Set up your ", "ember"),
            Self::Light => ("Now, ", "light it"),
            Self::Reach => ("Let it ", "reach you"),
            Self::World => ("Show it your ", "world"),
            Self::Share => ("Share your ", "ember"),
        }
    }
}

/// How a step stands.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) enum StepState {
    /// Not reached yet, or open now.
    #[default]
    Ahead,
    /// Done: its line says what came of it.
    Done(String),
    /// Put off with Later.
    Skipped,
}

/// Where the user wants the ember to live.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum HomeChoice {
    /// Here, on this computer.
    #[default]
    Here,
    /// On another computer already set up.
    Elsewhere,
}

/// How giving the ember a home here goes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) enum Here {
    /// Not asked yet.
    #[default]
    Idle,
    /// Emberhut is being started here.
    Starting,
    /// It runs here; this app is being paired with it.
    Pairing,
    /// It runs here, and the user types its pairing code.
    TypeCode,
    /// It couldn't: why, in words. Another computer is offered beside it.
    Failed(String),
}

/// Whether the ember thinks, as the setup checklist says.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum LitCheck {
    /// Not asked yet.
    #[default]
    Unknown,
    /// Being asked.
    Asking,
    /// It thinks.
    Lit,
    /// It doesn't: nothing it can think with is set up.
    Unlit,
}

/// How the setup ended, kept in `setup.json`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Ending {
    /// Done to the end.
    Finished,
    /// Put off before the ember was lit.
    Skipped,
}

/// `setup.json`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
struct SetupFile {
    /// How the setup ended, once it did.
    ended: Option<Ending>,
    /// The person asked to hear when renting a home in the cloud is ready (kept on their
    /// server too, once there is one).
    #[serde(rename = "cloudWaitlist", skip_serializing_if = "std::ops::Not::not")]
    cloud_waitlist: bool,
}

/// A connection on the "Show it your world" step.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum Tile {
    /// The calendars on the Mac.
    Calendar,
    /// Gmail.
    Gmail,
    /// Slack.
    Slack,
    /// Chrome.
    Chrome,
}

impl Tile {
    /// Every tile, in the order they show.
    pub(crate) const ALL: [Self; 4] = [Self::Calendar, Self::Gmail, Self::Slack, Self::Chrome];

    /// Its name.
    #[must_use]
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Calendar => "Calendar",
            Self::Gmail => "Gmail",
            Self::Slack => "Slack",
            Self::Chrome => "Chrome",
        }
    }

    /// What it does, in a few words.
    #[must_use]
    pub(crate) fn line(self) -> &'static str {
        match self {
            Self::Calendar => "Reads this Mac's calendars",
            Self::Gmail => "Reads new mail, never sends",
            Self::Slack => "Posts only after your OK",
            Self::Chrome => "Its own tab group, as you",
        }
    }

    /// The setup checklist's item for it.
    #[must_use]
    pub(crate) fn item(self) -> SetupItemId {
        match self {
            Self::Calendar => SetupItemId::Mac,
            Self::Gmail => SetupItemId::Gmail,
            Self::Slack => SetupItemId::Slack,
            Self::Chrome => SetupItemId::Chrome,
        }
    }
}

/// How a tile stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TileState {
    /// Connected and working.
    Connected,
    /// Connected, but failing now.
    Failing,
    /// Not connected: Connect is offered.
    Open,
    /// Connect was pressed and there is nothing more to do here: where it waits is said.
    Waiting,
}

/// Where a connection that can't be made from the setup waits.
pub(crate) const WAITING: &str = "Waiting in Settings › Connections.";

/// What the note beside the ember says at each glow.
pub(crate) const NOTES: [&str; 5] = [
    "Not lit yet.",
    "A spark.",
    "Lit. Say hi any time.",
    "Getting warmer.",
    "Your ember is lit.",
];

/// The brightest glow: the setup is done.
pub(crate) const BRIGHTEST: u8 = 4;

/// How a home here is kept lit once it starts: while the window is closed, and from login.
pub(crate) const KEEP_LIT: KeepLit = KeepLit {
    when_closed: true,
    at_login: true,
};

/// The setup.
pub(crate) struct Setup {
    /// Emberhut on this computer.
    home: Rc<dyn LocalHome>,
    /// How the setup ended, as kept.
    kept: SetupFile,
    /// The writes of `setup.json`.
    writes: Writes,
    /// Whether the setup shows now (until it ends).
    showing: bool,
    /// The step open; `None` once every step is behind (the "lit" screen).
    open: Option<SetupStep>,
    /// How each step stands, by its index.
    steps: [StepState; 5],
    /// Whether the store looks at the setup checklist (the app does; most tests don't).
    watching: bool,
    /// Whether the ember thinks.
    lit: LitCheck,
    /// The setup checklist, as last read.
    page: Option<SetupPage>,
    /// Where the user wants the ember to live.
    choice: HomeChoice,
    /// How giving it a home here goes.
    here: Here,
    /// Whether the pairing form for another computer shows.
    elsewhere: bool,
    /// The devices signed in when "Let it reach you" opened (none known before a state came),
    /// to tell a new one.
    known_devices: Option<Vec<DeviceId>>,
    /// The tiles Connect was pressed on.
    asked: BTreeSet<Tile>,
    /// How Emberhut here is kept lit, and its sheet.
    keep: Keeping,
    /// Emberhut here started again at launch.
    relighting: Relighting,
    /// The plans' accounts ("Light it").
    plans: Plans,
}

/// How Emberhut here is kept lit, and the sheet that changes it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Keeping {
    /// How it is kept, once the app keeps it (both off: it runs while the app does).
    kept: Option<KeepLit>,
    /// Whether the sheet shows.
    sheet: bool,
    /// Whether a change is on its way.
    changing: bool,
    /// Why the last change didn't take.
    problem: Option<String>,
}

impl Setup {
    /// The setup with no home here, ended as `files` keep it.
    pub(super) fn load(files: Option<&AppFiles>) -> Self {
        let kept = files
            .and_then(|files| {
                json_file::read(&files.setup(), "the setup's state", "showing the setup")
            })
            .unwrap_or_default();
        Self {
            home: Rc::new(NoLocalHome),
            kept,
            writes: Writes::default(),
            showing: false,
            open: None,
            steps: Default::default(),
            watching: false,
            lit: LitCheck::Unknown,
            page: None,
            choice: HomeChoice::Here,
            here: Here::Idle,
            elsewhere: false,
            known_devices: None,
            asked: BTreeSet::new(),
            keep: Keeping::default(),
            relighting: Relighting::default(),
            plans: Plans::default(),
        }
    }

    /// Emberhut on this computer is `home`.
    pub(super) fn set_home(&mut self, home: Rc<dyn LocalHome>) {
        self.home = home;
    }

    /// Whether the setup shows now.
    #[must_use]
    pub(crate) fn shows(&self) -> bool {
        self.showing
    }

    /// The step open; `None` on the "lit" screen.
    #[must_use]
    pub(crate) fn open(&self) -> Option<SetupStep> {
        self.open
    }

    /// How `step` stands.
    #[must_use]
    pub(crate) fn state(&self, step: SetupStep) -> &StepState {
        &self.steps[step.index()]
    }

    /// Whether `step` is done.
    #[must_use]
    pub(crate) fn is_done(&self, step: SetupStep) -> bool {
        matches!(self.state(step), StepState::Done(_))
    }

    /// How brightly the ember glows, from 0 (asleep and dim) to [`BRIGHTEST`]: a spark once
    /// it has a home, lit once it thinks, warmer once it can reach the user or see their
    /// world, and brightest when the setup is done.
    #[must_use]
    #[cfg(test)]
    pub(crate) fn glow(&self) -> u8 {
        if self.showing && self.open.is_none() {
            return BRIGHTEST;
        }
        let home = self.is_done(SetupStep::Home);
        let lit = home && self.is_done(SetupStep::Light);
        let warmer = lit && (self.is_done(SetupStep::Reach) || self.is_done(SetupStep::World));
        u8::from(home) + u8::from(lit) + u8::from(warmer)
    }

    /// The note the ember had beside it, by its glow.
    #[must_use]
    #[cfg(test)]
    pub(crate) fn note(&self) -> &'static str {
        NOTES[usize::from(self.glow())]
    }

    /// Where the user wants the ember to live.
    #[must_use]
    pub(crate) fn choice(&self) -> HomeChoice {
        self.choice
    }

    /// How giving it a home here goes.
    #[must_use]
    pub(crate) fn here(&self) -> &Here {
        &self.here
    }

    /// Whether the pairing form for another computer shows.
    #[must_use]
    pub(crate) fn shows_elsewhere(&self) -> bool {
        self.elsewhere
    }

    /// How Emberhut here is kept lit, once the app keeps it: step 1's Change shows then.
    #[must_use]
    pub(crate) fn keep(&self) -> Option<KeepLit> {
        self.keep.kept
    }

    /// Whether the sheet that changes how it is kept lit shows.
    #[must_use]
    pub(crate) fn shows_keep_sheet(&self) -> bool {
        self.keep.sheet
    }

    /// Whether a change of how it is kept lit is on its way.
    #[must_use]
    pub(crate) fn keeping(&self) -> bool {
        self.keep.changing
    }

    /// Why the last change of how it is kept lit didn't take.
    #[must_use]
    pub(crate) fn keep_problem(&self) -> Option<&str> {
        self.keep.problem.as_deref()
    }

    /// Whether the ember thinks, as far as the app knows.
    #[cfg(test)]
    #[must_use]
    pub(crate) fn lit(&self) -> LitCheck {
        self.lit
    }

    /// How the setup ended, if it did.
    #[cfg(test)]
    #[must_use]
    pub(crate) fn ended(&self) -> Option<Ending> {
        self.kept.ended
    }

    /// How `tile` stands, by the checklist (and, for Chrome, the profiles paired).
    #[must_use]
    pub(crate) fn tile(&self, tile: Tile, chrome_paired: bool) -> TileState {
        let state = self
            .page
            .as_ref()
            .and_then(|page| page.item(&tile.item()))
            .map(|item| &item.state);
        match state {
            Some(SetupState::Ready) => TileState::Connected,
            _ if tile == Tile::Chrome && chrome_paired => TileState::Connected,
            Some(SetupState::Error) => TileState::Failing,
            _ if self.asked.contains(&tile) => TileState::Waiting,
            _ => TileState::Open,
        }
    }

    /// The step `step` is done, with `line`; the next one not behind opens.
    fn finish(&mut self, step: SetupStep, line: String) {
        self.steps[step.index()] = StepState::Done(line);
        self.go_on_from(step);
    }

    /// The step `step` is put off; the next one opens.
    fn skip(&mut self, step: SetupStep) {
        self.steps[step.index()] = StepState::Skipped;
        self.go_on_from(step);
    }

    /// Opens the first step after `step` that isn't behind, or the "lit" screen.
    fn go_on_from(&mut self, step: SetupStep) {
        if self.open != Some(step) {
            return;
        }
        let mut next = step.next();
        while let Some(candidate) = next
            && self.steps[candidate.index()] != StepState::Ahead
        {
            next = candidate.next();
        }
        self.open = next;
    }
}

/// "Lit with Claude · its model.", or "Lit." when the app doesn't know with what.
#[must_use]
pub(crate) fn lit_line(saved: Option<&emberhut_api::agent_provider::AgentProvider>) -> String {
    let Some(saved) = saved else {
        return "Lit.".to_owned();
    };
    let Some(kind) = &saved.provider else {
        return "Lit.".to_owned();
    };
    let name = credentials::row(kind).map_or_else(|| credentials::name(kind), |row| row.name);
    match &saved.model {
        Some(model) => format!("Lit with {name} · {model}."),
        None => format!("Lit with {name}."),
    }
}

/// Where the ember lives, for its lines: this Mac (or computer) when the app talks to an ember
/// on this computer, another computer otherwise.
#[must_use]
pub(crate) fn place(server: &ServerUrl) -> &'static str {
    if !on_this_computer(server) {
        "another computer"
    } else if cfg!(target_os = "macos") {
        "this Mac"
    } else {
        "this computer"
    }
}

/// Words in a list: `a`, `a and b`, `a, b and c`.
#[must_use]
pub(crate) fn and_list(words: &[&str]) -> String {
    match words {
        [] => String::new(),
        [one] => (*one).to_owned(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

impl Store {
    /// The setup.
    #[must_use]
    pub(crate) fn setup(&self) -> &Setup {
        &self.setup
    }

    /// The app is starting: on a `fresh_install` (no server named) with no session, the setup
    /// shows, unless it ended before.
    pub(crate) fn begin_setup(&mut self, fresh_install: bool, cx: &mut Context<'_, Self>) {
        self.setup.watching = true;
        if fresh_install && !self.is_signed_in() && self.setup.kept.ended.is_none() {
            self.show_setup(first_screen(DIRECTION), cx);
        }
    }

    /// Shows the setup on `step` whatever the server says, the steps before it done as far as
    /// the app can tell; `None` shows the "lit" screen (screenshots).
    pub(crate) fn preview_setup(&mut self, step: Option<SetupStep>, cx: &mut Context<'_, Self>) {
        self.setup.watching = true;
        let until = step.map_or(SetupStep::ALL.len(), SetupStep::index);
        for earlier in &SetupStep::ALL[..until] {
            let line = match earlier {
                SetupStep::Home => self.home_line(),
                SetupStep::Light => self.light_line(),
                SetupStep::Reach | SetupStep::World | SetupStep::Share => {
                    self.setup.steps[earlier.index()] = StepState::Skipped;
                    continue;
                }
            };
            self.setup.steps[earlier.index()] = StepState::Done(line);
        }
        self.setup.showing = true;
        self.enter(step, cx);
    }

    /// Opens the setup on `step`.
    fn show_setup(&mut self, step: SetupStep, cx: &mut Context<'_, Self>) {
        self.setup.showing = true;
        self.enter(Some(step), cx);
    }

    /// Opens `step` (`None`: the "lit" screen), fetching what it shows.
    fn enter(&mut self, step: Option<SetupStep>, cx: &mut Context<'_, Self>) {
        self.setup.open = step;
        match step {
            Some(SetupStep::Light) => {
                self.open_credentials(cx);
                self.load_plans(cx);
            }
            Some(SetupStep::Reach) => {
                // Without a state yet, the first one to come says who was signed in already.
                self.setup.known_devices = self.state.is_some().then(|| self.signed_in_devices());
                self.make_pairing_code(cx);
            }
            Some(SetupStep::World) => self.read_setup_page(cx),
            Some(SetupStep::Home | SetupStep::Share) | None => {}
        }
        cx.notify();
    }

    /// `step` is done with `line`: the next step opens.
    fn step_done(&mut self, step: SetupStep, line: String, cx: &mut Context<'_, Self>) {
        if self.setup.open != Some(step) {
            return;
        }
        self.setup.finish(step, line);
        self.enter(self.setup.open, cx);
    }

    /// Later on the open step: an optional one is put off and the next opens; "Light it" put
    /// off ends the setup (kept), the ember not lit.
    pub(crate) fn setup_later(&mut self, cx: &mut Context<'_, Self>) {
        let Some(step) = self.setup.open else {
            return;
        };
        if step.optional() {
            self.setup.skip(step);
            self.enter(self.setup.open, cx);
        } else if step == SetupStep::Light {
            self.end_setup(Ending::Skipped, cx);
        }
    }

    /// "Say hello" on the "lit" screen, or Done on the last step: the setup ends (kept), and
    /// the app opens.
    pub(crate) fn finish_setup(&mut self, cx: &mut Context<'_, Self>) {
        if self.setup.open == Some(SetupStep::Share) {
            self.setup.finish(SetupStep::Share, String::new());
            self.enter(None, cx);
            return;
        }
        self.end_setup(Ending::Finished, cx);
    }

    /// The setup ends as `ending`, kept so it doesn't show again.
    fn end_setup(&mut self, ending: Ending, cx: &mut Context<'_, Self>) {
        self.setup.showing = false;
        self.setup.open = None;
        self.setup.kept.ended = Some(ending);
        self.keep_setup_file(cx);
        cx.notify();
    }

    /// "Lit it" step's line: the plan's account it is lit with, else the provider saved.
    fn light_line(&self) -> String {
        self.setup
            .plans
            .lit_line()
            .unwrap_or_else(|| lit_line(self.credentials().saved()))
    }

    /// "Tell me when it's ready" on renting a home in the cloud: kept in `setup.json`, and on
    /// the server once there is one (nothing is paid or sent anywhere else).
    pub(crate) fn join_cloud_waitlist(&mut self, cx: &mut Context<'_, Self>) {
        if self.setup.kept.cloud_waitlist {
            return;
        }
        self.setup.kept.cloud_waitlist = true;
        self.keep_setup_file(cx);
        self.tell_server_waitlist(cx);
        cx.notify();
    }

    /// Tells the server the person is on the cloud waitlist, when they are and it is signed in
    /// (again at every sign-in: the server keeps the first time).
    fn tell_server_waitlist(&self, cx: &mut Context<'_, Self>) {
        if !self.setup.kept.cloud_waitlist || !self.is_signed_in() {
            return;
        }
        let backend = Arc::clone(&self.backend);
        cx.background_spawn(async move {
            if let Err(error) = backend.join_waitlist("cloud-home") {
                log::info!("the server didn't keep the cloud waitlist: {error}");
            }
        })
        .detach();
    }

    /// Whether the person asked to hear when renting a home in the cloud is ready.
    #[must_use]
    pub(crate) fn on_cloud_waitlist(&self) -> bool {
        self.setup.kept.cloud_waitlist
    }

    /// Writes `setup.json`.
    fn keep_setup_file(&mut self, cx: &mut Context<'_, Self>) {
        if let Some(files) = self.files.clone() {
            let kept = self.setup.kept;
            let (number, written) = self.setup.writes.next();
            cx.background_spawn(async move {
                json_file::write(&files.setup(), &kept, number, &written, "the setup's state");
            })
            .detach();
        }
    }

    /// The user picks where the ember lives.
    pub(crate) fn choose_home(&mut self, choice: HomeChoice, cx: &mut Context<'_, Self>) {
        self.setup.choice = choice;
        if choice == HomeChoice::Here {
            self.setup.elsewhere = false;
        }
        cx.notify();
    }

    /// Continue on the first step: Emberhut is started here and this app paired with it, or
    /// the pairing form for another computer shows.
    pub(crate) fn continue_home(&mut self, cx: &mut Context<'_, Self>) {
        match self.setup.choice {
            HomeChoice::Elsewhere => {
                self.setup.elsewhere = true;
                cx.notify();
            }
            HomeChoice::Here => self.start_home_here(cx),
        }
    }

    /// Starts Emberhut here, then pairs with it.
    fn start_home_here(&mut self, cx: &mut Context<'_, Self>) {
        if matches!(self.setup.here, Here::Starting | Here::Pairing) {
            return;
        }
        if !self.setup.home.available() {
            self.home_failed(CANT_HERE.to_owned(), cx);
            return;
        }
        self.setup.here = Here::Starting;
        let started = self.setup.home.start(cx);
        cx.spawn(async move |this, cx| {
            let started = started.await;
            update_or_log(&this, cx, |store, cx| store.home_started(started, cx));
        })
        .detach();
        cx.notify();
    }

    /// Emberhut here started at `started`, or didn't: the app moves to its address (nothing
    /// to do when it talks to it already), then keeps it lit.
    fn home_started(&mut self, started: Result<ServerUrl, String>, cx: &mut Context<'_, Self>) {
        let url = match started {
            Ok(url) => url,
            Err(why) => return self.home_failed(why, cx),
        };
        let moved = self.switch_server(url, cx);
        cx.spawn(async move |this, cx| {
            let moved = moved.await;
            update_or_log(&this, cx, |store, cx| match moved {
                Ok(()) => store.keep_home_lit(cx),
                Err(why) => store.home_failed(why, cx),
            });
        })
        .detach();
    }

    /// The app talks to Emberhut here: it is kept lit as [`KEEP_LIT`] says (when it is this
    /// app's to keep), then paired with. Not staying lit without the app is no failure: the
    /// done line says it lives here while the app runs.
    fn keep_home_lit(&mut self, cx: &mut Context<'_, Self>) {
        let Some(kept) = self.setup.home.keep_lit(KEEP_LIT, cx) else {
            return self.pair_home_here(cx);
        };
        cx.spawn(async move |this, cx| {
            let kept = kept.await;
            update_or_log(&this, cx, |store, cx| {
                store.setup.keep.kept = Some(match kept {
                    Ok(()) => KEEP_LIT,
                    Err(why) => {
                        log::warn!("Emberhut here runs only while the app does: {why}");
                        KeepLit::default()
                    }
                });
                store.pair_home_here(cx);
            });
        })
        .detach();
    }

    /// Pairs this app with Emberhut here, through the code it makes (or the one the user
    /// types), unless it is paired already.
    fn pair_home_here(&mut self, cx: &mut Context<'_, Self>) {
        if self.is_signed_in() {
            return self.home_given(cx);
        }
        self.setup.here = Here::Pairing;
        let code = self.setup.home.pair_code(cx);
        cx.spawn(async move |this, cx| {
            let code = code.await;
            update_or_log(&this, cx, |store, cx| match code {
                // Paired, the store hears it (`setup_signed_in`); refused, the step says why.
                Ok(Some(code)) => store.pair(code, cx, |outcome, cx| {
                    if let Err(why) = outcome {
                        cx.spawn(async move |this, cx| {
                            update_or_log(&this, cx, |store, cx| store.home_failed(why, cx));
                        })
                        .detach();
                    }
                }),
                Ok(None) => {
                    store.setup.here = Here::TypeCode;
                    cx.notify();
                }
                Err(why) => store.home_failed(why, cx),
            });
        })
        .detach();
        cx.notify();
    }

    /// Giving the ember a home here failed: why shows, with another computer beside it.
    fn home_failed(&mut self, why: String, cx: &mut Context<'_, Self>) {
        self.setup.here = Here::Failed(why);
        self.setup.elsewhere = true;
        cx.notify();
    }

    /// The ember has a home and this app is paired with it: step 1 is done.
    fn home_given(&mut self, cx: &mut Context<'_, Self>) {
        self.setup.here = Here::Idle;
        self.setup.elsewhere = false;
        let line = self.home_line();
        self.step_done(SetupStep::Home, line, cx);
        self.ask_lit(cx);
    }

    /// Step 1's line once done: where the ember lives, and how it stays lit when the app keeps
    /// it.
    fn home_line(&self) -> String {
        let place = place(self.server());
        match self.setup.keep.kept {
            Some(keep) if keep.by_service() => {
                format!("It lives on {place} and stays lit when the window is closed.")
            }
            Some(_) => format!("It lives on {place} while Emberhut is open."),
            None => format!("It lives on {place}."),
        }
    }

    /// Change on step 1's done line: the sheet with how Emberhut here is kept lit.
    pub(crate) fn open_keep_sheet(&mut self, cx: &mut Context<'_, Self>) {
        if self.setup.keep.kept.is_some() {
            self.setup.keep.sheet = true;
            self.setup.keep.problem = None;
            cx.notify();
        }
    }

    /// Done on that sheet.
    pub(crate) fn close_keep_sheet(&mut self, cx: &mut Context<'_, Self>) {
        self.setup.keep.sheet = false;
        cx.notify();
    }

    /// A switch on that sheet: Emberhut here is kept lit as `keep` says from now on, and step
    /// 1's line says so; why not shows on the sheet.
    pub(crate) fn change_keep_lit(&mut self, keep: KeepLit, cx: &mut Context<'_, Self>) {
        if self.setup.keep.changing || self.setup.keep.kept.is_none() {
            return;
        }
        let Some(kept) = self.setup.home.keep_lit(keep, cx) else {
            return;
        };
        self.setup.keep.changing = true;
        self.setup.keep.problem = None;
        cx.spawn(async move |this, cx| {
            let kept = kept.await;
            update_or_log(&this, cx, |store, cx| {
                store.setup.keep.changing = false;
                match kept {
                    Ok(()) => {
                        store.setup.keep.kept = Some(keep);
                        if store.setup.is_done(SetupStep::Home) {
                            store.setup.steps[SetupStep::Home.index()] =
                                StepState::Done(store.home_line());
                        }
                    }
                    Err(why) => store.setup.keep.problem = Some(why),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// This device paired (from the setup's form, or the sign-in screen's): step 1 is done if
    /// it is open.
    pub(super) fn setup_signed_in(&mut self, cx: &mut Context<'_, Self>) {
        self.tell_server_waitlist(cx);
        if self.setup.showing && self.setup.open == Some(SetupStep::Home) {
            self.home_given(cx);
        } else {
            self.ask_lit(cx);
        }
    }

    /// A provider was saved from "Light it": the step is done.
    pub(crate) fn setup_lit(&mut self, cx: &mut Context<'_, Self>) {
        self.setup.lit = LitCheck::Lit;
        let line = self.light_line();
        self.step_done(SetupStep::Light, line, cx);
    }

    /// A state came: a phone new since "Let it reach you" opened finishes it, and whether the
    /// ember is lit is asked once this launch.
    pub(super) fn setup_state_came(&mut self, cx: &mut Context<'_, Self>) {
        if self.setup.open == Some(SetupStep::Reach) {
            let Some(known) = &self.setup.known_devices else {
                self.setup.known_devices = Some(self.signed_in_devices());
                return self.ask_lit(cx);
            };
            let this_one = self.this_device();
            let new = self.state.as_ref().and_then(|state| {
                state
                    .auth
                    .as_ref()?
                    .devices
                    .iter()
                    .find(|device| !known.contains(&device.id) && Some(device.id) != this_one)
                    .map(|device| device.name.clone())
            });
            if let Some(name) = new {
                let line = format!("{} can reach it.", name.as_deref().unwrap_or("A phone"));
                self.step_done(SetupStep::Reach, line, cx);
            }
        }
        self.ask_lit(cx);
    }

    /// The devices signed in now, by the last state.
    fn signed_in_devices(&self) -> Vec<DeviceId> {
        self.state
            .as_ref()
            .and_then(|state| state.auth.as_ref())
            .map_or_default(|auth| auth.devices.iter().map(|device| device.id).collect())
    }

    /// Asks the server once whether the ember is lit, while signed in.
    fn ask_lit(&mut self, cx: &mut Context<'_, Self>) {
        if self.setup.watching && self.setup.lit == LitCheck::Unknown && self.is_signed_in() {
            self.setup.lit = LitCheck::Asking;
            self.read_setup_page(cx);
        }
    }

    /// Reads the setup checklist.
    fn read_setup_page(&mut self, cx: &mut Context<'_, Self>) {
        let backend = Arc::clone(&self.backend);
        cx.spawn(async move |this, cx| {
            let page = cx.background_spawn(async move { backend.setup() }).await;
            update_or_log(&this, cx, |store, cx| store.setup_page_came(page, cx));
        })
        .detach();
    }

    /// The checklist came: whether the ember is lit, and the tiles' states. An ember not lit
    /// opens the setup on "Light it", unless the setup ended before (then the line over the
    /// chat says so); one that is lit while "Light it" is open finishes the step.
    fn setup_page_came(&mut self, page: Result<SetupPage, Error>, cx: &mut Context<'_, Self>) {
        let page = match page {
            Ok(page) => page,
            Err(error) => {
                // A server without the checklist, or one that didn't answer: the app opens as
                // it did before the setup.
                log::info!("couldn't read the setup checklist: {error}");
                if self.setup.lit == LitCheck::Asking {
                    self.setup.lit = LitCheck::Unknown;
                }
                return;
            }
        };
        let unlit = page
            .item(&SetupItemId::Provider)
            .is_some_and(|item| matches!(item.state, SetupState::Missing | SetupState::Off));
        self.setup.lit = if unlit {
            LitCheck::Unlit
        } else {
            LitCheck::Lit
        };
        self.setup.page = Some(page);
        if unlit && !self.setup.showing && self.setup.kept.ended.is_none() {
            self.setup.steps[SetupStep::Home.index()] = StepState::Done(self.home_line());
            self.show_setup(SetupStep::Light, cx);
        } else if !unlit && self.setup.open == Some(SetupStep::Light) {
            let line = lit_line(self.credentials().saved());
            self.step_done(SetupStep::Light, line, cx);
        }
        cx.notify();
    }

    /// Connect on a tile that can't be connected from the setup: where it waits shows.
    pub(crate) fn ask_tile(&mut self, tile: Tile, cx: &mut Context<'_, Self>) {
        if self.setup.asked.insert(tile) {
            cx.notify();
        }
    }

    /// Continue on "Show it your world" once something is connected: the step is done.
    pub(crate) fn world_done(&mut self, cx: &mut Context<'_, Self>) {
        let paired = !self.chrome_profiles().is_empty();
        let connected: Vec<&str> = Tile::ALL
            .into_iter()
            .filter(|&tile| self.setup.tile(tile, paired) == TileState::Connected)
            .map(Tile::name)
            .collect();
        if connected.is_empty() {
            self.setup_later(cx);
            return;
        }
        let line = format!("Connected: {}.", and_list(&connected));
        self.step_done(SetupStep::World, line, cx);
    }

    /// Whether the quiet line over the chat says the ember isn't lit: the setup ended before,
    /// and the server says so now.
    #[must_use]
    pub(crate) fn shows_unlit_line(&self) -> bool {
        !self.setup.showing && self.setup.lit == LitCheck::Unlit
    }

    /// What the "lit" screen says: where the ember lives, what it thinks with, the phone that
    /// reaches it and what it can read; then that it keeps working.
    #[must_use]
    pub(crate) fn setup_summary(&self) -> String {
        let mut parts = vec![format!("It lives on {}", place(self.server()))];
        if let Some(kind) = self
            .credentials()
            .saved()
            .and_then(|saved| saved.provider.as_ref())
        {
            let name =
                credentials::row(kind).map_or_else(|| credentials::name(kind), |row| row.name);
            parts.push(format!("thinks with {name}"));
        }
        if let StepState::Done(line) = self.setup.state(SetupStep::Reach)
            && let Some(phone) = line.strip_suffix(" can reach it.")
        {
            parts.push(format!("can reach you on {phone}"));
        }
        let paired = !self.chrome_profiles().is_empty();
        if self.setup.tile(Tile::Calendar, paired) == TileState::Connected {
            parts.push("can read your calendar".to_owned());
        }
        if self.setup.tile(Tile::Gmail, paired) == TileState::Connected {
            parts.push("can read your mail".to_owned());
        }
        let parts: Vec<&str> = parts.iter().map(String::as_str).collect();
        format!("{}. It keeps working while you're away.", and_list(&parts))
    }

    /// The line under the "lit" screen's buttons: the connections still waiting, if any.
    #[must_use]
    pub(crate) fn setup_waiting(&self) -> Option<String> {
        let paired = !self.chrome_profiles().is_empty();
        let waiting: Vec<&str> = Tile::ALL
            .into_iter()
            .filter(|&tile| {
                tile != Tile::Calendar && self.setup.tile(tile, paired) != TileState::Connected
            })
            .map(Tile::name)
            .collect();
        match waiting.as_slice() {
            [] => None,
            [one] => Some(format!(
                "{one} is waiting in Settings › Connections whenever you want it."
            )),
            many => Some(format!(
                "{} are waiting in Settings › Connections whenever you want them.",
                and_list(many)
            )),
        }
    }
}
