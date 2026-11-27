//! A main window: the sidebar, the chat with the window's own column beside it, the report
//! panel, the answer toast, and the window-wide shortcuts (⌘0…⌘9 the rooms, ⌃⌘S the sidebar as
//! on every Mac app, ⌘[ Back, ⌘⇧↩ the report filling the window beside the sidebar, Esc the
//! report or the column; Ctrl on Linux and Windows, where the sidebar is Ctrl+Shift+S).

use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, DispatchPhase, Div, Entity, FocusHandle, KeyBinding, MouseDownEvent,
    MouseEvent, MouseMoveEvent, Pixels, ScrollEvent, ScrollHandle, Window, actions, canvas, div,
    point, px,
};

use super::chat::ChatView;
use super::chat::ask::ShowRooms;
use super::column::{self, Column};
use super::files::status_line;
use super::pairing::PairingForm;
use super::palette::{Pal, pal};
use super::pause::PauseSheet;
use super::settings::SettingsView;
use super::spotlight::{OpenSpotlight, SpotlightView};
use super::task_page::{self, TaskView};
use super::tasks::TasksView;
use super::viewer::{self, FileViewer, OpenInViewer};
use super::{gate, machines, needs, report, sign_in, toast};
use crate::assets::UI_FONT;
use crate::menus;
use crate::prefs::Kept;
use crate::shell::{presence, windows};
use crate::store::files::{Kind, RemoteFile};
use crate::store::gate::Gate;
use crate::store::rooms::Shortcut;
use crate::store::{OpenReport, Page, Store};

mod expansion;

/// The page's selector, for the tests (a no-op in a normal build).
pub(super) const PAGE: &str = "page";

/// The column's selector, for the tests.
pub(super) const COLUMN: &str = "column";

/// The chat's selector, for the tests.
pub(super) const CHAT: &str = "chat";

actions!(
    window,
    [
        /// Opens Main.
        Room0,
        /// Opens the first listed room.
        Room1,
        /// Opens the second listed room.
        Room2,
        /// Opens the third listed room.
        Room3,
        /// Opens the fourth listed room.
        Room4,
        /// Opens the fifth listed room.
        Room5,
        /// Opens the sixth listed room.
        Room6,
        /// Opens the seventh listed room.
        Room7,
        /// Opens the eighth listed room.
        Room8,
        /// Opens the ninth listed room.
        Room9,
        /// Shows or hides the sidebar.
        ToggleSidebar,
        /// Closes the report beside the page (Esc does it too, before the column).
        CloseReport,
        /// Lets the report fill the window beside the sidebar, or brings it back to its width.
        ToggleReportExpanded,
        /// Brings an expanded report back to its width.
        RestoreReport,
        /// Opens Needs you.
        OpenNeeds,
        /// Opens Tasks.
        OpenTasks,
        /// Opens Machines.
        OpenMachines,
        /// Opens Settings.
        OpenSettings,
        /// The task page's main action: its first answer, Done, Unarchive.
        TaskPrimary,
        /// Not now on the task page: put off a week.
        TaskNotNow,
        /// Archive on the task page: put away for good.
        TaskArchive,
    ]
);

/// Archive's shortcut: ⌃⌘A on the Mac, Ctrl+Alt+A elsewhere (Ctrl stands in for ⌘ there).
#[cfg(target_os = "macos")]
const ARCHIVE_KEYS: &str = "ctrl-cmd-a";
/// Archive's shortcut: ⌃⌘A on the Mac, Ctrl+Alt+A elsewhere (Ctrl stands in for ⌘ there).
#[cfg(not(target_os = "macos"))]
const ARCHIVE_KEYS: &str = "ctrl-alt-a";

/// ⌘ on the Mac, Ctrl elsewhere.
#[cfg(target_os = "macos")]
macro_rules! primary {
    ($key:literal) => {
        concat!("cmd-", $key)
    };
}
/// ⌘ on the Mac, Ctrl elsewhere.
#[cfg(not(target_os = "macos"))]
macro_rules! primary {
    ($key:literal) => {
        concat!("ctrl-", $key)
    };
}

/// Shows or hides the sidebar: ⌃⌘S, the Mac's own shortcut for it (View › Show Sidebar).
#[cfg(target_os = "macos")]
const SIDEBAR_KEY: &str = "ctrl-cmd-s";
/// Shows or hides the sidebar: Ctrl+Shift+S (Ctrl+S alone is Save in most apps).
#[cfg(not(target_os = "macos"))]
const SIDEBAR_KEY: &str = "ctrl-shift-s";

