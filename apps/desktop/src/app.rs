//! Starting the app: the server to talk to, the store, the menus, the first main window, and
//! the links and the command line's place.

use std::rc::Rc;
use std::sync::Arc;

use anyhow::Context as _;
use gpui_kit::prelude::*;
use gpui_kit::{App, Bounds, Entity, WindowBounds, px};

use crate::assets::{self, Assets};
use crate::backend::Backend;
use crate::cli::{Cli, SettingsArg};
use crate::config::{self, AppFiles, SERVER_ENV, ServerUrl, Servers};
use crate::hut::{HutOptions, LocalHut};
use crate::prefs::{self, Kept as KeptPrefs};
use crate::shell::{self, LinkReceiver};
use crate::store::sessions::places::SessionPlace;
use crate::store::sessions::view::SessionRef;
use crate::store::setup::{DevHome, HutHome, LocalHome};
use crate::store::{
    Connected, Connector, DiskCache, Kept, Page, Reconnect, Store, ThemeState, ViewFile,
};
use crate::ui::look;
use crate::ui::palette::Pal;
use crate::ui::report::ReportSize;
use crate::ui::window;
use crate::updates::self_check::SelfCheck;
use crate::{backend, device, menus, motion, notifications};
use emberhut_api::theme::Appearance;

/// What the app reads before its run loop starts, so no file is read on the UI thread.
struct Startup {
    files: Option<AppFiles>,
    backend: Arc<dyn Backend>,
    theme: ThemeState,
    report_size: ReportSize,
    /// The open room's chat as kept on disk.
    kept: Kept,
    /// How the window was left.
    view: ViewFile,
    /// The windows' frame and the sidebar's folds.
    prefs: KeptPrefs,
    /// The smoke run, with `--self-check`.
    self_check: Option<SelfCheck>,
    /// No server is named anywhere (and this isn't the smoke run): the setup may show.
    fresh_install: bool,
    /// `EMBERHUT_URL` named the server: no move of the setup's takes it elsewhere.
    pinned: bool,
}

/// Makes the backend for a server the app moves to while it runs, as [`run`] made the first:
/// its servers as `config.json` says now (the new address is kept there first), its session
/// kept in the app's files.
struct AppConnector {
    /// The app's files, where sessions are kept (none: in memory).
    files: Option<AppFiles>,
    /// This device's name, for a pairing.
    device: String,
}

impl Connector for AppConnector {
    fn connect(&self, server: &ServerUrl) -> Result<Connected, backend::Error> {
        let file = self.files.as_ref().map_or_default(AppFiles::read_config);
        let servers = config::servers(None, &file);
        // The file names `server` once kept; without the app's files, there is only it.
        let servers = if &servers.primary.url == server {
            servers
        } else {
            Servers::only(server.clone())
        };
        Ok(Connected {
            backend: backend::connect(servers, self.files.as_ref(), &self.device)?,
            disk: DiskCache::locate(server),
        })
    }
}

/// Runs the app until it quits.
pub(crate) fn run(cli: Cli) -> anyhow::Result<()> {
    if crate::headless::wanted(&cli) {
        return crate::headless::run(&cli);
    }
    let self_check = cli
        .self_check
        .then(SelfCheck::start)
        .transpose()
        .context("couldn't make the self-check's folder")?;
    let files = self_check
        .as_ref()
        .map_or_else(AppFiles::locate, |check| Some(check.files()));
    if files.is_none() {
        log::warn!("this system has no home directory: the session and theme last only until quit");
    }
    let from_env = std::env::var(SERVER_ENV).ok();
    let file = files.as_ref().map_or_default(AppFiles::read_config);
    let fresh_install = self_check.is_none() && from_env.is_none() && file.server.is_none();
    // The server, and the fallback server the app turns to when it stops answering.
    let url = match &self_check {
        Some(_) => Servers::only(SelfCheck::server()?),
        None => config::servers(from_env.as_deref(), &file),
    };
    log::info!("talking to {url}");
    let view = ViewFile::read(files.as_ref());
    let kept = Kept::read(
        DiskCache::locate(&url.primary.url),
        cli.start_room(view.room),
    );
    let startup = Startup {
        kept,
        view,
        // The self-check keeps its session in memory: nothing in the Keychain.
        backend: backend::connect(
            url,
            files.as_ref().filter(|_| self_check.is_none()),
            &device::name(),
        )
        .context("couldn't start the server's client")?,
        theme: ThemeState::load(files.as_ref()),
        report_size: ReportSize::load(files.as_ref()),
        prefs: KeptPrefs::load(files.clone()),
        files,
        self_check,
        fresh_install,
        pinned: from_env.is_some(),
    };
    let app = gpui_kit::application().with_assets(Assets);
    // A link may come before the window is open (the one that launched the app): it waits.
    let (links_in, links) = shell::link_channel();
    app.on_open_urls(move |urls| links_in.send(urls));
    // The Dock icon clicked: a main window again, or the open one forward.
    // While this copy watches an update start, the click goes to the new copy.
    app.on_reopen(|cx| {
        if !crate::updates::forward_reopen() {
            shell::windows::reopen(cx);
        }
    });
    app.run(move |cx| {
        let files = startup.files.clone();
        if let Err(error) = launch(&cli, startup, links, cx) {
            // Opening the window is the one step here that can fail. GPUI's run loop doesn't
            // hand control back on every platform (on macOS quitting ends the process), so the
            // failure is reported here: on stderr, and in the app's directory for an app
            // started from Finder or the Dock, which has no visible stderr.
            log::error!("{error:#}");
            if let Some(files) = files {
                files.note_launch_failure(&error);
            }
            std::process::exit(1);
        }
    });
    Ok(())
}

