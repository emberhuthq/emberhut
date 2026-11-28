//! The sidebar: your ember (the assistant's row), Needs you, Tasks, the rooms, the recent
//! sessions, the agents (below all those unless Settings puts them above the rooms, or hidden),
//! and More (Machines, Settings, and the windows of their own this build has). On a server that
//! keeps plans, Tasks is the main place: Sessions moves under More and the recent sessions
//! leave the sidebar (the Tasks list shows the sessions with no task).
//! Its right edge drags it wider or narrower; Recent sessions and More fold, kept across
//! launches. Every row explains itself on hover and is in the Tab order.

use gpui_kit::TestSupportExt;
use gpui_kit::assets::IconName as AssetIcon;
use gpui_kit::component::{Icon, IconName};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, Div, DragMoveEvent, ElementId, Empty, FontWeight, MouseButton, MouseUpEvent, Role,
    Stateful, Window, div, px,
};

use super::controls::{tab_stop, tip, tip_for};
use super::palette::{Pal, pal};
use super::routes;
use super::widgets::{icon, pill, section_title};
use super::window::{MainWindow, OpenMachines, OpenNeeds, OpenSettings, OpenTasks};
use crate::store::sessions::lists as sessions_lists;
use crate::store::sessions::places::SessionPlace;
use crate::store::sessions::view::SessionRef;
use crate::store::sessions::words::{self as session_words, Mark as SessionMark};
use crate::store::settings::about::TITLE as ABOUT_TITLE;
use crate::ui::a11y::{self, Accessible as _};

use crate::brand::APP_NAME;
use crate::format;
use crate::prefs::Kept;
use crate::shell::{WindowKind, windows};
use crate::store::lookup;
use crate::store::room_line::WIDE_FROM;
use crate::store::settings::SettingsPane;
use crate::store::{AgentsPlace, Page, Store};

/// How many recent sessions the sidebar lists.
const RECENT_SESSIONS: usize = 6;

/// The sidebar.
pub(crate) fn sidebar(window: &mut MainWindow, cx: &mut Context<'_, MainWindow>) -> Div {
    let p = pal(cx);
    let (width, place) = {
        let store = window.store.read(cx);
        (store.sidebar_width(), store.agents_place())
    };
    // The agents sit above the rooms or below them all, as set in Settings › Appearance; hidden,
    // the section is empty.
    let agents = super::sidebar_agents::section(window, &p, cx);
    let (above, below) = match place {
        AgentsPlace::Above => (agents, Vec::new()),
        AgentsPlace::Below => (Vec::new(), agents),
    };
    let list = div()
        .id("sidebar-list")
        .flex()
        .flex_col()
        .flex_1()
        .overflow_y_scroll()
        .pb(px(12.))
        // Seen by the headless tests (a no-op outside them).
        .children(
            pages(window, &p, cx)
                .into_iter()
                .map(TestSupportExt::test_support),
        )
        .children(above)
        .children(super::rooms::section(window, width >= WIDE_FROM, &p, cx))
        .children(super::sidebar_places::projects(window, &p, cx))
        .children(sessions(window, &p, cx))
        .children(super::sidebar_places::goals(window, &p, cx))
        .children(below)
        .children(
            more(window, &p, cx)
                .into_iter()
                .map(TestSupportExt::test_support),
        );
    let sidebar = div()
        .relative()
        .flex()
        .flex_col()
        .w(px(width))
        .flex_none()
        .h_full()
        .bg(p.sidebar)
        .border_r_1()
        .border_color(p.border)
        .child(super::sidebar_ember::row_with_menu(window, &p, cx))
        .child(list)
        .child(edge(&p, cx))
        .on_drag_move(
            cx.listener(|this, event: &DragMoveEvent<SidebarEdge>, _, cx| {
                let dragged = f32::from(event.event.position.x - event.bounds.origin.x);
                this.store
                    .update(cx, |store, cx| store.drag_sidebar_to(dragged, cx));
            }),
        );
    super::sidebar_keys::with_keys(sidebar, window, cx)
}

/// What is dragged when the user drags the sidebar's edge.
#[derive(Clone, Copy, Debug)]
struct SidebarEdge;

impl Render for SidebarEdge {
    fn render(&mut self, _: &mut Window, _: &mut Context<'_, Self>) -> impl IntoElement {
        Empty
    }
}

