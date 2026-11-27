//! `correction_log`, `corrections`, `lesson_set`, `lesson_drop`, `trial_drop` and `chat_log`:
//! how the assistant learns from being corrected, and the chat the weekly review reads.

use std::sync::LazyLock;

use emberhut_core::memory::learning::{TRIAL_DAYS, TrialJudge};
use emberhut_core::memory::{iso, wire};
use serde::Deserialize;
use serde_json::json;

use super::{Tool, ToolContext, ToolError, ToolOutput};
use crate::services::learning::LogCorrection;

/// Logs a correction.
#[derive(Clone, Copy, Debug)]
pub(crate) struct CorrectionLog;

/// `correction_log`'s arguments.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema, garde::Validate)]
pub(crate) struct CorrectionLogArgs {
    /// What the user said.
    #[garde(length(max = 20_000))]
    said: String,
    /// What you did wrong, honestly, in one or two sentences.
    #[garde(length(max = 20_000))]
    wrong: String,
    /// The rule that would have avoided it.
    #[garde(length(max = 5000))]
    lesson: Option<String>,
    /// The same lesson as one short line to the user, in the second person, e.g. "Story reels
    /// run at least 60 seconds; I check each cut's length before showing you." The app shows
    /// it under their own words.
    #[garde(length(max = 1000))]
    lesson_to_user: Option<String>,
    /// When it happened (ISO); default now.
    #[garde(length(max = 64))]
    at: Option<String>,
}

impl Tool for CorrectionLog {
    type Args = CorrectionLogArgs;

    fn name(&self) -> &'static str {
        "correction_log"
    }

    fn description(&self) -> &'static str {
        "Log it whenever the user corrects you or doesn't understand you (\"you didn't \
         check\", \"I already told them\", \"what does that mean\"): said: what they said; \
         wrong: what you did wrong, honestly, in one or two sentences; lesson: the rule that \
         would have avoided it; lesson_to_user: the same lesson as one short line to the \
         owner, in the second person, e.g. 'Story reels run at least 60 seconds; I check each \
         cut's length before showing you.' (the app shows it to them under their own words). \
         The lesson is a trial rule in front of you at once, in every room, for 14 days; the \
         weekly learning review (or the user, in the app) keeps or drops it."
    }

    async fn call(
        &self,
        ctx: &ToolContext,
        args: CorrectionLogArgs,
    ) -> Result<ToolOutput, ToolError> {
        let with_lesson = args
            .lesson
            .as_deref()
            .is_some_and(|lesson| !lesson.trim().is_empty());
        let id = ctx
            .app
            .learning()
            .log(LogCorrection {
                said: args.said,
                wrong: args.wrong,
                lesson: args.lesson,
                lesson_to_owner: args.lesson_to_user,
                at: args.at,
            })
            .await
            .map_err(ToolError::failed)?;
        let trial = if with_lesson {
            format!(" Its lesson is trial rule T{id} for {TRIAL_DAYS} days.")
        } else {
            String::new()
        };
        Ok(ToolOutput::text(format!("Logged correction #{id}.{trial}")))
    }
}

/// The default days of the corrections log.
const fn thirty() -> i64 {
    30
}

/// The corrections log, with the standing and trial rules.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Corrections;

/// `corrections`' description: it names the stored `source` value for the assistant
/// ([`wire::ASSISTANT`]) so the model can read the log.
static CORRECTIONS_DESCRIPTION: LazyLock<String> = LazyLock::new(|| {
    format!(
        "The corrections log: the user's corrections with what went wrong (source {assistant}: \
         you, the assistant, logged it; auto: the server flagged a message that sounds like \
         one, a candidate), the standing rules (lessons) they already OKed, and the trial rules \
         (trials: each logged lesson of the last {TRIAL_DAYS} days, in your prompt until kept or \
         dropped).",
        assistant = wire::ASSISTANT
    )
});

/// `corrections`' arguments.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema, garde::Validate)]
pub(crate) struct CorrectionsArgs {
    /// How many days back, 1 to 365 (default 30).
    #[serde(default = "thirty")]
    #[garde(range(min = 1, max = 365))]
    days: i64,
}

impl Tool for Corrections {
    type Args = CorrectionsArgs;

    fn name(&self) -> &'static str {
        "corrections"
    }

    fn description(&self) -> &'static str {
        CORRECTIONS_DESCRIPTION.as_str()
    }

    async fn call(
        &self,
        ctx: &ToolContext,
        args: CorrectionsArgs,
    ) -> Result<ToolOutput, ToolError> {
        let learning = ctx.app.learning();
        let corrections = learning
            .corrections(args.days)
            .await
            .map_err(ToolError::failed)?;
        let lessons = learning.lessons().await.map_err(ToolError::failed)?;
        let trials = learning.trials().await.map_err(ToolError::failed)?;
        ToolOutput::json(&json!({
            "corrections": corrections.iter().map(|c| json!({
                "id": c.id,
                "at": iso(c.at),
                "said": c.said,
                "wrong": c.wrong,
                "lesson": c.lesson,
                "source": c.source,
            })).collect::<Vec<_>>(),
            "lessons": lessons.iter().map(|l| json!({
                "id": l.id,
                "text": l.text,
                "at": iso(l.at),
            })).collect::<Vec<_>>(),
            "trials": trials.iter().map(|t| json!({
                "id": t.id,
                "text": t.text,
                "at": iso(t.at),
                "ends": iso(t.ends),
                "said": t.said,
                "lesson_to_user": t.lesson_to_owner,
            })).collect::<Vec<_>>(),
        }))
    }
}

