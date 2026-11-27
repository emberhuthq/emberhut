import { describe, expect, it } from "vitest";
import type { StateStorage } from "zustand/middleware";
import { createConnectionStore, offlineReason, quietMs } from "@/state/connection";
import { createLiveStore, liveLine } from "@/state/live";
import { createPrefsStore } from "@/state/prefs";

describe("connection", () => {
  it("says Emberhut can't be reached only after failing for a while, and clears at once", () => {
    const store = createConnectionStore();
    store.getState().noteFailure("Can't reach Emberhut", 1_000);
    store.getState().noteFailure("Can't reach Emberhut", 1_000 + quietMs - 1);
    expect(offlineReason(store.getState())).toBeNull();
    store.getState().noteFailure("Can't reach Emberhut", 1_000 + quietMs);
    expect(offlineReason(store.getState())).toBe("Can't reach Emberhut");
    store.getState().noteSuccess();
    expect(offlineReason(store.getState())).toBeNull();
  });
});

describe("live", () => {
  it("prefers the stream's line while connected, the poll's otherwise", () => {
    const store = createLiveStore();
    const polled = { text: "", doing: "From the poll" };
    expect(liveLine(store.getState(), 1, polled)).toBe(polled);
    store.getState().setFollowing(1, true);
    expect(liveLine(store.getState(), 1, polled)).toBeNull();
    store.getState().set(1, { text: "", doing: "Streamed" });
    expect(liveLine(store.getState(), 1, polled)).toEqual({ text: "", doing: "Streamed" });
    store.getState().setFollowing(1, false);
    expect(store.getState().byRoom[1]).toBeNull();
  });

  it("keeps the same state for the same line", () => {
    const store = createLiveStore();
    store.getState().set(2, { text: "", doing: "A" });
    const before = store.getState();
    store.getState().set(2, { text: "", doing: "A" });
    expect(store.getState()).toBe(before);
  });
});

describe("prefs", () => {
  it("keeps the theme, room and Needs you grouping across launches", async () => {
    const saved = new Map<string, string>();
    const storage: StateStorage = {
      getItem: (k) => saved.get(k) ?? null,
      setItem: (k, v) => void saved.set(k, v),
      removeItem: (k) => void saved.delete(k),
    };
    const first = createPrefsStore(storage);
    first.getState().setTheme("dark");
    first.getState().setRoom(4);
    first.getState().setNeedsGrouping("when");
    first.getState().setSessionFilter({ workers: ["lead", "helper"], machines: ["machine-b"] });
    const second = createPrefsStore(storage);
    await second.persist.rehydrate();
    expect(second.getState()).toMatchObject({
      theme: "dark",
      room: 4,
      needsGrouping: "when",
      sessionFilter: { workers: ["lead", "helper"], machines: ["machine-b"] },
    });
  });
});
