//! Fakes for the tests: a desktop with apps and accessibility trees in memory, a clock moved by
//! hand, and a recorder's capture and uploads.

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use url::Url;

use crate::desktop::{
    AxError, Bar, Desktop, Flag, Focus, InstalledApp, Link, List, Posting, RunningApp,
    ScreenPicture, Text, Window,
};
use crate::fail::Fail;
use crate::hands::{Clock, Hands};
use crate::keys::Input;
use crate::record::{
    AudioSource, Capture, LiveSink, Lost, Mic, Recorder, Server, Upload, UploadError, Uploader,
};

/// One element.
#[derive(Clone, Debug, Default)]
pub(crate) struct Node {
    pub(crate) role: String,
    pub(crate) subrole: String,
    pub(crate) title: Option<String>,
    pub(crate) description: Option<String>,
    pub(crate) value: Option<String>,
    pub(crate) menu_key: Option<String>,
    pub(crate) enabled: Option<bool>,
    pub(crate) focused: Option<bool>,
    pub(crate) minimized: Option<bool>,
    pub(crate) actions: Vec<String>,
    pub(crate) children: Vec<usize>,
    pub(crate) windows: Vec<usize>,
    pub(crate) menu_bar: Option<usize>,
    pub(crate) focus: Option<usize>,
    pub(crate) settable: bool,
}

/// A node with `role` and `title`.
pub(crate) fn node(role: &str, title: &str) -> Node {
    Node {
        role: role.to_owned(),
        title: (!title.is_empty()).then(|| title.to_owned()),
        ..Node::default()
    }
}

/// A test's hook, called with the count of events posted so far.
pub(crate) type Hook = Box<dyn Fn(usize) + Send>;

/// A desktop in memory.
pub(crate) struct FakeDesktop {
    pub(crate) accessibility: AtomicBool,
    pub(crate) screen: AtomicBool,
    pub(crate) apps: Mutex<Vec<RunningApp>>,
    pub(crate) installed: Mutex<Vec<InstalledApp>>,
    pub(crate) front: Mutex<Option<i32>>,
    pub(crate) nodes: Mutex<Vec<Node>>,
    pub(crate) roots: Mutex<HashMap<i32, usize>>,
    pub(crate) system_focus: Mutex<Focus<usize>>,
    /// Apps say nothing of their focus.
    pub(crate) focus_unknown: AtomicBool,
    /// The app under the pointer.
    pub(crate) under_pointer: Mutex<Option<i32>>,
    /// Advanced by each pause posted.
    pub(crate) clock: Mutex<Option<Arc<HandClock>>>,
    /// Called with the count of events posted so far, after each.
    pub(crate) on_post: Mutex<Option<Hook>>,
    /// The bar doesn't come up.
    pub(crate) bar_fails: AtomicBool,
    /// The screen is locked.
    pub(crate) locked: AtomicBool,
    /// The Mac's input count.
    pub(crate) input_count: AtomicU64,
    /// Apps that run without a Dock icon: found by pid or bundle only.
    pub(crate) background: Mutex<Vec<RunningApp>>,
    pub(crate) idle: Mutex<f64>,
    /// Elements whose value was read.
    pub(crate) value_reads: Mutex<Vec<usize>>,
    /// Elements set, and to what.
    pub(crate) set: Mutex<Vec<(usize, String)>>,
    /// Actions performed.
    pub(crate) performed: Mutex<Vec<(usize, String)>>,
    /// Input posted: to an app (its pid) or the screen (`None`).
    pub(crate) posted: Mutex<Vec<(Option<i32>, Vec<Input>)>>,
    /// The bar's words, each time it changed.
    pub(crate) bars: Mutex<Vec<Option<String>>>,
    /// Apps opened, and whether hidden.
    pub(crate) opened: Mutex<Vec<(String, bool)>>,
    /// The app an action brings to the front, if any.
    pub(crate) jumps_front: Mutex<Option<i32>>,
}

impl FakeDesktop {
    /// A desktop with both grants and nothing running.
    pub(crate) fn new() -> Self {
        Self {
            accessibility: AtomicBool::new(true),
            screen: AtomicBool::new(true),
            apps: Mutex::new(Vec::new()),
            installed: Mutex::new(Vec::new()),
            front: Mutex::new(None),
            nodes: Mutex::new(Vec::new()),
            roots: Mutex::new(HashMap::new()),
            system_focus: Mutex::new(Focus::Nothing),
            focus_unknown: AtomicBool::new(false),
            under_pointer: Mutex::new(None),
            clock: Mutex::new(None),
            on_post: Mutex::new(None),
            bar_fails: AtomicBool::new(false),
            locked: AtomicBool::new(false),
            background: Mutex::new(Vec::new()),
            input_count: AtomicU64::new(0),
            idle: Mutex::new(600.0),
            value_reads: Mutex::new(Vec::new()),
            set: Mutex::new(Vec::new()),
            performed: Mutex::new(Vec::new()),
            posted: Mutex::new(Vec::new()),
            bars: Mutex::new(Vec::new()),
            opened: Mutex::new(Vec::new()),
            jumps_front: Mutex::new(None),
        }
    }

