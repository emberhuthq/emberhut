//! The command line.

use std::str::FromStr;

use clap::{Parser, ValueEnum};

use crate::links::{self, LinkError, Target};
use crate::material::Look;
use crate::shell::WindowKind;
use crate::store::settings::SettingsPane;
use crate::store::{OpenReport, Page};
use emberhut_api::ids::{EnvironmentId, RoomId, SessionId, TaskId};
use emberhut_api::theme::Appearance;

/// Emberhut's desktop app.
#[derive(Clone, Debug, Parser)]
#[command(name = "emberhut-desktop", version, about)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "independent command-line switches, one flag each"
)]
pub(crate) struct Cli {
    /// Open on a page instead of the chat.
    #[arg(long, value_enum)]
    pub(crate) page: Option<StartPage>,

    /// The room to open (its id); when not given, the one open when the app was last used
    /// (Main the first time).
    #[arg(long, value_name = "ID", value_parser = room_id)]
    pub(crate) room: Option<RoomId>,

    /// The window's size in logical pixels (else as it was left, or 1100x760 the first time).
    #[arg(long, value_name = "WIDTHxHEIGHT")]
    pub(crate) size: Option<WindowSize>,

    /// Open a place: an emberhut:// link (the system hands links over this way on Linux).
    #[arg(value_name = "LINK", value_parser = link)]
    pub(crate) link: Option<Target>,

    /// Open a window of its own: browser, calendar, sessions, dark, diary, party or note.
    #[arg(long, value_name = "NAME", value_parser = window_kind)]
    pub(crate) window: Option<WindowKind>,

    /// Open one party, in the Party window.
    #[arg(long, value_name = "ID")]
    pub(crate) party: Option<i64>,

    /// Don't ask for permission to show notifications at launch (tests, screenshots).
    #[arg(long)]
    pub(crate) no_prompt: bool,

    /// Open a report beside the page: its name and its path on the server.
    #[arg(long, value_name = "NAME=PATH")]
    pub(crate) report: Option<ReportArg>,

    /// Show the sign-in screen whatever the server says (for screenshots).
    #[arg(long)]
    pub(crate) sign_in: bool,

    /// Show a theme preset without saving it (for screenshots).
    #[arg(long, value_name = "PRESET")]
    pub(crate) theme: Option<String>,

    /// Show an appearance without saving it (for screenshots).
    #[arg(long, value_enum)]
    pub(crate) appearance: Option<AppearanceArg>,

    /// Open without taking the keyboard focus from the app in front.
    #[arg(long)]
    pub(crate) background: bool,

    /// Open on this display (0 is the first).
    #[arg(long, value_name = "N")]
    pub(crate) display: Option<usize>,

    /// Draw every surface opaque, without the sidebar's material (the look before it).
    #[arg(long)]
    pub(crate) opaque: bool,

    /// Open Settings on one of its pages (for screenshots).
    #[arg(long, value_enum, value_name = "PAGE")]
    pub(crate) settings: Option<SettingsArg>,

    /// Open Settings on a Chrome profile's sheet, by its id (for screenshots).
    #[arg(long, value_name = "ID")]
    pub(crate) chrome_profile: Option<i64>,

    /// Open Settings on an environment's page, by its id (for screenshots).
    #[arg(long, value_name = "ID", value_parser = text_id::<EnvironmentId>)]
    pub(crate) environment: Option<EnvironmentId>,

    /// Open a session's page beside the chat, by its id.
    #[arg(long, value_name = "ID", value_parser = text_id::<SessionId>)]
    pub(crate) session: Option<SessionId>,

    /// Measure the app and quit: start, scroll:SECONDS, rooms:ID,ID,… or demo.
    #[cfg(feature = "automation")]
    #[arg(long, value_name = "PLAN")]
    pub(crate) bench: Option<crate::automation::BenchPlan>,

    /// Replay the steps appended to this file into the window (see the README).
    #[cfg(feature = "automation")]
    #[arg(long, value_name = "FILE")]
    pub(crate) drive: Option<std::path::PathBuf>,

    /// Drop the connection 6 s of every 20, to see what reconnects do (debug builds only).
    #[cfg(debug_assertions)]
    #[arg(long)]
    pub(crate) flaky: bool,
    /// Open that task's page (its id), as a link to it does.
    #[arg(long, value_name = "ID", value_parser = task_id)]
    pub(crate) task: Option<TaskId>,

