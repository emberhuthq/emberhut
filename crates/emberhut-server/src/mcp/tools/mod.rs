//! The MCP tools: the [`Tool`] trait each tool implements, and [`room_tools`], the one list of
//! the tools a room's agent session gets.
//!
//! Adding a tool is a module here for its area (`mcp/tools/<area>.rs`, declared below) with a
//! type implementing [`Tool`], and one line in [`room_tools`]: `tool(<area>::TheTool),`. The
//! tool reaches the server's services through its [`ToolContext`] (`ctx.app.<service>()`), so
//! its logic stays in its area's service, where it is tested on its own.

use std::fmt;
use std::future::Future;

use emberhut_api::mcp::Content;
use emberhut_core::RoomId;
use futures_util::future::BoxFuture;
use garde::Validate as _;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::app::AppState;

pub(crate) use agent_set::{agent_tools, agent_worker_tools};
pub(crate) use task_plan::PLAN_TOOLS;

mod agent_memory;
mod agent_set;
mod agent_tasks;
mod agents;
mod alerts;
mod approvals;
mod browser;
mod calendar;
mod calls;
mod facts;
mod files;
mod gmail;
mod health_data;
mod learning;
mod mac;
mod messages;
mod notes;
mod parties;
mod plan;
mod recall;
mod rooms;
mod routines;
mod runners;
mod secret_links;
mod slack;
mod task_plan;
mod tasks;
mod theme;
mod threads;
mod ultra;
mod wake;
mod whatsapp;
mod workers;

/// The tools a room's agent session gets, in the order `tools/list` gives them.
#[expect(clippy::too_many_lines, reason = "every room tool, a line each")]
pub(crate) fn room_tools() -> ToolSet {
    ToolSet::new(vec![
        tool(theme::Theme),
        tool(secret_links::SecretLink),
        tool(threads::Threads),
        tool(threads::ThreadSend),
        tool(threads::Follow),
        tool(tasks::Tasks),
        tool(tasks::TaskTool),
        tool(tasks::TaskCreate),
        tool(tasks::TaskUpdate),
        tool(tasks::TaskProgress),
        tool(tasks::TaskNote),
        tool(tasks::TaskAsk),
        tool(tasks::TaskFindings),
        tool(tasks::TaskDone),
        tool(tasks::TaskWork),
        tool(tasks::TaskStop),
        tool(tasks::Projects),
        tool(tasks::ProjectSet),
        tool(tasks::AreaSet),
        tool(task_plan::PlanSet),
        tool(task_plan::PlanStepTool),
        tool(task_plan::PlanRead),
        tool(plan::Plan),
        tool(plan::GoalSet),
        tool(plan::GoalDrop),
        tool(plan::FocusSet),
        tool(plan::FocusDrop),
        tool(plan::LoopSet),
        tool(plan::CloseLoop),
        tool(plan::ConcernSet),
        tool(wake::WakeMeAt),
        tool(wake::WatchTool),
        tool(wake::WatchNote),
        tool(wake::ListWatches),
        tool(wake::StopWatch),
        tool(wake::ListCheckins),
        tool(wake::CancelCheckin),
        tool(files::SendFile),
        tool(routines::SetRoutines),
        tool(alerts::TellOwner),
        tool(rooms::RoomSend),
        tool(rooms::Rooms),
        tool(facts::Facts),
        tool(facts::FactSetTool),
        tool(facts::FactDropTool),
        tool(facts::PersonSetTool),
        tool(facts::Context),
        tool(recall::Recall),
        tool(notes::Notes),
        tool(notes::NoteSearch),
        tool(notes::DiaryWrite),
        tool(learning::CorrectionLog),
        tool(learning::Corrections),
        tool(learning::ChatLog),
        tool(learning::LessonSet),
        tool(learning::LessonDrop),
        tool(learning::TrialDropTool),
        tool(runners::ThreadStart),
        tool(runners::SessionRead),
        tool(runners::SessionSend),
        tool(runners::SessionSendFile),
        tool(runners::SessionStop),
        tool(runners::Machines),
        tool(runners::AssistantAccount),
        tool(calls::MacRecord),
        tool(calls::CallTranscribe),
        tool(calendar::Calendar),
        tool(calendar::Reminders),
        tool(calendar::AddReminder),
        tool(calendar::AddEvent),
        tool(health_data::Health),
        tool(health_data::ScreenTime),
        tool(messages::MessagesChats),
        tool(messages::MessagesRead),
        tool(messages::MessagesSearch),
        tool(messages::RepliedTool),
        tool(slack::SlackFileTool),
        tool(slack::SlackSend),
        tool(gmail::GmailConnect),
        tool(whatsapp::WaGroupReply),
        tool(whatsapp::WaGroups),
        tool(whatsapp::WaChatSet),
        tool(whatsapp::WaLink),
        tool(whatsapp::WaSwitch),
        tool(whatsapp::WhatsAppSendTool),
        tool(browser::Open),
        tool(browser::Snapshot),
        tool(browser::Click),
        tool(browser::Plan),
        tool(browser::Type),
        tool(browser::Press),
        tool(browser::Autofill),
        tool(browser::Scroll),
        tool(browser::WaitFor),
        tool(browser::Tabs),
        tool(browser::Close),
        tool(browser::Handoff),
        tool(browser::Takeback),
        tool(browser::SaveSecret),
        tool(browser::Upload),
        tool(browser::Download),
        tool(browser::ChromePair),
        tool(mac::Apps),
        tool(mac::AppOpen),
        tool(mac::AppSnapshot),
        tool(mac::AppClick),
        tool(mac::AppType),
        tool(mac::AppMenu),
        tool(mac::Screen),
        tool(parties::PartyStart),
        tool(parties::Parties),
        tool(approvals::ApprovalRequest),
        tool(approvals::ApprovalStatus),
        tool(approvals::ApprovalWithdraw),
        tool(approvals::ApprovalUse),
        tool(agents::AgentReply),
        tool(agents::AgentThread),
        tool(agents::Agents),
    ])
}