/// The sidebar's right edge: dragged, it makes the sidebar wider or narrower (below
/// [`WIDE_FROM`] its rooms take one line each); the width is kept when the pointer lets go,
/// on the edge or anywhere else.
fn edge(p: &Pal, cx: &mut Context<'_, MainWindow>) -> impl IntoElement {
    div()
        .id("sidebar-edge")
        .read_as(Role::Splitter, "Resize the sidebar")
        .hint("Drag to resize")
        .absolute()
        .top_0()
        .bottom_0()
        .right(px(-3.))
        .w(px(6.))
        .cursor_col_resize()
        .hover(|d| d.bg(p.border))
        .on_drag(SidebarEdge, |edge, _, _, cx| cx.new(|_| *edge))
        .on_mouse_up(MouseButton::Left, cx.listener(let_go))
        .on_mouse_up_out(MouseButton::Left, cx.listener(let_go))
}

/// The pointer let go: a width the sidebar was dragged to is kept.
fn let_go(
    this: &mut MainWindow,
    _: &MouseUpEvent,
    _: &mut Window,
    cx: &mut Context<'_, MainWindow>,
) {
    this.store.update(cx, Store::let_go_of_sidebar);
}

/// Needs you, Tasks, Scheduled, and Sessions (and the Browser while it is handed over).
fn pages(window: &MainWindow, p: &Pal, cx: &Context<'_, MainWindow>) -> Vec<Stateful<Div>> {
    let store = window.store.read(cx);
    let column = window.column.read(cx).current();
    let needs = store.needs_count();
    let open_tasks = store.needs().map_or(0, |needs| needs.open_tasks().len());
    let shows_tasks = store.state().is_none_or(lookup::lists_tasks);
    let plans = store.keeps_plans();
    let mut rows = Vec::new();
    let needs_row = row("needs", column == Some(Page::Needs), p);
    rows.push(
        tip_for(
            needs_row,
            "What waits on you: a board by the day to decide by",
            &OpenNeeds,
        )
        .aria_label(if needs > 0 {
            format!("Needs you, {needs}")
        } else {
            "Needs you".to_owned()
        })
        .child(icon(IconName::Inbox, p.accent, 14.))
        .child(div().font_weight(FontWeight::SEMIBOLD).child("Needs you"))
        .child(div().flex_1())
        .when(needs > 0, |d| {
            d.child(pill(format::badge(needs), p.accent, p.on_accent))
        })
        .on_click(cx.listener(|this, _, window, cx| this.open(Page::Needs, window, cx))),
    );
    if shows_tasks {
        let tasks_row = row("tasks", column == Some(Page::Tasks), p);
        rows.push(
            tip_for(
                tasks_row,
                if plans {
                    "Your tasks, each with its plan: needs you, what runs, then the rest"
                } else {
                    "Your open tasks: needs you, Today, then the rest"
                },
                &OpenTasks,
            )
            .aria_label(format!("Tasks, {open_tasks} open"))
            .child(icon(IconName::CircleCheck, p.muted, 14.))
            .child("Tasks")
            .child(div().flex_1())
            .child(
                div()
                    .text_size(px(11.5))
                    .text_color(p.faint)
                    .child(open_tasks.to_string()),
            )
            .on_click(cx.listener(|this, _, window, cx| this.open(Page::Tasks, window, cx))),
        );
    }
    rows.push(scheduled_row(
        column == Some(Page::Scheduled),
        store.scheduled().count(),
        p,
        cx,
    ));
    if !plans && let Some(row) = sessions_row(store, column == Some(Page::Sessions), p, cx) {
        rows.push(row);
    }
    if super::browser::sidebar::is_handed_over(store) {
        rows.push(browser_row(true, None, p, cx));
    }
    rows
}

