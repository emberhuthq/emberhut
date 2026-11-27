import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ChatMsg } from "@/api";
import { fakeApi, json } from "@/test/fakes";
import { createTalkStore, delivered, lostNote } from "./talk";

const msg = (id: number, text: string, mine = true): ChatMsg => ({ id, mine, text, time: "10:00" });

function setup(chat: ChatMsg[] = [msg(5, "earlier")]) {
  const { api, server } = fakeApi();
  const sent: number[] = [];
  const store = createTalkStore({ api, chat: () => chat, onSent: (room) => sent.push(room), lostMs: 45_000 });
  return { api, server, store, sent, chat };
}

describe("talk store", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it("sends the text, its files and the quote, and holds it until Emberhut answers", async () => {
    const { server, store, sent } = setup();
    server.on("POST", "/api/talk", () => json(200, { ok: true, started: true }));
    const file = { name: "a.png", url: "/uploads/a.png", kind: "image" as const, path: "/srv/uploads/a.png" };
    await store.getState().send(3, { text: "  look at this ", files: [file], reply: { id: 5, mine: true, text: "earlier" } });
    expect(server.to("/api/talk")[0]?.body).toEqual({ text: "look at this", files: [file], room: 3, replyTo: 5 });
    expect(store.getState().pending[3]).toMatchObject({ text: "look at this", after: 5, note: null, failed: false });
    expect(sent).toEqual([3]);
  });

  it("lets go when nothing is started", async () => {
    const { server, store } = setup();
    server.on("POST", "/api/talk", () => json(200, { ok: true }));
    await store.getState().send(1, { text: "fine", files: [] });
    expect(store.getState().pending[1]).toBeUndefined();
  });

  it("shows why it wasn't sent, and sends the same again on retry", async () => {
    const { server, store } = setup();
    server.on("POST", "/api/talk", () => json(500, { error: "disk full" }));
    await store.getState().send(1, { text: "hi", files: [] });
    expect(store.getState().pending[1]).toMatchObject({ note: "Not sent: disk full", failed: true });
    server.on("POST", "/api/talk", () => json(200, { ok: true, started: true }));
    await store.getState().retry(1);
    expect(server.to("/api/talk")).toHaveLength(2);
    expect(store.getState().pending[1]).toMatchObject({ text: "hi", failed: false, note: null });
  });

  it("calls a send lost after 45 s without its copy, but not once it arrived", async () => {
    const { server, store, chat } = setup();
    server.on("POST", "/api/talk", () => json(200, { ok: true, started: true }));
    await store.getState().send(1, { text: "one", files: [] });
    await vi.advanceTimersByTimeAsync(45_000);
    expect(store.getState().pending[1]).toMatchObject({ note: lostNote, failed: true });

    await store.getState().send(2, { text: "two", files: [] });
    chat.push(msg(6, "two"));
    await vi.advanceTimersByTimeAsync(45_000);
    expect(store.getState().pending[2]?.failed).toBe(false);
  });

  it("ignores an empty message, and forgets on request", async () => {
    const { server, store } = setup();
    server.on("POST", "/api/talk", () => json(200, { ok: true, started: true }));
    await store.getState().send(1, { text: "   ", files: [] });
    expect(server.to("/api/talk")).toHaveLength(0);
    await store.getState().send(1, { text: "x", files: [] });
    store.getState().forget(1);
    expect(store.getState().pending[1]).toBeUndefined();
  });
});

describe("delivered", () => {
  it("is your own newer message with the same words", () => {
    const pending = { text: "ok", files: [], reply: null, after: 5, note: null, failed: false };
    expect(delivered(pending, [msg(4, "ok")])).toBe(false);
    expect(delivered(pending, [msg(6, "ok", false)])).toBe(false);
    expect(delivered(pending, [msg(6, "ok")])).toBe(true);
  });
});
