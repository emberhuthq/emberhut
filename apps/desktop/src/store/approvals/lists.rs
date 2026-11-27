//! Who decides what: the pending requests split into what waits on the user and what waits on
//! an approver, Needs you's counts of them, and the agents with rooms of their own. Pure
//! functions over the state's lists, tested on their own.

use emberhut_api::agents::AgentRow;
use emberhut_api::approvals::{Approval, ApprovalState};
use emberhut_api::ids::{ConfiguredAgentId, RoomId};
use emberhut_api::rooms::RoomRow;

/// A pending request that waits on the user: they may approve or reject it.
#[must_use]
pub(crate) fn waits_on_you(approval: &Approval) -> bool {
    approval.state == ApprovalState::Pending && approval.waits_on_owner
}

/// The pending requests, split by who decides them, each group in the server's order (newest
/// first).
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Groups<'a> {
    /// Waiting on the user: Approve and Reject.
    pub(crate) yours: Vec<&'a Approval>,
    /// Waiting on an approver: shown with who that is, never decided here.
    pub(crate) others: Vec<&'a Approval>,
}

impl<'a> Groups<'a> {
    /// The pending ones of `approvals`, those of `agent` alone when one is given.
    #[must_use]
    pub(crate) fn of(approvals: &'a [Approval], agent: Option<&ConfiguredAgentId>) -> Self {
        let (yours, others) = approvals
            .iter()
            .filter(|approval| approval.state == ApprovalState::Pending)
            .filter(|approval| agent.is_none_or(|agent| approval.agent == *agent))
            .partition(|approval| waits_on_you(approval));
        Self { yours, others }
    }

    /// Whether neither group has a request.
    #[must_use]
    pub(crate) fn is_empty(&self) -> bool {
        self.yours.is_empty() && self.others.is_empty()
    }
}

/// Needs you's row of the pending requests: how many wait on the user, how many of those only
/// they can approve, and how many wait on others.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Summary {
    /// Waiting on the user (counted in Needs you's badge).
    pub(crate) yours: usize,
    /// Of those, the fixed core's: only the user approves them.
    pub(crate) only_you: usize,
    /// Waiting on an approver.
    pub(crate) others: usize,
}

impl Summary {
    /// The summary of the pending ones of `approvals`.
    #[must_use]
    pub(crate) fn of(approvals: &[Approval]) -> Self {
        let groups = Groups::of(approvals, None);
        Self {
            yours: groups.yours.len(),
            only_you: groups
                .yours
                .iter()
                .filter(|approval| approval.kind.is_fixed_core())
                .count(),
            others: groups.others.len(),
        }
    }

    /// Whether any request is pending.
    #[must_use]
    pub(crate) fn any(self) -> bool {
        self.yours + self.others > 0
    }

    /// The row's title: `2 wait on you`, `None wait on you`.
    #[must_use]
    pub(crate) fn title(self) -> String {
        match self.yours {
            0 => "None wait on you".to_owned(),
            1 => "1 waits on you".to_owned(),
            n => format!("{n} wait on you"),
        }
    }

    /// The row's second line: `1 only you can approve · 2 on others`; none when both are 0.
    #[must_use]
    pub(crate) fn subtitle(self) -> Option<String> {
        let parts: Vec<String> = [
            (self.only_you > 0).then(|| format!("{} only you can approve", self.only_you)),
            (self.others > 0).then(|| format!("{} on others", self.others)),
        ]
        .into_iter()
        .flatten()
        .collect();
        (!parts.is_empty()).then(|| parts.join(" · "))
    }
}

/// An agent with a room of its own, that room's line when the state has it, and how many of
/// its requests wait on the user.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct AgentRoom<'a> {
    /// The agent.
    pub(crate) agent: &'a AgentRow,
    /// Its room's line among the state's rooms.
    pub(crate) room: Option<&'a RoomRow>,
    /// Its pending requests that wait on the user (not those waiting on an approver), so the
    /// rooms' counts add up to Needs you's.
    pub(crate) waiting: usize,
}

/// The agents with rooms of their own, in the state's order: every agent but the default one,
/// whose room is Main; each with its requests among `approvals` that wait on the user.
#[must_use]
pub(crate) fn agent_rooms<'a>(
    agents: &'a [AgentRow],
    rooms: &'a [RoomRow],
    approvals: &[Approval],
) -> Vec<AgentRoom<'a>> {
    agents
        .iter()
        .filter(|agent| !agent.id.is_main() && !agent.room.is_main())
        .map(|agent| AgentRoom {
            agent,
            room: rooms.iter().find(|room| room.id == agent.room),
            waiting: waiting_on_you(approvals, &agent.id),
        })
        .collect()
}

