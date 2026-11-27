import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createActionsStore, failedMs, undoMs } from "@/state/actions";
import { fakeApi, json } from "@/test/fakes";

describe("actions with undo", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  function setup(answer = () => json(200, { ok: true })) {
    const { server, api } = fakeApi();
    server.on("POST", "/api/settings", answer);
    const applied = vi.fn();
    return { server, store: createActionsStore(api, { onApplied: applied }), applied };
  }

  it("hides the task at once and sends the change only after the undo window", async () => {
    const { server, store, applied } = setup();
    store.getState().decide(12, "Approve");
    expect(store.getState().gone.has(12)).toBe(true);
    expect(store.getState().toast).toMatchObject({ task: 12, text: "Approved", warn: false });
    expect(server.sent).toHaveLength(0);

    await vi.advanceTimersByTimeAsync(undoMs);
    expect(server.to("/api/settings")[0]?.body).toEqual({ taskDecide: 12, choice: "Approve" });
    expect(store.getState().gone.has(12)).toBe(false);
    expect(store.getState().toast).toBeNull();
    expect(applied).toHaveBeenCalledOnce();
  });

  it("sends nothing when undone", async () => {
    const { server, store } = setup();
    store.getState().archive(3);
    store.getState().undo();
    await vi.advanceTimersByTimeAsync(undoMs);
    expect(server.sent).toHaveLength(0);
    expect(store.getState().gone.size).toBe(0);
  });

  it("sends the held action at once when another comes, and ignores a second tap on the same task", async () => {
    const { server, store } = setup();
    store.getState().markDone(1);
    store.getState().markDone(1);
    store.getState().later(2, new Date(2026, 10, 24, 15));
    await vi.advanceTimersByTimeAsync(0);
    expect(server.to("/api/settings").map((r) => r.body)).toEqual([{ todoDone: 1, done: true }]);
    await vi.advanceTimersByTimeAsync(undoMs);
    expect(server.to("/api/settings")[1]?.body).toEqual({ todoPark: 2, until: new Date(2026, 11, 1).toISOString() });
  });

  it("brings the task back with the server's reason when refused, for a while", async () => {
    const { store, applied } = setup(() => json(400, { ok: false, error: "no such task" }));
    store.getState().decide(5, "No");
    expect(store.getState().toast).toMatchObject({ text: "No: task closed", warn: true });
    await vi.advanceTimersByTimeAsync(undoMs);
    expect(store.getState().gone.has(5)).toBe(false);
    expect(store.getState().toast).toMatchObject({ failed: true, text: "Answer not sent: no such task" });
    expect(applied).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(failedMs);
    expect(store.getState().toast).toBeNull();
  });

  it("words each answer plainly", () => {
    const { store } = setup();
    const said = (choice: string, task: number) => {
      store.getState().decide(task, choice);
      return store.getState().toast?.text;
    };
    expect(said("Done", 1)).toBe("Marked done");
    expect(said("Not now", 2)).toBe("Not now: task closed");
    expect(said("Leave it", 3)).toBe("Leave it: task closed");
    expect(said("Ship the beta", 4)).toBe("Answered: Ship the beta");
  });
});
