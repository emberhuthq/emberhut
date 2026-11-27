//! The whole screen, with the owner's OK (the server asks): real clicks and keys, while a bar
//! on every Space says the assistant is using the screen, with Stop. It ends when its time is
//! up, when the agent stops it, when the owner presses Stop, and the moment the owner moves the
//! mouse or types, even in the middle of a step: the assistant's events come from a source of
//! its own and are counted, so any other input the Mac counts is the owner's. The next
//! request after an end says why it ended, so the agent doesn't start again on its own, and a
//! new start waits a minute after the owner ended one. The app rules hold here too: nothing is
//! done while an app the helper never uses (or only looks at) is in front, or under a click, and
//! nothing is typed while a password field has the focus.

use std::time::{Duration, Instant};

use base64::Engine as _;
use serde_json::{Value, json};

use super::{Hands, lock, object};
use crate::desktop::{Desktop, Posting};
use crate::fail::{Fail, Reply, field, number, said};
use crate::keys::{Button, Input, check_length, chord, clicks, is_navigation, press, typing};
use crate::rules::{Use, refusal};
use crate::text::clip;

/// The longest session, in minutes.
const MAX_MINUTES: f64 = 60.0;
/// A session's minutes when none are asked.
const MINUTES: f64 = 15.0;
/// Input this recent while driving is the owner's, when the assistant posted none for
/// [`OURS_SETTLE`].
const TOUCH: f64 = 0.5;
/// How long the assistant's own input still shows as the Mac's last input.
const OURS_SETTLE: Duration = Duration::from_millis(1500);
/// Before a step: input this recent, with none of ours for [`STEP_SETTLE`], is the owner's.
const STEP_TOUCH: f64 = 1.5;
/// See [`STEP_TOUCH`].
const STEP_SETTLE: Duration = Duration::from_secs(2);
/// The default pixels a scroll moves.
const SCROLL: f64 = 400.0;
/// How long after the owner ended a session a new one may start.
const COOLDOWN: Duration = Duration::from_secs(60);
/// How long a start waits for the bar to be up.
pub(super) const BAR_UP: Duration = Duration::from_secs(3);
/// The largest picture sent, in bytes of JPEG (the server reads answers up to 16 MiB).
pub const MAX_PICTURE: usize = 6_000_000;

/// Why a screen session ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ended {
    /// The owner pressed Stop on the bar.
    Stop,
    /// The owner moved the mouse or typed.
    Touch,
    /// Its time was up.
    Time,
    /// The agent stopped it, or it couldn't start.
    Agent,
}

/// The session running now.
#[derive(Clone, Debug)]
struct Driving {
    who: String,
    reason: String,
    until: Instant,
}

/// The screen session's state.
#[derive(Debug, Default)]
pub struct Screen {
    driving: Option<Driving>,
    /// How and when the last session ended.
    ended: Option<(Ended, Instant)>,
    /// The assistant is posting input now.
    posting: bool,
    /// The Mac's input count (mouse moves, presses, keys, scrolls) as it should be if only the
    /// assistant had made input since the session started: anything above it is the owner's.
    expected: u64,
    /// When the assistant last posted an input event.
    ours: Option<Instant>,
}

impl Screen {
    /// Whether a session runs.
    #[must_use]
    pub fn driving(&self) -> bool {
        self.driving.is_some()
    }

    /// Who drives, when a session runs.
    #[must_use]
    pub fn who(&self) -> Option<String> {
        self.driving.as_ref().map(|d| d.who.clone())
    }

    /// The bar's reason (or who) while a session runs.
    #[must_use]
    pub fn reason(&self) -> Option<String> {
        self.driving.as_ref().map(|d| {
            if d.reason.is_empty() {
                d.who.clone()
            } else {
                d.reason.clone()
            }
        })
    }

    /// How long ago the owner ended a session (Stop or touch).
    #[must_use]
    pub fn stopped_for(&self, now: Instant) -> Option<Duration> {
        match self.ended {
            Some((Ended::Stop | Ended::Touch, at)) => Some(now.saturating_duration_since(at)),
            _ => None,
        }
    }

