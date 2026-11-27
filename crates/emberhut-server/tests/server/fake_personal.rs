//! The providers of the calls, voice, calendar and health services, scripted by the tests: a
//! Mac with a call recorder and calendars, a call transcriber, a live transcriber, a copilot
//! and a speech provider. Nothing here reaches the network.
#![expect(
    clippy::unwrap_used,
    reason = "test helpers: a poisoned lock fails the test"
)]

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::Bytes;
use emberhut_server::services::calendar::{CalendarError, CalendarProvider};
use emberhut_server::services::calls::{CallTranscriber, TranscribeFailure, TranscriptionJob};
use emberhut_server::services::health_data::{HealthProject, WalkTaskError, WalkTasks};
use emberhut_server::services::live_call::{
    Chunk, Copilot, CopilotError, Heard, LiveError, LiveTranscriber,
};
use emberhut_server::services::mac::{MacError, MacHealth, MacHelper, RecorderCommand};
use emberhut_server::services::personal::PersonalDependencies;
use emberhut_server::services::voice::{RealtimeVoice, SpeechToText, VoiceError, VoiceKey};
use futures_util::future::BoxFuture;
use jiff::Timestamp;
use jiff::tz::TimeZone;
use serde_json::{Map, Value, json};

/// The Mac of these tests: its recorder answers with a text, or refuses; its calendars give
/// scripted events and reminders, or don't answer.
#[derive(Debug, Default)]
pub(crate) struct FakeMacDesk {
    script: Mutex<DeskScript>,
}

#[derive(Debug, Default)]
struct DeskScript {
    refuse: Option<String>,
    recorded: Vec<(RecorderCommand, Value)>,
    events: Option<Value>,
    reminders: Option<Value>,
    added: Vec<(String, Map<String, Value>)>,
    reads: Vec<(i64, String, String)>,
}

impl FakeMacDesk {
    /// Its recorder refuses, saying `why`.
    pub(crate) fn refuses(&self, why: &str) {
        self.script.lock().unwrap().refuse = Some(why.to_owned());
    }

    /// What its recorder was asked, in order.
    pub(crate) fn recorded(&self) -> Vec<(RecorderCommand, Value)> {
        self.script.lock().unwrap().recorded.clone()
    }

    /// Its calendars answer `events` from now on (`None`: they don't answer).
    pub(crate) fn events(&self, events: Option<Value>) {
        self.script.lock().unwrap().events = events;
    }

    /// Its reminders answer `reminders` from now on (`None`: they don't answer).
    pub(crate) fn reminders(&self, reminders: Option<Value>) {
        self.script.lock().unwrap().reminders = reminders;
    }

    /// What was added: (`event` or `reminder`, its fields).
    pub(crate) fn added(&self) -> Vec<(String, Map<String, Value>)> {
        self.script.lock().unwrap().added.clone()
    }

    /// The calendar reads: (days, from, to).
    pub(crate) fn reads(&self) -> Vec<(i64, String, String)> {
        self.script.lock().unwrap().reads.clone()
    }
}

impl MacHelper for FakeMacDesk {
    fn health(&self) -> BoxFuture<'_, Result<MacHealth, MacError>> {
        Box::pin(async { Err(MacError::NotSetUp) })
    }

    fn lock(&self) -> BoxFuture<'_, Result<(), MacError>> {
        Box::pin(async { Ok(()) })
    }

    fn record(
        &self,
        action: RecorderCommand,
        body: Value,
    ) -> BoxFuture<'_, Result<Map<String, Value>, MacError>> {
        let mut script = self.script.lock().unwrap();
        script.recorded.push((action, body));
        let text = match action {
            RecorderCommand::Start => "Recording.",
            RecorderCommand::Stop => "Stopped; sending the tracks.",
            RecorderCommand::Status => "Not recording.",
        };
        let answer = script.refuse.as_ref().map_or_else(
            || Ok(json!({ "text": text }).as_object().cloned().unwrap()),
            |why| Err(MacError::Refused(why.clone())),
        );
        Box::pin(async move { answer })
    }
}