/// Emberhut on this computer, for the setup's first step, and how the store moves to it: the
/// hut this app runs when its server binary is found (a hut that runs as the app's child is
/// asked to stop as the app quits), else one already running at the app's address. The
/// self-check has neither a hut nor a move.
fn home_here(
    self_check: bool,
    files: Option<&AppFiles>,
    backend: &Arc<dyn Backend>,
    pinned: bool,
    cx: &mut App,
) -> (Rc<dyn LocalHome>, Reconnect) {
    let dev = || Rc::new(DevHome::new(Arc::clone(backend))) as Rc<dyn LocalHome>;
    if self_check {
        return (dev(), Reconnect::none());
    }
    let connector = AppConnector {
        files: files.cloned(),
        device: device::name(),
    };
    let reconnect = Reconnect::new(Arc::new(connector), pinned.then_some(SERVER_ENV));
    let Some(hut) = LocalHut::system(HutOptions::for_app()).filter(LocalHut::available) else {
        log::info!("no hut's server binary here: the setup uses one already running");
        return (dev(), reconnect);
    };
    let going = hut.clone();
    cx.on_app_quit(move |_| {
        going.let_go();
        async {}
    })
    .detach();
    (Rc::new(HutHome::new(hut)), reconnect)
}

/// The windows: the main ones, then each window of its own's view.
fn install_windows(store: &Entity<Store>, report_size: ReportSize, cx: &mut App) {
    shell::windows::install(store.clone(), report_size, cx);
    crate::ui::sessions::window::install(cx);
    crate::ui::diary::register(cx);
    crate::ui::calendar::register(cx);
    crate::ui::party::register(cx);
    crate::ui::dark::register(cx);
    crate::ui::browser::register(cx);
}

fn launch(cli: &Cli, startup: Startup, links: LinkReceiver, cx: &mut App) -> anyhow::Result<()> {
    // First, so no window opens before an unseen run says where windows go.
    if let Some(unseen) = cli.unseen() {
        shell::unseen::install(unseen, cx).context("this system can't run unseen")?;
    }
    gpui_kit::init(cx);
    assets::register_fonts(cx);
    window::bind_keys(cx);
    menus::install(cx);
    // `--no-prompt` (screenshots, test harnesses) opens without the lock too.
    crate::lock::install(cli.no_prompt, cx);

    let Startup {
        files,
        backend,
        theme,
        report_size,
        kept,
        view,
        prefs,
        self_check,
        fresh_install,
        pinned,
    } = startup;
    cx.set_global(prefs);
    let (room, start) = (cli.start_room(view.room), cli.start_page(view.room));
    let update_files = files.clone();
    let sidebar = KeptPrefs::get(cx).sidebar;
    let linking = Arc::clone(&backend);
    let (home, reconnect) = home_here(self_check.is_some(), files.as_ref(), &backend, pinned, cx);
    let store = cx.new(|cx| Store::new(backend, files, theme, room, home, reconnect, cx));
    // The account, in a build that has one; a build from source makes nothing here.
    crate::account::install(linking, &store, cx);
    store.update(cx, |store, cx| {
        crate::ui::sidebar_places::start_folds(store, sidebar, cx);
        if let Some(report) = &cli.report {
            store.show_report(Some(report.0.clone()), cx);
        }
        if cli.sign_in {
            store.preview_sign_in();
        }
        store.begin_setup(fresh_install, cx);
        // The hut this app talks to, started again when it is out (a child of the last run).
        store.relight_home(cx);
        if let Some(step) = cli.setup {
            store.preview_setup(step.step(), cx);
        }
        #[cfg(debug_assertions)]
        if cli.flaky {
            store.make_flaky();
        }
        if cli.theme.is_some() || cli.appearance.is_some() {
            store.fix_theme(cli.theme.clone(), cli.appearance.map(Appearance::from));
        }
        store.load_theme(cx);
        store.start_view(view);
        store.start_from(kept, cx);
    });
    cx.set_global(Pal::new(store.read(cx).theme().page(), false));
    // An answer still in its Undo window isn't lost to a quit (or to closing the window).
    Store::send_held_on_quit(&store, cx);
    // The windows' frames, the windows of their own that are open, and the sidebar, kept for
    // the next launch.
    cx.on_app_quit(|cx| {
        shell::restore::note_open(cx);
        KeptPrefs::save_on_quit(cx);
        async {}
    })
    .detach();
    install_windows(&store, report_size, cx);
    look::start(cli.look(), cx);
    motion::follow(cx);
    // An unseen run asks nothing: the system's question would show on the person's display.
    notifications::ask(cli.no_prompt || cli.unseen().is_some(), cx);

    let opening = first_window(cli, cx);
    // The windows of their own open at the last quit, behind the main window once the gate
    // opens.
    if !opening.background {
        shell::restore::reopen(cx);
    }
    let (handle, view) = shell::windows::open_main(opening, cx)?;
    // An update waiting on this copy hears once the window has drawn its first frame.
    crate::updates::report_when_drawn(handle, update_files, cx);
    if let Some(check) = self_check {
        check.exit_when_drawn(handle, cx);
    }
    // The page the command line names (or the one left open), in the first window's column;
    // Settings on the page asked for.
    if !matches!(start, Page::Room(_)) {
        let opened = handle.update(cx, |_, window, cx| {
            view.update(cx, |main, cx| main.open(start, window, cx));
        });
        if let Err(error) = opened {
            log::warn!("the window closed before its first page opened: {error}");
        }
    }
    open_asked(cli, handle, &view, cx);
    open_over_page(cli, handle, &store, cx);
    #[cfg(feature = "automation")]
    crate::automation::start(cli, handle, view, cx);
    // Without the automation nothing else needs them: the window lives until it is closed.
    #[cfg(not(feature = "automation"))]
    drop((handle, view));
    // The place the command line names, then the links the system hands over.
    for target in cli.targets() {
        shell::go(&target, cx);
    }
    shell::follow_links(links, cx);
    if !cli.background && !cli.self_check {
        shell::unseen::activate_app(cx);
    }
    Ok(())
}