/// Binds the window's shortcuts.
pub(crate) fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new(primary!("0"), Room0, None),
        KeyBinding::new(primary!("1"), Room1, None),
        KeyBinding::new(primary!("2"), Room2, None),
        KeyBinding::new(primary!("3"), Room3, None),
        KeyBinding::new(primary!("4"), Room4, None),
        KeyBinding::new(primary!("5"), Room5, None),
        KeyBinding::new(primary!("6"), Room6, None),
        KeyBinding::new(primary!("7"), Room7, None),
        KeyBinding::new(primary!("8"), Room8, None),
        KeyBinding::new(primary!("9"), Room9, None),
        KeyBinding::new(SIDEBAR_KEY, ToggleSidebar, None),
        KeyBinding::new(primary!(","), OpenSettings, None),
        KeyBinding::new(primary!("shift-enter"), ToggleReportExpanded, None),
        KeyBinding::new("escape", CloseReport, None),
        // After Close, so Esc tries it first. It is handled only while the report is expanded
        // (see `render`); otherwise GPUI goes on to the next binding and Esc closes the report
        // as before.
        KeyBinding::new("escape", RestoreReport, None),
        KeyBinding::new(primary!("d"), TaskPrimary, None),
        KeyBinding::new(primary!("l"), TaskNotNow, None),
        KeyBinding::new(ARCHIVE_KEYS, TaskArchive, None),
    ]);
    column::bind_keys(cx);
    super::sidebar_keys::bind_keys(cx);
    super::pause::bind_keys(cx);
    super::chat::bind_keys(cx);
    super::settings::parts::bind_keys(cx);
    cx.bind_keys([KeyBinding::new(primary!("k"), OpenSpotlight, None)]);
    super::sessions::bind_keys(cx);
    super::browser::bind_keys(cx);
    viewer::bind_keys(cx);
}

/// `element` with the window's shortcuts held: each that reaches it goes no further, so a
/// sheet with the keyboard keeps the window from changing under it.
pub(super) fn hold_shortcuts<E: InteractiveElement>(element: E) -> E {
    macro_rules! hold {
        ($element:expr, $($action:ty),+ $(,)?) => {
            $element$(.on_action(|_: &$action, _: &mut Window, _: &mut App| {}))+
        };
    }
    hold!(
        element,
        Room0,
        Room1,
        Room2,
        Room3,
        Room4,
        Room5,
        Room6,
        Room7,
        Room8,
        Room9,
        ToggleSidebar,
        CloseReport,
        ToggleReportExpanded,
        RestoreReport,
        OpenNeeds,
        OpenTasks,
        OpenMachines,
        OpenSettings,
        column::GoBack,
        column::CloseColumn,
        TaskPrimary,
        TaskNotNow,
        TaskArchive,
    )
}

/// The window's root view.
#[expect(
    clippy::struct_excessive_bools,
    reason = "the sidebar's toggle, its folds and the spotlight are independent of each other"
)]
pub(crate) struct MainWindow {
    pub(super) store: Entity<Store>,
    /// This window's column: the page beside the chat and the ones under it.
    pub(super) column: Entity<Column>,
    /// The page the column showed when last drawn, so a page shown afresh starts at its top.
    column_shown: Option<Page>,
    pub(super) chat: Entity<ChatView>,
    settings: Entity<SettingsView>,
    pairing: Entity<PairingForm>,
    pub(super) sidebar_open: bool,
    pub(super) more_open: bool,
    /// Whether Recent sessions is unfolded.
    pub(super) sessions_open: bool,
    pub(super) page_scroll: ScrollHandle,
    pub(super) report_scroll: ScrollHandle,
    /// The report panel's web view.
    pub(super) report_body: report::ReportBody,
    /// The report panel's width, and whether it fills the window beside the sidebar.
    pub(super) report_size: report::ReportSize,
    /// What the window last showed besides the report (an expanded report comes back when it
    /// changes).
    report_beside: expansion::Beside,
    theme_revision: u64,
    /// The window's own focus, so the shortcuts and the menu bar reach these handlers on a page
    /// with no field focused (GPUI sends them down the focused element's path).
    focus: FocusHandle,
    /// "Pause until…", over the window while a room's pause day is picked.
    pause: Entity<PauseSheet>,
    /// The sidebar's focus: it keeps the keyboard after a click, for its arrows.
    pub(super) sidebar_focus: FocusHandle,
    /// The Tasks page.
    tasks: Entity<TasksView>,
    /// A task's page, for the task the column shows.
    task_page: Entity<TaskView>,
    /// The spotlight (⌘K), over the window while it is up.
    pub(super) spotlight: Entity<SpotlightView>,
    pub(super) spotlight_open: bool,
    /// The Sessions screen.
    sessions: Entity<super::sessions::SessionsView>,
    /// A session's page, for the session the column shows.
    session_page: Entity<super::sessions::SessionPage>,
    /// The file viewer, a sheet over the window while a file is viewed.
    viewer: Entity<FileViewer>,
    /// The two-finger swipes over the rows of a place's page (a goal's to-dos).
    pub(super) row_swipes: crate::ui::tasks::swipe::RowSwipes,
}