impl CalendarProvider for FakeMacDesk {
    fn events(
        &self,
        days: i64,
        from: String,
        to: String,
        _wait: Duration,
    ) -> BoxFuture<'_, Result<Value, CalendarError>> {
        let mut script = self.script.lock().unwrap();
        script.reads.push((days, from, to));
        let answer = script
            .events
            .clone()
            .ok_or_else(|| CalendarError::Unreachable("the Mac is asleep".to_owned()));
        Box::pin(async move { answer })
    }

    fn reminders(&self) -> BoxFuture<'_, Result<Value, CalendarError>> {
        let answer = self
            .script
            .lock()
            .unwrap()
            .reminders
            .clone()
            .ok_or_else(|| CalendarError::Unreachable("the Mac is asleep".to_owned()));
        Box::pin(async move { answer })
    }

    fn add_event(&self, event: Map<String, Value>) -> BoxFuture<'_, Result<Value, CalendarError>> {
        self.script
            .lock()
            .unwrap()
            .added
            .push(("event".to_owned(), event));
        Box::pin(async { Ok(json!({ "ok": "Added to Home." })) })
    }

    fn add_reminder(
        &self,
        reminder: Map<String, Value>,
    ) -> BoxFuture<'_, Result<Value, CalendarError>> {
        self.script
            .lock()
            .unwrap()
            .added
            .push(("reminder".to_owned(), reminder));
        Box::pin(async { Ok(json!({ "ok": true })) })
    }
}

/// A call transcriber: writes `Me: <title>` as the transcript, or fails when told.
#[derive(Debug, Default)]
pub(crate) struct FakeTranscriber {
    script: Mutex<(Option<String>, Vec<TranscriptionJob>)>,
}

impl FakeTranscriber {
    /// It fails from now on, saying `why`.
    pub(crate) fn fails(&self, why: &str) {
        self.script.lock().unwrap().0 = Some(why.to_owned());
    }

    /// The jobs it was given.
    pub(crate) fn jobs(&self) -> Vec<TranscriptionJob> {
        self.script.lock().unwrap().1.clone()
    }
}

impl CallTranscriber for FakeTranscriber {
    fn transcribe(&self, job: TranscriptionJob) -> BoxFuture<'_, Result<(), TranscribeFailure>> {
        let failure = {
            let mut script = self.script.lock().unwrap();
            script.1.push(job.clone());
            script.0.clone()
        };
        Box::pin(async move {
            if let Some(why) = failure {
                return Err(TranscribeFailure(why));
            }
            tokio::fs::write(
                &job.out,
                format!("# {}\n\nMe: we ship on Friday.\n", job.title),
            )
            .await
            .map_err(|error| TranscribeFailure(error.to_string()))
        })
    }
}

/// A live transcriber: each chunk says the next scripted line (or nothing), or it can't start.
#[derive(Debug, Default)]
pub(crate) struct FakeLive {
    script: Mutex<(VecDeque<String>, Option<String>, Vec<Chunk>)>,
}

impl FakeLive {
    /// The next chunks say these, in order.
    pub(crate) fn says(&self, lines: &[&str]) {
        self.script
            .lock()
            .unwrap()
            .0
            .extend(lines.iter().map(|line| (*line).to_owned()));
    }

    /// It can't start, saying `why`.
    pub(crate) fn cant_start(&self, why: &str) {
        self.script.lock().unwrap().1 = Some(why.to_owned());
    }

    /// The chunks it heard.
    pub(crate) fn chunks(&self) -> Vec<Chunk> {
        self.script.lock().unwrap().2.clone()
    }
}

impl LiveTranscriber for FakeLive {
    fn hear(&self, chunk: Chunk) -> BoxFuture<'_, Result<Heard, LiveError>> {
        let answer = {
            let mut script = self.script.lock().unwrap();
            script.2.push(chunk);
            match &script.1 {
                Some(why) => Err(LiveError::NoStart(why.clone())),
                None => Ok(Heard {
                    text: script.0.pop_front(),
                    lang: Some("en".to_owned()),
                    sure: true,
                }),
            }
        };
        Box::pin(async move { answer })
    }
}

