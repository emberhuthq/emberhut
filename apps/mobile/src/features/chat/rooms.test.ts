import { describe, expect, it } from "vitest";
import { ASSISTANT, OWNER, type AgentRow, type Approval, type EmberhutState, type RoomRow, type WorkRow } from "@/api";
import { needsCount, needsRows } from "@/features/needsYou/model";
import { goalColors } from "@/theme/goalColors";
import { dayFromNow, foldedRooms, openRooms, roomColor, roomNeeds, roomPreview, shortAgo, splitAgentRooms } from "./rooms";
import { emberhutStatus } from "./status";

const room = (id: number, extra: Partial<RoomRow> = {}): RoomRow => ({
  id,
  name: `R${id}`,
  project: id,
  unread: 0,
  needs: 0,
  busy: false,
  active: true,
  ...extra,
});

const ask = (id: string, fields: Partial<WorkRow> = {}): WorkRow => ({
  id,
  kind: "task",
  title: id,
  who: "You",
  state: "waiting",
  ...fields,
});

describe("rooms", () => {
  it("counts each room's asks from the rows the Needs you badge counts, so they add up to it", () => {
    const now = new Date("2026-11-24T12:00:00");
    const rooms = [
      room(1, { project: null }),
      room(2, { project: 20 }),
      room(3, { project: 30, active: false, status: "paused" }),
      room(4, { project: 40, active: false, status: "archived" }),
    ];
    const rows = [
      ask("task:1", { task: 1, pid: 20 }),
      ask("task:7", { task: 7, pid: 20 }),
      // Not on Needs you (no task of its own), so in no room either.
      ask("thread:a", { kind: "thread", pid: 20 }),
      ask("task:2", { task: 2, pid: null }),
      // An archived project's room isn't in the switcher: its asks are Main's.
      ask("task:3", { task: 3, pid: 40 }),
      // A project the state has no room for.
      ask("task:4", { task: 4, pid: 99 }),
      // Later (a paused project's, or more than a month out) is counted nowhere.
      ask("task:5", { task: 5, pid: 30, paused: true }),
      ask("task:6", { task: 6, pid: 20, decideBy: "2027-03-01" }),
    ];
    const state = { working: rows, issues: [] } as unknown as EmberhutState;
    const counts = roomNeeds(rooms, needsRows(state, new Set()), now);
    expect(Object.fromEntries(counts)).toEqual({ 1: 3, 2: 2 });
    const badge = needsCount(state, new Set(), now);
    expect([...counts.values()].reduce((a, b) => a + b, 0)).toBe(badge);
  });

  it("lists each agent's room apart, under the agent's name, never as Main; the default agent stays Main", () => {
    const rooms = [room(1, { project: null }), room(2), room(9, { project: null, name: "client-a", unread: 2 })];
    const agents: AgentRow[] = [
      { id: "main", name: "Emberhut", principal: "the owner", room: 1, enabled: true, pendingApprovals: 0 },
      { id: "client-a", name: "Example helper", principal: "Example Ltd", room: 9, enabled: true, pendingApprovals: 1 },
      // Its room isn't in the state's rooms (yet): it's listed all the same, as off.
      { id: "client-b", name: "Other helper", principal: "Example Co", room: 12, enabled: false, pendingApprovals: 0 },
    ];
    const split = splitAgentRooms(rooms, agents);
    expect(split.rooms.map((r) => r.id)).toEqual([1, 2]);
    expect(split.agents.map((a) => [a.room.id, a.room.name, a.agent.id])).toEqual([
      [9, "Example helper", "client-a"],
      [12, "Other helper", "client-b"],
    ]);
    expect(split.agents[0]!.room.unread).toBe(2);
    expect(split.agents[1]!.room).toMatchObject({ unread: 0, needs: 0, busy: false, active: false });
    expect(splitAgentRooms(rooms, undefined).rooms).toHaveLength(3);
  });

  it("counts an approval waiting on you in its agent's room, the default agent's in Main's, so they add up to the badge", () => {
    const now = new Date("2026-11-24T12:00:00");
    const rooms = [room(1, { project: null }), room(2, { project: 20 }), room(9, { project: null })];
    const agents: AgentRow[] = [
      { id: "main", name: "Emberhut", principal: "the owner", room: 1, enabled: true, pendingApprovals: 1 },
      { id: "client-a", name: "Example helper", principal: "Example Ltd", room: 9, enabled: true, pendingApprovals: 3 },
    ];
    const pending = (id: number, agent: string, waitsOnOwner = true): Approval => ({
      id,
      agent,
      kind: "post",
      target: "example blog",
      summary: "A post",
      payloadHash: "cd".repeat(32),
      approver: waitsOnOwner ? "owner" : "example-lead",
      waitsOnOwner,
      state: "pending",
      createdAt: "2026-11-24T10:00:00.000Z",
    });
    const approvals = [pending(1, "client-a"), pending(2, "client-a"), pending(3, "client-a", false), pending(4, "main")];
    const rows = [ask("task:1", { task: 1, pid: 20 })];
    const state = { working: rows, issues: [], approvals, agents } as unknown as EmberhutState;
    const counts = roomNeeds(rooms, needsRows(state, new Set()), now, approvals, agents);
    expect(Object.fromEntries(counts)).toEqual({ 1: 1, 2: 1, 9: 2 });
    expect([...counts.values()].reduce((a, b) => a + b, 0)).toBe(needsCount(state, new Set(), now));
  });

  it("keeps Main and active rooms open, folds paused ones, leaves archived out", () => {
    const rooms = [
      room(1, { project: null, active: false }),
      room(2),
      room(3, { active: false, status: "paused" }),
      room(4, { active: false, status: "archived" }),
    ];
    expect(openRooms(rooms).map((r) => r.id)).toEqual([1, 2]);
    expect(foldedRooms(rooms).map((r) => r.id)).toEqual([3]);
  });

  it("previews the last line without its marks", () => {
    expect(roomPreview(room(2, { last: "- **Done**: the [PR](https://x.y)", lastMine: true }))).toBe("You: Done: the PR");
    expect(roomPreview(room(2, { last: "" }))).toBeNull();
  });

  it("says when, short", () => {
    const now = Date.parse("2026-11-24T12:00:00Z");
    expect(shortAgo("2026-11-24T11:59:30Z", now)).toBe("now");
    expect(shortAgo("2026-11-24T11:58:00Z", now)).toBe("2m");
    expect(shortAgo("2026-11-24T09:00:00Z", now)).toBe("3h");
    expect(shortAgo(null, now)).toBeNull();
    expect(dayFromNow(7, new Date(2026, 10, 28))).toBe("2026-12-05");
  });
});

