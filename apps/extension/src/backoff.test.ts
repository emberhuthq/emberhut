import { describe, expect, it } from "vitest";
import { fromBeforeSleep, isStale, retryAt, slept } from "./backoff";

describe("the socket's back-off", () => {
  it("waits longer after each failure, up to 2 minutes, and not at all once in", () => {
    expect(retryAt(0, 1_000)).toBe(0);
    expect(retryAt(1, 0)).toBe(20_000);
    expect(retryAt(2, 0)).toBe(40_000);
    expect(retryAt(3, 0)).toBe(80_000);
    expect(retryAt(9, 0)).toBe(120_000);
  });

  it("drops a socket stuck connecting, or one whose answered pings stopped", () => {
    expect(isStale({ open: false, since: 0, answers: false, missed: 0 }, 61_000)).toBe(true);
    expect(isStale({ open: false, since: 0, answers: false, missed: 0 }, 30_000)).toBe(false);
    expect(isStale({ open: true, since: 0, answers: true, missed: 3 }, 1)).toBe(true);
    expect(isStale({ open: true, since: 0, answers: true, missed: 2 }, 1)).toBe(false);
    expect(isStale({ open: true, since: 0, answers: false, missed: 9 }, 1)).toBe(false);
  });

  it("sees a sleep by a late tick, and drops only sockets from before it", () => {
    expect(slept(0, 61_000)).toBe(true);
    expect(slept(0, 21_000)).toBe(false);
    expect(fromBeforeSleep(0, 25_000)).toBe(true);
    expect(fromBeforeSleep(10_000, 25_000)).toBe(false);
  });
});