    /// Open with the spotlight up (for screenshots).
    #[arg(long)]
    pub(crate) spotlight: bool,

    /// Open a project's page beside the chat, by its id.
    #[arg(long, value_name = "ID")]
    pub(crate) project: Option<i64>,

    /// Start with fresh, empty files and no server, and exit 0 once the window has drawn its
    /// first frame (1 after a minute): a release's smoke run.
    #[arg(long)]
    pub(crate) self_check: bool,
}

impl Cli {
    /// Whether the automation runs this window (`--bench` or `--drive`): its session stays in
    /// memory, so it never reaches the Keychain.
    #[must_use]
    #[cfg_attr(
        not(feature = "automation"),
        expect(
            clippy::unused_self,
            reason = "a build without the automation has no such run"
        )
    )]
    pub(crate) fn automated(&self) -> bool {
        #[cfg(feature = "automation")]
        {
            self.bench.is_some() || self.drive.is_some()
        }
        #[cfg(not(feature = "automation"))]
        {
            false
        }
    }

    /// The look asked for: `--opaque`, or the material where it is allowed.
    #[must_use]
    pub(crate) fn look(&self) -> Look {
        if self.opaque {
            Look::Opaque
        } else {
            Look::Material
        }
    }

    /// The places to open once the window is: the link, the task, the window, the party.
    #[must_use]
    pub(crate) fn targets(&self) -> Vec<Target> {
        let mut targets: Vec<Target> = self.link.iter().cloned().collect();
        targets.extend(self.task.map(Target::Task));
        targets.extend(self.window.map(Target::Window));
        targets.extend(self.party.map(Target::Party));
        targets
    }

    /// The room to open: the one asked for, else the one `kept` from the last launch, else
    /// Main.
    #[must_use]
    pub(crate) fn start_room(&self, kept: Option<RoomId>) -> RoomId {
        self.room.or(kept).unwrap_or(RoomId::MAIN)
    }

    /// The page to open on, in the room `kept` from the last launch unless one is asked for.
    #[must_use]
    pub(crate) fn start_page(&self, kept: Option<RoomId>) -> Page {
        if self.settings.is_some() || self.chrome_profile.is_some() || self.environment.is_some() {
            return Page::Settings;
        }
        match self.page {
            None => Page::Room(self.start_room(kept)),
            Some(StartPage::Needs) => Page::Needs,
            Some(StartPage::Tasks) => Page::Tasks,
            Some(StartPage::Machines) => Page::Machines,
            Some(StartPage::Settings) => Page::Settings,
            Some(StartPage::Scheduled) => Page::Scheduled,
        }
    }

    /// The page to open over Settings at launch, besides an environment's: the one of its
    /// pages asked for (the Chrome profiles under a profile's sheet), or the machines.
    #[must_use]
    pub(crate) fn over_settings(&self) -> Option<Page> {
        self.settings.and_then(SettingsArg::page).or_else(|| {
            self.chrome_profile
                .map(|_| Page::SettingsPane(SettingsPane::ChromeProfiles))
        })
    }
}

/// A link from the command line.
fn link(text: &str) -> Result<Target, LinkError> {
    links::parse(text)
}

/// A window's name from the command line.
fn window_kind(text: &str) -> Result<WindowKind, ArgError> {
    WindowKind::from_link_name(text).ok_or_else(|| ArgError::Window(text.to_owned()))
}

/// A text id from the command line (an environment's, a session's): not blank.
fn text_id<T: for<'a> From<&'a str>>(text: &str) -> Result<T, ArgError> {
    if text.trim().is_empty() {
        Err(ArgError::Id(text.to_owned()))
    } else {
        Ok(T::from(text))
    }
}

/// A room id from the command line.
fn room_id(text: &str) -> Result<RoomId, std::num::ParseIntError> {
    text.parse().map(RoomId::new)
}

fn task_id(text: &str) -> Result<TaskId, &'static str> {
    crate::prose::links::task_id(text).ok_or("a task's id is a whole number above zero")
}

/// A page to open on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub(crate) enum StartPage {
    /// Needs you.
    Needs,
    /// Tasks.
    Tasks,
    /// Machines.
    Machines,
    /// Settings.
    Settings,
    /// What is scheduled.
    Scheduled,
}

/// One of Settings' pages to open on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub(crate) enum SettingsArg {
    /// Settings' main page.
    Main,
    /// Appearance.
    Appearance,
    /// The Chrome profiles.
    ChromeProfiles,
    /// What the assistant knows about you.
    AboutYou,
    /// The archived rooms, unfolded on Settings' main page.
    ArchivedRooms,
    /// The machines, over Settings.
    Machines,
}