/// Where and how the first window opens: on the display and at the size the command line
/// names, else where it was left; in the background for `--background` and an unseen run
/// (which places it beside the displays besides).
fn first_window(cli: &Cli, cx: &App) -> shell::windows::Opening {
    let display = cli
        .display
        .and_then(|n| cx.displays().get(n).map(|display| display.id()));
    let bounds = cli.size.map(|size| {
        WindowBounds::Windowed(Bounds::centered(
            display,
            gpui_kit::size(px(size.width), px(size.height)),
            cx,
        ))
    });
    shell::windows::Opening {
        bounds: bounds.or_else(|| display.map(|display| first_bounds(display, cx))),
        display,
        background: cli.background || cli.unseen().is_some(),
    }
}

/// What the command line asks the first window to show over its page: a project's page, and
/// the spotlight over the window (screenshots).
fn open_asked(
    cli: &Cli,
    handle: gpui_kit::AnyWindowHandle,
    view: &Entity<window::MainWindow>,
    cx: &mut App,
) {
    let (project, spotlight) = (cli.project, cli.spotlight);
    if project.is_none() && !spotlight {
        return;
    }
    let opened = handle.update(cx, |_, window, cx| {
        view.update(cx, |main, cx| {
            if let Some(id) = project {
                main.open(
                    Page::Project(emberhut_api::ids::ProjectId::new(id)),
                    window,
                    cx,
                );
            }
            if spotlight {
                main.open_spotlight(window, cx);
            }
        });
    });
    if let Err(error) = opened {
        log::warn!("the window closed before what the command line asked opened: {error}");
    }
}

/// The first launch's bounds, centred on `display`.
fn first_bounds(display: gpui_kit::DisplayId, cx: &App) -> WindowBounds {
    let (width, height) = prefs::FIRST_SIZE;
    WindowBounds::Windowed(Bounds::centered(
        Some(display),
        gpui_kit::size(px(width), px(height)),
        cx,
    ))
}

/// What the command line asks the first window to show over Settings (one of its pages, the
/// machines, an environment's page, a Chrome profile's sheet, the archived rooms unfolded)
/// or beside the chat (a session's page), each in the window's column so Back returns.
fn open_over_page(
    cli: &Cli,
    handle: gpui_kit::AnyWindowHandle,
    store: &Entity<Store>,
    cx: &mut App,
) {
    let mut pages: Vec<Page> = cli.over_settings().into_iter().collect();
    store.update(cx, |store, cx| {
        if let Some(id) = &cli.environment {
            pages.push(Page::Environment(store.environment_key(id.clone())));
        }
        if let Some(id) = &cli.session {
            let place = SessionPlace::Session(SessionRef::whole(id.clone()));
            pages.push(Page::Session(store.session_key(place)));
        }
        if let Some(id) = cli.chrome_profile {
            store.open_chrome_profile(emberhut_api::ids::ChromeProfileId::new(id), cx);
        }
        if cli.settings == Some(SettingsArg::ArchivedRooms) && !store.archived_open() {
            store.toggle_archived(cx);
        }
    });
    if pages.is_empty() {
        return;
    }
    let opened = handle.update(cx, |_, window, cx| {
        for page in pages {
            crate::ui::column::push(page, window, cx);
        }
    });
    if let Err(error) = opened {
        log::warn!("the window closed before the page the command line asked opened: {error}");
    }
}