impl MainWindow {
    /// The window over `store`, its report panel at `report_size`.
    pub(crate) fn new(
        store: Entity<Store>,
        report_size: report::ReportSize,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Self {
        cx.observe(&store, |_, _, cx| cx.notify()).detach();
        let column = Column::new(store.clone(), window, cx);
        cx.observe_in(&column, window, Self::column_moved).detach();
        Self::follow_for_report(&store, window, cx);
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        let pairing = cx.new(|cx| PairingForm::new(store.clone(), window, cx));
        let chat = cx.new(|cx| ChatView::new(store.clone(), window, cx));
        let settings = cx.new(|cx| SettingsView::new(store.clone(), pairing.clone(), window, cx));
        let pause = cx.new(|cx| PauseSheet::new(store.clone(), window, cx));
        let tasks = cx.new(|cx| TasksView::new(store.clone(), window, cx));
        let task_page = cx.new(|cx| TaskView::new(store.clone(), column.clone(), window, cx));
        let spotlight = cx.new(|cx| SpotlightView::new(store.clone(), window, cx));
        cx.subscribe_in(&spotlight, window, Self::spotlight_asked)
            .detach();
        let sessions = cx.new(|cx| super::sessions::SessionsView::new(store.clone(), cx));
        let session_page =
            cx.new(|cx| super::sessions::SessionPage::new(store.clone(), &column, window, cx));
        let viewer = cx.new(|cx| FileViewer::new(store.clone(), cx));
        // The viewer closed: the window takes the keyboard back, for its shortcuts.
        cx.subscribe_in(
            &viewer,
            window,
            |this, _, _: &viewer::Closed, window, cx| {
                this.keep_focus(window, cx);
            },
        )
        .detach();
        // A room on screen is read only while a main window is the active one; out of sight
        // (minimised, the app hidden, covered) the store stops polling. The store hears every
        // main window's state summed (`shell::presence`).
        let id = window.window_handle().window_id();
        presence::set_visible(id, window.is_visible(), &store, cx);
        presence::set_active(id, window.is_window_active(), &store, cx);
        cx.observe_window_activation(window, move |this, window, cx| {
            let active = window.is_window_active();
            if active {
                windows::note_front(window.window_handle(), cx);
            }
            presence::set_active(id, active, &this.store, cx);
            menus::refresh(this.sidebar_open, cx);
        })
        .detach();
        cx.observe_window_visibility(window, move |this, visibility, _, cx| {
            presence::set_visible(id, visibility.is_visible(), &this.store, cx);
        })
        .detach();
        // Its size and place, kept for the next launch.
        cx.observe_window_bounds(window, |_, window, cx| {
            Kept::note_frame(cx, window.window_bounds());
        })
        .detach();
        // The user at the keyboard (the pointer is watched by `input_watch`).
        cx.observe_keystrokes(|this, _, _, cx| this.store.update(cx, Store::note_input))
            .detach();
        // While the answer toast offers Undo, its shortcut presses it.
        toast::undo_on_shortcut(&store, cx).detach();
        // Follow the system's light or dark while the theme does.
        cx.observe_window_appearance(window, |this, window, cx| {
            let page = this.store.read(cx).theme().page().clone();
            super::look::apply(&page, window, cx);
            cx.notify();
        })
        .detach();
        // A new theme from the server or Settings: into the palette and gpui-kit's theme.
        cx.observe_in(&store, window, |this, store, window, cx| {
            let theme = store.read(cx).theme();
            if theme.revision() != this.theme_revision {
                this.theme_revision = theme.revision();
                let page = theme.page().clone();
                super::look::apply(&page, window, cx);
                window.refresh();
            }
        })
        .detach();
        let report_beside = expansion::Beside::of(store.read(cx), None);
        let kept = Kept::get(cx).sidebar;
        Self {
            store,
            column,
            column_shown: None,
            chat,
            settings,
            pairing,
            sidebar_open: true,
            more_open: kept.more_open,
            sessions_open: kept.sessions_open,
            page_scroll: ScrollHandle::new(),
            report_scroll: ScrollHandle::new(),
            report_body: report::ReportBody::default(),
            report_beside,
            report_size,
            theme_revision: 0,
            focus,
            pause,
            sidebar_focus: cx.focus_handle(),
            tasks,
            task_page,
            spotlight,
            spotlight_open: false,
            sessions,
            session_page,
            viewer,
            row_swipes: crate::ui::tasks::swipe::RowSwipes::default(),
        }
    }

    /// The store the window shows.
    #[cfg(feature = "automation")]
    #[must_use]
    pub(crate) fn store(&self) -> &Entity<Store> {
        &self.store
    }

    /// The chat view.
    #[cfg(feature = "automation")]
    #[must_use]
    pub(crate) fn chat(&self) -> &Entity<ChatView> {
        &self.chat
    }

    /// Shows `page` as the sidebar would: a room's chat, or a page afresh in the column.
    #[cfg(feature = "automation")]
    pub(crate) fn show(&mut self, page: Page, cx: &mut Context<'_, Self>) {
        match page {
            Page::Room(room) => self
                .store
                .update(cx, |store, cx| store.switch_room(room, cx)),
            page => self.column.update(cx, |column, cx| column.open(page, cx)),
        }
    }

    /// The page this window's column shows, if it is open.
    #[cfg(feature = "automation")]
    #[must_use]
    pub(crate) fn column_page(&self, cx: &App) -> Option<Page> {
        self.column.read(cx).current()
    }

    /// Opens a page afresh (at its top) in this window's column, or a room's chat beside
    /// whatever the column shows. Choosing a page, even the one showing, brings an expanded
    /// report back, so the choice is seen.
    pub(crate) fn open(&mut self, page: Page, window: &mut Window, cx: &mut Context<'_, Self>) {
        // With the column in the chat's place, a room chosen brings its chat back.
        let chat_hidden =
            self.column.read(cx).current().is_some() && self.room_for_column(window, cx).is_none();
        if let Page::Room(room) = page {
            self.store
                .update(cx, |store, cx| store.switch_room(room, cx));
            if chat_hidden {
                self.column.update(cx, Column::close);
            }
            // A room opened: its composer takes the keyboard, to write in it at once.
            self.focus_composer(window, cx);
        } else {
            self.column.update(cx, |column, cx| column.open(page, cx));
        }
        self.restore_report(window, cx);
    }

    /// The column changed: a page shown afresh starts at its top, and the window keeps the
    /// keyboard if the page that had it went.
    fn column_moved(&mut self, _: Entity<Column>, window: &mut Window, cx: &mut Context<'_, Self>) {
        let shown = self.column.read(cx).current();
        if shown != self.column_shown {
            self.column_shown = shown;
            self.page_scroll.set_offset(point(px(0.), px(0.)));
        }
        // The composer may have just taken the keyboard (a room chosen as the column shut),
        // before the chat is drawn again: it keeps it.
        if !self.chat.read(cx).composer_focused(window, cx) {
            self.keep_focus(window, cx);
        }
        self.note_beside(window, cx);
        cx.notify();
    }

    /// Whether the viewer's sheet is over the window.
    #[must_use]
    pub(super) fn viewer_shows(&self, cx: &App) -> bool {
        self.viewer.read(cx).is_open()
    }

    /// Opens `file` in the viewer over the window; a report opens in the panel beside the page.
    fn view_file(&mut self, file: RemoteFile, window: &mut Window, cx: &mut Context<'_, Self>) {
        if file.kind() == Kind::Report {
            let report = OpenReport {
                name: file.name,
                path: file.path,
            };
            self.store
                .update(cx, |store, cx| store.show_report(Some(report), cx));
            return;
        }
        self.viewer
            .update(cx, |viewer, cx| viewer.show(file, window, cx));
    }

    /// Shows or hides the sidebar; the View menu's item says which it will do next.
    pub(super) fn toggle_sidebar(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.sidebar_open = !self.sidebar_open;
        let hidden = !self.sidebar_open;
        self.chat
            .update(cx, |chat, cx| chat.set_sidebar_hidden(hidden, cx));
        // With the sidebar hidden nothing is translucent: the window stops blurring.
        let page = self.store.read(cx).theme().page().clone();
        super::look::show_sidebar(self.sidebar_open, &page, window, cx);
        menus::refresh(self.sidebar_open, cx);
        cx.notify();
    }

    /// The window's width beside the sidebar (as wide as it was dragged, its border included)
    /// when it shows.
    #[must_use]
    pub(crate) fn beside_sidebar(&self, window: &Window, cx: &App) -> Pixels {
        let width = window.viewport_size().width;
        if self.sidebar_open {
            width - px(self.store.read(cx).sidebar_width())
        } else {
            width
        }
    }

    /// The column's width while it shows: a page is open in it, no report is expanded over the
    /// chat, and the chat (and a report beside it) leave it room ([`task_page::column_width`]:
    /// one rule for every page).
    #[must_use]
    pub(crate) fn column_width(&self, window: &Window, cx: &App) -> Option<Pixels> {
        let store = self.store.read(cx);
        let open = self.column.read(cx).current().is_some();
        if self.report_size.is_expanded() || store.gate() != Gate::Open || !open {
            return None;
        }
        self.room_for_column(window, cx)
    }

    /// The width the column has beside the chat (and a report open beside both), if it fits
    /// there at all, whether or not it is open: the width its edge was dragged to, else the
    /// standard one.
    #[must_use]
    fn room_for_column(&self, window: &Window, cx: &App) -> Option<Pixels> {
        let dragged = Kept::get(cx).column_width.map(px);
        self.column_width_for(window, dragged, cx)
    }

    /// The width the column has beside the chat (and a report open beside both) if it is asked
    /// to be `dragged` wide (`None`: the standard width), if it fits there at all.
    #[must_use]
    pub(crate) fn column_width_for(
        &self,
        window: &Window,
        dragged: Option<Pixels>,
        cx: &App,
    ) -> Option<Pixels> {
        task_page::column_width(
            window.viewport_size().width,
            self.beside_sidebar(window, cx),
            self.store.read(cx).open_report().is_some(),
            dragged,
        )
    }

    /// Whether the column shows a task's page now.
    #[must_use]
    pub(crate) fn shows_task_page(&self, cx: &App) -> bool {
        matches!(self.column.read(cx).current(), Some(Page::Task(_))) && self.column_shows(cx)
    }

    /// Whether the column shows: something is open in it, past the gate, and no report is
    /// expanded over the page. It shows beside the chat at [`Self::column_width`], else (no
    /// room for it there) in the chat's place.
    #[must_use]
    pub(crate) fn column_shows(&self, cx: &App) -> bool {
        let open = self.column.read(cx).current().is_some();
        !self.report_size.is_expanded() && self.store.read(cx).gate() == Gate::Open && open
    }

    /// The page shown may have dropped the field that had focus: the window takes it back, so
    /// the shortcuts keep working.
    pub(super) fn keep_focus(&self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if !self.focus.contains_focused(window, cx) {
            window.focus(&self.focus, cx);
        }
    }

    fn open_shortcut(
        &mut self,
        shortcut: Shortcut,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let room = self.store.read(cx).switcher().room_for(shortcut);
        match room {
            Some(room) => self.open(Page::Room(room), window, cx),
            // No room for it: still a choice of the page, so an expanded report comes back.
            None => self.restore_report(window, cx),
        }
    }

    /// The chat, with the column beside it when a page is open there.
    fn content(&mut self, window: &Window, cx: &mut Context<'_, Self>) -> AnyElement {
        let p = pal(cx);
        let store = self.store.read(cx);
        match store.gate() {
            Gate::NotYourDevice => {
                return gate::not_your_device(store.server().as_str(), &p).into_any_element();
            }
            Gate::SignIn { can_wait } => {
                return sign_in::screen(&self.pairing, &self.store, can_wait, &p)
                    .into_any_element();
            }
            Gate::Connecting => {
                return gate::connecting(store.server().as_str(), store.offline(), &p)
                    .into_any_element();
            }
            Gate::Open => {}
        }
        let chat = self.chat.clone().into_any_element();
        let chat_alone = |chat| {
            div()
                .debug_selector(|| CHAT.to_owned())
                .size_full()
                .child(chat)
                .into_any_element()
        };
        if !self.column_shows(cx) {
            return chat_alone(chat);
        }
        let width = self.column_width(window, cx);
        let Some(page) = self.column.read(cx).current() else {
            return chat_alone(chat);
        };
        let body = self.page_view(page, cx);
        column::frame::beside(self, page, chat, body, width, cx).into_any_element()
    }

    /// A page other than the chat, as the column shows it (a new page's view goes here).
    fn page_view(&mut self, page: Page, cx: &mut Context<'_, Self>) -> AnyElement {
        let scroll = self.page_scroll.clone();
        match page {
            Page::Room(_) => self.chat.clone().into_any_element(),
            Page::Needs => needs::page(self, &scroll, cx).into_any_element(),
            Page::Tasks => self.tasks.clone().into_any_element(),
            Page::Machines => machines::page(self, &scroll, cx).into_any_element(),
            Page::Settings => self.settings.clone().into_any_element(),
            Page::Task(_) => self.task_page.clone().into_any_element(),
            Page::Project(_) | Page::Goal(_) | Page::Person(_) | Page::Report => {
                super::places::page(self, page, &scroll, cx)
            }
            Page::Sessions => self.sessions.clone().into_any_element(),
            Page::Session(_) => self.session_page.clone().into_any_element(),
            Page::Environment(key) => {
                super::settings::environment::page(&self.store, key, &scroll, &pal(cx), cx)
                    .into_any_element()
            }
            Page::Jobs => {
                super::working::jobs::page(&self.store, &scroll, &pal(cx), cx).into_any_element()
            }
            Page::Job(key) => super::working::job::page(&self.store, key, &scroll, &pal(cx), cx)
                .into_any_element(),
            Page::Scheduled => super::scheduled::page(self, &scroll, cx).into_any_element(),
            Page::Approvals(filter) => {
                super::approvals::list::page(&self.store, filter, &scroll, &pal(cx), cx)
                    .into_any_element()
            }
            Page::Approval(id) => {
                super::approvals::detail::page(&self.store, id, &scroll, &pal(cx), cx)
                    .into_any_element()
            }
            Page::SettingsPane(part) => self
                .settings
                .read(cx)
                .pane(part, &pal(cx))
                .into_any_element(),
        }
    }

    /// The page beside the sidebar, with the answer toast over it.
    fn page(&mut self, p: &Pal, window: &Window, cx: &mut Context<'_, Self>) -> Div {
        let content = self.content(window, cx);
        let toast = toast::toast(self, cx).map(toast::over_page);
        div()
            .debug_selector(|| PAGE.to_owned())
            .flex_1()
            .min_w_0()
            .h_full()
            .relative()
            .bg(p.surface)
            .child(content)
            .children(toast)
    }

    fn with_actions(div: Div, cx: &mut Context<'_, Self>) -> Div {
        let [k0, k1, k2, k3, k4, k5, k6, k7, k8, k9] = Shortcut::ALL;
        div.on_action(
            cx.listener(move |this, _: &Room0, window, cx| this.open_shortcut(k0, window, cx)),
        )
        .on_action(
            cx.listener(move |this, _: &Room1, window, cx| this.open_shortcut(k1, window, cx)),
        )
        .on_action(
            cx.listener(move |this, _: &Room2, window, cx| this.open_shortcut(k2, window, cx)),
        )
        .on_action(
            cx.listener(move |this, _: &Room3, window, cx| this.open_shortcut(k3, window, cx)),
        )
        .on_action(
            cx.listener(move |this, _: &Room4, window, cx| this.open_shortcut(k4, window, cx)),
        )
        .on_action(
            cx.listener(move |this, _: &Room5, window, cx| this.open_shortcut(k5, window, cx)),
        )
        .on_action(
            cx.listener(move |this, _: &Room6, window, cx| this.open_shortcut(k6, window, cx)),
        )
        .on_action(
            cx.listener(move |this, _: &Room7, window, cx| this.open_shortcut(k7, window, cx)),
        )
        .on_action(
            cx.listener(move |this, _: &Room8, window, cx| this.open_shortcut(k8, window, cx)),
        )
        .on_action(
            cx.listener(move |this, _: &Room9, window, cx| this.open_shortcut(k9, window, cx)),
        )
        .on_action(
            cx.listener(|this, _: &ToggleSidebar, window, cx| this.toggle_sidebar(window, cx)),
        )
        .on_action(
            cx.listener(|this, _: &OpenNeeds, window, cx| this.open(Page::Needs, window, cx)),
        )
        .on_action(
            cx.listener(|this, _: &OpenTasks, window, cx| this.open(Page::Tasks, window, cx)),
        )
        .on_action(cx.listener(|this, _: &OpenMachines, window, cx| {
            this.more_open = true;
            this.open(Page::Machines, window, cx);
        }))
        .on_action(cx.listener(|this, _: &OpenSettings, window, cx| {
            this.more_open = true;
            this.open(Page::Settings, window, cx);
        }))
        // Esc closes the report first, then the column (while it shows).
        .on_action(cx.listener(|this, _: &CloseReport, window, cx| {
            if this.store.read(cx).open_report().is_some() {
                this.store
                    .update(cx, |store, cx| store.show_report(None, cx));
            } else if this.column_shows(cx) {
                this.close_column(window, cx);
            }
        }))
        // The task page's keys act only on a task page that shows.
        .on_action(cx.listener(|this, _: &TaskPrimary, window, cx| {
            if this.shows_task_page(cx) {
                this.task_page
                    .update(cx, |page, cx| page.press_primary(window, cx));
            }
        }))
        .on_action(cx.listener(|this, _: &TaskNotNow, window, cx| {
            if this.shows_task_page(cx) {
                this.task_page
                    .update(cx, |page, cx| page.press_not_now(window, cx));
            }
        }))
        .on_action(cx.listener(|this, _: &TaskArchive, window, cx| {
            if this.shows_task_page(cx) {
                this.task_page
                    .update(cx, |page, cx| page.press_archive(window, cx));
            }
        }))
        .on_action(cx.listener(|this, _: &ToggleReportExpanded, window, cx| {
            this.toggle_report_expanded(window, cx);
        }))
        .on_action(cx.listener(|this, _: &OpenSpotlight, window, cx| {
            this.open_spotlight(window, cx);
        }))
        .on_action(cx.listener(|this, action: &OpenInViewer, window, cx| {
            this.view_file(action.file.clone(), window, cx);
        }))
        .on_action(cx.listener(|this, _: &ShowRooms, window, cx| {
            if !this.sidebar_open {
                window.dispatch_action(Box::new(ToggleSidebar), cx);
            }
        }))
    }
}

/// An empty element that tells the store whenever the pointer moves, clicks or scrolls anywhere
/// in the window. It listens to the window's events in their capture phase: an element's own
/// listeners hear only what reaches its hitbox, which a popover or menu on top hides.
fn input_watch(store: &Entity<Store>) -> impl IntoElement {
    let store = store.clone();
    canvas(
        |_, _, _| {},
        move |_, (), window, _| {
            note_input_on::<MouseMoveEvent>(window, &store);
            note_input_on::<MouseDownEvent>(window, &store);
            note_input_on::<ScrollEvent>(window, &store);
        },
    )
    .absolute()
    .size_0()
}

/// Tells `store` of each `E` the window gets. A window's listeners last one frame:
/// `input_watch` registers them again each time it paints.
fn note_input_on<E: MouseEvent>(window: &mut Window, store: &Entity<Store>) {
    let store = store.clone();
    window.on_mouse_event(move |_: &E, phase, _, cx| {
        if phase == DispatchPhase::Capture {
            store.update(cx, Store::note_input);
        }
    });
}

impl Render for MainWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let p = pal(cx);
        // Locked, the window shows the lock's cover and nothing of the chat; the report's web
        // view and the viewer's video or document are native, so they are dropped, as they
        // would show (and play) over the cover.
        if let Some(cover) = super::lock::cover(&p, cx) {
            self.report_body.clear();
            self.viewer
                .update(cx, |viewer, _| viewer.let_go_of_native());
            return cover.into_any_element();
        }
        // The column in the chat's place takes the composer away: the window keeps the
        // keyboard, so the shortcuts still reach it.
        if self.column_shows(cx)
            && self.column_width(window, cx).is_none()
            && self.chat.read(cx).composer_focused(window, cx)
        {
            window.focus(&self.focus, cx);
        }
        // Behind the gate (not this device, sign-in, connecting) the window shows only its
        // screen: no sidebar, no report.
        let open = self.store.read(cx).gate() == Gate::Open;
        let panel = if open {
            report::panel(self, window, cx)
        } else {
            // A web view left over is native: dropped, not merely left undrawn.
            self.report_body = report::ReportBody::default();
            None
        };
        // Only once the panel is laid out: closed, it is never expanded.
        let expanded = self.report_size.is_expanded();
        // An expanded report takes the page's place.
        let page = (!expanded).then(|| self.page(&p, window, cx));
        // No fill of its own: the sidebar, the page and the report each fill their part, so
        // the window's material shows only through the sidebar. The row holds those three and
        // nothing else that takes room (no gap, no padding): the report panel's bounds are worked
        // out from the window's width less the sidebar's (`report::space`).
        let root = div()
            .track_focus(&self.focus)
            .relative()
            .size_full()
            .flex()
            .text_color(p.text)
            .font_family(UI_FONT)
            .text_size(px(13.));
        Self::with_column_actions(Self::with_actions(root, cx), cx)
            .when(open && self.sidebar_open, |d| {
                d.child(super::sidebar::sidebar(self, cx))
            })
            .children(page)
            .children(panel)
            // Esc brings an expanded report back; otherwise it isn't handled here and goes on to
            // close the report (see `bind_keys`).
            .when(expanded, |d| {
                d.on_action(cx.listener(|this, _: &RestoreReport, window, cx| {
                    this.restore_report(window, cx);
                }))
            })
            .when(open && self.spotlight_open, |d| {
                d.child(self.spotlight.clone())
            })
            .child(self.pause.clone())
            .child(self.viewer.clone())
            .children(status_line::view(&self.store, &p, cx))
            .child(input_watch(&self.store))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use emberhut_api::ids::RoomId;
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{Modifiers, TestAppContext, VisualTestContext, WindowVisibility};