    /// Adds `node`; its id.
    pub(crate) fn add(&self, node: Node) -> usize {
        let mut nodes = self.nodes.lock().unwrap();
        nodes.push(node);
        nodes.len() - 1
    }

    /// Changes node `id`.
    pub(crate) fn edit(&self, id: usize, change: impl FnOnce(&mut Node)) {
        change(&mut self.nodes.lock().unwrap()[id]);
    }

    /// Runs an app `pid`; its accessibility root's id.
    pub(crate) fn run(&self, pid: i32, name: &str, bundle: &str) -> usize {
        self.apps.lock().unwrap().push(RunningApp {
            pid,
            name: name.to_owned(),
            bundle: bundle.to_owned(),
            hidden: false,
        });
        let root = self.add(node("AXApplication", name));
        self.roots.lock().unwrap().insert(pid, root);
        root
    }

    /// Gives app root `root` a window holding `children`; the window's id.
    pub(crate) fn window(&self, root: usize, title: &str, children: Vec<usize>) -> usize {
        let mut window = node("AXWindow", title);
        window.children = children;
        let id = self.add(window);
        self.edit(root, |app| app.windows.push(id));
        id
    }

    /// Quits app `pid`.
    pub(crate) fn quit(&self, pid: i32) {
        self.apps.lock().unwrap().retain(|app| app.pid != pid);
    }

    fn node(&self, id: usize) -> Node {
        self.nodes.lock().unwrap()[id].clone()
    }

    /// Posts `inputs` as the real ones do: `go_on` before each event, a pause moves the clock.
    fn post(
        &self,
        to: Option<i32>,
        inputs: &[Input],
        go_on: &mut dyn FnMut(Posting<'_>) -> bool,
    ) -> bool {
        let before: usize = self
            .posted
            .lock()
            .unwrap()
            .iter()
            .map(|(_, inputs)| inputs.len())
            .sum();
        let mut sent = Vec::new();
        let mut all = true;
        for input in inputs {
            if let Input::Pause(time) = input {
                if let Some(clock) = self.clock.lock().unwrap().as_ref() {
                    clock.advance(*time);
                }
                continue;
            }
            if !go_on(Posting::Before(input)) {
                all = false;
                break;
            }
            sent.push(input.clone());
            // The Mac counts the assistant's own screen input, and it is the latest input.
            if to.is_none() && input.counted() {
                self.input_count.fetch_add(1, Ordering::SeqCst);
                *self.idle.lock().unwrap() = 0.0;
            }
            go_on(Posting::After(input));
            if let Some(hook) = self.on_post.lock().unwrap().as_ref() {
                hook(before + sent.len());
            }
        }
        self.posted.lock().unwrap().push((to, sent));
        all
    }
}

impl Desktop for FakeDesktop {
    type Element = usize;

    fn accessibility(&self) -> bool {
        self.accessibility.load(Ordering::SeqCst)
    }

    fn screen_recording(&self) -> bool {
        self.screen.load(Ordering::SeqCst)
    }

    fn running(&self) -> Vec<RunningApp> {
        self.apps.lock().unwrap().clone()
    }

    fn running_with_bundle(&self, bundle: &str) -> Vec<RunningApp> {
        self.running()
            .into_iter()
            .filter(|app| app.bundle == bundle)
            .collect()
    }

    fn front(&self) -> Option<i32> {
        *self.front.lock().unwrap()
    }

    fn app(&self, pid: i32) -> Option<RunningApp> {
        let background = self.background.lock().unwrap().clone();
        self.running()
            .into_iter()
            .chain(background)
            .find(|app| app.pid == pid)
    }

    fn app_at(&self, _x: f64, _y: f64) -> Option<i32> {
        *self.under_pointer.lock().unwrap()
    }

    fn activate(&self, pid: i32) {
        *self.front.lock().unwrap() = Some(pid);
    }

    fn installed(&self, want: &str) -> Option<InstalledApp> {
        self.installed
            .lock()
            .unwrap()
            .iter()
            .find(|app| app.bundle == want || app.name.eq_ignore_ascii_case(want))
            .cloned()
    }

    fn open(&self, app: &InstalledApp, hidden: bool) -> Result<(), String> {
        self.opened.lock().unwrap().push((app.name.clone(), hidden));
        Ok(())
    }

