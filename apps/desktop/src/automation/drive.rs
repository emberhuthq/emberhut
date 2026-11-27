//! `--drive FILE`: the app driven from outside without taking the keyboard or mouse. Each line
//! appended to the file is one step, replayed into this window only as GPUI's own events: keys go
//! through the window's bindings and the focused field, clicks through its hit-testing, as a
//! real key or click would. The run is unseen (`crate::shell::unseen`): its windows open out of
//! sight and the app never comes to the front.
//!
//! ```text
//! page needs|tasks|machines|settings   room ID    open a page or a room (prints the time to the chat's paint)
//! click X Y                            a left click at that point of the window (logical pixels)
//! type TEXT                            the text, a keystroke a character, into whatever has focus
//! key KEYSTROKE                        one keystroke: enter, escape, cmd-a, backspace…
//! scroll X Y DY                        a scroll wheel at that point (DY < 0: down the page)
//! guard TASK                           undo the answer just given unless it is for that task
//! wait MS                              let the window paint (clicks hit what was painted last)
//! mem                                  the memory now
//! state                                what the app holds: signed in, page, room, needs, the pending message, the toast
//! quit
//! a11y                                 what a screen reader is told: the window's accessibility tree, its names, roles and ids (while one listens)
//! ```

use std::path::PathBuf;
use std::str::FromStr;
use std::time::{Duration, Instant};

use gpui_kit::AppContext as _;
use gpui_kit::{
    AnyWindowHandle, App, AsyncApp, Entity, Keystroke, Modifiers, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, PlatformInput, ScrollDelta, ScrollEvent, Window, point, px,
};

use super::bench::{millis, room_painted};
use super::{report, usage};
use crate::store::Page;
use crate::ui::window::MainWindow;
use crate::wire::TASK_KEY_PREFIX;
use emberhut_api::ids::{RoomId, TaskId};

/// How often the file is read for new steps.
const READ_EVERY: Duration = Duration::from_millis(200);

/// One step.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Step {
    /// Open a page or a room.
    Open(Page),
    /// A left click at a point.
    Click { x: f32, y: f32 },
    /// Text typed a character at a time.
    Type(String),
    /// One keystroke, as GPUI writes them.
    Key(String),
    /// A scroll wheel at a point.
    Scroll { x: f32, y: f32, dy: f32 },
    /// Undo the answer just given unless it is for this task.
    Guard(TaskId),
    /// Let the window paint.
    Wait(Duration),
    /// The memory now.
    Memory,
    /// What the app holds.
    State,
    /// Quit.
    Quit,
    /// The window's accessibility tree as last sent to the screen reader.
    Accessibility,
}

/// Why a line isn't a step.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("not a step: {0}")]
pub(crate) struct BadStep(String);

impl FromStr for Step {
    type Err = BadStep;

    fn from_str(line: &str) -> Result<Self, Self::Err> {
        let bad = || BadStep(line.to_owned());
        let (command, rest) = line.trim().split_once(' ').unwrap_or((line.trim(), ""));
        let numbers = || -> Result<Vec<f32>, BadStep> {
            rest.split_whitespace()
                .map(|n| n.parse().map_err(|_| bad()))
                .collect()
        };
        Ok(match command {
            "page" => Self::Open(match rest.trim() {
                "needs" => Page::Needs,
                "tasks" => Page::Tasks,
                "machines" => Page::Machines,
                "settings" => Page::Settings,
                _ => return Err(bad()),
            }),
            "room" => Self::Open(Page::Room(RoomId::new(
                rest.trim().parse().map_err(|_| bad())?,
            ))),
            "click" => match numbers()?.as_slice() {
                &[x, y] => Self::Click { x, y },
                _ => return Err(bad()),
            },
            "scroll" => match numbers()?.as_slice() {
                &[x, y, dy] => Self::Scroll { x, y, dy },
                _ => return Err(bad()),
            },
            "type" => Self::Type(rest.to_owned()),
            "key" if !rest.trim().is_empty() => Self::Key(rest.trim().to_owned()),
            "guard" => {
                let id = rest.trim().trim_start_matches(TASK_KEY_PREFIX);
                Self::Guard(TaskId::new(id.parse().map_err(|_| bad())?))
            }
            "wait" => Self::Wait(Duration::from_millis(
                rest.trim().parse().map_err(|_| bad())?,
            )),
            "mem" => Self::Memory,
            "state" => Self::State,
            "quit" => Self::Quit,
            "a11y" => Self::Accessibility,
            _ => return Err(bad()),
        })
    }
}

