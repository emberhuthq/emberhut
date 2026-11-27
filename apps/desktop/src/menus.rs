//! The menu bar and the app-wide shortcuts a Mac app is expected to have: the app menu (About,
//! Settings, Services, Hide, Quit), File (New Window, Close), Edit for the text fields, View
//! (the sidebar, the pages, Back, full screen), Window (which `AppKit` fills with the open
//! windows and its tiling) and Help. A bare GPUI window has none of these. On Linux and
//! Windows the same actions take Ctrl, and there is no menu bar.
//!
//! Each menu is one function listing its items in order, and each item's action is handled
//! where it is declared (here for the app's own, in the views for theirs): a feature's menu
//! item is one line appended to its menu's list. The menus are set again when what they say
//! changes ([`refresh`]: Show or Hide Sidebar, as the front window has it).

use gpui_kit::component::input::{Copy, Cut, Delete, Paste, Redo, SelectAll, Undo};
use gpui_kit::{
    App, Global, KeyBinding, Menu, MenuItem, OsAction, PromptLevel, SystemMenuType, Window, actions,
};

use crate::brand::{APP_NAME, FULL_NAME};
use crate::shell::windows;
use crate::ui::calendar::OpenCalendar;
use crate::ui::column::GoBack;
use crate::ui::diary::{OpenDiary, OpenQuickNote};
use crate::ui::party::OpenParty;
use crate::ui::window::{
    OpenMachines, OpenNeeds, OpenSettings, OpenTasks, Room0, TaskArchive, TaskNotNow, TaskPrimary,
    ToggleSidebar,
};

/// Where the app's help lives.
const HELP_URL: &str = "https://emberhut.com";

actions!(
    app,
    [
        /// Shows the app's name, version and website.
        About,
        /// Quits the app.
        Quit,
        /// Hides the app.
        Hide,
        /// Hides the other apps.
        HideOthers,
        /// Shows every app again.
        ShowAll,
        /// Closes the front window (on macOS the app keeps running).
        CloseWindow,
        /// Minimizes the window.
        Minimize,
        /// Zooms the window.
        Zoom,
        /// Opens another main window on the same chat.
        NewWindow,
        /// Brings every window of the app to the front.
        BringAllToFront,
        /// Takes the front window to full screen, or back.
        ToggleFullScreen,
        /// Opens the app's help on its website.
        ShowHelp,
        /// Checks for a newer build now, and says what it found.
        CheckForUpdates,
    ]
);

/// What the menus say that changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Shown {
    /// Whether the front window's sidebar is shown (View › Hide Sidebar, else Show Sidebar).
    sidebar: bool,
}

impl Default for Shown {
    fn default() -> Self {
        Self { sidebar: true }
    }
}

impl Global for Shown {}

/// Installs the app's actions, shortcuts and menus.
pub(crate) fn install(cx: &mut App) {
    cx.on_action(|_: &About, cx| with_window(cx, about));
    cx.on_action(|_: &Quit, cx| cx.quit());
    cx.on_action(|_: &Hide, cx| cx.hide());
    cx.on_action(|_: &HideOthers, cx| cx.hide_other_apps());
    cx.on_action(|_: &ShowAll, cx| cx.unhide_other_apps());
    cx.on_action(|_: &CloseWindow, cx| with_window(cx, |window, _| window.remove_window()));
    cx.on_action(|_: &Minimize, cx| with_window(cx, |window, _| window.minimize_window()));
    cx.on_action(|_: &Zoom, cx| with_window(cx, |window, _| window.zoom_window()));
    cx.on_action(|_: &ToggleFullScreen, cx| {
        with_window(cx, |window, _| window.toggle_fullscreen());
    });
    cx.on_action(|_: &NewWindow, cx| {
        if let Err(error) = windows::open_main(windows::Opening::default(), cx) {
            log::error!("{error:#}");
        }
    });
    cx.on_action(|_: &BringAllToFront, cx| bring_all_to_front(cx));
    cx.on_action(|_: &ShowHelp, cx| cx.open_url(HELP_URL));
    cx.on_action(|_: &CheckForUpdates, cx| {
        with_window(cx, crate::ui::update_dialog::check_for_updates);
    });
    // On macOS the app outlives its last window, as Mac apps do: the Dock icon opens one
    // again. Elsewhere GPUI quits when the last window closes, as there is no Dock to come
    // back from.

    #[cfg(target_os = "macos")]
    cx.bind_keys([
        KeyBinding::new("cmd-q", Quit, None),
        KeyBinding::new("cmd-h", Hide, None),
        KeyBinding::new("alt-cmd-h", HideOthers, None),
        KeyBinding::new("cmd-n", NewWindow, None),
        KeyBinding::new("cmd-w", CloseWindow, None),
        KeyBinding::new("cmd-m", Minimize, None),
    ]);
    #[cfg(not(target_os = "macos"))]
    cx.bind_keys([
        KeyBinding::new("ctrl-q", Quit, None),
        KeyBinding::new("ctrl-n", NewWindow, None),
        KeyBinding::new("ctrl-w", CloseWindow, None),
        KeyBinding::new("f11", ToggleFullScreen, None),
    ]);

    cx.set_global(Shown::default());
    cx.set_menus(menus(Shown::default()));
}

