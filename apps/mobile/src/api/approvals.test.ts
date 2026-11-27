import { describe, expect, it } from "vitest";
import { followRoomLive, type Approval, type ApprovalsEvent, type Live, type EmberhutState } from "@/api";
import { fakeApi, json, streamBody } from "@/test/fakes";

const hash = (pair: string) => pair.repeat(32);

/** A request as the server writes it (`emberhut_api::approvals::Approval`, camelCase). */
const wire = {
  id: 7,
  agent: "client-a",
  agentName: "Example helper",
  kind: "post",
  target: "example blog",
  summary: "The launch post",
  text: "We shipped it.",
  files: [{ name: "cover.png", size: 9, sha256: hash("ab") }],
  payloadHash: hash("cd"),
  approver: "example-lead",
  approverName: "Example Lead",
  waitsOnOwner: false,
  amountCents: null,
  task: 12,
  state: "pending",
  stateReason: null,
  createdAt: "2026-11-27T10:00:00.000Z",
  expiresAt: "2026-11-30T10:00:00.000Z",
  decidedAt: null,
  decidedBy: null,
  decidedVia: null,
  usedAt: null,
  usedBy: null,
  imported: false,
};

describe("approvals", () => {
  it("lists with the state and agent filters, leaving out the ones not given", async () => {
    const { server, api } = fakeApi();
    server.on("GET", "/api/approvals", () => json(200, { approvals: [wire] }));
    const all = await api.approvals.list();
    await api.approvals.list({ state: "pending", agent: "client-a" });
    const sent = server.to("/api/approvals");
    expect(sent[0]!.url.search).toBe("");
    expect(Object.fromEntries(sent[1]!.url.searchParams)).toEqual({ state: "pending", agent: "client-a" });
    // Read as the server wrote it: every field, under its wire name.
    const approval: Approval = all[0]!;
    expect(approval).toEqual(wire);
    expect(approval.files?.[0]?.sha256).toHaveLength(64);
  });

  it("encodes an agent id in the query", async () => {
    const { server, api } = fakeApi();
    server.on("GET", "/api/approvals", () => json(200, { approvals: [] }));
    await api.approvals.list({ agent: "a&b" });
    expect(server.to("/api/approvals")[0]!.url.search).toBe("?agent=a%26b");
  });

  it("reads one request from its reply", async () => {
    const { server, api } = fakeApi();
    server.on("GET", "/api/approvals/7", () => json(200, { approval: wire }));
    expect(await api.approvals.get(7)).toEqual(wire);
  });

  it("sends a decision with the payload hash shown, and answers the request as it is after", async () => {
    const { server, api } = fakeApi();
    const decided = { ...wire, state: "approved", decidedVia: "app", decidedBy: "owner" };
    server.on("POST", "/api/approvals/7/decision", () => json(200, { approval: decided }));
    expect(await api.approvals.decide(7, "approve", hash("cd"))).toEqual(decided);
    await api.approvals.decide(7, "reject", hash("cd"), "Not this week");
    const sent = server.to("/api/approvals/7/decision");
    expect(sent[0]!.method).toBe("POST");
    expect(sent[0]!.headers["content-type"]).toBe("application/json");
    expect(sent[0]!.body).toEqual({ decision: "approve", payloadHash: hash("cd") });
    expect(sent[1]!.body).toEqual({ decision: "reject", payloadHash: hash("cd"), reason: "Not this week" });
  });

  it("fails a decision on a changed or decided request with its 409", async () => {
    const { server, api } = fakeApi();
    server.on("POST", "/api/approvals/7/decision", () => json(409, { error: "the payload changed" }));
    await expect(api.approvals.decide(7, "approve", hash("00"))).rejects.toMatchObject({ status: 409, message: "the payload changed" });
  });

  it("withdraws, with a reason only when given", async () => {
    const { server, api } = fakeApi();
    server.on("POST", "/api/approvals/7/withdraw", () => json(200, { approval: { ...wire, state: "withdrawn" } }));
    expect((await api.approvals.withdraw(7)).state).toBe("withdrawn");
    await api.approvals.withdraw(7, "Asked twice");
    const sent = server.to("/api/approvals/7/withdraw");
    expect(sent[0]!.body).toEqual({});
    expect(sent[1]!.body).toEqual({ reason: "Asked twice" });
  });
});

describe("agents", () => {
  it("lists every agent, the default one first", async () => {
    const { server, api } = fakeApi();
    const agents = [
      { id: "main", name: "Emberhut", principal: "the owner", room: 1, enabled: true, pendingApprovals: 0 },
      {
        id: "client-a",
        name: "Example helper",
        persona: "You help Example Ltd.",
        principal: "Example Ltd",
        room: 9,
        project: 4,
        spendCapCents: 10_000,
        spendCurrency: "EUR",
        enabled: true,
        pendingApprovals: 2,
      },
    ];
    server.on("GET", "/api/agents", () => json(200, { agents }));
    expect(await api.agents.list()).toEqual(agents);
  });
});

describe("the state's agents and approvals", () => {
  it("are read when there, and left out by a server without them", async () => {
    const { server, api } = fakeApi();
    const agent = { id: "client-a", name: "Example helper", principal: "Example Ltd", room: 9, enabled: false, pendingApprovals: 1 };
    server.on("GET", "/api/state", (r) =>
      r.url.searchParams.get("room") === "9"
        ? json(200, { on: true, room: 9, chat: [], agents: [agent], approvals: [wire] })
        : json(200, { on: true, room: 1, chat: [] }),
    );
    const full: EmberhutState = await api.state.get({ room: 9, looking: false });
    expect(full.agents).toEqual([agent]);
    expect(full.approvals?.[0]?.payloadHash).toBe(hash("cd"));
    const without = await api.state.get({ room: 1, looking: false });
    expect(without.agents).toBeUndefined();
    expect(without.approvals).toBeUndefined();
  });
});

describe("the room stream's approvals event", () => {
  it("goes to onApprovals, never as a live line, and an unknown named event is left alone", async () => {
    const { server, http } = fakeApi();
    const stream = streamBody();
    server.on("GET", "/api/live", () => new Response(stream.body, { status: 200, headers: { "Content-Type": "text/event-stream" } }));
    const lines: (Live | null)[] = [];
    const events: ApprovalsEvent[] = [];
    const controller = new AbortController();
    const done = followRoomLive(http, 9, (live) => lines.push(live), {
      signal: controller.signal,
      retryMs: 5,
      onApprovals: (event) => events.push(event),
    });
    stream.write('data: {"text":"","doing":"Drafting"}\n\n');
    stream.write('event: approvals\ndata: {"id":7,"state":"approved","pending":2}\n\n');
    stream.write('event: something-new\ndata: {"text":"not a line"}\n\n');
    stream.write("event: approvals\ndata: not json\n\n");
    stream.write("data: null\n\n");
    for (let i = 0; i < 200 && lines.length < 2; i++) await new Promise((resolve) => setTimeout(resolve, 2));
    controller.abort();
    await done;
    expect(lines).toEqual([{ text: "", doing: "Drafting" }, null]);
    expect(events).toEqual([{ id: 7, state: "approved", pending: 2 }]);
  });
});