/// Starts reading steps from `file`.
pub(crate) fn start(
    file: PathBuf,
    view: Entity<MainWindow>,
    window: AnyWindowHandle,
    cx: &mut App,
) {
    cx.spawn(async move |cx| {
        let mut steps = StepFile::new(file);
        loop {
            cx.background_executor().timer(READ_EVERY).await;
            // The file is read on a background thread: only what was appended since.
            let (back, lines) = cx
                .background_spawn(async move {
                    let lines = steps.read_new();
                    (steps, lines)
                })
                .await;
            steps = back;
            let lines = match lines {
                Ok(lines) => lines,
                Err(error) => {
                    report(
                        "DRIVE",
                        &format!("can't read {}: {error}", steps.path.display()),
                    );
                    continue;
                }
            };
            for line in lines {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                report("DRIVE", &format!("> {line}"));
                match line.parse::<Step>() {
                    Ok(Step::Quit) => {
                        cx.update(|cx| cx.quit());
                        return;
                    }
                    Ok(step) => run(step, &view, window, cx).await,
                    Err(error) => report("DRIVE", &error.to_string()),
                }
                refresh(window, cx);
            }
        }
    })
    .detach();
}

/// The steps file, read from where the last read stopped: only whole lines (ending in `\n`)
/// are taken, so a line still being written waits for the next read.
#[derive(Debug)]
struct StepFile {
    path: PathBuf,
    /// How many bytes were taken so far.
    offset: u64,
}

impl StepFile {
    fn new(path: PathBuf) -> Self {
        Self { path, offset: 0 }
    }

    /// The whole lines appended since the last read (none while the file doesn't exist).
    fn read_new(&mut self) -> std::io::Result<Vec<String>> {
        use std::io::{Read as _, Seek as _, SeekFrom};

        let mut file = match std::fs::File::open(&self.path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error),
        };
        file.seek(SeekFrom::Start(self.offset))?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        let Some(end) = bytes.iter().rposition(|&byte| byte == b'\n') else {
            return Ok(Vec::new());
        };
        let whole = &bytes[..=end];
        self.offset += u64::try_from(whole.len()).map_err(std::io::Error::other)?;
        Ok(String::from_utf8_lossy(whole)
            .lines()
            .map(str::to_owned)
            .collect())
    }
}

async fn run(step: Step, view: &Entity<MainWindow>, window: AnyWindowHandle, cx: &mut AsyncApp) {
    match step {
        Step::Open(page) => {
            let switched = Instant::now();
            view.update(cx, |main, cx| main.show(page, cx));
            if let Page::Room(room) = page {
                let took = room_painted(view, room, switched, cx).await;
                report(
                    "DRIVE",
                    &format!("room={room} switch_to_paint_ms={}", millis(took)),
                );
            }
        }
        Step::Click { x, y } => in_window(window, cx, |window, cx| click(window, cx, x, y)),
        Step::Type(text) => in_window(window, cx, |window, cx| type_text(window, cx, &text)),
        Step::Key(source) => {
            let os = cx.update(|cx| cx.platform_info().os);
            match Keystroke::parse_with_os(&source, os) {
                Ok(keystroke) => in_window(window, cx, |window, cx| {
                    window.dispatch_keystroke(keystroke, cx);
                }),
                Err(error) => report("DRIVE", &format!("not a keystroke: {source} ({error})")),
            }
        }
        Step::Scroll { x, y, dy } => {
            in_window(window, cx, |window, cx| scroll(window, cx, x, y, dy));
        }
        Step::Guard(task) => guard(task, view, cx),
        Step::Wait(length) => cx.background_executor().timer(length).await,
        Step::Memory => report("DRIVE", &usage::memory()),
        Step::State => report("DRIVE", &state_line(view, cx)),
        Step::Quit => cx.update(|cx| cx.quit()),
        Step::Accessibility => in_window(window, cx, |window, _| {
            // GPUI builds the tree only while assistive technology is connected.
            let tree = window.debug_a11y_tree_json();
            report(
                "DRIVE",
                tree.as_deref()
                    .unwrap_or("a11y=inactive (no screen reader is listening)"),
            );
        }),
    }
}

/// A left click at a point: the pointer moves there, then presses and lets go.
fn click(window: &mut Window, cx: &mut App, x: f32, y: f32) {
    let position = point(px(x), px(y));
    let modifiers = Modifiers::default();
    let button = MouseButton::Left;
    window.dispatch_event(
        PlatformInput::MouseMove(MouseMoveEvent {
            position,
            pressed_button: None,
            modifiers,
        }),
        cx,
    );
    window.dispatch_event(
        PlatformInput::MouseDown(MouseDownEvent {
            button,
            position,
            modifiers,
            click_count: 1,
            first_mouse: false,
        }),
        cx,
    );
    window.dispatch_event(
        PlatformInput::MouseUp(MouseUpEvent {
            button,
            position,
            modifiers,
            click_count: 1,
        }),
        cx,
    );
}

/// Text typed a keystroke a character.
fn type_text(window: &mut Window, cx: &mut App, text: &str) {
    for ch in text.chars() {
        let key = if ch == ' ' {
            "space".to_owned()
        } else {
            ch.to_lowercase().to_string()
        };
        let modifiers = Modifiers {
            shift: ch.is_uppercase(),
            ..Modifiers::default()
        };
        window.dispatch_keystroke(
            Keystroke {
                modifiers,
                key,
                key_char: Some(ch.to_string()),
            },
            cx,
        );
    }
}