/// The room tools a worker reads with: what the owner's messages, calendar, facts, plan and
/// tasks say, nothing that acts.
const WORKER_READS: [&str; 19] = [
    "threads",
    "machines",
    "health",
    "screen_time",
    "messages_chats",
    "messages_read",
    "messages_search",
    "slack_file",
    "replied",
    "context",
    "corrections",
    "chat_log",
    "facts",
    "recall",
    "plan",
    "tasks",
    "projects",
    "calendar",
    "reminders",
];

/// The tools a worker's session gets, in order: the room's read tools, its own task's tools,
/// and an ultra run's lead's when it is one.
pub(crate) fn worker_tools(ultra: bool) -> ToolSet {
    let mut tools: Vec<Box<dyn AnyTool>> = room_tools()
        .tools
        .into_iter()
        .filter(|t| WORKER_READS.contains(&t.name()))
        .collect();
    tools.extend([
        tool(workers::WorkerTask),
        tool(workers::WorkerNote),
        tool(workers::WorkerFindings),
        tool(workers::WorkerAsk),
        tool(workers::TaskRunner),
        tool(workers::WorkerDone),
        tool(task_plan::PlanSet),
        tool(task_plan::PlanStepTool),
        tool(task_plan::PlanRead),
    ]);
    if ultra {
        tools.extend([
            tool(ultra::UltraPlan),
            tool(ultra::UltraLane),
            tool(ultra::UltraWait),
        ]);
    }
    ToolSet::new(tools)
}

/// The tools a family member's room gets (a short list): their reminders
/// (check-ins in their room), what the assistant knows and keeps about the home, the owner's
/// calendar when they ask whether the owner is free, and passing something on. Nothing that
/// acts as the owner (their messages, tasks, browser, Mac).
const FAMILY_TOOLS: [&str; 10] = [
    "wake_me_at",
    "list_checkins",
    "cancel_checkin",
    "facts",
    "fact_set",
    "fact_drop",
    "recall",
    "calendar",
    "room_send",
    alerts::TELL_OWNER,
];

