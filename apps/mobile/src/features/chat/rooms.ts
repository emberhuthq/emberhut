import type { AgentRow, Approval, Goal, RoomRow, WorkRow } from "@/api";
import { agentsByRoom, visible, waitsOnYou } from "@/features/approvals/model";
import { rowWhen } from "@/features/needsYou/model";
import type { GoalPalette } from "@/theme";
import type { AgentsPlace } from "@/state/prefs";
import { plainText } from "@/ui/markdownParse";

export const isMain = (r: RoomRow): boolean => r.project === null || r.project === undefined;
export const isArchived = (r: RoomRow): boolean => r.status === "archived";
export const isPaused = (r: RoomRow): boolean => r.status === "paused";

/** The switcher's order: Main, then the active projects' rooms by last activity, as the server sends them. */
export const openRooms = (rooms: readonly RoomRow[]): RoomRow[] => rooms.filter((r) => r.active || isMain(r));
/** Folded at the bottom: paused and done projects' rooms; archived ones are out of the switcher. */
export const foldedRooms = (rooms: readonly RoomRow[]): RoomRow[] => rooms.filter((r) => !r.active && !isMain(r) && !isArchived(r));

/** An agent's room in the switcher: its room's line (or one made from the agent while the state has none) and the agent. */
export interface AgentRoom {
  room: RoomRow;
  agent: AgentRow;
}

/**
 * The switcher's rooms split from the agents' own: every agent but the default one has a room of its own, listed under
 * Agents with the agent's name, in the state's order of agents, and never as Main or a project's room.
 */
export function splitAgentRooms(
  rooms: readonly RoomRow[],
  agents: readonly AgentRow[] | null | undefined,
): { rooms: RoomRow[]; agents: AgentRoom[] } {
  const byRoom = agentsByRoom(agents);
  const own = [...byRoom.values()].map((agent) => {
    const row = rooms.find((r) => r.id === agent.room);
    const room: RoomRow = row
      ? { ...row, name: visible(agent.name) }
      : { id: agent.room, name: visible(agent.name), unread: 0, needs: 0, busy: false, active: agent.enabled };
    return { room, agent };
  });
  return { rooms: rooms.filter((r) => !byRoom.has(r.id)), agents: own };
}

/** A section of the rooms list: the open rooms, the agents' rooms, the paused fold. */
export type RoomsSection = "open" | "agents" | "paused";

/**
 * The rooms list's sections in order: the open rooms first, then the agents' rooms after the paused fold ("below", the
 * default, so agents sit out of the way) or before it ("above"); no agents' section at all while they are hidden.
 */
export function roomsSections(showAgents: boolean, agentsPlace: AgentsPlace): RoomsSection[] {
  if (!showAgents) return ["open", "paused"];
  return agentsPlace === "above" ? ["open", "agents", "paused"] : ["open", "paused", "agents"];
}

/**
 * What waits on you in each room, counted from what the Needs you tab counts (its badge, its project groups and its
 * approvals), so the rooms' counts add up to the badge: a project's asks in its room, the rest (no project, or a project
 * whose room isn't in the switcher, as an archived one's) in Main's; an approval waiting on you in its agent's room, or
 * Main's for the default agent's. Later is left out, as from the badge; a paused project's asks wait with Later, so its
 * room shows none.
 */
export function roomNeeds(
  rooms: readonly RoomRow[],
  rows: readonly WorkRow[],
  now: Date = new Date(),
  approvals: readonly Approval[] = [],
  agents: readonly AgentRow[] = [],
): Map<number, number> {
  const agentRooms = agentsByRoom(agents);
  const main = rooms.find((r) => isMain(r) && !agentRooms.has(r.id))?.id;
  const home = new Map<number, number>();
  for (const r of rooms) if (!isMain(r) && !isArchived(r) && r.project != null) home.set(r.project, r.id);
  const counts = new Map<number, number>();
  const add = (id: number | undefined) => {
    if (id !== undefined) counts.set(id, (counts.get(id) ?? 0) + 1);
  };
  for (const row of rows) {
    if (rowWhen(row, now) === "later") continue;
    add((row.pid != null ? home.get(row.pid) : undefined) ?? main);
  }
  const agentRoom = new Map([...agentRooms.values()].map((a) => [a.id, a.room]));
  for (const approval of approvals) if (waitsOnYou(approval)) add(agentRoom.get(approval.agent) ?? main);
  return counts;
}

/** A room's last line: one line, markdown taken out, "You: " when it was yours. */
export function roomPreview(room: RoomRow): string | null {
  if (!room.last) return null;
  const text = plainText(room.last)
    .replace(/^\s*(#{1,6}|[-*>]|\d+\.)\s+/, "")
    .trim();
  return text === "" ? null : (room.lastMine ? "You: " : "") + text;
}

/** When it was said, short: "now", "2m", "3h", "Fri", "2 Oct". */
export function shortAgo(lastAt: string | null | undefined, now = Date.now()): string | null {
  const at = lastAt ? Date.parse(lastAt) : NaN;
  if (Number.isNaN(at)) return null;
  const s = (now - at) / 1000;
  if (s < 60) return "now";
  if (s < 3_600) return `${Math.floor(s / 60)}m`;
  if (s < 86_400) return `${Math.floor(s / 3_600)}h`;
  const date = new Date(at);
  return s < 6 * 86_400
    ? date.toLocaleDateString("en-GB", { weekday: "short" })
    : date.toLocaleDateString("en-GB", { day: "numeric", month: "short" });
}

/** A day as the server writes it ("2026-12-01"), `days` from now in local time. */
export function dayFromNow(days: number, now = new Date()): string {
  const d = new Date(now.getFullYear(), now.getMonth(), now.getDate() + days);
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
}

/** A project room's dot: its goal's color by rank (the theme's goal palette); gray for one under no goal, or one gone. */
export function roomColor(room: RoomRow, goals: readonly Goal[] | null | undefined, palette: GoalPalette): string {
  const goal = goals?.find((g) => g.id === room.goal);
  return goal ? (palette.ranks[(Math.max(goal.rank, 1) - 1) % palette.ranks.length] ?? palette.none) : palette.none;
}