    use super::*;
    use crate::backend::fake::FakeBackend;
    use crate::store::presence::Presence;
    use crate::store::reading::AWAY_AFTER;
    use crate::ui::palette::Pal;

    #[gpui_kit::test]
    fn the_pause_sheet_holds_the_task_pages_keys(cx: &mut TestAppContext) {
        use emberhut_api::ids::{ProjectId, TaskId};
        use gpui_kit::size;
        use serde_json::json;

        use crate::backend::fake::Call;
        use crate::store::answers::UNDO_WINDOW;
        use crate::store::room_actions::Pausing;

        cx.update(|cx| {
            gpui_kit::init(cx);
            cx.set_global(Pal::new(&crate::theme::built_in(), false));
            bind_keys(cx);
        });
        let backend = Arc::new(FakeBackend::new([]));
        // A to-do task of the user's: its main action is Mark done.
        backend.put_task(crate::fixtures::task_page(5, json!({})));
        let shared = Arc::clone(&backend) as Arc<dyn crate::backend::Backend>;
        let store = cx.new(|_| Store::for_tests(shared));
        let (view, cx) = cx.add_window_view(|window, cx| {
            MainWindow::new(store.clone(), report::ReportSize::default(), window, cx)
        });
        cx.simulate_resize(size(px(1440.), px(900.)));
        store.update(cx, |store, cx| {
            store.take_state_for_tests(crate::fixtures::state(json!({})), cx);
        });
        view.update_in(cx, |main, window, cx| {
            main.open_task(TaskId::new(5), window, cx);
        });
        cx.run_until_parked();
        let pausing = Pausing {
            project: ProjectId::new(2),
            name: "Room 2".to_owned(),
        };
        store.update(cx, |store, cx| store.set_pausing(Some(pausing), cx));
        cx.run_until_parked();
        let acted = |backend: &FakeBackend| {
            backend
                .calls()
                .into_iter()
                .any(|call| matches!(call, Call::Act(_) | Call::Decide(..)))
        };

        let primary = if cfg!(target_os = "macos") {
            "cmd-d"
        } else {
            "ctrl-d"
        };
        cx.simulate_keystrokes(primary);
        cx.executor().advance_clock(UNDO_WINDOW * 2);
        cx.run_until_parked();
        assert!(!acted(&backend), "nothing done to the task under the sheet");
        assert!(store.read_with(cx, |store, _| store.answers().gone().is_empty()));

        store.update(cx, |store, cx| store.set_pausing(None, cx));
        cx.run_until_parked();
        cx.simulate_keystrokes(primary);
        cx.executor().advance_clock(UNDO_WINDOW * 2);
        cx.run_until_parked();
        assert!(
            acted(&backend),
            "with the sheet gone, the key marks it done"
        );
    }

