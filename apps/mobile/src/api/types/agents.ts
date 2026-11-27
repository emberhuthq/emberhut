/** An agent in the state (`agents` in GET /api/state): a configured assistant with a room of its own. */
export interface AgentRow {
  /** A slug ("client-a"); "main" is the default agent. */
  id: string;
  name: string;
  /** Who it works for, a display name. */
  principal: string;
  /** Its own room. */
  room: number;
  /** A disabled agent's room takes no turns and its approvals aren't routed. */
  enabled: boolean;
  /** How many of its requests wait on an answer, the owner's or an approver's. */
  pendingApprovals: number;
}

/** An agent as GET /api/agents lists it. */
export interface Agent extends AgentRow {
  /** Its system prompt; none for the default agent. */
  persona?: string | null;
  /** The project it is scoped to. */
  project?: number | null;
  /** The most its approved spending may add up to in a month, in cents; none: no spending at all. */
  spendCapCents?: number | null;
  /** The cap's currency ("EUR"). */
  spendCurrency?: string | null;
}

/** GET /api/agents: every agent, the default agent first. */
export interface AgentsPage {
  agents: Agent[];
}
