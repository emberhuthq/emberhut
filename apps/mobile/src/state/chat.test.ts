import { describe, expect, it } from "vitest";
import type { ChatMsg } from "@/api";
import { createChatStore, join, pageSize } from "@/state/chat";
import { fakeApi, json } from "@/test/fakes";

const msg = (id: number, text = `m${id}`): ChatMsg => ({ id, mine: id % 2 === 0, text, time: "10:00" });
const range = (from: number, to: number) => Array.from({ length: to - from + 1 }, (_, i) => msg(from + i));
const ids = (list: readonly ChatMsg[]) => list.map((m) => m.id);

describe("join", () => {
  it("keeps what's held before an overlapping page, and the page's copies win", () => {
    const held = [msg(1), msg(2), msg(3, "old")];
    expect(join(held, [msg(3, "new"), msg(4)])).toEqual([msg(1), msg(2), msg(3, "new"), msg(4)]);
  });

  it("keeps what's held before a page that isn't full, gap or not", () => {
    expect(ids(join([msg(1), msg(2)], [msg(10), msg(11)]))).toEqual([1, 2, 10, 11]);
  });

  it("starts over after a full page with a gap before it", () => {
    const page = range(100, 100 + pageSize - 1);
    expect(ids(join([msg(1), msg(2)], page))).toEqual(ids(page));
  });

  it("keeps what's held after the page", () => {
    expect(ids(join([msg(5), msg(6), msg(7)], [msg(5), msg(6)]))).toEqual([5, 6, 7]);
  });

  it("leaves the held chat alone for an empty page", () => {
    expect(ids(join([msg(1)], []))).toEqual([1]);
  });
});

describe("chat store", () => {
  it("holds a room's empty first page: a room with no messages yet is open, not still opening", () => {
    const { api } = fakeApi();
    const store = createChatStore(api);
    store.getState().hold(5, []);
    expect(store.getState().chats[5]).toEqual([]);
  });

  it("catches up from the last message held, then loads earlier ones until the first", async () => {
    const { server, api } = fakeApi();
    server.on("GET", "/api/history", (r) => {
      const after = r.url.searchParams.get("after");
      const before = r.url.searchParams.get("before");
      if (after === "0") return json(200, { chat: range(100, 159) });
      if (after === "159") return json(200, { chat: [msg(160)] });
      if (before === "100") return json(200, { chat: range(40, 99) });
      if (before === "40") return json(200, { chat: range(1, 39) });
      return json(400, { error: "unexpected" });
    });
    const store = createChatStore(api);

    await store.getState().catchUp(5);
    await store.getState().catchUp(5);
    expect(store.getState().chats[5]?.at(-1)?.id).toBe(160);

    await store.getState().loadEarlier(5);
    expect(store.getState().complete[5]).toBe(false);
    await store.getState().loadEarlier(5);
    expect(store.getState().complete[5]).toBe(true);
    expect(ids(store.getState().chats[5]!)).toEqual(ids(range(1, 160)));
    await store.getState().loadEarlier(5);
    expect(server.to("/api/history")).toHaveLength(4);
  });

  it("loads earlier pages once when asked twice at the same time", async () => {
    const { server, api } = fakeApi();
    server.on("GET", "/api/history", () => json(200, { chat: range(40, 99) }));
    const store = createChatStore(api);
    store.getState().hold(2, range(100, 110));
    await Promise.all([store.getState().loadEarlier(2), store.getState().loadEarlier(2)]);
    expect(server.to("/api/history")).toHaveLength(1);
    expect(ids(store.getState().chats[2]!)).toEqual(ids(range(40, 110)));
  });

  it("forgets that it held the first message once a page starts the chat over", async () => {
    const { server, api } = fakeApi();
    server.on("GET", "/api/history", () => json(200, { chat: range(1, 9) }));
    const store = createChatStore(api);
    store.getState().hold(3, range(10, 20));
    await store.getState().loadEarlier(3);
    expect(store.getState().complete[3]).toBe(true);
    store.getState().hold(3, range(500, 500 + pageSize - 1));
    expect(store.getState().complete[3]).toBe(false);
  });

  it("doesn't change its chats for a page it already holds", () => {
    const { api } = fakeApi();
    const store = createChatStore(api);
    store.getState().hold(1, [msg(1), msg(2)]);
    const before = store.getState().chats;
    store.getState().hold(1, store.getState().chats[1]!);
    expect(store.getState().chats).toBe(before);
  });
});