/// The tools a family member's room's agent session gets: [`FAMILY_TOOLS`], in the room
/// tools' order.
pub(crate) fn family_tools() -> ToolSet {
    ToolSet::new(
        room_tools()
            .tools
            .into_iter()
            .filter(|t| FAMILY_TOOLS.contains(&t.name()))
            .collect(),
    )
}

/// What a tool call runs with: the server's services, and the room whose session called.
#[derive(Clone, Debug)]
pub(crate) struct ToolContext {
    /// Every service.
    pub(crate) app: AppState,
    /// The room whose agent session called: the room its token names.
    pub(crate) room: RoomId,
    /// The task whose worker called (the worker endpoint), when it was one.
    pub(crate) worker: Option<emberhut_core::task::TaskId>,
    /// The runner session that called, as its request named it
    /// ([`emberhut_api::wire::mcp::SESSION_HEADER`]): a session id in its checked form, else
    /// none.
    pub(crate) session: Option<String>,
}

/// One MCP tool.
///
/// Its arguments are a type of their own: read from the call's JSON with serde (fields the
/// type doesn't know are ignored), then checked with
/// its `garde` rules; their JSON Schema, which the agent reads, is derived from the same type
/// with utoipa. A field the type doesn't require is `Option` or `#[serde(default)]`. Arguments
/// that fail either way never reach [`Tool::call`]: the agent gets the reason as an error.
pub(crate) trait Tool: Send + Sync + 'static {
    /// The tool's arguments. A tool without any takes [`NoArgs`].
    type Args: DeserializeOwned + utoipa::ToSchema + garde::Validate<Context = ()> + Send;

    /// Its name, as the agent calls it (the wire contract).
    fn name(&self) -> &'static str;

    /// What the agent reads about it: what it does and when to use it. Generic: "the owner",
    /// "the user", never a person.
    fn description(&self) -> &'static str;

    /// Runs the tool on `args`, for `ctx.room`'s session.
    fn call(
        &self,
        ctx: &ToolContext,
        args: Self::Args,
    ) -> impl Future<Output = Result<ToolOutput, ToolError>> + Send;
}

/// The arguments of a tool that takes none.
#[derive(Clone, Copy, Debug, Default, Deserialize, utoipa::ToSchema, garde::Validate)]
pub(crate) struct NoArgs {}

/// What a tool answers: text (most tools), or pictures too.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ToolOutput {
    /// What it says, in order.
    pub(crate) content: Vec<Content>,
    /// It is an error the agent should see as one.
    pub(crate) is_error: bool,
}

impl ToolOutput {
    /// Text.
    pub(crate) fn text(text: impl Into<String>) -> Self {
        Self {
            content: vec![Content::text(text)],
            is_error: false,
        }
    }

    /// An error, as text the agent reads.
    pub(crate) fn error(text: impl Into<String>) -> Self {
        Self {
            content: vec![Content::text(text)],
            is_error: true,
        }
    }

    /// `value` as pretty-printed JSON text, as the tools answer data.
    pub(crate) fn json(value: &impl serde::Serialize) -> Result<Self, ToolError> {
        serde_json::to_string_pretty(value)
            .map(Self::text)
            .map_err(ToolError::failed)
    }
}

/// Why a tool call has no ordinary answer. Every case reaches the agent as an error result
/// with its text; a failure's whole cause chain goes to the log too.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub(crate) enum ToolError {
    /// The arguments are not ones the tool takes.
    #[error("{0}")]
    Invalid(String),
    /// The tool refuses this call; the text says why, for the agent.
    #[error("{0}")]
    Refused(String),
    /// Something failed underneath (the database, a service).
    #[error("{0}")]
    Failed(#[source] Box<dyn std::error::Error + Send + Sync>),
}

impl ToolError {
    /// A refusal with this text.
    pub(crate) fn refused(text: impl Into<String>) -> Self {
        Self::Refused(text.into())
    }