/// The Sessions row, with how many run now, when there is any session: among the top rows, or
/// under More on a server that keeps plans.
fn sessions_row(
    store: &Store,
    selected: bool,
    p: &Pal,
    cx: &Context<'_, MainWindow>,
) -> Option<Stateful<Div>> {
    let state = store
        .state()
        .filter(|state| sessions_lists::has_any(state))?;
    let running = sessions_lists::running_now(state);
    // Under More, beside the tasks that hold most of them, it says it is every session.
    let label = if store.keeps_plans() {
        "All sessions"
    } else {
        "Sessions"
    };
    Some(
        tip(
            row("sessions", selected, p),
            format!("{APP_NAME}'s sessions and threads"),
        )
        .aria_label(if running > 0 {
            format!("{label}, {running} running")
        } else {
            label.to_owned()
        })
        .child(icon(IconName::SquareTerminal, p.muted, 14.))
        .child(label)
        .child(div().flex_1())
        .when(running > 0, |d| {
            d.child(
                div()
                    .text_size(px(11.5))
                    .text_color(p.success)
                    .child(running.to_string()),
            )
        })
        .on_click(cx.listener(|this, _, window, cx| this.open(Page::Sessions, window, cx))),
    )
}

/// The Browser row: among the top rows with `needs you` while the assistant has handed the
/// browser over (`handed`), else under More with who drives beside it. It opens the window.
fn browser_row(
    handed: bool,
    driver: Option<&'static str>,
    p: &Pal,
    cx: &Context<'_, MainWindow>,
) -> Stateful<Div> {
    let kind = WindowKind::Browser;
    let note = if handed {
        Some(("needs you", p.accent))
    } else {
        driver.map(|driver| (driver, p.faint))
    };
    tip(
        row(
            ElementId::Name(format!("window-{}", kind.link_name()).into()),
            false,
            p,
        )
        .when(!handed, |d| d.pl(px(26.))),
        format!("Opens the {} window", kind.title()),
    )
    .aria_label(a11y::line(&[
        kind.title(),
        note.map_or_default(|(note, _)| note),
    ]))
    .child(icon(
        window_icon(kind),
        if handed { p.accent } else { p.muted },
        14.,
    ))
    .child(kind.title())
    .child(div().flex_1())
    .when_some(note, |d, (note, color)| {
        d.child(div().text_size(px(11.5)).text_color(color).child(note))
    })
    .on_click(cx.listener(move |_, _, _, cx| {
        windows::open_single(kind, cx);
    }))
}

/// Scheduled, with how many things are scheduled.
fn scheduled_row(
    selected: bool,
    count: usize,
    p: &Pal,
    cx: &Context<'_, MainWindow>,
) -> Stateful<Div> {
    tip(
        row("scheduled", selected, p),
        format!("Everything {APP_NAME} has scheduled, soonest first"),
    )
    .aria_label(if count > 0 {
        format!("Scheduled, {count}")
    } else {
        "Scheduled".to_owned()
    })
    .child(
        Icon::new(AssetIcon::CalendarClock)
            .size(px(14.))
            .text_color(p.muted),
    )
    .child("Scheduled")
    .child(div().flex_1())
    .when(count > 0, |d| {
        d.child(
            div()
                .text_size(px(11.5))
                .text_color(p.faint)
                .child(count.to_string()),
        )
    })
    .on_click(cx.listener(|this, _, window, cx| this.open(Page::Scheduled, window, cx)))
}

