import { describe, expect, it } from "vitest";
import { followParty, followRoomLive, followSession, type Live, type PartyLiveEvent, type SessionLiveEvent } from "@/api";
import { parseSse, type SseMessage } from "@/api/sse";
import { fakeApi, streamBody } from "@/test/fakes";

async function collect(chunks: string[]): Promise<SseMessage[]> {
  const encoder = new TextEncoder();
  const body = new ReadableStream<Uint8Array>({
    start(c) {
      for (const chunk of chunks) c.enqueue(encoder.encode(chunk));
      c.close();
    },
  });
  const out: SseMessage[] = [];
  for await (const message of parseSse(body)) out.push(message);
  return out;
}

const sse = (body: ReadableStream<Uint8Array>) => new Response(body, { status: 200, headers: { "Content-Type": "text/event-stream" } });

describe("parseSse", () => {
  it("reads data, event names and ids across arbitrary chunk splits", async () => {
    const text = 'data: {"text":"","doing":"Reading"}\n\n: ping\n\nevent: state\nid: 4\ndata: one\ndata: two\n\ndata: null\n\n';
    const whole = await collect([text]);
    expect(whole).toEqual([
      { event: "message", data: '{"text":"","doing":"Reading"}' },
      { event: "state", data: "one\ntwo", id: "4" },
      { event: "message", data: "null", id: "4" },
    ]);
    for (const size of [1, 2, 3, 7]) {
      const chunks = text.match(new RegExp(`[\\s\\S]{1,${size}}`, "g"))!;
      expect(await collect(chunks)).toEqual(whole);
    }
  });

  it("takes CRLF and CR line ends, also split between chunks", async () => {
    expect(await collect(["data: a\r", "\n\r", "\ndata:b\r\r"])).toEqual([
      { event: "message", data: "a" },
      { event: "message", data: "b" },
    ]);
  });

  it("drops a message cut off before its blank line", async () => {
    expect(await collect(["data: whole\n\ndata: cut"])).toEqual([{ event: "message", data: "whole" }]);
  });
});

describe("followRoomLive", () => {
  it("hands over each live line and the end of the turn, and reconnects after a drop", async () => {
    const { server, http } = fakeApi();
    const streams: ReturnType<typeof streamBody>[] = [];
    server.on("GET", "/api/live", () => {
      const s = streamBody();
      streams.push(s);
      return sse(s.body);
    });
    const seen: (Live | null)[] = [];
    const connections: boolean[] = [];
    const controller = new AbortController();
    const done = followRoomLive(http, 3, (live) => seen.push(live), {
      signal: controller.signal,
      retryMs: 5,
      onConnection: (c) => connections.push(c),
    });

    await until(() => streams.length === 1);
    streams[0]!.write('data: {"text":"","doing":"Checking the Mac"}\n\n');
    streams[0]!.write(": ping\n\ndata: null\n\n");
    await until(() => seen.length === 2);
    streams[0]!.end();
    await until(() => streams.length === 2);
    streams[1]!.write('data: {"text":"","doing":"Again"}\n\n');
    await until(() => seen.length === 3);
    controller.abort();
    await done;

    expect(seen).toEqual([{ text: "", doing: "Checking the Mac" }, null, { text: "", doing: "Again" }]);
    expect(connections).toEqual([true, false, true, false]);
    expect(server.to("/api/live")[0]!.url.searchParams.get("room")).toBe("3");
  });
});

describe("a quiet stream", () => {
  it("is dropped and opened again when nothing arrives, not even a ping", async () => {
    const { server, http } = fakeApi();
    let opened = 0;
    server.on("GET", "/api/live", () => {
      opened++;
      return sse(streamBody().body);
    });
    const controller = new AbortController();
    const done = followRoomLive(http, 1, () => undefined, { signal: controller.signal, retryMs: 1, idleMs: 15 });
    await until(() => opened >= 3);
    controller.abort();
    await done;
  });
});

describe("followSession", () => {
  it("asks only for steps after the last one seen when it reconnects", async () => {
    const { server, http } = fakeApi();
    const streams: ReturnType<typeof streamBody>[] = [];
    server.on("GET", "/api/runner/sessions/r-1a2b/live", () => {
      const s = streamBody();
      streams.push(s);
      return sse(s.body);
    });
    const events: SessionLiveEvent[] = [];
    const controller = new AbortController();
    const done = followSession(http, "r-1a2b", { after: 10 }, (e) => events.push(e), { signal: controller.signal, retryMs: 5 });

    await until(() => streams.length === 1);
    streams[0]!.write('data: {"type":"step","step":{"seq":11,"turn":"r-1a2b:2","at":null,"kind":"text","text":"Hi"}}\n\n');
    streams[0]!.write('data: {"type":"delta","turn":"r-1a2b:2","text":"Wor","start":true}\n\n');
    await until(() => events.length === 2);
    streams[0]!.end();
    await until(() => streams.length === 2);
    controller.abort();
    await done;

    expect(server.to("/api/runner/sessions/r-1a2b/live").map((r) => r.url.searchParams.get("after"))).toEqual(["10", "11"]);
    expect(events.map((e) => e.type)).toEqual(["step", "delta"]);
  });
});

describe("followParty", () => {
  it("hands over each event, skips what isn't one, and opens the stream again after a drop", async () => {
    const { server, http } = fakeApi();
    const streams: ReturnType<typeof streamBody>[] = [];
    server.on("GET", "/api/parties/3/live", () => {
      const s = streamBody();
      streams.push(s);
      return sse(s.body);
    });
    const events: PartyLiveEvent[] = [];
    const controller = new AbortController();
    const done = followParty(http, 3, (e) => events.push(e), { signal: controller.signal, retryMs: 5 });

    await until(() => streams.length === 1);
    streams[0]!.write(": ping\n\ndata: not json\n\ndata: null\n\n");
    streams[0]!.write('data: {"type":"speaking","speaking":{"who":"p1","name":"Nora","text":"Hi"}}\n\n');
    await until(() => events.length === 1);
    streams[0]!.end();
    await until(() => streams.length === 2);
    controller.abort();
    await done;

    expect(events).toEqual([{ type: "speaking", speaking: { who: "p1", name: "Nora", text: "Hi" } }]);
  });
});

async function until(check: () => boolean): Promise<void> {
  for (let i = 0; i < 200 && !check(); i++) await new Promise((resolve) => setTimeout(resolve, 2));
  expect(check()).toBe(true);
}
