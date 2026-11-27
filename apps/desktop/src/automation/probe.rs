//! What the chat view tells the automation: when a room's chat was first painted, and the
//! scroll benchmark's frames.

use std::time::Instant;

use emberhut_api::ids::RoomId;

/// How many frames the scroll benchmark goes one way before turning.
const FRAMES_PER_SWIPE: u32 = 90;

/// How far it scrolls each frame, in pixels.
const STEP: f32 = 40.;

/// The chat's side of the automation.
#[derive(Debug, Default)]
pub(crate) struct ChatProbe {
    painted: Option<(RoomId, Instant)>,
    scroll: Option<ScrollBench>,
}

impl ChatProbe {
    /// The chat painted `count` messages of `room`.
    pub(crate) fn painted(&mut self, room: RoomId, count: usize) {
        if count > 0 && self.painted.is_none_or(|(painted, _)| painted != room) {
            self.painted = Some((room, Instant::now()));
        }
    }

    /// The room whose chat was painted last, and when.
    #[must_use]
    pub(crate) fn last_painted(&self) -> Option<(RoomId, Instant)> {
        self.painted
    }

    /// Starts the scroll benchmark.
    pub(crate) fn start_scrolling(&mut self) {
        self.scroll = Some(ScrollBench::default());
    }

    /// Ends it: the time between frames, in milliseconds.
    pub(crate) fn stop_scrolling(&mut self) -> Vec<f64> {
        self.scroll.take().map_or_default(|bench| bench.frames)
    }

    /// A frame while the benchmark runs: how far to scroll (and to ask for the next frame).
    pub(crate) fn bench_frame(&mut self) -> Option<f32> {
        let bench = self.scroll.as_mut()?;
        Some(bench.frame(Instant::now(), self.painted.is_some()))
    }
}

/// The scroll benchmark: swipes up and back down, one step a frame, timing each frame.
#[derive(Debug, Default)]
struct ScrollBench {
    last: Option<Instant>,
    frames: Vec<f64>,
    step: u32,
}

impl ScrollBench {
    /// A frame at `now`: how far to scroll (nothing until the chat has messages).
    fn frame(&mut self, now: Instant, painted: bool) -> f32 {
        if !painted {
            return 0.;
        }
        if let Some(last) = self.last {
            self.frames.push((now - last).as_secs_f64() * 1000.);
        }
        self.last = Some(now);
        self.step += 1;
        if (self.step / FRAMES_PER_SWIPE).is_multiple_of(2) {
            -STEP
        } else {
            STEP
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn a_room_is_painted_once_until_another_is() {
        let mut probe = ChatProbe::default();
        probe.painted(RoomId::new(1), 0);
        assert_eq!(probe.last_painted(), None, "nothing painted yet");
        probe.painted(RoomId::new(1), 5);
        let first = probe.last_painted().unwrap();
        probe.painted(RoomId::new(1), 6);
        assert_eq!(probe.last_painted(), Some(first));
        probe.painted(RoomId::new(2), 1);
        assert_eq!(
            probe.last_painted().map(|(room, _)| room),
            Some(RoomId::new(2))
        );
    }

    #[test]
    fn the_scroll_bench_swipes_up_then_down_and_times_frames() {
        let mut bench = ScrollBench::default();
        let start = Instant::now();
        assert_eq!(bench.frame(start, false), 0.);
        let deltas: Vec<f32> = (0..200)
            .map(|i| bench.frame(start + Duration::from_millis(16 * i), true))
            .collect();
        assert_eq!(deltas[0], -STEP);
        assert_eq!(deltas[100], STEP);
        assert_eq!(bench.frames.len(), 199);
        assert!((bench.frames[0] - 16.).abs() < 1e-9);
    }

    #[test]
    fn the_probe_hands_back_the_frames() {
        let mut probe = ChatProbe::default();
        assert_eq!(probe.bench_frame(), None);
        probe.start_scrolling();
        assert_eq!(probe.bench_frame(), Some(0.));
        assert_eq!(probe.stop_scrolling(), Vec::<f64>::new());
        assert_eq!(probe.bench_frame(), None);
    }
}