    fn windows(&self, pid: i32) -> Vec<Window> {
        let Some(root) = self.roots.lock().unwrap().get(&pid).copied() else {
            return Vec::new();
        };
        self.node(root)
            .windows
            .iter()
            .map(|&id| Window {
                id: u32::try_from(id).unwrap(),
                title: self.node(id).title.unwrap_or_default(),
                on_screen: true,
            })
            .collect()
    }

    fn application(&self, pid: i32) -> usize {
        self.roots.lock().unwrap()[&pid]
    }

    fn focused(&self) -> Focus<usize> {
        self.system_focus.lock().unwrap().clone()
    }

    fn focus_in(&self, app: &usize) -> Focus<usize> {
        if self.focus_unknown.load(Ordering::SeqCst) {
            return Focus::Unknown;
        }
        self.node(*app).focus.map_or(Focus::Nothing, Focus::On)
    }

    fn text(&self, element: &usize, attribute: Text) -> Option<String> {
        let node = self.node(*element);
        match attribute {
            Text::Role => Some(node.role).filter(|r| !r.is_empty()),
            Text::Subrole => Some(node.subrole).filter(|r| !r.is_empty()),
            Text::Title => node.title,
            Text::Description => node.description,
            Text::Placeholder | Text::Help => None,
            Text::Value => {
                self.value_reads.lock().unwrap().push(*element);
                node.value
            }
            Text::MenuKey => node.menu_key,
        }
    }

    fn flag(&self, element: &usize, attribute: Flag) -> Option<bool> {
        let node = self.node(*element);
        match attribute {
            Flag::Enabled => node.enabled,
            Flag::Focused => node.focused,
            Flag::Minimized => node.minimized,
        }
    }

    fn link(&self, element: &usize, attribute: Link) -> Option<usize> {
        let node = self.node(*element);
        match attribute {
            Link::TitleElement => None,
            Link::MenuBar => node.menu_bar,
        }
    }

    fn list(&self, element: &usize, attribute: List) -> Vec<usize> {
        let node = self.node(*element);
        match attribute {
            List::Children => node.children,
            List::Windows => node.windows,
        }
    }

    fn actions(&self, element: &usize) -> Vec<String> {
        self.node(*element).actions
    }

    fn perform(&self, element: &usize, action: &str) -> Result<(), AxError> {
        self.performed
            .lock()
            .unwrap()
            .push((*element, action.to_owned()));
        if let Some(pid) = *self.jumps_front.lock().unwrap() {
            self.activate(pid);
        }
        Ok(())
    }

    fn settable(&self, element: &usize) -> bool {
        self.node(*element).settable
    }

    fn set_value(&self, element: &usize, text: &str) -> Result<(), AxError> {
        self.set.lock().unwrap().push((*element, text.to_owned()));
        self.edit(*element, |node| node.value = Some(text.to_owned()));
        Ok(())
    }

    fn set_flag(&self, element: &usize, attribute: Flag, on: bool) -> Result<(), AxError> {
        self.edit(*element, |node| match attribute {
            Flag::Enabled => node.enabled = Some(on),
            Flag::Focused => node.focused = Some(on),
            Flag::Minimized => node.minimized = Some(on),
        });
        Ok(())
    }

