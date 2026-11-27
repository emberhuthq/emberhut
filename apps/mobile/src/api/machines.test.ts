import { describe, expect, it } from "vitest";
import { fakeApi, json } from "@/test/fakes";

const page = (history?: unknown[]) => ({
  at: "2026-11-24T10:00:00.000Z",
  thresholds: {},
  machines: [{ id: "machine-a", name: "Studio", placeable: true, ...(history ? { history } : {}) }],
});

describe("machines", () => {
  it("asks for the whole page, history included, by default", async () => {
    const { server, api } = fakeApi();
    server.on("GET", "/api/machines", () => json(200, page([{ at: "2026-11-24T09:55:00.000Z" }])));
    const got = await api.machines.list();
    expect(server.to("/api/machines")[0]!.url.search).toBe("");
    expect(got.machines[0]!.history).toHaveLength(1);
  });

  it("leaves the history out with ?history=0, and reads it as empty", async () => {
    const { server, api } = fakeApi();
    server.on("GET", "/api/machines", () => json(200, page()));
    const got = await api.machines.list(undefined, { history: false });
    expect(server.to("/api/machines")[0]!.url.searchParams.get("history")).toBe("0");
    expect(got.machines[0]).toMatchObject({ id: "machine-a", name: "Studio", history: [] });
  });
});
