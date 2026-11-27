//! `--bench`: the app measures itself and quits.
//!
//! - `start`: launch to the window, the first state and the chat's first paint; memory then and
//!   once it settles.
//! - `scroll:SECONDS`: swipes up and down the chat, one step a frame; the frame times.
//! - `rooms:ID,ID,…`: opens those rooms in turn, three rounds, timing each switch to its chat's
//!   paint.
//! - `demo`: a tour of every screen, for a screen recording (it doesn't quit).

use std::str::FromStr;
use std::time::{Duration, Instant};

use gpui_kit::{App, AsyncApp, Entity};

use super::{report, since_start, usage};
use crate::store::{OpenReport, Page};
use crate::ui::window::MainWindow;
use emberhut_api::chat::RefKind;
use emberhut_api::ids::RoomId;

/// What to measure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum BenchPlan {
    /// The launch.
    Start,
    /// Scrolling the chat for that long.
    Scroll(Duration),
    /// Switching between these rooms.
    Rooms(Vec<RoomId>),
    /// A tour of every screen.
    Demo,
}

/// Why a plan wasn't understood.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("`{0}` isn't a benchmark: start, scroll:SECONDS, rooms:ID,ID,… or demo")]
pub(crate) struct BadPlan(String);

impl FromStr for BenchPlan {
    type Err = BadPlan;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let bad = || BadPlan(text.to_owned());
        match text.split_once(':') {
            None if text == "start" => Ok(Self::Start),
            None if text == "demo" => Ok(Self::Demo),
            Some(("scroll", seconds)) => seconds
                .parse()
                .map(|s| Self::Scroll(Duration::from_secs(s)))
                .map_err(|_| bad()),
            Some(("rooms", ids)) => {
                let rooms: Result<Vec<RoomId>, _> = ids
                    .split(',')
                    .map(|id| id.trim().parse().map(RoomId::new))
                    .collect();
                rooms
                    .ok()
                    .filter(|rooms| !rooms.is_empty())
                    .map(Self::Rooms)
                    .ok_or_else(bad)
            }
            _ => Err(bad()),
        }
    }
}

/// How long to wait for the chat's first paint.
const PAINT_TIMEOUT: Duration = Duration::from_secs(60);

/// Starts the plan.
pub(crate) fn start(plan: BenchPlan, view: Entity<MainWindow>, cx: &mut App) {
    report("BENCH", &format!("window_opened_ms={}", since_start()));
    cx.spawn(async move |cx| {
        match &plan {
            BenchPlan::Start => launch(&view, cx).await,
            BenchPlan::Scroll(length) => scroll(&view, *length, cx).await,
            BenchPlan::Rooms(rooms) => switches(&view, rooms, cx).await,
            BenchPlan::Demo => demo(&view, cx).await,
        }
        if plan_quits(&plan) {
            cx.update(|cx| cx.quit());
        }
    })
    .detach();
}

fn plan_quits(plan: &BenchPlan) -> bool {
    !matches!(plan, BenchPlan::Demo)
}

async fn sleep(cx: &AsyncApp, length: Duration) {
    cx.background_executor().timer(length).await;
}

/// Waits for the chat's first paint; whether it came.
async fn chat_painted(view: &Entity<MainWindow>, cx: &mut AsyncApp) -> bool {
    let deadline = Instant::now() + PAINT_TIMEOUT;
    while Instant::now() < deadline {
        if view.read_with(cx, |window, cx| {
            window.chat().read(cx).probe.last_painted().is_some()
        }) {
            return true;
        }
        sleep(cx, Duration::from_millis(100)).await;
    }
    false
}

async fn launch(view: &Entity<MainWindow>, cx: &mut AsyncApp) {
    let painted = chat_painted(view, cx).await;
    let first_state = view.read_with(cx, |window, cx| {
        let metrics = window.store().read(cx).metrics();
        metrics.first_state().unwrap_or_default().as_millis()
    });
    report(
        "BENCH",
        &format!(
            "first_state_ms={first_state} chat_painted={painted} chat_paint_ms={}",
            since_start()
        ),
    );
    report("BENCH", &format!("after_paint {}", usage::memory()));
    // The processor time the 10 s at rest cost (compare with and without the material).
    let rest_from = usage::process_cpu_ms();
    sleep(cx, Duration::from_secs(10)).await;
    let rest_cpu = usage::process_cpu_ms() - rest_from;
    report(
        "BENCH",
        &format!("settled_10s rest_cpu_ms={rest_cpu:.0} {}", usage::memory()),
    );
}