/// The recent sessions, under a header that folds them: the user's waiting ones, what runs,
/// then their last few finished ones, each opening its page; and Your sessions, the Sessions
/// window (⇧⌘T), noting how many of theirs wait or run. None on a server that keeps plans:
/// the Tasks list shows the sessions with no task, and Sessions is under More.
fn sessions(window: &MainWindow, p: &Pal, cx: &Context<'_, MainWindow>) -> Vec<AnyElement> {
    let store = window.store.read(cx);
    let Some(state) = store.state().filter(|_| !store.keeps_plans()) else {
        return Vec::new();
    };
    let shown = sessions_lists::sidebar(state);
    if shown.is_empty() && state.my_sessions.is_empty() {
        return Vec::new();
    }
    let open = window.sessions_open;
    let header = fold_title("sessions-fold", "Recent sessions", open, p).on_click(cx.listener(
        |this, _, _, cx| {
            this.sessions_open = !this.sessions_open;
            let open = this.sessions_open;
            Kept::set_sidebar(cx, |sidebar| sidebar.sessions_open = open);
            cx.notify();
        },
    ));
    let mut rows = vec![header.into_any_element()];
    if !open {
        return rows;
    }
    for (i, session) in (0_u64..).zip(shown.into_iter().take(RECENT_SESSIONS)) {
        let mark = SessionMark::of(&session.status);
        let word_color = if mark == SessionMark::Waiting {
            p.accent
        } else {
            p.faint
        };
        let place = SessionPlace::Session(SessionRef::whole(session.id.clone()));
        let store = window.store.clone();
        rows.push(
            tip(row(("session", i), false, p), session_words::whose(session))
                .child(super::sessions::state_mark(
                    p,
                    &session.status,
                    ElementId::NamedInteger("sidebar-session-mark".into(), i),
                    cx,
                ))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .child(session_words::short_title(&session.title).to_owned()),
                )
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(word_color)
                        .child(mark.word()),
                )
                .on_click(move |_, window, cx| {
                    super::sessions::open(
                        &store,
                        super::sessions::Open::Place(place.clone()),
                        window,
                        cx,
                    );
                })
                .into_any_element(),
        );
    }
    let (waiting, running) = sessions_lists::mine_waiting_and_running(state);
    let note = if waiting > 0 {
        Some((format!("{waiting} waiting"), p.accent))
    } else if running > 0 {
        Some((format!("{running} running"), p.faint))
    } else {
        None
    };
    rows.push(
        tip_for(
            row("your-sessions", false, p),
            "Your Claude Code sessions",
            &super::sessions::OpenSessionsWindow,
        )
        .child(icon(IconName::SquareTerminal, p.muted, 14.))
        .child(div().flex_1().child("Your sessions"))
        .when_some(note, |d, (note, color)| {
            d.child(div().text_size(px(11.)).text_color(color).child(note))
        })
        .on_click(|_, _, cx| {
            windows::open_single(WindowKind::Sessions, cx);
        })
        .into_any_element(),
    );
    rows
}

/// A section's title that folds what is under it, with its chevron.
fn fold_title(id: &'static str, title: &'static str, open: bool, p: &Pal) -> Stateful<Div> {
    tab_stop(
        div()
            .id(id)
            // Read as the section's name, open or folded.
            .button_named(title)
            .aria_expanded(open)
            .aria_value(a11y::folded(open))
            .flex()
            .items_center()
            .gap(px(4.))
            .mx(px(8.))
            .mt(px(10.))
            .px(px(2.))
            .pb(px(2.))
            .border_1()
            .rounded(px(4.))
            .cursor_pointer()
            .child(section_title(p, title).px(px(0.)).pt(px(0.)).pb(px(0.)))
            .child(icon(
                if open {
                    IconName::ChevronDown
                } else {
                    IconName::ChevronRight
                },
                p.faint,
                10.,
            )),
        p,
    )
}

/// More: Sessions (on a server that keeps plans), Machines, Settings, and the windows of their
/// own this build has, folded at first.
fn more(window: &MainWindow, p: &Pal, cx: &Context<'_, MainWindow>) -> Vec<Stateful<Div>> {
    let column = window.column.read(cx).current();
    let open = window.more_open;
    let mut rows = vec![
        row("more", false, p)
            .mt(px(10.))
            .text_color(p.muted)
            .child(icon(
                if open {
                    IconName::ChevronDown
                } else {
                    IconName::ChevronRight
                },
                p.muted,
                12.,
            ))
            .child("More")
            .on_click(cx.listener(|this, _, _, cx| {
                this.more_open = !this.more_open;
                let open = this.more_open;
                Kept::set_sidebar(cx, |sidebar| sidebar.more_open = open);
                cx.notify();
            })),
    ];
    if open {
        let store = window.store.read(cx);
        if store.keeps_plans()
            && let Some(row) = sessions_row(store, column == Some(Page::Sessions), p, cx)
        {
            rows.push(row.pl(px(26.)));
        }
        // Windows of their own open their window and never light. The Dark room's row is the
        // last of Rooms; the quick note opens from the chat's bar and File, not from here.
        for kind in windows::available(cx).into_iter().filter(|kind| {
            !matches!(
                kind,
                WindowKind::Sessions | WindowKind::Dark | WindowKind::QuickNote
            )
        }) {
            if kind == WindowKind::Browser {
                let store = window.store.read(cx);
                if !super::browser::sidebar::is_handed_over(store) {
                    let driver = super::browser::sidebar::driver_note(store);
                    rows.push(browser_row(false, driver, p, cx));
                }
                continue;
            }
            rows.push(
                tip(
                    row(
                        ElementId::Name(format!("window-{}", kind.link_name()).into()),
                        false,
                        p,
                    )
                    .pl(px(26.)),
                    window_help(kind),
                )
                .child(icon(window_icon(kind), p.muted, 14.))
                .child(kind.title())
                .child(div().flex_1())
                .when_some(window_note(kind), |d, note| {
                    d.child(div().text_size(px(11.5)).text_color(p.faint).child(note))
                })
                .on_click(cx.listener(move |_, _, _, cx| {
                    windows::open_single(kind, cx);
                })),
            );
        }
        rows.push(super::sidebar_places::report_row(window, p, cx));
        let machines = row("machines", column == Some(Page::Machines), p).pl(px(26.));
        rows.push(
            tip_for(
                machines,
                "Your huts: online or asleep, their accounts' room, CPU, memory, disks, sessions",
                &OpenMachines,
            )
            .child(icon(IconName::Cpu, p.muted, 14.))
            .child("Huts")
            .on_click(cx.listener(|this, _, window, cx| this.open(Page::Machines, window, cx))),
        );
        let in_settings = matches!(column, Some(Page::Settings | Page::SettingsPane(_)));
        let settings = row("settings", in_settings, p).pl(px(26.));
        rows.push(
            tip_for(settings, "Settings", &OpenSettings)
                .child(icon(IconName::Settings, p.muted, 14.))
                .child("Settings")
                .on_click(cx.listener(|this, _, window, cx| this.open(Page::Settings, window, cx))),
        );
        rows.push(about_row(column, p));
    }
    rows
}

