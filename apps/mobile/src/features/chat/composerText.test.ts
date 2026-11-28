import { describe, expect, it } from "vitest";
import { minTap } from "@/theme/tokens";
import { appendSaid, sendAction, setDraft, slopFor } from "./composerText";

describe("composer", () => {
  it("keeps each room's draft: typing in one leaves the others as they were", () => {
    let drafts = setDraft({}, 1, "half a thought for Main");
    drafts = setDraft(drafts, 4, "for the project");
    drafts = setDraft(drafts, 4, (typed) => `${typed}, more`);
    expect(drafts[1]).toBe("half a thought for Main");
    expect(drafts[4]).toBe("for the project, more");
    expect(drafts[7]).toBeUndefined();
    drafts = setDraft(drafts, 1, "");
    expect(drafts).toEqual({ 4: "for the project, more" });
  });

  it("puts dictated words after what's typed", () => {
    expect(appendSaid("", "hello")).toBe("hello");
    expect(appendSaid("so", "hello")).toBe("so hello");
    expect(appendSaid("so ", "hello")).toBe("so hello");
  });

  it("sends, stops or waits", () => {
    expect(sendAction({ text: " ", files: 0, sending: false, working: false })).toBe("none");
    expect(sendAction({ text: " ", files: 0, sending: false, working: true })).toBe("stop");
    expect(sendAction({ text: "hi", files: 0, sending: false, working: true })).toBe("send");
    expect(sendAction({ text: "", files: 1, sending: false, working: false })).toBe("send");
    expect(sendAction({ text: "hi", files: 1, sending: true, working: false })).toBe("wait");
  });

  it("gives every button at least the minimum touch area", () => {
    for (const size of [18, 20, 32, 44, 60]) expect(size + 2 * slopFor(size)).toBeGreaterThanOrEqual(minTap);
    expect(slopFor(32)).toBe(6);
    expect(slopFor(60)).toBe(0);
  });
});