/// A copilot: answers the scripted replies in order (`-` when there are none left), or fails.
#[derive(Debug, Default)]
pub(crate) struct FakeCopilot {
    script: Mutex<CopilotScript>,
}

#[derive(Debug, Default)]
struct CopilotScript {
    replies: VecDeque<String>,
    fail: Option<String>,
    opened: Vec<(String, String)>,
    said: Vec<String>,
    closed: Vec<String>,
}

impl FakeCopilot {
    /// Its next answers.
    pub(crate) fn replies(&self, replies: &[&str]) {
        self.script
            .lock()
            .unwrap()
            .replies
            .extend(replies.iter().map(|reply| (*reply).to_owned()));
    }

    /// It fails from now on, saying `why`.
    pub(crate) fn fails(&self, why: &str) {
        self.script.lock().unwrap().fail = Some(why.to_owned());
    }

    /// The conversations it opened: (call, instructions).
    pub(crate) fn opened(&self) -> Vec<(String, String)> {
        self.script.lock().unwrap().opened.clone()
    }

    /// What it was told.
    pub(crate) fn said(&self) -> Vec<String> {
        self.script.lock().unwrap().said.clone()
    }

    /// The conversations it closed.
    pub(crate) fn closed(&self) -> Vec<String> {
        self.script.lock().unwrap().closed.clone()
    }
}

impl Copilot for FakeCopilot {
    fn available(&self) -> bool {
        true
    }

    fn open(&self, call: &str, instructions: String) -> BoxFuture<'_, Result<(), CopilotError>> {
        self.script
            .lock()
            .unwrap()
            .opened
            .push((call.to_owned(), instructions));
        Box::pin(async { Ok(()) })
    }

    fn say(&self, _call: &str, text: String) -> BoxFuture<'_, Result<String, CopilotError>> {
        let answer = {
            let mut script = self.script.lock().unwrap();
            script.said.push(text);
            match &script.fail {
                Some(why) => Err(CopilotError(why.clone())),
                None => Ok(script.replies.pop_front().unwrap_or_else(|| "-".to_owned())),
            }
        };
        Box::pin(async move { answer })
    }

    fn close(&self, call: &str) {
        self.script.lock().unwrap().closed.push(call.to_owned());
    }
}

/// A speech provider: dictation says `heard: <prompt>`; a call's key is `key-1`. It can be
/// switched off.
#[derive(Debug)]
pub(crate) struct FakeVoice {
    script: Mutex<VoiceScript>,
}

#[derive(Debug)]
struct VoiceScript {
    ready: bool,
    heard: Vec<(usize, String)>,
    instructions: Vec<String>,
}

impl Default for FakeVoice {
    fn default() -> Self {
        Self {
            script: Mutex::new(VoiceScript {
                ready: true,
                heard: Vec::new(),
                instructions: Vec::new(),
            }),
        }
    }
}

impl FakeVoice {
    /// It is not set up from now on.
    pub(crate) fn off(&self) {
        self.script.lock().unwrap().ready = false;
    }

    /// The recordings it was given: (size, name).
    pub(crate) fn heard(&self) -> Vec<(usize, String)> {
        self.script.lock().unwrap().heard.clone()
    }

    /// The calls' instructions.
    pub(crate) fn instructions(&self) -> Vec<String> {
        self.script.lock().unwrap().instructions.clone()
    }
}

impl SpeechToText for FakeVoice {
    fn ready(&self) -> BoxFuture<'_, bool> {
        let ready = self.script.lock().unwrap().ready;
        Box::pin(async move { ready })
    }

    fn transcribe(
        &self,
        audio: Bytes,
        name: String,
        prompt: String,
    ) -> BoxFuture<'_, Result<String, VoiceError>> {
        let answer = {
            let mut script = self.script.lock().unwrap();
            script.heard.push((audio.len(), name));
            if script.ready {
                Ok(format!("heard: {prompt}"))
            } else {
                Err(VoiceError("no speech provider is set up".to_owned()))
            }
        };
        Box::pin(async move { answer })
    }
}