/// About you, under More: it opens that page of Settings.
fn about_row(column: Option<Page>, p: &Pal) -> Stateful<Div> {
    let about = column == Some(Page::SettingsPane(SettingsPane::AboutYou));
    row("about-you", about, p)
        .pl(px(26.))
        .child(
            Icon::new(AssetIcon::Contact)
                .size(px(14.))
                .text_color(p.muted),
        )
        .child(ABOUT_TITLE)
        .on_click(|_, window, cx| {
            routes::open_settings(Some(SettingsPane::AboutYou), window, cx);
        })
}

/// A window of its own's tip in the sidebar.
fn window_help(kind: WindowKind) -> String {
    match kind {
        WindowKind::Party => format!(
            "AI personas talking to each other about your topic, live ({})",
            if cfg!(target_os = "macos") {
                "⇧⌘P"
            } else {
                "Ctrl+Shift+P"
            }
        ),
        WindowKind::Calendar => "Your calendar, with deadlines and decide-by days".to_owned(),
        WindowKind::Diary => format!("Your diary: your quick notes by day, and {APP_NAME}'s"),
        kind => format!("Opens the {} window", kind.title()),
    }
}

/// A window of its own's icon in the sidebar.
fn window_icon(kind: WindowKind) -> IconName {
    match kind {
        WindowKind::Browser => IconName::Globe,
        WindowKind::Calendar => IconName::Calendar,
        WindowKind::Sessions => IconName::SquareTerminal,
        WindowKind::Dark => IconName::Moon,
        WindowKind::Diary => IconName::BookOpen,
        WindowKind::Party => IconName::User,
        WindowKind::QuickNote => IconName::FileText,
    }
}

/// What a window of its own's row says beside its name: the Calendar, today's day (`Thu 26`).
fn window_note(kind: WindowKind) -> Option<String> {
    (kind == WindowKind::Calendar)
        .then(|| jiff::ZonedDateTime::now().strftime("%a %-d").to_string())
}

/// A one-line row.
pub(super) fn row(id: impl Into<ElementId>, selected: bool, p: &Pal) -> Stateful<Div> {
    let row = div()
        .id(id)
        // A screen reader hears a row as a button, named by its caller (else by its text).
        .role(Role::Button)
        .aria_selected(selected)
        .flex()
        .items_center()
        .gap(px(8.))
        .h(px(28.))
        .px(px(10.))
        .mx(px(8.))
        .rounded(px(6.))
        .border_1()
        .text_size(px(13.))
        .when(selected, |d| d.bg(p.accent_soft))
        .when(!selected, |d| d.hover(|d| d.bg(p.hover)))
        .cursor_pointer();
    tab_stop(row, p)
}