    fn reading(store: &Entity<Store>, cx: &VisualTestContext) -> Option<RoomId> {
        store.read_with(cx, Store::reading)
    }

    #[gpui_kit::test]
    fn the_window_tells_the_store_when_the_user_is_in_it(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            cx.set_global(Pal::new(&crate::theme::built_in(), false));
        });
        let backend = Arc::new(FakeBackend::new([])) as Arc<dyn crate::backend::Backend>;
        let store = cx.new(|_| Store::for_tests(backend));
        let window = cx.add_window(|window, cx| {
            MainWindow::new(store.clone(), report::ReportSize::default(), window, cx)
        });
        let mut cx = VisualTestContext::from_window(window.into(), cx);
        cx.run_until_parked();
        assert_eq!(reading(&store, &cx), None, "it opened in the background");

        cx.update(|window, _| window.activate_window());
        cx.run_until_parked();
        assert_eq!(
            reading(&store, &cx),
            Some(RoomId::MAIN),
            "brought to the front"
        );

        cx.executor().advance_clock(AWAY_AFTER);
        assert_eq!(
            reading(&store, &cx),
            None,
            "no one used it for five minutes"
        );
        cx.simulate_keystrokes("a");
        assert_eq!(reading(&store, &cx), Some(RoomId::MAIN), "a key");

        cx.executor().advance_clock(AWAY_AFTER);
        cx.update(Window::render_frame);
        cx.simulate_mouse_move(point(px(40.), px(40.)), None, Modifiers::default());
        assert_eq!(reading(&store, &cx), Some(RoomId::MAIN), "the pointer");

        cx.deactivate_window();
        assert_eq!(reading(&store, &cx), None, "sent to the back");
    }

    #[gpui_kit::test]
    fn behind_the_gate_the_window_shows_only_its_screen(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            cx.set_global(Pal::new(&crate::theme::built_in(), false));
        });
        let backend = Arc::new(FakeBackend::new([])) as Arc<dyn crate::backend::Backend>;
        let store = cx.new(|_| Store::for_tests(backend));
        let window = cx.add_window(|window, cx| {
            MainWindow::new(store.clone(), report::ReportSize::default(), window, cx)
        });
        cx.update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
            assert!(
                window.try_find("assistant-menu").is_none(),
                "connecting: no sidebar"
            );
        })
        .unwrap();
        store.update(cx, |store, cx| {
            store.take_state_for_tests(crate::fixtures::state(serde_json::json!({})), cx);
        });
        cx.update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
            assert!(
                window.try_find("assistant-menu").is_some(),
                "connected: the sidebar"
            );
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn the_window_tells_the_store_when_it_is_out_of_sight(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            cx.set_global(Pal::new(&crate::theme::built_in(), false));
        });
        let backend = Arc::new(FakeBackend::new([])) as Arc<dyn crate::backend::Backend>;
        let store = cx.new(|_| Store::for_tests(backend));
        let window = cx.add_window(|window, cx| {
            MainWindow::new(store.clone(), report::ReportSize::default(), window, cx)
        });
        let mut cx = VisualTestContext::from_window(window.into(), cx);
        let presence = |cx: &VisualTestContext| store.read_with(cx, |store, _| store.presence());
        cx.run_until_parked();
        assert_eq!(
            presence(&cx),
            Presence::Behind,
            "it opened in the background"
        );

        cx.simulate_visibility_change(WindowVisibility::Hidden);
        assert_eq!(presence(&cx), Presence::Hidden, "minimised");
        cx.simulate_visibility_change(WindowVisibility::Visible);
        assert_eq!(presence(&cx), Presence::Behind, "shown again");

        cx.update(|window, _| window.activate_window());
        cx.run_until_parked();
        assert_eq!(presence(&cx), Presence::Front, "brought to the front");
        cx.simulate_visibility_change(WindowVisibility::Hidden);
        assert_eq!(presence(&cx), Presence::Hidden, "the app hidden");
    }
}