    /// Ends the session (if one runs) for `cause`.
    pub fn end(&mut self, cause: Ended, now: Instant) {
        if self.driving.take().is_some() || matches!(cause, Ended::Stop) {
            self.ended = Some((cause, now));
        }
    }

    /// Seconds since the assistant's own last input event.
    fn since_ours(&self, now: Instant) -> f64 {
        self.ours.map_or(f64::INFINITY, |at| {
            now.saturating_duration_since(at).as_secs_f64()
        })
    }

    /// Whether the owner made input: the Mac counted more input than the assistant made
    /// (`count`, its input count now). Between steps, once the assistant's own input has
    /// settled, recent input (`idle` seconds ago) is the owner's too, and the expected count
    /// follows the Mac's, in case the assistant's own events aren't counted.
    fn touched(&mut self, now: Instant, idle: f64, count: u64) -> bool {
        // While it posts, each event checks before it goes (an event just posted may be counted
        // before its expected count is raised).
        if self.posting {
            return false;
        }
        if count > self.expected {
            return true;
        }
        let settled = self.since_ours(now) > OURS_SETTLE.as_secs_f64();
        if settled {
            self.expected = count;
        }
        settled && idle < TOUCH
    }

    /// The watchdog's check at `now`, with the Mac idle `idle` seconds and its input count
    /// `count`: why the session ended, if it did.
    pub fn watch(&mut self, now: Instant, idle: f64, count: u64) -> Option<Ended> {
        let until = self.driving.as_ref()?.until;
        let cause = if until <= now {
            Ended::Time
        } else if self.touched(now, idle, count) {
            Ended::Touch
        } else {
            return None;
        };
        self.end(cause, now);
        Some(cause)
    }

    /// Starts counting at `count`, the Mac's input count when the session starts.
    fn count_from(&mut self, count: u64) {
        self.expected = count;
        self.ours = None;
    }

    /// Before each event the assistant posts (with the Mac's input count `count`): whether it
    /// may. Not once the session ended (Stop); not once the Mac counted input the assistant
    /// didn't make, which ends the session (the owner's touch).
    fn may_post(&mut self, now: Instant, count: u64) -> bool {
        if !self.driving() {
            return false;
        }
        if count > self.expected {
            self.end(Ended::Touch, now);
            return false;
        }
        true
    }

    /// After `input` was posted: the Mac counts it as the assistant's.
    fn sent(&mut self, input: &Input) {
        if input.counted() {
            self.expected += 1;
        }
    }

    /// After a step's input was posted (or stopped) at `now`.
    fn posted(&mut self, now: Instant) {
        self.posting = false;
        self.ours = Some(now);
    }

    /// Before a step: whether the owner is using the Mac.
    fn in_use(&mut self, now: Instant, idle: f64, count: u64) -> bool {
        count > self.expected
            || (idle < STEP_TOUCH && self.since_ours(now) > STEP_SETTLE.as_secs_f64())
    }

    /// Why a new session can't start yet: the owner ended one less than [`COOLDOWN`] ago.
    fn cooling(&self, now: Instant) -> Option<Fail> {
        let ago = self.stopped_for(now)?;
        (ago < COOLDOWN).then(|| {
            Fail::clash(format!(
                "The owner ended the assistant's use of the screen {} s ago: a new start waits a \
                 minute, and their OK.",
                ago.as_secs()
            ))
        })
    }

    /// The refusal for a step by `who` when no session of theirs runs, saying why.
    fn not_driving(&self, who: &str, now: Instant) -> Option<Fail> {
        match &self.driving {
            Some(d) if d.who == who => None,
            Some(d) => Some(Fail::clash(format!("{} is using the screen now.", d.who))),
            None => {
                let ago = |at: Instant| now.saturating_duration_since(at).as_secs();
                Some(Fail::clash(match self.ended {
                    Some((Ended::Stop, at)) => format!(
                        "The owner pressed Stop on the bar {} s ago, so the assistant is not \
                         using the screen. Don't start again on your own: ask them first \
                         (mac_screen start asks).",
                        ago(at)
                    ),
                    Some((Ended::Touch, at)) => format!(
                        "The owner used the mouse or keyboard {} s ago, so the assistant \
                         stopped using the screen. Don't start again on your own: tell them, \
                         or wait for their OK.",
                        ago(at)
                    ),
                    Some((Ended::Time, _)) => {
                        "The screen session's time ran out: mac_screen start asks again.".to_owned()
                    }
                    Some((Ended::Agent, _)) | None => {
                        "Not using the whole screen (it ended, or the owner pressed Stop): \
                         mac_screen start asks them again."
                            .to_owned()
                    }
                }))
            }
        }
    }
}