describe("emberhutStatus", () => {
  const state = (extra: Partial<EmberhutState> = {}) => ({ on: true, tasks: [], ...extra }) as EmberhutState;

  it("says what Emberhut is doing", () => {
    expect(emberhutStatus(undefined, false, false).text).toBe("Connecting…");
    expect(emberhutStatus(state(), false, true)).toEqual({ text: "Offline", tone: "warning" });
    expect(emberhutStatus(state(), false, false).text).toBe("Watching");
    expect(emberhutStatus(state({ on: false }), false, false).text).toBe("Paused");
    const jobs = [{ id: "a", description: "", type: "", status: "running", startedAt: "" }];
    expect(emberhutStatus(state({ tasks: jobs }), true, false).text).toBe("Working… · 1 job");
  });

  it("warns as the quota runs low, red when it's out", () => {
    const now = Date.parse("2026-11-24T12:00:00Z");
    expect(emberhutStatus(state({ account: { used5h: 0.85, used7d: 0.9 } }), false, false, now)).toEqual({
      text: "Watching · 90% of weekly used",
      tone: "warning",
    });
    expect(emberhutStatus(state({ account: { used5h: 0.97 } }), false, false, now).tone).toBe("danger");
    expect(emberhutStatus(state({ account: { limitedUntil: "2026-11-24T14:00:00Z" } }), false, false, now).text).toMatch(
      /^Out of quota until/,
    );
  });
});

describe("roomColor", () => {
  const inGoal = (goal: number | null) => room(5, { project: 3, goal });
  const goals = [
    { id: 7, title: "Ship", why: "", rank: 1, week: { [OWNER]: 0, [ASSISTANT]: 0 } },
    { id: 8, title: "Learn", why: "", rank: 9, week: { [OWNER]: 0, [ASSISTANT]: 0 } },
  ];
  it("takes its goal's color by rank, round the seven, and gray without one", () => {
    const light = goalColors("light");
    expect(roomColor(inGoal(7), goals, light)).toBe("#FF9500");
    expect(roomColor(inGoal(8), goals, light)).toBe("#34C759");
    expect(roomColor(inGoal(null), goals, light)).toBe("#8E8E93");
    expect(roomColor(inGoal(99), goals, light)).toBe("#8E8E93");
  });

  it("follows dark mode", () => {
    expect(roomColor(inGoal(7), goals, goalColors("dark"))).toBe("#FF9F0A");
    expect(goalColors("dark").ranks).toHaveLength(goalColors("light").ranks.length);
  });
});