async fn scroll(view: &Entity<MainWindow>, length: Duration, cx: &mut AsyncApp) {
    chat_painted(view, cx).await;
    sleep(cx, Duration::from_secs(2)).await; // the first pictures and markdown settle
    view.update(cx, |window, cx| {
        window.chat().update(cx, |chat, cx| {
            chat.probe.start_scrolling();
            cx.notify();
        });
    });
    let started = Instant::now();
    let (main0, process0) = (usage::main_thread_cpu_ms(), usage::process_cpu_ms());
    sleep(cx, length).await;
    let (main1, process1) = (usage::main_thread_cpu_ms(), usage::process_cpu_ms());
    let frames = view.update(cx, |window, cx| {
        window
            .chat()
            .update(cx, |chat, _| chat.probe.stop_scrolling())
    });
    let messages = view.read_with(cx, |window, cx| {
        let store = window.store().read(cx);
        store.chats().messages(store.room()).len()
    });
    let seconds = started.elapsed().as_secs_f64();
    let stats = FrameStats::of(&frames);
    let per_frame =
        (main1 - main0) / f64::from(u32::try_from(frames.len().max(1)).unwrap_or(u32::MAX));
    report(
        "BENCH",
        &format!(
            "scroll secs={seconds:.1} frames={} fps={:.1} p50_ms={:.2} p95_ms={:.2} p99_ms={:.2} max_ms={:.2} \
             over_16ms={} over_33ms={} messages={messages} main_thread_cpu_ms={:.0} main_cpu_per_frame_ms={per_frame:.2} \
             process_cpu_ms={:.0} {}",
            frames.len(),
            stats.count / seconds,
            stats.p50,
            stats.p95,
            stats.p99,
            stats.max,
            stats.over_16,
            stats.over_33,
            main1 - main0,
            process1 - process0,
            usage::memory()
        ),
    );
}

async fn switches(view: &Entity<MainWindow>, rooms: &[RoomId], cx: &mut AsyncApp) {
    chat_painted(view, cx).await;
    sleep(cx, Duration::from_secs(2)).await;
    for round in 0..3 {
        for &room in rooms {
            let switched = Instant::now();
            view.update(cx, |window, cx| {
                window
                    .store()
                    .update(cx, |store, cx| store.switch_room(room, cx));
            });
            let took = room_painted(view, room, switched, cx).await;
            report(
                "BENCH",
                &format!(
                    "room={room} round={round} held_before={} switch_to_paint_ms={}",
                    round > 0,
                    millis(took)
                ),
            );
            sleep(cx, Duration::from_secs(3)).await;
        }
    }
    report("BENCH", &format!("rooms done {}", usage::memory()));
}

/// How long `room`'s chat took to paint after `since` (up to 2 s, polled every 5 ms).
pub(super) async fn room_painted(
    view: &Entity<MainWindow>,
    room: RoomId,
    since: Instant,
    cx: &mut AsyncApp,
) -> Option<Duration> {
    for _ in 0..400 {
        let painted = view.read_with(cx, |window, cx| window.chat().read(cx).probe.last_painted());
        if let Some((_, at)) = painted.filter(|(painted, at)| *painted == room && *at >= since) {
            return Some(at - since);
        }
        sleep(cx, Duration::from_millis(5)).await;
    }
    None
}

/// A duration in milliseconds, `-1` when there is none.
pub(super) fn millis(took: Option<Duration>) -> String {
    took.map_or_else(|| "-1".to_owned(), |took| took.as_millis().to_string())
}