/// The bar's words: the screen session's reason, a recording's title, or both; `None` hides it.
#[must_use]
pub fn bar_text(driving: Option<&str>, recording: Option<&str>) -> Option<String> {
    match (driving, recording) {
        (Some(reason), Some(title)) => Some(format!(
            "Emberhut is using your screen: {reason} · recording {title}"
        )),
        (Some(reason), None) => Some(format!("Emberhut is using your screen: {reason}")),
        (None, Some(title)) => Some(format!("Emberhut is recording: {title}")),
        (None, None) => None,
    }
}

/// What the agent is told when the owner's touch ended the session.
fn owner_touched() -> Fail {
    Fail::clash(
        "The owner is using the Mac (the mouse or keyboard moved), so the assistant stopped \
         using the screen. Don't start again on your own: tell them, or wait for their OK.",
    )
}

/// Who a request is from: the server's driver, or the assistant.
fn who(body: &Reply) -> String {
    let who = field(body, "who");
    if who.is_empty() {
        "the assistant".to_owned()
    } else {
        who
    }
}

impl<D: Desktop> Hands<D> {
    /// `/screen/start`: starts a session for `who` (`reason`, up to `minutes`), once the bar
    /// with Stop is up. Never while the screen is locked, nor within a minute of the owner
    /// ending one.
    pub(super) fn screen_start(&self, body: &Reply) -> Result<Reply, Fail> {
        if !self.desktop.accessibility() || !self.desktop.screen_recording() {
            return Err(Fail::never(
                "Emberhut Hands needs Accessibility and Screen Recording first.",
            ));
        }
        if self.desktop.locked() {
            return Err(Fail::clash(
                "The Mac's screen is locked: the assistant never unlocks it.",
            ));
        }
        let who = who(body);
        let reason = clip(&field(body, "reason"), 70);
        let minutes = number(body, "minutes")
            .unwrap_or(MINUTES)
            .clamp(1.0, MAX_MINUTES);
        let now = self.clock.now();
        {
            let mut screen = lock(&self.screen);
            if let Some(refused) = screen.cooling(now) {
                return Err(refused);
            }
            if let Some(d) = &screen.driving
                && d.who != who
            {
                let left = d.until.saturating_duration_since(now).as_secs() / 60;
                return Err(Fail::clash(format!(
                    "{} is using the screen for {left} more minutes.",
                    d.who
                )));
            }
            screen.driving = Some(Driving {
                who: who.clone(),
                reason: reason.clone(),
                until: now + Duration::from_secs_f64(minutes * 60.0),
            });
            screen.count_from(self.desktop.input_count());
        }
        let asked = self.show_bar();
        if !self.bar.wait(asked, BAR_UP) {
            lock(&self.screen).end(Ended::Agent, self.clock.now());
            self.show_bar();
            return Err(Fail::broke(
                "The bar with Stop could not be shown on the Mac, so the screen is not used.",
            ));
        }
        // Whole minutes, as the bar and the server say them.
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "clamped to 1..=60 minutes"
        )]
        let minutes = minutes.floor() as u32;
        tracing::info!(%who, minutes, %reason, "started using the whole screen");
        Ok(said(format!(
            "Using the owner's whole screen for up to {minutes} minutes (a bar on it says so, \
             with Stop). mac_screen shot first."
        )))
    }

    /// `/screen/shot`: a picture of the main screen, its pixels the points clicks use.
    pub(super) fn screen_shot(&self, body: &Reply) -> Result<Reply, Fail> {
        self.driver(body)?;
        let picture = self
            .desktop
            .screen_picture()
            .filter(|picture| picture.jpeg.len() <= MAX_PICTURE)
            .ok_or_else(|| Fail::broke("No picture of the screen"))?;
        let mut reply = said(format!(
            "The owner's main screen, {}×{} points: x and y for mac_screen act are in this \
             picture's pixels.",
            picture.width.round(),
            picture.height.round()
        ));
        reply.insert(
            "image".to_owned(),
            Value::String(base64::engine::general_purpose::STANDARD.encode(picture.jpeg)),
        );
        Ok(reply)
    }

    /// `/screen/act`: one click, move, scroll, typed text or key, as the real mouse and keyboard.
    pub(super) fn screen_act(&self, body: &Reply) -> Result<Reply, Fail> {
        self.driver(body)?;
        let now = self.clock.now();
        {
            let mut screen = lock(&self.screen);
            let (idle, count) = (self.desktop.idle_seconds(), self.desktop.input_count());
            if screen.in_use(now, idle, count) {
                screen.end(Ended::Touch, now);
                drop(screen);
                self.show_bar();
                return Err(owner_touched());
            }
        }
        let action = field(body, "action");
        self.allowed_on_screen(&action, body)?;
        let done = if action == "type" {
            let typed = field(body, "text");
            check_length(&typed)?;
            self.type_checked(
                &typed,
                "The app in front",
                &|| self.desktop.focused(),
                &mut |piece| self.post_on_screen(&typing(piece)),
            )?
        } else {
            let inputs = Self::screen_inputs(&action, body)?;
            if action == "key" && !is_navigation(&field(body, "key")) {
                self.check_focus(self.desktop.focused(), "The app in front")?;
            }
            self.post_on_screen(&inputs)
        };
        if !done {
            self.show_bar();
            let ended = lock(&self.screen).ended.map(|(cause, _)| cause);
            return Err(if ended == Some(Ended::Touch) {
                owner_touched()
            } else {
                Fail::clash(
                    "The owner pressed Stop, so the assistant stopped in the middle of that \
                     step. Don't start again on your own: ask them first.",
                )
            });
        }
        let at = match (number(body, "x"), number(body, "y")) {
            (Some(x), y) => format!(" at {},{}", x.round(), y.unwrap_or(0.0).round()),
            _ => String::new(),
        };
        tracing::info!(%action, "acted on the screen");
        Ok(said(format!(
            "Done: {action}{at}. mac_screen shot to see it."
        )))
    }

    /// Posts `inputs` as the real mouse and keyboard, event by event while the session runs
    /// and the owner doesn't touch the Mac: whether all were posted.
    fn post_on_screen(&self, inputs: &[Input]) -> bool {
        lock(&self.screen).posting = true;
        let done = self
            .desktop
            .post_to_screen(inputs, &mut |posting| match posting {
                Posting::Before(_) => {
                    let now = self.clock.now();
                    let count = self.desktop.input_count();
                    lock(&self.screen).may_post(now, count)
                }
                Posting::After(input) => {
                    lock(&self.screen).sent(input);
                    true
                }
            });
        lock(&self.screen).posted(self.clock.now());
        done
    }

    /// Refuses a step while an app the helper never acts in is in front, or (for a click) owns
    /// the window under the pointer.
    fn allowed_on_screen(&self, action: &str, body: &Reply) -> Result<(), Fail> {
        let mut pids = vec![self.desktop.front()];
        if action.contains("click")
            && let (Some(x), Some(y)) = (number(body, "x"), number(body, "y"))
        {
            let Some(under) = self.desktop.app_at(x, y) else {
                return Err(Fail::never(format!(
                    "The Mac doesn't say which app is under {},{}, so the click isn't made (it \
                     could be an app the assistant never uses). Take a shot and click elsewhere, \
                     or ask the owner.",
                    x.round(),
                    y.round()
                )));
            };
            pids.push(Some(under));
        }
        for app in pids
            .into_iter()
            .flatten()
            .filter_map(|pid| self.desktop.app(pid))
        {
            if let Some(refused) = refusal(&app.name, &app.bundle, Use::Act) {
                return Err(refused);
            }
        }
        Ok(())
    }

    /// The input events for a screen action other than typing.
    fn screen_inputs(action: &str, body: &Reply) -> Result<Vec<Input>, Fail> {
        let x = number(body, "x");
        let y = number(body, "y");
        let at = || match (x, y) {
            (Some(x), Some(y)) => Ok((x, y)),
            _ => Err(Fail::bad(format!("{action} needs x and y"))),
        };
        Ok(match action {
            "click" => {
                let (x, y) = at()?;
                clicks(Button::Left, x, y, 1)
            }
            "double_click" => {
                let (x, y) = at()?;
                clicks(Button::Left, x, y, 2)
            }
            "right_click" => {
                let (x, y) = at()?;
                clicks(Button::Right, x, y, 1)
            }
            "move" => {
                let (x, y) = at()?;
                vec![Input::Move { x, y }]
            }
            "scroll" => {
                let mut inputs = Vec::new();
                if x.is_some() {
                    let (x, y) = at()?;
                    inputs.push(Input::Move { x, y });
                }
                // Whole pixels, kept within what a wheel event carries.
                #[expect(
                    clippy::cast_possible_truncation,
                    reason = "clamped to ±100 000 pixels first"
                )]
                let dy = number(body, "dy")
                    .unwrap_or(SCROLL)
                    .clamp(-100_000.0, 100_000.0)
                    .round() as i32;
                inputs.push(Input::Scroll { dy });
                inputs
            }
            "key" => press(chord(&field(body, "key"))?),
            _ => {
                return Err(Fail::bad(
                    "action: click, double_click, right_click, move, scroll, type or key",
                ));
            }
        })
    }

    /// `/screen/stop`: ends `who`'s session and takes the bar away.
    pub(super) fn screen_stop(&self, body: &Reply) -> Reply {
        let who = who(body);
        let now = self.clock.now();
        {
            let mut screen = lock(&self.screen);
            if screen.driving.as_ref().is_some_and(|d| d.who == who) {
                screen.end(Ended::Agent, now);
            }
        }
        self.show_bar();
        tracing::info!(%who, "stopped using the screen");
        object(json!({ "text": "Stopped using the screen; the bar is gone." }))
    }

    /// Refuses a step unless `who`'s session runs, saying why it ended.
    fn driver(&self, body: &Reply) -> Result<(), Fail> {
        let now = self.clock.now();
        match lock(&self.screen).not_driving(&who(body), now) {
            Some(refused) => Err(refused),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn driving(now: Instant) -> Screen {
        Screen {
            driving: Some(Driving {
                who: "agent".to_owned(),
                reason: "fill the form".to_owned(),
                until: now + Duration::from_secs(60),
            }),
            ..Screen::default()
        }
    }

    const KEY: Input = Input::Key {
        chord: crate::keys::Chord {
            code: 0,
            modifiers: crate::keys::Modifiers {
                command: false,
                shift: false,
                option: false,
                control: false,
            },
        },
        down: true,
    };

    #[test]
    fn the_assistant_s_own_counted_input_is_no_touch() {
        let now = Instant::now();
        let mut screen = driving(now);
        screen.count_from(100);
        screen.posting = true;
        // Each of its own key presses is counted by the Mac, and makes the Mac's idle time 0.
        assert!(screen.may_post(now, 100));
        screen.sent(&KEY);
        assert!(screen.may_post(now, 101));
        screen.sent(&KEY);
        assert_eq!(screen.watch(now, 0.0, 102), None);
        // Its events counted late are no touch either.
        assert!(screen.may_post(now, 101));
        screen.sent(&KEY);
        screen.posted(now);
        assert_eq!(
            screen.watch(now + Duration::from_millis(100), 0.0, 103),
            None
        );
    }

    #[test]
    fn the_owner_s_key_right_after_the_assistant_s_stops_the_next_event() {
        let now = Instant::now();
        let mut screen = driving(now);
        screen.count_from(100);
        screen.posting = true;
        assert!(screen.may_post(now, 100));
        screen.sent(&KEY);
        // The assistant's key was counted (101), and the owner's too (102).
        assert!(!screen.may_post(now, 102));
        assert!(!screen.driving());
        assert_eq!(screen.ended.map(|(cause, _)| cause), Some(Ended::Touch));
    }

    #[test]
    fn between_steps_recent_input_is_the_owner_s_once_the_assistant_s_settled() {
        let now = Instant::now();
        let mut screen = driving(now);
        screen.count_from(10);
        screen.posted(now);
        // The assistant's own events weren't counted: the count follows the Mac's once settled.
        assert_eq!(screen.watch(now + Duration::from_secs(1), 3.0, 10), None);
        assert_eq!(
            screen.watch(now + Duration::from_secs(2), 0.1, 10),
            Some(Ended::Touch)
        );
        assert_eq!(
            screen.stopped_for(now + Duration::from_secs(5)),
            Some(Duration::from_secs(3))
        );
    }

    #[test]
    fn an_event_that_wasn_t_posted_isn_t_counted_as_the_assistant_s() {
        let now = Instant::now();
        let mut screen = driving(now);
        screen.count_from(100);
        screen.posting = true;
        // The event couldn't be made, so it wasn't posted: nothing is added for it.
        assert!(screen.may_post(now, 100));
        // The watchdog, between a post and its count, sees no touch while posting.
        assert_eq!(screen.watch(now, 0.0, 101), None);
        // The owner's key is then the only input the Mac counted.
        assert!(!screen.may_post(now, 101));
        assert_eq!(screen.ended.map(|(cause, _)| cause), Some(Ended::Touch));
    }

    #[test]
    fn stop_stops_the_next_event() {
        let now = Instant::now();
        let mut screen = driving(now);
        screen.posting = true;
        assert!(screen.may_post(now, 0));
        screen.sent(&KEY);
        screen.end(Ended::Stop, now);
        assert!(!screen.may_post(now, 1));
    }

    #[test]
    fn a_new_start_waits_a_minute_after_the_owner_ended_one() {
        let now = Instant::now();
        let mut screen = driving(now);
        screen.end(Ended::Stop, now);
        assert!(screen.cooling(now + Duration::from_secs(30)).is_some());
        assert!(screen.cooling(now + COOLDOWN).is_none());
        let mut timed = driving(now);
        timed.end(Ended::Time, now);
        assert!(timed.cooling(now).is_none());
    }

    #[test]
    fn it_ends_when_its_time_is_up() {
        let now = Instant::now();
        let mut screen = driving(now);
        assert_eq!(screen.watch(now, 30.0, 0), None);
        assert_eq!(
            screen.watch(now + Duration::from_secs(60), 30.0, 0),
            Some(Ended::Time)
        );
        assert_eq!(screen.stopped_for(now), None);
    }

    #[test]
    fn after_stop_the_next_step_says_the_owner_stopped_it() {
        let now = Instant::now();
        let mut screen = driving(now);
        screen.end(Ended::Stop, now);
        let refused = screen
            .not_driving("agent", now + Duration::from_secs(4))
            .unwrap();
        assert_eq!(refused.status, 409);
        assert!(
            refused
                .text
                .starts_with("The owner pressed Stop on the bar 4 s ago"),
            "{}",
            refused.text
        );
    }

    #[test]
    fn another_driver_is_refused() {
        let now = Instant::now();
        let screen = driving(now);
        assert!(screen.not_driving("agent", now).is_none());
        assert_eq!(
            screen.not_driving("worker", now).unwrap().text,
            "agent is using the screen now."
        );
    }

    #[test]
    fn the_bar_names_what_runs() {
        assert_eq!(bar_text(None, None), None);
        assert_eq!(
            bar_text(Some("fill the form"), None).as_deref(),
            Some("Emberhut is using your screen: fill the form")
        );
        assert_eq!(
            bar_text(None, Some("Weekly call")).as_deref(),
            Some("Emberhut is recording: Weekly call")
        );
        assert_eq!(
            bar_text(Some("x"), Some("y")).as_deref(),
            Some("Emberhut is using your screen: x · recording y")
        );
    }
}