    /// A failure caused by `error`.
    pub(crate) fn failed(error: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self::Failed(Box::new(error))
    }
}

/// A tool of any argument type, as the endpoint lists and calls it.
pub(crate) trait AnyTool: Send + Sync {
    /// Its name.
    fn name(&self) -> &'static str;
    /// Its description.
    fn description(&self) -> &'static str;
    /// Its arguments' JSON Schema.
    fn input_schema(&self) -> Value;
    /// Reads `args` as its arguments and runs it.
    fn call<'a>(
        &'a self,
        ctx: &'a ToolContext,
        args: Value,
    ) -> BoxFuture<'a, Result<ToolOutput, ToolError>>;
}

impl fmt::Debug for dyn AnyTool {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Tool").field(&self.name()).finish()
    }
}

/// `tool` for the list in [`room_tools`].
pub(crate) fn tool<T: Tool>(tool: T) -> Box<dyn AnyTool> {
    Box::new(Typed(tool))
}

struct Typed<T>(T);

impl<T: Tool> AnyTool for Typed<T> {
    fn name(&self) -> &'static str {
        self.0.name()
    }

    fn description(&self) -> &'static str {
        self.0.description()
    }

    fn input_schema(&self) -> Value {
        input_schema::<T::Args>()
    }

    fn call<'a>(
        &'a self,
        ctx: &'a ToolContext,
        args: Value,
    ) -> BoxFuture<'a, Result<ToolOutput, ToolError>> {
        Box::pin(async move {
            let args: T::Args = serde_json::from_value(args)
                .map_err(|error| ToolError::Invalid(error.to_string()))?;
            args.validate().map_err(|report| {
                ToolError::Invalid(
                    report
                        .iter()
                        .map(|(path, error)| format!("{path}: {error}"))
                        .collect::<Vec<_>>()
                        .join("; "),
                )
            })?;
            self.0.call(ctx, args).await
        })
    }
}

/// Where utoipa points a reference to a named schema: the `OpenAPI` document's components.
const COMPONENTS: &str = "#/components/schemas/";
/// Where the input schema keeps them instead: its own `$defs`, so the schema stands alone.
const DEFS: &str = "#/$defs/";

/// `schema` as JSON. A utoipa schema is plain data and always writes (a test lists every tool's
/// schema); were one ever not to, the failure is logged and it reads as `null`, which
/// [`input_schema`] turns into an empty object schema.
fn schema_json(schema: impl serde::Serialize) -> Value {
    serde_json::to_value(schema).unwrap_or_else(|error| {
        tracing::error!(%error, "a tool's input schema could not be written as JSON");
        Value::Null
    })
}

/// An arguments type's schema as MCP takes it: a JSON Schema object (`type: object`, with its
/// `properties`, even when there are none) that stands alone: the named types it uses are in
/// its `$defs`, and its `$ref`s point there.
fn input_schema<T: utoipa::ToSchema>() -> Value {
    let mut schema = schema_json(T::schema());
    let mut named = Vec::new();
    T::schemas(&mut named);
    if let Value::Object(object) = &mut schema {
        object
            .entry("type")
            .or_insert_with(|| Value::from("object"));
        object
            .entry("properties")
            .or_insert_with(|| Value::Object(serde_json::Map::new()));
        if !named.is_empty() {
            let defs = named
                .into_iter()
                .map(|(name, schema)| (name, schema_json(schema)))
                .collect();
            object.insert("$defs".to_owned(), Value::Object(defs));
        }
    } else {
        schema = serde_json::json!({ "type": "object", "properties": {} });
    }
    point_refs_at_defs(&mut schema);
    schema
}

/// Rewrites every `$ref` to the document's components in `value` to the schema's `$defs`.
fn point_refs_at_defs(value: &mut Value) {
    match value {
        Value::Object(object) => {
            for (key, inner) in object.iter_mut() {
                if key == "$ref"
                    && let Value::String(target) = inner
                    && let Some(name) = target.strip_prefix(COMPONENTS)
                {
                    *target = format!("{DEFS}{name}");
                } else {
                    point_refs_at_defs(inner);
                }
            }
        }
        Value::Array(items) => items.iter_mut().for_each(point_refs_at_defs),
        _ => {}
    }
}

