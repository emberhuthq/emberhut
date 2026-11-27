import { afterAll, afterEach, describe, expect, it } from "vitest";
import { inBackground, logError, setLogSink } from ".";

const logged: [string, unknown][] = [];
const before = setLogSink((where, cause) => logged.push([where, cause]));
afterEach(() => void logged.splice(0));
afterAll(() => void setLogSink(before));

describe("log", () => {
  it("writes down where a failure happened", () => {
    logError("push", "no channel");
    expect(logged).toEqual([["push", "no channel"]]);
  });

  it("catches what background work rejects with, and leaves what it resolves alone", async () => {
    const cause = new Error("picker failed");
    inBackground(Promise.reject(cause), "attach");
    inBackground(Promise.resolve(1), "fine");
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(logged).toEqual([["attach", cause]]);
  });
});
