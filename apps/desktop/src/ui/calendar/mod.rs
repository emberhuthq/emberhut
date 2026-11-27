//! The Calendar window (⇧⌘Y, the sidebar's More, `emberhut://calendar`): the user's events from
//! every calendar account the server reads, and apart from them, in colours of their own, what
//! the user has to work on (deadlines, decide-by days, today's plan). Day, Week and Month (kept
//! across launches), Today (⌘T), the arrows (⌘← ⌘→), a sideways drag or swipe to move, the
//! month or day as a big title. An event opens its details; a work item its task's page in the
//! main window. Read-only.
//!
//! The days, layout and words are the store's (`store::calendar`); this draws them.

mod chips;
mod controls;
mod details;
mod grid;
mod month;

use std::rc::Rc;
use std::time::Duration;

use emberhut_api::calendar::{CalendarEvent, CalendarItem, CalendarPage};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::prelude::*;
use gpui_kit::{
    App, Entity, FocusHandle, KeyBinding, MouseButton, MouseDownEvent, MouseUpEvent, Pixels, Point,
    ScrollHandle, Task, Window, actions, div, point, px,
};
use jiff::civil::Date;
use jiff::tz::TimeZone;
use jiff::{Timestamp, ZonedDateTime};

use crate::assets::UI_FONT;
use crate::links::Target;
use crate::prefs::Kept;
use crate::shell::{self, WindowKind, windows};
use crate::store::calendar::range::{self, Mode};
use crate::store::presence::Presence;
use crate::store::{Page, Store};
use crate::ui::palette::pal;
use crate::ui::tasks::swipe::{Swipe, Swiped};

/// How often the days shown are asked for again while the window is in sight.
const EVERY: Duration = Duration::from_secs(60);
/// An hour's height on the Day and Week grid.
const HOUR: f32 = 52.;

actions!(
    calendar,
    [
        /// Opens the Calendar window, from any window.
        OpenCalendar,
        /// Back to today.
        CalendarToday,
        /// The previous day, week or month.
        CalendarPrevious,
        /// The next day, week or month.
        CalendarNext,
        /// Closes the event's details or the legend.
        CalendarDismiss,
    ]
);

/// Registers the Calendar window, its menu action and its keys.
pub(crate) fn register(cx: &mut App) {
    windows::register_view(
        WindowKind::Calendar,
        Rc::new(|store, window, cx| cx.new(|cx| CalendarView::new(store, window, cx)).into()),
        cx,
    );
    cx.on_action(|_: &OpenCalendar, cx| {
        windows::open_single(WindowKind::Calendar, cx);
    });
    cx.bind_keys([
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-shift-y", OpenCalendar, None),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("ctrl-shift-y", OpenCalendar, None),
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-t", CalendarToday, Some(CONTEXT)),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("ctrl-t", CalendarToday, Some(CONTEXT)),
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-left", CalendarPrevious, Some(CONTEXT)),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("ctrl-left", CalendarPrevious, Some(CONTEXT)),
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-right", CalendarNext, Some(CONTEXT)),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("ctrl-right", CalendarNext, Some(CONTEXT)),
        KeyBinding::new("escape", CalendarDismiss, Some(CONTEXT)),
    ]);
}

/// The window's key context.
const CONTEXT: &str = "Calendar";

/// The window's view.
pub(crate) struct CalendarView {
    store: Entity<Store>,
    focus: FocusHandle,
    zone: TimeZone,
    mode: Mode,
    /// A day of what is shown.
    anchor: Date,
    /// Day: every chip in the strip, not the first few.
    strip_open: bool,
    /// The event whose details show.
    event: Option<CalendarEvent>,
    /// Whether the colours' legend shows.
    legend: bool,
    /// The hours of Day and Week.
    hours: ScrollHandle,
    /// The hours scroll to their start (two hours before now, or 8 AM) at the next draw.
    scroll_due: bool,
    /// A sideways swipe over the days.
    swipe: Swipe,
    /// Where a drag across the days started.
    drag_from: Option<Point<Pixels>>,
    /// Asks for the days shown again every minute.
    _poll: Task<()>,
}