/// A scroll wheel at a point.
fn scroll(window: &mut Window, cx: &mut App, x: f32, y: f32, dy: f32) {
    let position = point(px(x), px(y));
    window.dispatch_event(
        PlatformInput::MouseMove(MouseMoveEvent {
            position,
            pressed_button: None,
            modifiers: Modifiers::default(),
        }),
        cx,
    );
    window.dispatch_event(
        PlatformInput::Scroll(ScrollEvent {
            position,
            delta: ScrollDelta::Pixels(point(px(0.), px(dy))),
            ..ScrollEvent::default()
        }),
        cx,
    );
}

/// What the app holds, on one line.
fn state_line(view: &Entity<MainWindow>, cx: &mut AsyncApp) -> String {
    view.read_with(cx, |main, cx| {
        let store = main.store().read(cx);
        format!(
            "signed_in={} needs_sign_in={} column={:?} room={} needs={} open_tasks={} pending={:?} toast={:?}",
            store.is_signed_in(),
            store.shows_sign_in(),
            main.column_page(cx),
            store.room(),
            store.needs().map_or(0, |needs| needs.count()),
            store.needs().map_or(0, |needs| needs.open_tasks().len()),
            store.pending().map(crate::store::PendingMessage::note),
            store.answers().toast().map(|toast| toast.text.clone()),
        )
    })
}

/// The safety net for answering a test card on a page of real ones: an answer just given for
/// any other task is taken back at once, inside its Undo window, before it reaches the server.
fn guard(want: TaskId, view: &Entity<MainWindow>, cx: &mut AsyncApp) {
    let toast = view.read_with(cx, |main, cx| {
        main.store().read(cx).answers().toast().cloned()
    });
    let line = match toast {
        Some(toast) if toast.undoable() && toast.task != want => {
            view.update(cx, |main, cx| {
                main.store()
                    .update(cx, |store, cx| store.undo(toast.serial, cx));
            });
            format!(
                "guard: the answer for task {} ({}) was not the test card's: undone",
                toast.task, toast.text
            )
        }
        Some(toast) => format!(
            "guard: the answer for task {}: {} (sent={})",
            toast.task,
            toast.text,
            !toast.undoable()
        ),
        None => "guard: no answer given".to_owned(),
    };
    report("DRIVE", &line);
}

fn in_window(window: AnyWindowHandle, cx: &mut AsyncApp, f: impl FnOnce(&mut Window, &mut App)) {
    if let Err(error) = cx.update_window(window, |_, window, cx| f(window, cx)) {
        report("DRIVE", &format!("the window is gone: {error}"));
    }
}

fn refresh(window: AnyWindowHandle, cx: &mut AsyncApp) {
    in_window(window, cx, |window, _| window.refresh());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_are_read_from_lines() {
        let step = |line: &str| line.parse::<Step>().unwrap();
        assert_eq!(step("page needs"), Step::Open(Page::Needs));
        assert_eq!(step("room 12"), Step::Open(Page::Room(RoomId::new(12))));
        assert_eq!(step("click 10 20.5"), Step::Click { x: 10., y: 20.5 });
        assert_eq!(step("a11y"), Step::Accessibility);
        assert_eq!(
            step("scroll 1 2 -300"),
            Step::Scroll {
                x: 1.,
                y: 2.,
                dy: -300.
            }
        );
        assert_eq!(step("type Hello there"), Step::Type("Hello there".into()));
        assert_eq!(step("key cmd-a"), Step::Key("cmd-a".into()));
        assert_eq!(
            step(&format!("guard {TASK_KEY_PREFIX}7")),
            Step::Guard(TaskId::new(7))
        );
        assert_eq!(step("guard 42"), Step::Guard(TaskId::new(42)));
        assert_eq!(step("wait 500"), Step::Wait(Duration::from_millis(500)));
        assert_eq!(step("  mem "), Step::Memory);
        assert_eq!(step("state"), Step::State);
        assert_eq!(step("quit"), Step::Quit);
    }

    #[test]
    fn the_file_is_read_a_whole_line_at_a_time() {
        use std::io::Write as _;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("steps");
        let mut steps = StepFile::new(path.clone());
        assert_eq!(
            steps.read_new().unwrap(),
            Vec::<String>::new(),
            "no file yet"
        );
        let mut file = std::fs::File::create(&path).unwrap();
        file.write_all(b"page needs\nwait 5").unwrap();
        assert_eq!(steps.read_new().unwrap(), ["page needs"]);
        assert_eq!(
            steps.read_new().unwrap(),
            Vec::<String>::new(),
            "the half line waits"
        );
        file.write_all(b"00\nquit\n").unwrap();
        assert_eq!(steps.read_new().unwrap(), ["wait 500", "quit"]);
    }

    #[test]
    fn malformed_lines_are_not_steps() {
        for line in [
            "page inbox",
            "room x",
            "click 1",
            "scroll 1 2",
            "key",
            "guard me",
            "wait soon",
            "fly",
        ] {
            assert!(line.parse::<Step>().is_err(), "{line}");
        }
    }
}
