import { describe, expect, it } from "vitest";
import { PAUSE_MS, isPause, newPause, pauseLength, pausedNow } from "./pause";

describe("the pause", () => {
  it("lasts until its time, then is over by itself", () => {
    const pause = newPause("you pressed Pause", PAUSE_MS, 1_000, false, null);
    expect(pause).toEqual({ why: "you pressed Pause", until: 1_000 + PAUSE_MS, turn: false });
    expect(pausedNow(pause, 2_000)).toBe(pause);
    expect(pausedNow(pause, 1_000 + PAUSE_MS)).toBeNull();
    expect(pausedNow(null, 0)).toBeNull();
  });

  it("remembers it was pressed during the owner's turn", () => {
    const during = newPause("why", PAUSE_MS, 0, true, null);
    expect(during.turn).toBe(true);
    expect(newPause("again", PAUSE_MS, 5, false, during).turn).toBe(true);
  });

  it("takes 1 to 10 minutes from the app, 10 by default", () => {
    expect(pauseLength(5)).toBe(5 * 60_000);
    expect(pauseLength(undefined)).toBe(PAUSE_MS);
    expect(pauseLength(0)).toBe(PAUSE_MS);
    expect(pauseLength(60)).toBe(PAUSE_MS);
    expect(pauseLength(0.5)).toBe(60_000);
    expect(pauseLength("x")).toBe(PAUSE_MS);
  });

  it("reads a stored pause", () => {
    expect(isPause({ why: "w", until: 1, turn: false })).toBe(true);
    expect(isPause({ why: "w", until: "1" })).toBe(false);
    expect(isPause(null)).toBe(false);
  });
});