impl RealtimeVoice for FakeVoice {
    fn session(&self, instructions: String) -> BoxFuture<'_, Result<VoiceKey, VoiceError>> {
        let answer = {
            let mut script = self.script.lock().unwrap();
            script.instructions.push(instructions);
            if script.ready {
                Ok(VoiceKey {
                    key: "key-1".to_owned(),
                    expires_at: 1_800_000_600.0,
                    model: "realtime-test".to_owned(),
                })
            } else {
                Err(VoiceError("no speech provider is set up".to_owned()))
            }
        };
        Box::pin(async move { answer })
    }
}

/// The health project's tasks: project 9 with the open tasks it is given; what was closed and
/// logged.
#[derive(Debug, Default)]
pub(crate) struct FakeWalks {
    script: Mutex<WalkScript>,
}

#[derive(Debug, Default)]
struct WalkScript {
    open: Vec<(i64, String)>,
    closed: Vec<(i64, String, Timestamp)>,
    logged: Vec<(String, String)>,
}

impl FakeWalks {
    /// The project's open tasks.
    pub(crate) fn open(&self, tasks: &[(i64, &str)]) {
        self.script.lock().unwrap().open = tasks
            .iter()
            .map(|(id, title)| (*id, (*title).to_owned()))
            .collect();
    }

    /// The tasks closed: (task, evidence, done at).
    pub(crate) fn closed(&self) -> Vec<(i64, String, Timestamp)> {
        self.script.lock().unwrap().closed.clone()
    }

    /// The walks logged as new done tasks: (title, evidence).
    pub(crate) fn logged(&self) -> Vec<(String, String)> {
        self.script.lock().unwrap().logged.clone()
    }
}

impl WalkTasks for FakeWalks {
    fn project(&self) -> BoxFuture<'_, Result<Option<HealthProject>, WalkTaskError>> {
        Box::pin(async { Ok(Some(HealthProject { id: 9, goal: None })) })
    }

    fn open_tasks(
        &self,
        _project: HealthProject,
    ) -> BoxFuture<'_, Result<Vec<(i64, String)>, WalkTaskError>> {
        let open = self.script.lock().unwrap().open.clone();
        Box::pin(async move { Ok(open) })
    }

    fn close(
        &self,
        task: i64,
        evidence: String,
        done_at: Timestamp,
    ) -> BoxFuture<'_, Result<(), WalkTaskError>> {
        let mut script = self.script.lock().unwrap();
        script.open.retain(|(id, _)| *id != task);
        script.closed.push((task, evidence, done_at));
        Box::pin(async { Ok(()) })
    }

    fn log(
        &self,
        _project: HealthProject,
        title: String,
        evidence: String,
        _done_at: Timestamp,
    ) -> BoxFuture<'_, Result<i64, WalkTaskError>> {
        self.script.lock().unwrap().logged.push((title, evidence));
        Box::pin(async { Ok(77) })
    }
}

/// Every fake, and the providers they make.
#[derive(Debug, Clone, Default)]
pub(crate) struct Fakes {
    pub(crate) mac: Arc<FakeMacDesk>,
    pub(crate) transcriber: Arc<FakeTranscriber>,
    pub(crate) live: Arc<FakeLive>,
    pub(crate) copilot: Arc<FakeCopilot>,
    pub(crate) voice: Arc<FakeVoice>,
    pub(crate) walks: Option<Arc<FakeWalks>>,
}

impl Fakes {
    /// The providers, in UTC.
    pub(crate) fn dependencies(&self) -> PersonalDependencies {
        PersonalDependencies {
            calendar: Arc::clone(&self.mac) as _,
            call_transcriber: Arc::clone(&self.transcriber) as _,
            live_transcriber: Arc::clone(&self.live) as _,
            copilot: Arc::clone(&self.copilot) as _,
            speech: Arc::clone(&self.voice) as _,
            realtime: Arc::clone(&self.voice) as _,
            walk_tasks: self.walks.as_ref().map(|walks| Arc::clone(walks) as _),
            zone: TimeZone::UTC,
            ..PersonalDependencies::default()
        }
    }
}