/// How many of `agent`'s requests among `approvals` wait on the user.
#[must_use]
pub(crate) fn waiting_on_you(approvals: &[Approval], agent: &ConfiguredAgentId) -> usize {
    approvals
        .iter()
        .filter(|approval| approval.agent == *agent && waits_on_you(approval))
        .count()
}

/// The agent whose own room `room` is, if any.
#[must_use]
pub(crate) fn agent_of_room(agents: &[AgentRow], room: RoomId) -> Option<&AgentRow> {
    agents
        .iter()
        .find(|agent| !agent.id.is_main() && !agent.room.is_main() && agent.room == room)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::fixtures::room;

    fn approval(id: i64, more: &serde_json::Value) -> Approval {
        let mut base = json!({
            "id": id, "agent": "client-a", "kind": "post", "target": "example blog",
            "summary": "A post", "files": [], "payloadHash": "ab".repeat(32),
            "approver": "owner", "waitsOnOwner": true, "state": "pending",
            "createdAt": "2026-11-27T10:00:00Z"
        });
        if let (Some(base), Some(more)) = (base.as_object_mut(), more.as_object()) {
            base.extend(more.clone());
        }
        serde_json::from_value(base).unwrap()
    }

    pub(crate) fn agent(id: &str, room: i64, enabled: bool) -> AgentRow {
        AgentRow {
            id: ConfiguredAgentId::new(id),
            name: format!("Agent {id}"),
            principal: "Example Ltd".to_owned(),
            room: RoomId::new(room),
            enabled,
            pending_approvals: 0,
        }
    }

    #[test]
    fn the_pending_split_by_who_decides() {
        let approvals = [
            approval(1, &json!({})),
            approval(
                2,
                &json!({"waitsOnOwner": false, "approver": "example-lead"}),
            ),
            approval(3, &json!({"state": "approved", "waitsOnOwner": false})),
            approval(4, &json!({"agent": "client-b", "kind": "payment"})),
        ];
        let groups = Groups::of(&approvals, None);
        let ids = |list: &[&Approval]| list.iter().map(|a| a.id.get()).collect::<Vec<_>>();
        assert_eq!(ids(&groups.yours), [1, 4]);
        assert_eq!(ids(&groups.others), [2]);
        let one = Groups::of(&approvals, Some(&ConfiguredAgentId::new("client-b")));
        assert_eq!(ids(&one.yours), [4]);
        assert_eq!(one.others.len(), 0);
        assert!(
            Groups::of(&approvals[2..3], None).is_empty(),
            "decided: not listed"
        );
    }

    #[test]
    fn needs_you_counts_what_waits_on_you_and_what_only_you_can_approve() {
        let approvals = [
            approval(1, &json!({"kind": "payment"})),
            approval(2, &json!({})),
            approval(
                3,
                &json!({"waitsOnOwner": false, "approver": "example-lead"}),
            ),
        ];
        let summary = Summary::of(&approvals);
        assert_eq!(
            summary,
            Summary {
                yours: 2,
                only_you: 1,
                others: 1
            }
        );
        assert_eq!(summary.title(), "2 wait on you");
        assert_eq!(
            summary.subtitle().as_deref(),
            Some("1 only you can approve · 1 on others")
        );
        let none = Summary::of(&approvals[2..]);
        assert_eq!(none.title(), "None wait on you");
        assert_eq!(none.subtitle().as_deref(), Some("1 on others"));
        assert!(none.any());
        assert!(!Summary::of(&[]).any());
        assert_eq!(Summary::of(&approvals[1..2]).subtitle(), None);
    }

    #[test]
    fn every_agent_but_the_default_one_has_a_room_of_its_own() {
        let agents = [agent("main", 1, true), agent("client-a", 9, false)];
        let rooms = [room(1, json!({})), room(9, json!({"name": "client-a"}))];
        let approvals = [
            approval(1, &json!({})),
            approval(
                2,
                &json!({"waitsOnOwner": false, "approver": "example-lead"}),
            ),
            approval(3, &json!({"agent": "client-b"})),
        ];
        let own = agent_rooms(&agents, &rooms, &approvals);
        assert_eq!(own.len(), 1);
        assert_eq!(own[0].agent.id.as_str(), "client-a");
        assert_eq!(own[0].room.map(|room| room.id), Some(RoomId::new(9)));
        assert_eq!(own[0].waiting, 1, "only what waits on you counts");
        assert_eq!(
            agent_of_room(&agents, RoomId::new(9)).map(|a| a.id.as_str()),
            Some("client-a")
        );
        assert_eq!(
            agent_of_room(&agents, RoomId::MAIN),
            None,
            "Main is no agent's room"
        );
        let without_line = agent_rooms(&agents, &rooms[..1], &[]);
        assert_eq!(without_line[0].room, None);
    }
}