/// The tools one endpoint serves.
#[derive(Debug)]
pub(crate) struct ToolSet {
    tools: Vec<Box<dyn AnyTool>>,
}

impl ToolSet {
    /// These tools; of two with one name, the later one.
    pub(crate) fn new(tools: Vec<Box<dyn AnyTool>>) -> Self {
        let mut set: Vec<Box<dyn AnyTool>> = Vec::with_capacity(tools.len());
        for tool in tools {
            set.retain(|kept| kept.name() != tool.name());
            set.push(tool);
        }
        Self { tools: set }
    }

    /// Every tool, in order.
    pub(crate) fn iter(&self) -> impl Iterator<Item = &dyn AnyTool> {
        self.tools.iter().map(AsRef::as_ref)
    }

    /// The tool called `name`.
    pub(crate) fn get(&self, name: &str) -> Option<&dyn AnyTool> {
        self.iter().find(|tool| tool.name() == name)
    }

    /// These tools without the ones called one of `names`.
    #[must_use]
    pub(crate) fn without(mut self, names: &[&str]) -> Self {
        self.tools.retain(|tool| !names.contains(&tool.name()));
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_family_room_gets_only_the_family_tools() {
        let names: Vec<&str> = family_tools().iter().map(AnyTool::name).collect();
        let mut sorted = names.clone();
        sorted.sort_unstable();
        let mut want = FAMILY_TOOLS.to_vec();
        want.sort_unstable();
        assert_eq!(sorted, want, "every family tool is a room tool");
    }

    #[test]
    fn every_room_tool_has_a_unique_name_and_an_object_schema() {
        let tools = room_tools();
        let mut names: Vec<&str> = tools.iter().map(AnyTool::name).collect();
        let count = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), count);
        for tool in tools.iter() {
            let schema = tool.input_schema();
            assert_eq!(schema["type"], "object", "{}", tool.name());
            assert!(schema["properties"].is_object(), "{}", tool.name());
            assert!(!tool.description().is_empty(), "{}", tool.name());
            assert_refs_resolve(tool.name(), &schema, &schema);
        }
    }

    /// Fails unless every `$ref` in `value` points into `root`'s own `$defs`, at a schema
    /// that is there.
    fn assert_refs_resolve(tool: &str, root: &Value, value: &Value) {
        match value {
            Value::Object(object) => {
                for (key, inner) in object {
                    if key == "$ref" {
                        let target = inner.as_str().unwrap_or_default();
                        assert!(!target.starts_with("#/components/"), "{tool}: {target}");
                        let name = target
                            .strip_prefix(DEFS)
                            .unwrap_or_else(|| panic!("{tool}: {target} is not in $defs"));
                        assert!(
                            root["$defs"][name].is_object(),
                            "{tool}: {target} does not resolve"
                        );
                    } else {
                        assert_refs_resolve(tool, root, inner);
                    }
                }
            }
            Value::Array(items) => {
                for item in items {
                    assert_refs_resolve(tool, root, item);
                }
            }
            _ => {}
        }
    }

    #[test]
    fn a_named_type_an_argument_uses_is_in_the_schemas_own_defs() {
        let schema = room_tools()
            .get("theme")
            .map_or(Value::Null, AnyTool::input_schema);
        assert!(
            schema["properties"]["appearance"]
                .to_string()
                .contains(r##""$ref":"#/$defs/ThemeAppearance""##),
            "{schema}"
        );
        assert!(schema["$defs"]["ThemeAppearance"]["enum"].is_array());
        assert_refs_resolve("theme", &schema, &schema);
    }

    #[test]
    fn no_arguments_is_an_empty_object_schema() {
        let schema = input_schema::<NoArgs>();
        assert_eq!(schema["type"], "object");
        assert_eq!(schema["properties"], serde_json::json!({}));
        assert!(schema.get("$defs").is_none());
    }
}
