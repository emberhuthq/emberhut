//! The video playing in place, on macOS: which one it is and how far it got (one at a time, in
//! a global), its frames painted as GPUI surfaces, and its bar.

use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui_kit::assets::IconName;
use gpui_kit::component::Icon;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, AsyncApp, Bounds, Entity, Global, MouseButton, ObjectFit, Pixels, Window,
    black, canvas, div, px, relative, size, white,
};

use super::mac::Player;
use super::{clock_line, share};
use crate::store::Store;
use crate::store::files::RemoteFile;
use crate::ui::a11y::Accessible as _;

/// How long a playing video may go unseen (its row scrolled away, another room shown, its
/// window closed) before it pauses.
const UNSEEN: Duration = Duration::from_secs(1);

/// The video playing in place, if one is.
#[derive(Default)]
struct InlineVideo {
    now: Option<Now>,
    /// The number of the last video started: a fetch for an older one is dropped.
    started: u64,
}

impl Global for InlineVideo {}

/// The video in place now.
struct Now {
    file: RemoteFile,
    stage: Stage,
    /// Its number ([`InlineVideo::started`]).
    number: u64,
    /// When its frame was last painted.
    seen: Instant,
}

/// Where it is.
enum Stage {
    /// Its file is being fetched.
    Fetching,
    /// It can't play, and why.
    Failed(String),
    /// Playing or paused.
    Playing(Player),
}

/// The video in place, if it is number `number` (or any, with none).
fn now_mut(cx: &mut App, number: Option<u64>) -> Option<&mut Now> {
    if !cx.has_global::<InlineVideo>() {
        return None;
    }
    cx.global_mut::<InlineVideo>()
        .now
        .as_mut()
        .filter(|now| number.is_none_or(|number| now.number == number))
}

/// Plays `file` in place: fetched first (as the viewer does), then playing, the one playing
/// before stopped. Always `true`: it plays here.
pub(crate) fn play(store: &Entity<Store>, file: &RemoteFile, cx: &mut App) -> bool {
    let video = cx.default_global::<InlineVideo>();
    video.started += 1;
    let number = video.started;
    video.now = Some(Now {
        file: file.clone(),
        stage: Stage::Fetching,
        number,
        seen: Instant::now(),
    });
    let fetching = store.update(cx, |store, cx| store.fetch_file(file, cx));
    cx.spawn(async move |cx: &mut AsyncApp| {
        let kept = fetching.kept().await;
        cx.update(|cx| {
            let stage = match kept {
                Ok(path) => Player::open(&path).map_or_else(
                    || Stage::Failed("Couldn't play the video".to_owned()),
                    Stage::Playing,
                ),
                Err(why) => Stage::Failed(why),
            };
            if let Some(now) = now_mut(cx, Some(number)) {
                now.stage = stage;
                now.seen = Instant::now();
            }
            cx.refresh_windows();
        });
        pause_when_unseen(number, cx).await;
    })
    .detach();
    cx.refresh_windows();
    true
}

/// Pauses video `number` whenever it goes unseen for [`UNSEEN`], for as long as it is the one
/// in place.
async fn pause_when_unseen(number: u64, cx: &mut AsyncApp) {
    loop {
        cx.background_executor().timer(UNSEEN).await;
        let still_here = cx.update(|cx| {
            let Some(now) = now_mut(cx, Some(number)) else {
                return false;
            };
            let unseen = now.seen.elapsed() > UNSEEN;
            let paused = match &now.stage {
                Stage::Playing(player) if unseen && player.is_playing() => {
                    player.pause();
                    true
                }
                _ => false,
            };
            if paused {
                cx.refresh_windows();
            }
            true
        });
        if !still_here {
            break;
        }
    }
}

/// Runs `f` on the player in place, then draws again.
fn with_player(cx: &mut App, f: impl FnOnce(&Player)) {
    if let Some(now) = now_mut(cx, None)
        && let Stage::Playing(player) = &now.stage
    {
        f(player);
        now.seen = Instant::now();
    }
    cx.refresh_windows();
}

/// Play or pause.
fn toggle(cx: &mut App) {
    with_player(cx, |player| {
        if player.is_playing() {
            player.pause();
        } else {
            player.play();
        }
    });
}

/// Full screen: the video moves to a window of its own (`AVKit`'s player there), paused here.
fn full_screen(store: &Entity<Store>, cx: &mut App) {
    let Some(file) = now_mut(cx, None).map(|now| now.file.clone()) else {
        return;
    };
    with_player(cx, Player::pause);
    crate::ui::viewer::own_window::open(store, file, true, cx);
}