/// Sets the menus again if what they say changed: `sidebar`, whether the front window's
/// sidebar is shown.
pub(crate) fn refresh(sidebar: bool, cx: &mut App) {
    let shown = Shown { sidebar };
    if cx.try_global::<Shown>() == Some(&shown) {
        return;
    }
    cx.set_global(shown);
    cx.set_menus(menus(shown));
}

/// The menu bar, in order.
fn menus(shown: Shown) -> Vec<Menu> {
    vec![
        app_menu(),
        file_menu(),
        edit_menu(),
        view_menu(shown),
        task_menu(),
        window_menu(),
        help_menu(),
    ]
}

/// Task: the open task page's actions (they act only while its page shows).
fn task_menu() -> Menu {
    Menu::new("Task").items([
        MenuItem::action("Main Action", TaskPrimary),
        MenuItem::action("Not Now", TaskNotNow),
        MenuItem::action("Archive", TaskArchive),
        MenuItem::separator(),
        MenuItem::action("Back", GoBack),
    ])
}

/// The app's own menu.
fn app_menu() -> Menu {
    Menu::new(APP_NAME).items([
        MenuItem::action(format!("About {APP_NAME}"), About),
        MenuItem::action("Check for Updates…", CheckForUpdates),
        MenuItem::separator(),
        MenuItem::action("Settings…", OpenSettings),
        MenuItem::separator(),
        MenuItem::os_submenu("Services", SystemMenuType::Services),
        MenuItem::separator(),
        MenuItem::action(format!("Hide {APP_NAME}"), Hide),
        MenuItem::action("Hide Others", HideOthers),
        MenuItem::action("Show All", ShowAll),
        MenuItem::separator(),
        MenuItem::action(format!("Quit {APP_NAME}"), Quit),
    ])
}

/// File.
fn file_menu() -> Menu {
    Menu::new("File").items([
        MenuItem::action("New Window", NewWindow),
        MenuItem::action("Quick Note", OpenQuickNote),
        MenuItem::action("Diary", OpenDiary),
        MenuItem::action("Calendar", OpenCalendar),
        MenuItem::action("Party", OpenParty),
        MenuItem::separator(),
        MenuItem::action("Close", CloseWindow),
    ])
}

/// Edit, for the text fields. `AppKit` adds its own Start Dictation and Emoji & Symbols to a
/// menu titled Edit.
fn edit_menu() -> Menu {
    Menu::new("Edit").items([
        MenuItem::os_action("Undo", Undo, OsAction::Undo),
        MenuItem::os_action("Redo", Redo, OsAction::Redo),
        MenuItem::separator(),
        MenuItem::os_action("Cut", Cut, OsAction::Cut),
        MenuItem::os_action("Copy", Copy, OsAction::Copy),
        MenuItem::os_action("Paste", Paste, OsAction::Paste),
        MenuItem::action("Delete", Delete),
        MenuItem::os_action("Select All", SelectAll, OsAction::SelectAll),
    ])
}

/// View: the sidebar, the places, the column, and full screen off macOS (`AppKit` adds its own
/// Enter Full Screen, on ⌃⌘F, to a menu titled View).
fn view_menu(shown: Shown) -> Menu {
    let sidebar = if shown.sidebar {
        "Hide Sidebar"
    } else {
        "Show Sidebar"
    };
    let mut items = vec![
        MenuItem::action(sidebar, ToggleSidebar),
        MenuItem::separator(),
        MenuItem::action("Your ember", Room0),
        MenuItem::action("Needs you", OpenNeeds),
        MenuItem::action("Tasks", OpenTasks),
        MenuItem::action("Machines", OpenMachines),
        MenuItem::separator(),
        MenuItem::action("Back", GoBack),
        MenuItem::action("Search…", crate::ui::spotlight::OpenSpotlight),
    ];
    if !cfg!(target_os = "macos") {
        items.extend([
            MenuItem::separator(),
            MenuItem::action("Toggle Full Screen", ToggleFullScreen),
        ]);
    }
    Menu::new("View").items(items)
}