    fn post_to_app(
        &self,
        pid: i32,
        inputs: &[Input],
        go_on: &mut dyn FnMut(Posting<'_>) -> bool,
    ) -> bool {
        self.post(Some(pid), inputs, go_on)
    }

    fn post_to_screen(&self, inputs: &[Input], go_on: &mut dyn FnMut(Posting<'_>) -> bool) -> bool {
        self.post(None, inputs, go_on)
    }

    fn input_count(&self) -> u64 {
        self.input_count.load(Ordering::SeqCst)
    }

    fn idle_seconds(&self) -> f64 {
        *self.idle.lock().unwrap()
    }

    fn window_picture(&self, id: u32, _max: u32) -> Option<Vec<u8>> {
        Some(format!("window {id}").into_bytes())
    }

    fn screen_picture(&self) -> Option<ScreenPicture> {
        Some(ScreenPicture {
            jpeg: b"screen".to_vec(),
            width: 1512.0,
            height: 982.0,
        })
    }

    fn locked(&self) -> bool {
        self.locked.load(Ordering::SeqCst)
    }

    fn lock(&self) -> Result<(), String> {
        Ok(())
    }

    fn show_bar(&self, bar: &Arc<Bar>) {
        let (asked, text) = bar.wanted();
        let up = text.is_some() && !self.bar_fails.load(Ordering::SeqCst);
        self.bars.lock().unwrap().push(text);
        bar.shown(asked, up);
    }

    fn pause(&self, _time: Duration) {}
}

/// A clock moved by hand.
#[derive(Debug)]
pub(crate) struct HandClock(Mutex<Instant>);

impl HandClock {
    pub(crate) fn new() -> Self {
        Self(Mutex::new(Instant::now()))
    }

    /// Moves it on by `time`.
    pub(crate) fn advance(&self, time: Duration) {
        *self.0.lock().unwrap() += time;
    }
}

impl Clock for HandClock {
    fn now(&self) -> Instant {
        *self.0.lock().unwrap()
    }
}

/// A capture that writes both tracks' files and remembers it was finished.
#[derive(Default)]
pub(crate) struct FakeAudio {
    pub(crate) refused: AtomicBool,
    pub(crate) no_mic: AtomicBool,
    pub(crate) finished: Arc<AtomicBool>,
    pub(crate) sinks: Mutex<Vec<LiveSink>>,
    pub(crate) losts: Mutex<Vec<Lost>>,
}

struct FakeCapture(Arc<AtomicBool>);

impl Capture for FakeCapture {
    fn finish(self: Box<Self>) {
        self.0.store(true, Ordering::SeqCst);
    }
}

impl AudioSource for FakeAudio {
    fn permitted(&self) -> bool {
        !self.refused.load(Ordering::SeqCst)
    }

    fn microphone(&self) -> Mic {
        if self.no_mic.load(Ordering::SeqCst) {
            Mic::Refused
        } else {
            Mic::Allowed
        }
    }

    fn start(
        &self,
        dir: &Path,
        mic: bool,
        live: LiveSink,
        lost: Lost,
    ) -> Result<Box<dyn Capture>, Fail> {
        self.losts.lock().unwrap().push(lost);
        std::fs::write(dir.join("sys.m4a"), b"sys").unwrap();
        if mic {
            std::fs::write(dir.join("mic.m4a"), b"mic").unwrap();
        }
        self.sinks.lock().unwrap().push(live);
        Ok(Box::new(FakeCapture(Arc::clone(&self.finished))))
    }
}

/// Remembers what it was sent; fails while `failing`.
#[derive(Debug, Default)]
pub(crate) struct FakeUploader {
    pub(crate) failing: AtomicBool,
    /// The status a failure answers (none: the network failed).
    pub(crate) status: Mutex<Option<u16>>,
    pub(crate) sent: Mutex<Vec<(String, String, Vec<u8>)>>,
}

impl Uploader for FakeUploader {
    fn post(&self, url: &Url, token: &str, body: Upload) -> Result<(), UploadError> {
        if self.failing.load(Ordering::SeqCst) {
            let status = *self.status.lock().unwrap();
            return Err(UploadError {
                status,
                text: "refused".to_owned(),
            });
        }
        let bytes = match body {
            Upload::File(path) => std::fs::read(path).map_err(|e| UploadError {
                status: None,
                text: e.to_string(),
            })?,
            Upload::Bytes(bytes) => bytes,
        };
        self.sent
            .lock()
            .unwrap()
            .push((url.to_string(), token.to_owned(), bytes));
        Ok(())
    }
}

/// A server for the recorder.
pub(crate) fn server() -> Server {
    Server {
        url: Url::parse("http://emberhut.test:8791/").unwrap(),
        token: "device-token".to_owned(),
    }
}

/// Everything a test of the helper looks at.
pub(crate) struct Rig {
    pub(crate) desktop: Arc<FakeDesktop>,
    pub(crate) clock: Arc<HandClock>,
    pub(crate) audio: Arc<FakeAudio>,
    pub(crate) hands: Arc<Hands<FakeDesktop>>,
    /// Holds the calls' folder for the test's length.
    pub(crate) _calls: tempfile::TempDir,
}

/// The helper over a fake desktop, recording to a fake server.
pub(crate) fn rig() -> Rig {
    let desktop = Arc::new(FakeDesktop::new());
    let clock = Arc::new(HandClock::new());
    let audio = Arc::new(FakeAudio::default());
    let uploader = Arc::new(FakeUploader::default());
    let calls = tempfile::tempdir().unwrap();
    let recorder = Recorder::new(
        Arc::clone(&audio) as Arc<dyn AudioSource>,
        Arc::clone(&uploader) as Arc<dyn Uploader>,
        Some(server()),
        calls.path().join("calls"),
        Arc::clone(&clock) as Arc<dyn Clock>,
    );
    *desktop.clock.lock().unwrap() = Some(Arc::clone(&clock));
    let hands = Arc::new(Hands::new(
        Arc::clone(&desktop),
        Arc::clone(&clock) as Arc<dyn Clock>,
        recorder,
    ));
    Rig {
        desktop,
        clock,
        audio,
        hands,
        _calls: calls,
    }
}

/// Waits up to two seconds for `done`.
pub(crate) fn eventually(done: impl Fn() -> bool) -> bool {
    for _ in 0..200 {
        if done() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    false
}