impl CalendarView {
    /// The calendar over `store`, on this week (or the mode left last time).
    pub(crate) fn new(
        store: Entity<Store>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Self {
        cx.observe(&store, |_, _, cx| cx.notify()).detach();
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        let zone = TimeZone::system();
        let anchor = Timestamp::now().to_zoned_datetime(zone.clone()).date();
        let poll = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(EVERY).await;
                let asked = this.update(cx, |this, cx| {
                    if this.store.read(cx).presence() != Presence::Hidden {
                        this.load(cx);
                    }
                });
                if asked.is_err() {
                    return;
                }
            }
        });
        let view = Self {
            store,
            focus,
            zone,
            mode: Kept::get(cx).calendar_mode,
            anchor,
            strip_open: false,
            event: None,
            legend: false,
            hours: ScrollHandle::new(),
            scroll_due: true,
            swipe: Swipe::default(),
            drag_from: None,
            _poll: poll,
        };
        view.load(cx);
        view
    }

    /// The days shown.
    fn days(&self) -> Vec<Date> {
        range::days(self.mode, self.anchor)
    }

    /// The first and last day shown.
    fn range(&self) -> (Date, Date) {
        let days = self.days();
        (
            days.first().copied().unwrap_or(self.anchor),
            days.last().copied().unwrap_or(self.anchor),
        )
    }

    /// Asks for the days shown.
    fn load(&self, cx: &mut Context<'_, Self>) {
        let (from, to) = self.range();
        self.store
            .update(cx, |store, cx| store.load_calendar(from, to, cx));
    }

    /// Today, in the device's zone.
    fn today(&self) -> Date {
        self.now().date()
    }

    /// Now, in the device's zone.
    fn now(&self) -> ZonedDateTime {
        Timestamp::now().to_zoned_datetime(self.zone.clone())
    }

    /// Shows `mode`, kept for the next launch.
    fn set_mode(&mut self, mode: Mode, cx: &mut Context<'_, Self>) {
        if self.mode == mode {
            return;
        }
        self.mode = mode;
        self.strip_open = false;
        self.scroll_due = true;
        Kept::set_calendar_mode(cx, mode);
        self.load(cx);
        cx.notify();
    }

    /// Moves to the next (`1`) or previous (`-1`) day, week or month.
    fn step(&mut self, by: i64, cx: &mut Context<'_, Self>) {
        self.anchor = range::step(self.mode, self.anchor, by);
        self.strip_open = false;
        self.load(cx);
        cx.notify();
    }

    /// Back to today.
    fn go_today(&mut self, cx: &mut Context<'_, Self>) {
        self.anchor = self.today();
        self.scroll_due = true;
        self.load(cx);
        cx.notify();
    }

    /// Opens `day` alone.
    fn pick(&mut self, day: Date, cx: &mut Context<'_, Self>) {
        self.anchor = day;
        self.strip_open = false;
        self.mode = Mode::Day;
        Kept::set_calendar_mode(cx, Mode::Day);
        self.scroll_due = true;
        self.load(cx);
        cx.notify();
    }

    /// Opens an event's details.
    fn open_event(&mut self, event: CalendarEvent, cx: &mut Context<'_, Self>) {
        self.event = Some(event);
        cx.notify();
    }

    /// Closes the details, else the legend.
    fn dismiss(&mut self, cx: &mut Context<'_, Self>) {
        if self.event.take().is_none() {
            self.legend = false;
        }
        cx.notify();
    }

    /// The hours scroll to two hours before now when today shows, else to 8 AM.
    fn scroll_hours(&mut self) {
        if !std::mem::take(&mut self.scroll_due) {
            return;
        }
        let today = self.today();
        let hour = if self.days().contains(&today) {
            (self.now().hour() - 2).max(0)
        } else {
            8
        };
        let top = (f32::from(hour) * HOUR - 14.).max(0.);
        self.hours.set_offset(point(px(0.), px(-top)));
    }
}

/// Opens a work item in the main window: its task's page, else its project's (while the
/// project screens show).
fn open_item(item: &CalendarItem, cx: &mut App) {
    let hides_projects = windows::store(cx).is_none_or(|store| store.read(cx).hides_projects());
    if let Some(target) = item_target(item, hides_projects) {
        shell::go(&target, cx);
    }
}

/// Where a work item leads: its task, else its project's page unless projects are hidden.
fn item_target(item: &CalendarItem, hides_projects: bool) -> Option<Target> {
    match (item.task, item.project) {
        (Some(task), _) => Some(Target::Task(task)),
        (None, Some(project)) if !hides_projects => Some(Target::Page(Page::Project(project))),
        (None, _) => None,
    }
}