async fn demo(view: &Entity<MainWindow>, cx: &mut AsyncApp) {
    let open = |page: Page, cx: &mut AsyncApp| {
        view.update(cx, |window, cx| window.show(page, cx));
    };
    chat_painted(view, cx).await;
    report("DEMO", "chat");
    sleep(cx, Duration::from_millis(3500)).await;
    for i in 0..120 {
        view.update(cx, |window, cx| {
            window.chat().update(cx, |chat, cx| {
                chat.scroll(if i < 60 { -18. } else { 18. });
                cx.notify();
            });
        });
        sleep(cx, Duration::from_millis(16)).await;
    }
    sleep(cx, Duration::from_millis(800)).await;
    // The newest report in the chat, beside it.
    let newest = view.read_with(cx, |window, cx| {
        let store = window.store().read(cx);
        store
            .chats()
            .messages(store.room())
            .iter()
            .rev()
            .find_map(|message| {
                let reference = message.refs.iter().find(|r| r.kind == RefKind::Report)?;
                Some(OpenReport {
                    name: reference
                        .name
                        .clone()
                        .unwrap_or_else(|| reference.label.clone()),
                    path: reference.url.clone()?,
                })
            })
    });
    if let Some(newest) = newest {
        report("DEMO", &format!("report {}", newest.name));
        view.update(cx, |window, cx| {
            window
                .store()
                .update(cx, |store, cx| store.show_report(Some(newest), cx));
        });
        sleep(cx, Duration::from_millis(4500)).await;
        view.update(cx, |window, cx| {
            window
                .store()
                .update(cx, |store, cx| store.show_report(None, cx));
        });
    }
    for (name, page) in [
        ("needs", Page::Needs),
        ("tasks", Page::Tasks),
        ("machines", Page::Machines),
    ] {
        report("DEMO", name);
        open(page, cx);
        sleep(cx, Duration::from_millis(3500)).await;
    }
    let rooms: Vec<RoomId> = view.read_with(cx, |window, cx| {
        window
            .store()
            .read(cx)
            .switcher()
            .listed()
            .iter()
            .take(3)
            .map(|room| room.id)
            .collect()
    });
    for room in rooms {
        report("DEMO", &format!("room {room}"));
        open(Page::Room(room), cx);
        sleep(cx, Duration::from_millis(2500)).await;
    }
    report("DEMO", "settings");
    open(Page::Settings, cx);
    sleep(cx, Duration::from_millis(2500)).await;
    open(Page::Room(RoomId::MAIN), cx);
    report("DEMO", "done");
}

/// The frame times' percentiles, in milliseconds.
#[derive(Clone, Debug, Default, PartialEq)]
struct FrameStats {
    count: f64,
    p50: f64,
    p95: f64,
    p99: f64,
    max: f64,
    over_16: usize,
    over_33: usize,
}

impl FrameStats {
    fn of(frames: &[f64]) -> Self {
        let mut sorted = frames.to_vec();
        sorted.sort_by(f64::total_cmp);
        let at = |share: f64| {
            let last = sorted.len().saturating_sub(1);
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                clippy::cast_precision_loss,
                reason = "an index into a list of frames, rounded down: small and not negative"
            )]
            let index = (last as f64 * share) as usize;
            sorted.get(index).copied().unwrap_or(0.)
        };
        #[expect(
            clippy::cast_precision_loss,
            reason = "a count of frames stays far below 2^52"
        )]
        let count = frames.len() as f64;
        Self {
            count,
            p50: at(0.5),
            p95: at(0.95),
            p99: at(0.99),
            max: at(1.),
            over_16: frames.iter().filter(|&&ms| ms > 16.7).count(),
            over_33: frames.iter().filter(|&&ms| ms > 33.4).count(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plans_are_read_from_the_command_line() {
        assert_eq!("start".parse(), Ok(BenchPlan::Start));
        assert_eq!("demo".parse(), Ok(BenchPlan::Demo));
        assert_eq!(
            "scroll:15".parse(),
            Ok(BenchPlan::Scroll(Duration::from_secs(15)))
        );
        assert_eq!(
            "rooms:5, 10,2".parse(),
            Ok(BenchPlan::Rooms(vec![
                RoomId::new(5),
                RoomId::new(10),
                RoomId::new(2)
            ]))
        );
        for bad in ["", "scroll", "scroll:x", "rooms:", "rooms:1,a", "fly"] {
            assert!(bad.parse::<BenchPlan>().is_err(), "{bad}");
        }
    }

    #[test]
    fn only_the_demo_stays_open() {
        assert!(plan_quits(&BenchPlan::Start));
        assert!(!plan_quits(&BenchPlan::Demo));
    }

    #[test]
    fn frame_stats_read_the_sorted_times() {
        let frames: Vec<f64> = (1..=100).map(f64::from).collect();
        let stats = FrameStats::of(&frames);
        assert_eq!(
            (stats.p50, stats.p95, stats.p99, stats.max),
            (50., 95., 99., 100.)
        );
        assert_eq!((stats.over_16, stats.over_33), (84, 67));
        assert_eq!(FrameStats::of(&[]), FrameStats::default());
    }

    #[test]
    fn missing_durations_read_minus_one() {
        assert_eq!(millis(None), "-1");
        assert_eq!(millis(Some(Duration::from_millis(12))), "12");
    }
}
