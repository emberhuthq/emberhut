import { describe, expect, it } from "vitest";
import { LOOKING_HEADER, type EmberhutState } from "@/api";
import { fakeApi, json } from "@/test/fakes";

const state = (room: number, text: string) =>
  ({ on: true, busy: false, room, chat: [{ id: 1, mine: false, text, time: "10:00" }] }) as unknown as EmberhutState;

describe("GET /api/state", () => {
  it("sends the room's ETag back and returns the same object on 304", async () => {
    const { server, api } = fakeApi();
    const answers = [
      json(200, state(1, "a"), { ETag: '"t1"' }),
      json(304, null, { ETag: '"t1"' }),
      json(200, state(1, "b"), { ETag: '"t2"' }),
    ];
    server.on("GET", "/api/state", () => answers.shift()!);

    const first = await api.state.get({ room: 1, looking: true });
    const second = await api.state.get({ room: 1, looking: false });
    const third = await api.state.get({ room: 1, looking: false });

    const sent = server.to("/api/state");
    expect(sent[0]!.headers["if-none-match"]).toBeUndefined();
    expect(sent[0]!.headers[LOOKING_HEADER.toLowerCase()]).toBe("1");
    expect(sent[1]!.headers["if-none-match"]).toBe('"t1"');
    expect(sent[1]!.headers[LOOKING_HEADER.toLowerCase()]).toBeUndefined();
    expect(second).toBe(first);
    expect(third.chat[0]!.text).toBe("b");
    expect(sent[2]!.headers["if-none-match"]).toBe('"t1"');
  });

  it("keeps tags per room, and forgets them on reset", async () => {
    const { server, api } = fakeApi();
    server.on("GET", "/api/state", (r) => {
      const room = Number(r.url.searchParams.get("room"));
      return json(200, state(room, "x"), { ETag: `"room-${room}"` });
    });
    await api.state.get({ room: 1, looking: false });
    await api.state.get({ room: 4, looking: false });
    await api.state.get({ room: 1, looking: false });
    api.state.reset();
    await api.state.get({ room: 4, looking: false });
    expect(server.to("/api/state").map((r) => r.headers["if-none-match"])).toEqual([undefined, undefined, '"room-1"', undefined]);
  });

  it("fails on a 304 it has nothing for", async () => {
    const { server, api } = fakeApi();
    server.on("GET", "/api/state", () => json(304, null));
    await expect(api.state.get({ room: 1, looking: false })).rejects.toMatchObject({ status: 304 });
  });
});