impl Render for CalendarView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let p = pal(cx);
        let (from, to) = self.range();
        let store = self.store.read(cx);
        let page: Option<CalendarPage> = store.calendar_page(from, to).cloned();
        let failed = store.calendar_failed(from, to);
        let note = page.as_ref().and_then(|page| page.note.clone());
        if self.mode != Mode::Month {
            self.scroll_hours();
        }
        let body = match self.mode {
            Mode::Month => self.month(page.as_ref(), window, &p, cx).into_any_element(),
            Mode::Day | Mode::Week => self.grid(page.as_ref(), &p, cx).into_any_element(),
        };
        let waiting = page.is_none().then(|| {
            div()
                .absolute()
                .inset_0()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(6.))
                .text_color(p.muted)
                .map(|d| {
                    if failed {
                        d.child(
                            div()
                                .text_size(px(15.))
                                .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                                .text_color(p.text)
                                .child(format!("Can't reach {}", crate::brand::APP_NAME)),
                        )
                        .child("Retrying.")
                    } else {
                        d.child(Spinner::new().color(p.muted))
                    }
                })
        });
        let controls = self.controls(&p, cx);
        div()
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &CalendarToday, _, cx| this.go_today(cx)))
            .on_action(cx.listener(|this, _: &CalendarPrevious, _, cx| this.step(-1, cx)))
            .on_action(cx.listener(|this, _: &CalendarNext, _, cx| this.step(1, cx)))
            .on_action(cx.listener(|this, _: &CalendarDismiss, _, cx| this.dismiss(cx)))
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .bg(p.surface)
            .text_color(p.text)
            .font_family(UI_FONT)
            .text_size(px(13.))
            .child(controls)
            .when_some(note, |d, note| d.child(details::note_line(&note, &p)))
            .child(
                div()
                    .id("calendar-body")
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .px(px(10.))
                    .child(body)
                    .children(waiting)
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, event: &MouseDownEvent, _, _| {
                            this.drag_from = Some(event.position);
                        }),
                    )
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(|this, event: &MouseUpEvent, _, cx| {
                            let Some(from) = this.drag_from.take() else {
                                return;
                            };
                            let moved = event.position - from;
                            if let Some(by) =
                                range::drag_step(f32::from(moved.x), f32::from(moved.y))
                            {
                                this.step(by, cx);
                            }
                        }),
                    )
                    .on_scroll(cx.listener(
                        |this, event, _, cx| match this.swipe.feed_event(event) {
                            Some(Swiped::Left) => this.step(1, cx),
                            Some(Swiped::Right) => this.step(-1, cx),
                            None => {}
                        },
                    )),
            )
            .when(self.legend, |d| d.child(details::legend(&p, cx)))
            .when_some(self.event.clone(), |d, event| {
                d.child(details::event_sheet(&event, &self.zone, &p, cx))
            })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{ElementId, TestAppContext, WindowHandle};

    use super::*;
    use crate::backend::fake::{Call, FakeBackend};
    use crate::ui::palette::Pal;
    use crate::wire::TASK_KEY_PREFIX;

    fn open(cx: &mut TestAppContext) -> (Arc<FakeBackend>, WindowHandle<CalendarView>) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            cx.set_global(Pal::new(&crate::theme::built_in(), false));
        });
        let backend = Arc::new(FakeBackend::new([]));
        let shared = Arc::clone(&backend) as Arc<dyn crate::backend::Backend>;
        let store = cx.new(|_| Store::for_tests(shared));
        let window = cx.add_window(|window, cx| CalendarView::new(store.clone(), window, cx));
        (backend, window)
    }

    fn asked(backend: &FakeBackend) -> Vec<String> {
        backend
            .calls()
            .into_iter()
            .filter_map(|call| match call {
                Call::Calendar(query) => Some(query),
                _ => None,
            })
            .collect()
    }

    #[gpui_kit::test]
    fn the_week_shows_and_the_bar_moves_and_changes_mode(cx: &mut TestAppContext) {
        let (backend, window) = open(cx);
        cx.run_until_parked();
        let week = window
            .update(cx, |view, _, _| (view.mode, view.range()))
            .unwrap();
        assert_eq!(week.0, Mode::Week);
        assert_eq!(asked(&backend).len(), 1, "its days, at once");
        cx.update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
            assert!(window.try_find("calendar-hours").is_some());
            window.click("calendar-next", cx);
        })
        .unwrap();
        cx.run_until_parked();
        let next = window.update(cx, |view, _, _| view.range()).unwrap();
        assert_eq!(next.0, range::step(Mode::Week, week.1.0, 1));
        cx.update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
            window.click("calendar-mode-Month", cx);
            window.render_frame(cx);
            assert!(window.try_find("calendar-month").is_some());
        })
        .unwrap();
        cx.run_until_parked();
        assert_eq!(asked(&backend).len(), 3, "each range as it shows");
    }

    #[gpui_kit::test]
    fn a_day_of_the_month_opens_alone(cx: &mut TestAppContext) {
        let (_, window) = open(cx);
        window
            .update(cx, |view, _, cx| view.set_mode(Mode::Month, cx))
            .unwrap();
        let first = window.update(cx, |view, _, _| view.days()[0]).unwrap();
        let key = first.strftime("%Y%m%d").to_string();
        cx.update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
            window.click(ElementId::Name(format!("month-{key}").into()), cx);
        })
        .unwrap();
        let shown = window
            .update(cx, |view, _, _| (view.mode, view.anchor))
            .unwrap();
        assert_eq!(shown, (Mode::Day, first));
    }

    #[test]
    fn a_chip_opens_its_task_else_its_project_while_projects_show() {
        let item =
            |value: serde_json::Value| -> CalendarItem { serde_json::from_value(value).unwrap() };
        let task = item(
            serde_json::json!({"kind": "deadline", "day": "2026-11-26", "task": 4,
            "key": format!("{TASK_KEY_PREFIX}4"), "project": 2, "title": "Ship", "hard": false, "status": "open"}),
        );
        assert_eq!(
            item_target(&task, false),
            Some(Target::Task(emberhut_api::ids::TaskId::new(4)))
        );
        let project = item(serde_json::json!({"kind": "deadline", "day": "2026-11-26",
            "project": 2, "title": "Launch", "hard": true, "count": 3}));
        assert_eq!(
            item_target(&project, false),
            Some(Target::Page(Page::Project(
                emberhut_api::ids::ProjectId::new(2)
            )))
        );
        assert_eq!(item_target(&project, true), None, "projects hidden");
    }
}
