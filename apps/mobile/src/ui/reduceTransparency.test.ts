import { describe, expect, it, vi } from "vitest";
import { createReduceTransparencyStore, transparencyWaitMs, type TransparencySource } from "./reduceTransparency";

function source(first: Promise<boolean>) {
  let listener: ((reduce: boolean) => void) | null = null;
  let reads = 0;
  const s: TransparencySource = {
    read: () => {
      reads += 1;
      return first;
    },
    listen: (l) => {
      listener = l;
      return () => (listener = null);
    },
  };
  return { s, change: (on: boolean) => listener?.(on), reads: () => reads };
}

const settle = () => new Promise((resolve) => setTimeout(resolve, 0));

describe("reduce transparency", () => {
  it("starts unknown, then holds the system's real value, read once", async () => {
    const { s, reads } = source(Promise.resolve(true));
    const store = createReduceTransparencyStore(s);
    expect(store.getState()).toEqual({ reduce: false, known: false });
    await settle();
    expect(store.getState()).toEqual({ reduce: true, known: true });
    expect(reads()).toBe(1);
  });

  it("follows a change, and a change beats a read still out", async () => {
    let answer: (on: boolean) => void = () => undefined;
    const { s, change } = source(new Promise<boolean>((resolve) => (answer = resolve)));
    const store = createReduceTransparencyStore(s);
    change(true);
    answer(false);
    await settle();
    expect(store.getState()).toEqual({ reduce: true, known: true });
    change(false);
    expect(store.getState().reduce).toBe(false);
  });

  it("counts a failed read as known and off", async () => {
    const { s } = source(Promise.reject(new Error("no accessibility service")));
    const store = createReduceTransparencyStore(s);
    await settle();
    expect(store.getState()).toEqual({ reduce: false, known: true });
  });

  it("draws as if off when the read never answers, and still takes a later change", () => {
    vi.useFakeTimers();
    try {
      const { s, change } = source(new Promise<boolean>(() => undefined));
      const store = createReduceTransparencyStore(s);
      vi.advanceTimersByTime(transparencyWaitMs - 1);
      expect(store.getState().known).toBe(false);
      vi.advanceTimersByTime(1);
      expect(store.getState()).toEqual({ reduce: false, known: true });
      change(true);
      expect(store.getState()).toEqual({ reduce: true, known: true });
    } finally {
      vi.useRealTimers();
    }
  });
});
