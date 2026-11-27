import { describe, expect, it } from "vitest";
import { agentEvent, agentStep, type SessionStep } from "@/api";
import { fakeApi, json } from "@/test/fakes";

const step = (seq: number, parent?: string): SessionStep => ({
  seq,
  turn: "r-1:1",
  at: null,
  kind: "text",
  text: `s${seq}`,
  ...(parent ? { parent } : {}),
});

describe("a sub-agent's view of a session", () => {
  it("keeps only a runner session's steps inside the agent's call, as its own", () => {
    expect(agentStep("r-1", "call-7", step(1))).toBeNull();
    expect(agentStep("r-1", "call-7", step(2, "call-9"))).toBeNull();
    expect(agentStep("r-1", "call-7", step(3, "call-7"))).toEqual(step(3));
  });

  it("leaves a worker's steps and the whole session's view alone", () => {
    expect(agentStep("w-12", "call-7", step(1))).toEqual(step(1));
    expect(agentStep("r-1", undefined, step(2, "call-7"))).toEqual(step(2, "call-7"));
  });

  it("drops the parent agent's streaming text from the agent's live view, and keeps the head", () => {
    expect(agentEvent("r-1", "call-7", { type: "delta", turn: "r-1:1", text: "Hi" })).toBeNull();
    expect(agentEvent("r-1", "call-7", { type: "head", session: { id: "r-1", title: "t", status: "running" } })?.type).toBe("head");
  });

  it("narrows a runner session's transcript", async () => {
    const { server, api } = fakeApi();
    server.on("GET", "/api/runner/sessions/r-1/transcript", () =>
      json(200, { session: { id: "r-1", title: "t", status: "running" }, events: [step(1), step(2, "call-7"), step(3, "call-8")] }),
    );
    const page = await api.sessions.transcript("r-1", { after: 0, agent: "call-7" });
    expect(page.events).toEqual([step(2)]);
    expect(server.to("/api/runner/sessions/r-1/transcript")[0]!.url.searchParams.get("agent")).toBe("call-7");
  });
});
