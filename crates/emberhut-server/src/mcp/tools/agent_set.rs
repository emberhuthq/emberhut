//! The tools a configured agent's room and its tasks' workers get, and the agent a call is
//! from.
//!
//! A configured agent's room gets the agent tools ([`AGENT_TOOLS`]: its approvals, its answers
//! and threads in its channels, its own memory) and, of the server tools it lists, those that
//! act only for it: the ones that act in its own room ([`IN_ITS_ROOM`]) and its tasks' tools,
//! scoped to the tasks made in its room ([`ITS_TASKS`]). A listed tool that acts as the owner
//! is never served (`acts_as_owner`), and neither is one that reads or changes the owner's
//! own things (their messages, plan, notes, other rooms, every task): it is left out, logged.
//!
//! The agent is always the one whose room's token called: never an argument.

use emberhut_core::agents::{Agent, AgentTool, acts_as_owner};
use emberhut_core::task::TaskId;

use super::{
    AnyTool, ToolContext, ToolError, ToolSet, agent_memory, agent_tasks, agents, approvals,
    room_tools, task_plan, tool, ultra, workers,
};

/// Every configured agent's own tools, whatever it lists.
pub(crate) const AGENT_TOOLS: [&str; 9] = [
    "approval_request",
    "approval_status",
    "approval_withdraw",
    "approval_use",
    "agent_reply",
    "agent_thread",
    "facts",
    "fact_set",
    "fact_drop",
];

/// The room tools an agent may list that act only in the room that called: its check-ins and
/// watches (listed, cancelled, noted and stopped only when its room made them), and a file
/// shown in its room.
pub(crate) const IN_ITS_ROOM: [&str; 8] = [
    "wake_me_at",
    "list_checkins",
    "cancel_checkin",
    "watch",
    "watch_note",
    "list_watches",
    "stop_watch",
    "send_file",
];

/// The task tools an agent may list, each scoped to the tasks made in its room.
pub(crate) const ITS_TASKS: [&str; 5] = [
    "tasks",
    "task_create",
    "task_send",
    "task_stop",
    "task_cancel",
];

/// The agent whose room `ctx`'s token names: the default agent for a room no configured
/// agent owns.
pub(crate) async fn calling_agent(ctx: &ToolContext) -> Result<Agent, ToolError> {
    ctx.app
        .agents()
        .store()
        .by_room(ctx.room)
        .await
        .map_err(ToolError::failed)
}

/// Fails unless the task `id` is `agent`'s own: made in its room (for the default agent, made
/// in no configured agent's room).
pub(crate) async fn own_task(
    ctx: &ToolContext,
    agent: &Agent,
    id: TaskId,
) -> Result<(), ToolError> {
    let room = ctx.app.tasks().room_of(id).await?;
    let owner = match room {
        Some(room) => {
            ctx.app
                .agents()
                .store()
                .by_room(room)
                .await
                .map_err(ToolError::failed)?
                .id
        }
        None => emberhut_core::agents::AgentId::MAIN,
    };
    if owner == agent.id {
        Ok(())
    } else {
        Err(ToolError::refused(format!(
            "{} isn't one of your tasks.",
            id.key()
        )))
    }
}

/// The agent tools, in order.
fn own() -> Vec<Box<dyn AnyTool>> {
    vec![
        tool(approvals::ApprovalRequest),
        tool(approvals::ApprovalStatus),
        tool(approvals::ApprovalWithdraw),
        tool(approvals::ApprovalUse),
        tool(agents::AgentReply),
        tool(agents::AgentThread),
        tool(agent_memory::AgentFacts),
        tool(agent_memory::AgentFactSet),
        tool(agent_memory::AgentFactDrop),
    ]
}

/// A task tool scoped to the agent's tasks, by name.
fn its_task_tool(name: &str) -> Option<Box<dyn AnyTool>> {
    Some(match name {
        "tasks" => tool(agent_tasks::AgentTasks),
        "task_create" => tool(agent_tasks::AgentTaskCreate),
        "task_send" => tool(agent_tasks::AgentTaskSend),
        "task_stop" => tool(agent_tasks::AgentTaskStop),
        "task_cancel" => tool(agent_tasks::AgentTaskCancel),
        _ => return None,
    })
}