impl SettingsArg {
    /// The page it opens over Settings; none for Settings' main page itself.
    #[must_use]
    pub(crate) fn page(self) -> Option<Page> {
        match self {
            Self::Main | Self::ArchivedRooms => None,
            Self::Appearance => Some(Page::SettingsPane(SettingsPane::Appearance)),
            Self::ChromeProfiles => Some(Page::SettingsPane(SettingsPane::ChromeProfiles)),
            Self::AboutYou => Some(Page::SettingsPane(SettingsPane::AboutYou)),
            Self::Machines => Some(Page::Machines),
        }
    }
}

/// An appearance to show.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub(crate) enum AppearanceArg {
    /// Follow the system.
    System,
    /// Light.
    Light,
    /// Dark.
    Dark,
}

impl From<AppearanceArg> for Appearance {
    fn from(arg: AppearanceArg) -> Self {
        match arg {
            AppearanceArg::System => Self::System,
            AppearanceArg::Light => Self::Light,
            AppearanceArg::Dark => Self::Dark,
        }
    }
}

/// A window size, `1440x900`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct WindowSize {
    /// Its width.
    pub(crate) width: f32,
    /// Its height.
    pub(crate) height: f32,
}

/// Why a command-line value was refused.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub(crate) enum ArgError {
    /// Not `WIDTHxHEIGHT` with two positive numbers.
    #[error("`{0}` isn't a size like 1440x900")]
    Size(String),
    /// Not `NAME=PATH`.
    #[error("`{0}` isn't a report like Week=/files/week.html")]
    Report(String),
    /// Not a window's name.
    #[error("`{0}` isn't a window: browser, calendar, sessions, dark, diary, party or note")]
    Window(String),
    /// An empty id.
    #[error("`{0}` isn't an id")]
    Id(String),
}

impl FromStr for WindowSize {
    type Err = ArgError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let bad = || ArgError::Size(text.to_owned());
        let (width, height) = text.split_once('x').ok_or_else(bad)?;
        let (width, height): (f32, f32) = (
            width.trim().parse().map_err(|_| bad())?,
            height.trim().parse().map_err(|_| bad())?,
        );
        if width > 0. && height > 0. && width.is_finite() && height.is_finite() {
            Ok(Self { width, height })
        } else {
            Err(bad())
        }
    }
}

/// A report to open: `NAME=PATH`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ReportArg(pub(crate) OpenReport);

