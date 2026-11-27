import { afterAll, afterEach, describe, expect, it } from "vitest";
import { inBackground, logError, messageOf, setLogSink } from "./log";

const logged: [string, unknown][] = [];
const before = setLogSink((where, cause) => logged.push([where, cause]));
afterEach(() => void logged.splice(0));
afterAll(() => void setLogSink(before));

describe("log", () => {
  it("writes down where a failure happened", () => {
    logError("socket", "closed");
    expect(logged).toEqual([["socket", "closed"]]);
  });

  it("catches what background work rejects with", async () => {
    const cause = new Error("no tab");
    inBackground(Promise.reject(cause), "sync");
    inBackground(Promise.resolve(1), "fine");
    await new Promise((done) => setTimeout(done, 0));
    expect(logged).toEqual([["sync", cause]]);
    expect(messageOf(cause)).toBe("no tab");
    expect(messageOf("plain")).toBe("plain");
  });
});