/// Window: GPUI hands a menu titled Window to `AppKit` as the windows menu, which lists the open
/// windows and adds its tiling items.
fn window_menu() -> Menu {
    Menu::new("Window").items([
        MenuItem::action("Minimize", Minimize),
        MenuItem::action("Zoom", Zoom),
        MenuItem::separator(),
        MenuItem::action("Bring All to Front", BringAllToFront),
        MenuItem::separator(),
        MenuItem::action("Your Sessions", crate::ui::sessions::OpenSessionsWindow),
    ])
}

/// Help: on macOS, the menu titled Help gets the system's search over every menu item.
fn help_menu() -> Menu {
    Menu::new("Help").items([MenuItem::action(format!("{APP_NAME} Help"), ShowHelp)])
}

/// The About box: the full name, the version and the website.
fn about(window: &mut Window, cx: &mut App) {
    let detail = format!(
        "{APP_NAME} {}\n{}",
        crate::updates::version::Running::this(),
        env!("CARGO_PKG_HOMEPAGE")
    );
    // Its one button only closes it: nothing to wait for.
    drop(window.prompt(PromptLevel::Info, FULL_NAME, Some(&detail), &["OK"], cx));
}

/// Brings every window of the app forward, the front one last so it stays in front.
fn bring_all_to_front(cx: &mut App) {
    let front = cx.active_window();
    let mut all = cx.windows();
    all.sort_by_key(|handle| Some(*handle) == front);
    for handle in all {
        if let Err(error) = handle.update(cx, |_, window, _| window.activate_window()) {
            log::debug!("a window closed before it came forward: {error}");
        }
    }
    cx.activate(true);
}

/// Runs `f` on the active window (or the only one).
fn with_window(cx: &mut App, f: impl FnOnce(&mut Window, &mut App)) {
    let Some(handle) = cx.active_window().or_else(|| cx.windows().first().copied()) else {
        return;
    };
    if let Err(error) = handle.update(cx, |_, window, cx| f(window, cx)) {
        log::debug!("the window closed before the action reached it: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn titles(menu: &Menu) -> Vec<String> {
        menu.items
            .iter()
            .filter_map(|item| match item {
                MenuItem::Action { name, .. } => Some(name.to_string()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn the_menu_bar_has_the_mac_menus_in_their_order() {
        let names: Vec<String> = menus(Shown::default())
            .iter()
            .map(|menu| menu.name.to_string())
            .collect();
        assert_eq!(
            names,
            [APP_NAME, "File", "Edit", "View", "Task", "Window", "Help"].map(str::to_owned)
        );
    }

    #[test]
    fn view_says_what_the_sidebar_item_will_do() {
        assert!(titles(&view_menu(Shown { sidebar: true })).contains(&"Hide Sidebar".to_owned()));
        assert!(titles(&view_menu(Shown { sidebar: false })).contains(&"Show Sidebar".to_owned()));
    }

    #[test]
    fn full_screen_is_in_view_once() {
        let full_screen = titles(&view_menu(Shown::default()))
            .iter()
            .filter(|title| title.contains("Full Screen"))
            .count();
        // On the Mac it is AppKit's own item.
        assert_eq!(full_screen, usize::from(!cfg!(target_os = "macos")));
    }

    #[test]
    fn file_and_edit_have_their_standard_items() {
        assert_eq!(
            titles(&file_menu()),
            [
                "New Window",
                "Quick Note",
                "Diary",
                "Calendar",
                "Party",
                "Close"
            ]
        );
        let edit = titles(&edit_menu());
        for item in [
            "Undo",
            "Redo",
            "Cut",
            "Copy",
            "Paste",
            "Delete",
            "Select All",
        ] {
            assert!(edit.contains(&item.to_owned()), "{item}");
        }
        assert!(titles(&app_menu()).contains(&format!("About {APP_NAME}")));
    }

    #[test]
    fn check_for_updates_comes_right_after_about() {
        let app = titles(&app_menu());
        assert_eq!(
            app[..2],
            [format!("About {APP_NAME}"), "Check for Updates…".to_owned()]
        );
    }
}
