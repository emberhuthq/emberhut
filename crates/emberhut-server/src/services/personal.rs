//! The services over what the server reads of the owner's own day: their calls (recorded and
//! live), voice calls and dictation, their calendar and reminders, and the health and screen
//! time their devices send in. [`PersonalDependencies`] is what they are built from (each
//! provider behind its trait), [`PersonalServices`] the services, wired once in
//! [`AppState::new`](crate::app::AppState::new).

use std::path::Path;
use std::sync::Arc;

use emberhut_core::{Clock, RandomSource};
use emberhut_db::Database;
use jiff::tz::TimeZone;

use crate::services::activity::ActivityLog;
use crate::services::calendar::{
    CalendarItems, CalendarParts, CalendarProvider, CalendarService, NoCalendar,
};
use crate::services::calls::{CallParts, CallService, CallTranscriber, CallsConfig, NoTranscriber};
use crate::services::health_data::{HealthDataService, HealthParts, WalkTasks};
use crate::services::live_call::{
    Copilot, LiveCallParts, LiveCallService, LiveTranscriber, NoCopilot, NoLiveTranscriber,
};
use crate::services::mac::{MacHelper, MacService};
use crate::services::personal_tasks::PersonalTasks;
use crate::services::tasks::TaskService;
use crate::services::uploads::UploadService;
use crate::services::voice::{
    NoVoice, RealtimeVoice, SpeechToText, VoiceConfig, VoiceParts, VoiceService,
};

/// The providers the personal services are built from; a test swaps any of them.
#[derive(Clone, Debug)]
pub struct PersonalDependencies {
    /// Where the owner's calendar and reminders come from.
    pub calendar: Arc<dyn CalendarProvider>,
    /// What the owner has to work on, for the calendar; none: the server's tasks
    /// ([`PersonalTasks`]).
    pub calendar_items: Option<Arc<dyn CalendarItems>>,
    /// What transcribes a recorded call.
    pub call_transcriber: Arc<dyn CallTranscriber>,
    /// What hears a call's chunks while it is recorded.
    pub live_transcriber: Arc<dyn LiveTranscriber>,
    /// What gives a live call's hints.
    pub copilot: Arc<dyn Copilot>,
    /// Dictation's speech to text.
    pub speech: Arc<dyn SpeechToText>,
    /// Voice calls' realtime voice.
    pub realtime: Arc<dyn RealtimeVoice>,
    /// The health project's walk tasks; none: the server's tasks ([`PersonalTasks`]).
    pub walk_tasks: Option<Arc<dyn WalkTasks>>,
    /// The server's time zone: whose days the calendar's are.
    pub zone: TimeZone,
    /// How long a recorded call's audio is kept.
    pub calls: CallsConfig,
    /// The labels of the person's lines in a voice call's transcript.
    pub voice: VoiceConfig,
}

impl Default for PersonalDependencies {
    /// None of the providers, the server's own tasks, the system's time zone, 14 days of call
    /// audio, and both labels of the person's lines.
    fn default() -> Self {
        Self {
            calendar: Arc::new(NoCalendar),
            calendar_items: None,
            call_transcriber: Arc::new(NoTranscriber),
            live_transcriber: Arc::new(NoLiveTranscriber),
            copilot: Arc::new(NoCopilot),
            speech: Arc::new(NoVoice),
            realtime: Arc::new(NoVoice),
            walk_tasks: None,
            zone: TimeZone::system(),
            calls: CallsConfig::default(),
            voice: VoiceConfig::default(),
        }
    }
}

/// The personal services.
#[derive(Clone, Debug)]
pub struct PersonalServices {
    /// The calendar and reminders.
    pub calendar: CalendarService,
    /// Recorded and shared calls.
    pub calls: CallService,
    /// Calls being recorded: their live transcript and copilot.
    pub live_calls: LiveCallService,
    /// Voice calls and dictation.
    pub voice: VoiceService,
    /// Health and screen time.
    pub health_data: HealthDataService,
}

impl PersonalServices {
    /// The services over `db`, from `dependencies`, writing to the server's `activity` log,
    /// telling the time with `clock`, making ids with `random`, reaching the Mac's recorder
    /// through `mac`, reading the server's `tasks` where `dependencies` names no others.
    #[must_use]
    pub fn new(
        db: &Database,
        tasks: &TaskService,
        activity: &ActivityLog,
        clock: &Arc<dyn Clock>,
        random: &Arc<dyn RandomSource>,
        mac: &Arc<dyn MacHelper>,
        dependencies: PersonalDependencies,
    ) -> Self {
        let PersonalDependencies {
            calendar,
            calendar_items,
            call_transcriber,
            live_transcriber,
            copilot,
            speech,
            realtime,
            walk_tasks,
            zone,
            calls: config,
            voice: voice_config,
        } = dependencies;
        let own = || Arc::new(PersonalTasks::new(tasks.clone()));
        let calendar_items = calendar_items.unwrap_or_else(|| own() as _);
        let walk_tasks = walk_tasks.unwrap_or_else(|| own() as _);
        let calls = CallService::new(CallParts {
            calls: db.calls(),
            rooms: db.rooms(),
            events: db.events(),
            mac: MacService::new(Arc::clone(mac)),
            transcriber: call_transcriber,
            clock: Arc::clone(clock),
            random: Arc::clone(random),
            data_dir: db.path().parent().map_or_default(Path::to_path_buf),
            uploads_dir: UploadService::folder_beside(db.path()),
            config,
        });
        let live_calls = LiveCallService::new(LiveCallParts {
            calls: db.calls(),
            rooms: db.rooms(),
            messages: db.messages(),
            events: db.events(),
            transcriber: live_transcriber,
            copilot,
            clock: Arc::clone(clock),
            dir: calls.dir().to_path_buf(),
        });
        let voice = VoiceService::new(VoiceParts {
            speech,
            realtime,
            messages: db.messages(),
            events: db.events(),
            settings: db.settings(),
            clock: Arc::clone(clock),
            zone: zone.clone(),
            config: voice_config,
        });
        let health_data = HealthDataService::new(HealthParts {
            store: db.health_data(),
            settings: db.settings(),
            activity: activity.clone(),
            walks: walk_tasks,
            clock: Arc::clone(clock),
        });
        let calendar = CalendarService::new(CalendarParts {
            provider: calendar,
            items: calendar_items,
            days: db.calendar(),
            settings: db.settings(),
            activity: activity.clone(),
            clock: Arc::clone(clock),
            zone,
        });
        Self {
            calendar,
            calls,
            live_calls,
            voice,
            health_data,
        }
    }
}