/// The video `file` in place, `width` by `height`, while it plays there; `None` while it
/// doesn't (its row shows its still).
pub(crate) fn playing(
    file: &RemoteFile,
    (width, height): (f32, f32),
    store: &Entity<Store>,
    cx: &App,
) -> Option<AnyElement> {
    let now = cx
        .try_global::<InlineVideo>()?
        .now
        .as_ref()
        .filter(|now| now.file.path == file.path)?;
    let frame = div()
        .relative()
        .w(px(width))
        .h(px(height))
        .rounded(px(12.))
        .overflow_hidden()
        .bg(black());
    let centred = || {
        div()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .text_size(px(12.))
            .text_color(white().opacity(0.8))
    };
    Some(match &now.stage {
        Stage::Fetching => frame
            .child(centred().child(Spinner::new().color(white())))
            .into_any_element(),
        Stage::Failed(why) => frame
            .child(centred().px(px(12.)).child(why.clone()))
            .into_any_element(),
        Stage::Playing(player) => {
            let path = file.path.clone();
            frame
                .child(
                    canvas(
                        |_, _, _| {},
                        move |bounds, (), window, cx| paint(&path, bounds, window, cx),
                    )
                    .absolute()
                    .inset_0(),
                )
                .child(bar(player, store))
                .into_any_element()
        }
    })
}

/// Paints the newest frame of the video at `path` in `bounds`, and asks for the next frame
/// while it plays.
fn paint(path: &str, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut App) {
    let Some(now) = now_mut(cx, None).filter(|now| now.file.path == path) else {
        return;
    };
    now.seen = Instant::now();
    let Stage::Playing(player) = &mut now.stage else {
        return;
    };
    let moving = player.is_playing();
    if let Some((frame, (width, height))) = player.frame()
        && width > 0
        && height > 0
    {
        let natural = size(width.into(), height.into());
        window.paint_surface(ObjectFit::Contain.get_bounds(bounds, natural), frame);
    }
    if moving {
        window.request_animation_frame();
    }
}

/// The bar over the video's foot: play or pause, how far in of how long, a track to click to a
/// place, full screen.
fn bar(player: &Player, store: &Entity<Store>) -> impl IntoElement {
    let (position, duration, moving) = (player.position(), player.duration(), player.is_playing());
    let track = Rc::new(Cell::new(None::<Bounds<Pixels>>));
    let (measured, read) = (Rc::clone(&track), track);
    let to_full = store.clone();
    div()
        .absolute()
        .left_0()
        .right_0()
        .bottom_0()
        .flex()
        .items_center()
        .gap(px(8.))
        .px(px(10.))
        .py(px(6.))
        .bg(black().opacity(0.55))
        .text_size(px(11.5))
        .text_color(white())
        .child(control(
            "inline-video-play",
            if moving {
                IconName::Pause
            } else {
                IconName::Play
            },
            if moving { "Pause" } else { "Play" },
            toggle,
        ))
        .child(div().flex_none().child(clock_line(position, duration)))
        .child(
            div()
                .id("inline-video-track")
                .button_named("Playback position")
                .relative()
                .flex_1()
                .h(px(14.))
                .flex()
                .items_center()
                .cursor_pointer()
                .child(
                    div()
                        .relative()
                        .w_full()
                        .h(px(4.))
                        .rounded_full()
                        .bg(white().opacity(0.3))
                        .child(
                            div()
                                .absolute()
                                .left_0()
                                .top_0()
                                .bottom_0()
                                .w(relative(share(position, duration)))
                                .rounded_full()
                                .bg(white()),
                        ),
                )
                .child(
                    canvas(
                        move |bounds, _, _| measured.set(Some(bounds)),
                        |_, (), _, _| {},
                    )
                    .absolute()
                    .inset_0(),
                )
                .on_mouse_down(MouseButton::Left, move |event, _, cx| {
                    cx.stop_propagation();
                    let (Some(track), Some(duration)) = (read.get(), duration) else {
                        return;
                    };
                    let across = f32::from(event.position.x - track.origin.x)
                        / f32::from(track.size.width).max(1.);
                    let to = duration * f64::from(across.clamp(0., 1.));
                    with_player(cx, |player| player.seek(to));
                }),
        )
        .child(control(
            "inline-video-full-screen",
            IconName::Maximize,
            "Full screen",
            move |cx| full_screen(&to_full, cx),
        ))
}

/// One of the bar's buttons, named `label` for a screen reader.
fn control(
    id: &'static str,
    icon: IconName,
    label: &'static str,
    press: impl Fn(&mut App) + 'static,
) -> impl IntoElement {
    div()
        .id(id)
        .button_named(label)
        .flex_none()
        .cursor_pointer()
        .child(Icon::new(icon).size(px(16.)).text_color(white()))
        .on_click(move |_, _, cx| {
            cx.stop_propagation();
            press(cx);
        })
}
