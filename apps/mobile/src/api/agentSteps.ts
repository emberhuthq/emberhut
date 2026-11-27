import type { SessionLiveEvent, SessionStep } from "./types";

/**
 * One sub-agent's view of a session. The server narrows a worker's ("w-…") steps to the agent itself, but a runner
 * session's ("r-…") come whole: only the steps inside the agent's call are kept, as its own (no parent).
 */
export function agentStep(session: string, agent: string | undefined, step: SessionStep): SessionStep | null {
  if (agent === undefined || session.startsWith("w-")) return step;
  if (step.parent !== agent) return null;
  const { parent: _parent, ...own } = step;
  return own;
}

/** A live event as the sub-agent's view shows it: its own steps and the head; the parent agent's streaming text is not its. */
export function agentEvent(session: string, agent: string | undefined, event: SessionLiveEvent): SessionLiveEvent | null {
  if (agent === undefined || session.startsWith("w-")) return event;
  if (event.type === "head") return event;
  if (event.type !== "step") return null;
  const step = agentStep(session, agent, event.step);
  return step === null ? null : { type: "step", step };
}