/// Keeps a standing rule.
#[derive(Clone, Copy, Debug)]
pub(crate) struct LessonSet;

/// `lesson_set`'s arguments.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema, garde::Validate)]
pub(crate) struct LessonSetArgs {
    /// The rule's number, to reword it; left out: a new rule.
    #[garde(range(min = 1))]
    id: Option<i64>,
    /// The rule.
    #[garde(length(chars, min = 10, max = 5000))]
    text: String,
    /// The trial rule it keeps (T252: 252).
    #[garde(range(min = 1))]
    trial: Option<i64>,
}

impl Tool for LessonSet {
    type Args = LessonSetArgs;

    fn name(&self) -> &'static str {
        "lesson_set"
    }

    fn description(&self) -> &'static str {
        "Keep a standing rule you will always follow, only one the user OKed (from the weekly \
         learning review or when they say so). New: no id. trial: the trial rule it keeps \
         (T252: 252), as worded or reworded. It is put in front of you at every start and \
         after every compaction."
    }

    async fn call(&self, ctx: &ToolContext, args: LessonSetArgs) -> Result<ToolOutput, ToolError> {
        let id = ctx
            .app
            .learning()
            .set_lesson(args.id, &args.text, args.trial)
            .await
            .map_err(ToolError::failed)?;
        Ok(ToolOutput::text(format!("Lesson #{id} kept.")))
    }
}

/// Drops a standing rule.
#[derive(Clone, Copy, Debug)]
pub(crate) struct LessonDrop;

/// The arguments of the tools that take a rule's number.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema, garde::Validate)]
pub(crate) struct RuleId {
    /// The rule's number.
    #[garde(skip)]
    id: i64,
}

impl Tool for LessonDrop {
    type Args = RuleId;

    fn name(&self) -> &'static str {
        "lesson_drop"
    }

    fn description(&self) -> &'static str {
        "Drop a standing rule the user no longer wants."
    }

    async fn call(&self, ctx: &ToolContext, args: RuleId) -> Result<ToolOutput, ToolError> {
        ctx.app
            .learning()
            .drop_lesson(args.id)
            .await
            .map_err(ToolError::failed)?;
        Ok(ToolOutput::text("Dropped."))
    }
}

/// Drops a trial rule.
#[derive(Clone, Copy, Debug)]
pub(crate) struct TrialDropTool;

impl Tool for TrialDropTool {
    type Args = RuleId;

    fn name(&self) -> &'static str {
        "trial_drop"
    }

    fn description(&self) -> &'static str {
        "Drop a trial rule (T252: id 252) the weekly learning review or the user drops."
    }

    async fn call(&self, ctx: &ToolContext, args: RuleId) -> Result<ToolOutput, ToolError> {
        let dropped = ctx
            .app
            .learning()
            .drop_trial(args.id, TrialJudge::Review)
            .await
            .map_err(ToolError::failed)?;
        Ok(ToolOutput::text(if dropped {
            "Dropped.".to_owned()
        } else {
            format!(
                "No trial rule T{} is running (kept, dropped or past its {TRIAL_DAYS} days).",
                args.id
            )
        }))
    }
}

/// The default days of the chat log.
const fn seven() -> i64 {
    7
}

/// The chat with the assistant, a page at a time.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ChatLog;

/// `chat_log`'s arguments.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema, garde::Validate)]
pub(crate) struct ChatLogArgs {
    /// How many days back, 1 to 31 (default 7).
    #[serde(default = "seven")]
    #[garde(range(min = 1, max = 31))]
    days: i64,
    /// The last page's last time, for the next page.
    #[garde(length(max = 64))]
    after: Option<String>,
}

impl Tool for ChatLog {
    type Args = ChatLogArgs;

    fn name(&self) -> &'static str {
        "chat_log"
    }

    fn description(&self) -> &'static str {
        "The user's chat with you over the last days, in every room (their messages and your \
         answers, not events), oldest first, for reviews. A page at a time: when more follow, \
         its last line says so; call again with after (that line's time) for the next page, \
         until the end."
    }

    async fn call(&self, ctx: &ToolContext, args: ChatLogArgs) -> Result<ToolOutput, ToolError> {
        let days = args.days;
        let page = ctx
            .app
            .learning()
            .chat_log(days, args.after.as_deref())
            .await
            .map_err(ToolError::failed)?;
        if page.total == 0 {
            return Ok(ToolOutput::text("No messages in those days."));
        }
        if page.from > page.to {
            return Ok(ToolOutput::text(format!(
                "No messages after {} ({} in those days). End of the log.",
                args.after.unwrap_or_default(),
                page.total
            )));
        }
        let head = format!(
            "Messages {}-{} of {} in the last {days} days.",
            page.from, page.to, page.total
        );
        let tail = match &page.next {
            None => "End of the log.".to_owned(),
            Some(next) => format!(
                "More follow: {} messages. Next page: chat_log with days {days} and after \"{next}\".",
                page.total - page.to
            ),
        };
        Ok(ToolOutput::text(format!("{head}\n{}\n{tail}", page.text)))
    }
}