/// The tools `agent`'s room gets, its lists being `listed`.
pub(crate) fn agent_tools(agent: &Agent, listed: &[AgentTool]) -> ToolSet {
    let mut tools = own();
    let mut room = room_tools();
    for listed in listed {
        let AgentTool::Server { name } = listed else {
            continue;
        };
        let name = name.as_str();
        if acts_as_owner(name) {
            tracing::warn!(agent = %agent.id, tool = name, "a tool that acts as the owner is never an agent's");
        } else if ITS_TASKS.contains(&name)
            && let Some(scoped) = its_task_tool(name)
        {
            tools.push(scoped);
        } else if IN_ITS_ROOM.contains(&name)
            && let Some(at) = room.tools.iter().position(|t| t.name() == name)
        {
            tools.push(room.tools.swap_remove(at));
        } else if !AGENT_TOOLS.contains(&name) {
            tracing::warn!(agent = %agent.id, tool = name, "a tool that reaches past the agent's own room is not served to it");
        }
    }
    ToolSet::new(tools)
}

/// The tools a worker on a configured agent's task gets: its own task's, an ultra run's lead's
/// when it is one, the agent's approvals and, read-only, its memory. None of the owner's.
pub(crate) fn agent_worker_tools(ultra: bool) -> ToolSet {
    let mut tools = vec![
        tool(workers::WorkerTask),
        tool(workers::WorkerNote),
        tool(workers::WorkerFindings),
        tool(workers::WorkerAsk),
        tool(workers::TaskRunner),
        tool(workers::WorkerDone),
        tool(task_plan::PlanSet),
        tool(task_plan::PlanStepTool),
        tool(task_plan::PlanRead),
        tool(approvals::ApprovalRequest),
        tool(approvals::ApprovalStatus),
        tool(approvals::ApprovalWithdraw),
        tool(approvals::ApprovalUse),
        tool(agent_memory::AgentFacts),
    ];
    if ultra {
        tools.extend([
            tool(ultra::UltraPlan),
            tool(ultra::UltraLane),
            tool(ultra::UltraWait),
        ]);
    }
    ToolSet::new(tools)
}

#[cfg(test)]
mod tests {
    use emberhut_core::RoomId;
    use emberhut_core::agents::AgentId;

    use super::*;

    fn agent() -> Agent {
        Agent {
            id: AgentId::parse("client-a").unwrap(),
            name: "Example helper".to_owned(),
            principal: "Example Ltd".to_owned(),
            room: RoomId::new(5),
            ..Agent::main()
        }
    }

    fn server(name: &str) -> AgentTool {
        AgentTool::Server {
            name: name.to_owned(),
        }
    }

    fn names(set: &ToolSet) -> Vec<&'static str> {
        set.iter().map(AnyTool::name).collect()
    }

    #[test]
    fn an_agent_gets_its_own_tools_and_only_what_acts_for_it() {
        let set = agent_tools(
            &agent(),
            &[
                server("tasks"),
                server("wake_me_at"),
                server("slack_send"),
                server("messages_read"),
                server("recall"),
                server("room_send"),
                AgentTool::Mcp {
                    name: "example-admin".to_owned(),
                    config: "/srv/a.json".to_owned(),
                },
            ],
        );
        let mut want = AGENT_TOOLS.to_vec();
        want.extend(["tasks", "wake_me_at"]);
        assert_eq!(names(&set), want);
    }

    #[test]
    fn with_nothing_listed_an_agent_has_its_own_tools() {
        assert_eq!(names(&agent_tools(&agent(), &[])), AGENT_TOOLS.to_vec());
    }

    #[test]
    fn every_listable_tool_is_served_and_acts_for_no_owner() {
        let listed: Vec<AgentTool> = IN_ITS_ROOM
            .iter()
            .chain(ITS_TASKS.iter())
            .map(|name| server(name))
            .collect();
        let set = agent_tools(&agent(), &listed);
        for name in IN_ITS_ROOM.iter().chain(ITS_TASKS.iter()) {
            assert!(set.get(name).is_some(), "{name}");
            assert!(!acts_as_owner(name), "{name}");
        }
    }

    #[test]
    fn an_agent_s_worker_reads_none_of_the_owner_s_things() {
        let set = agent_worker_tools(false);
        for name in [
            "messages_read",
            "calendar",
            "health",
            "threads",
            "plan",
            "recall",
        ] {
            assert!(set.get(name).is_none(), "{name}");
        }
        assert!(set.get("approval_request").is_some());
        assert!(set.get("task_done").is_some());
        assert!(
            set.get("fact_set").is_none(),
            "a worker reads the agent's memory only"
        );
    }
}