impl FromStr for ReportArg {
    type Err = ArgError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text.split_once('=') {
            Some((name, path)) if !name.is_empty() && path.starts_with('/') => {
                Ok(Self(OpenReport {
                    name: name.to_owned(),
                    path: path.to_owned(),
                }))
            }
            _ => Err(ArgError::Report(text.to_owned())),
        }
    }
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory as _;

    use super::*;

    fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
        Cli::try_parse_from(std::iter::once("emberhut-desktop").chain(args.iter().copied()))
    }

    #[test]
    fn the_command_line_is_well_formed() {
        Cli::command().debug_assert();
    }

    #[test]
    fn with_no_flags_it_opens_main_in_a_default_window() {
        let cli = parse(&[]).unwrap();
        assert_eq!(cli.start_page(None), Page::Room(RoomId::MAIN));
        assert_eq!(
            cli.start_page(Some(RoomId::new(3))),
            Page::Room(RoomId::new(3)),
            "the room open at the last launch"
        );
        assert_eq!(cli.size, None, "the size it was left at");
        assert!(!cli.background && !cli.sign_in && !cli.no_prompt);
        assert_eq!(cli.report, None);
        assert_eq!(cli.targets(), []);
    }

    #[test]
    fn a_page_wins_over_a_room() {
        assert_eq!(
            parse(&["--room", "5"])
                .unwrap()
                .start_page(Some(RoomId::new(3))),
            Page::Room(RoomId::new(5))
        );
        assert_eq!(
            parse(&["--page", "machines", "--room", "5"])
                .unwrap()
                .start_page(None),
            Page::Machines
        );
        assert!(parse(&["--page", "inbox"]).is_err());
    }

    #[test]
    fn a_task_is_opened_by_its_id() {
        assert_eq!(
            parse(&["--task", "212"]).unwrap().task,
            Some(TaskId::new(212))
        );
        assert_eq!(parse(&[]).unwrap().task, None);
        for loose in ["x", "-3", "+3", "0", "3/"] {
            assert!(parse(&["--task", loose]).is_err(), "{loose}");
        }
    }

    #[test]
    fn sizes_are_two_positive_numbers() {
        assert_eq!(
            "800x600".parse::<WindowSize>(),
            Ok(WindowSize {
                width: 800.,
                height: 600.
            })
        );
        assert!("800".parse::<WindowSize>().is_err());
        assert!("0x600".parse::<WindowSize>().is_err());
        assert!("axb".parse::<WindowSize>().is_err());
        assert!(parse(&["--size", "wide"]).is_err());
        assert_eq!(
            parse(&["--size", "800x600"]).unwrap().size,
            Some(WindowSize {
                width: 800.,
                height: 600.
            })
        );
    }

    #[test]
    fn a_link_a_task_a_window_or_a_party_opens_at_launch() {
        let cli = parse(&["emberhut://chat?room=4"]).unwrap();
        assert_eq!(cli.targets(), [Target::Chat(Some(RoomId::new(4)))]);
        let cli = parse(&["--task", "119", "--window", "diary", "--party", "3"]).unwrap();
        assert_eq!(
            cli.targets(),
            [
                Target::Task(TaskId::new(119)),
                Target::Window(WindowKind::Diary),
                Target::Party(3)
            ]
        );
        assert!(parse(&["https://example.com"]).is_err());
        assert!(parse(&["--window", "attic"]).is_err());
        assert!(parse(&["--task", "x"]).is_err());
    }

    #[test]
    fn a_report_is_a_name_and_a_server_path() {
        let report: ReportArg = "Week=/files/week.html".parse().unwrap();
        assert_eq!(
            report.0,
            OpenReport {
                name: "Week".into(),
                path: "/files/week.html".into()
            }
        );
        assert!("Week".parse::<ReportArg>().is_err());
        assert!("=/x".parse::<ReportArg>().is_err());
        assert!("Week=files".parse::<ReportArg>().is_err());
    }

    #[test]
    fn a_screenshot_theme_and_appearance_are_read() {
        let cli = parse(&[
            "--theme",
            "green",
            "--appearance",
            "dark",
            "--sign-in",
            "--display",
            "1",
        ])
        .unwrap();
        assert_eq!(cli.theme.as_deref(), Some("green"));
        assert_eq!(cli.appearance.map(Appearance::from), Some(Appearance::Dark));
        assert!(cli.sign_in);
        assert_eq!(cli.display, Some(1));
    }

    #[test]
    fn a_settings_page_opens_over_settings() {
        let cli = parse(&["--settings", "chrome-profiles"]).unwrap();
        assert_eq!(cli.start_page(None), Page::Settings);
        assert_eq!(
            cli.over_settings(),
            Some(Page::SettingsPane(SettingsPane::ChromeProfiles))
        );
        let cli = parse(&["--chrome-profile", "2"]).unwrap();
        assert_eq!(
            (cli.start_page(None), cli.chrome_profile),
            (Page::Settings, Some(2))
        );
        assert_eq!(
            cli.over_settings(),
            Some(Page::SettingsPane(SettingsPane::ChromeProfiles)),
            "the sheet is over the profiles"
        );
        let cli = parse(&["--settings", "machines"]).unwrap();
        assert_eq!(cli.over_settings(), Some(Page::Machines));
        let cli = parse(&["--settings", "archived-rooms"]).unwrap();
        assert_eq!(
            (cli.start_page(None), cli.over_settings()),
            (Page::Settings, None),
            "unfolded on Settings itself"
        );
        assert_eq!(
            parse(&["--settings", "main"]).unwrap().over_settings(),
            None
        );
        assert!(parse(&["--settings", "archive"]).is_err());
    }

    #[test]
    fn an_environment_opens_over_settings_and_a_session_beside_the_chat() {
        let cli = parse(&["--environment", "mail"]).unwrap();
        assert_eq!(cli.start_page(None), Page::Settings);
        assert_eq!(cli.environment, Some(EnvironmentId::from("mail")));
        let cli = parse(&["--session", "r-1"]).unwrap();
        assert_eq!(cli.session, Some(SessionId::from("r-1")));
        assert_eq!(cli.start_page(None), Page::Room(RoomId::MAIN));
        assert!(parse(&["--environment", " "]).is_err());
        assert!(parse(&["--session", ""]).is_err());
    }
}
