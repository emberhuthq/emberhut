import { describe, expect, it } from "vitest";
import type { Pending } from "@/state/talk";
import { NEAR_NEWEST, fromNewest, isAway, isNewSend, newestOffset } from "./pinning";

const pending = (extra: Partial<Pending> = {}): Pending => ({
  text: "hi",
  files: [],
  reply: null,
  after: 4,
  note: null,
  failed: false,
  ...extra,
});

describe("the newest message's offset", () => {
  it("is before 0 by the keyboard's inset on iOS, and 0 on Android whether the keyboard is up or not", () => {
    expect(newestOffset("ios", 0)).toBe(0);
    expect(newestOffset("ios", 291)).toBe(-291);
    expect(newestOffset("android", 0)).toBe(0);
    expect(newestOffset("android", 291)).toBe(0);
  });

  it("measures how far up the list sits from it", () => {
    expect(fromNewest("ios", -291, 291)).toBe(0);
    expect(fromNewest("ios", 200, 291)).toBe(491);
    expect(fromNewest("android", 0, 291)).toBe(0);
    expect(fromNewest("android", 200, 291)).toBe(200);
  });
});

describe("away", () => {
  it("is not at the newest with the keyboard up, on either platform", () => {
    // On Android the keyboard's inset is reported while the newest sits at 0: added to the offset, it read as away.
    expect(isAway(fromNewest("android", 0, 291))).toBe(false);
    expect(isAway(fromNewest("ios", -291, 291))).toBe(false);
  });

  it("is further up than the margin, and not within it", () => {
    expect(isAway(fromNewest("android", NEAR_NEWEST, 0))).toBe(false);
    expect(isAway(fromNewest("android", NEAR_NEWEST + 1, 0))).toBe(true);
    expect(isAway(fromNewest("ios", -291 + NEAR_NEWEST + 1, 291))).toBe(true);
    expect(isAway(fromNewest("ios", 40, 0))).toBe(false);
  });
});

describe("a new send", () => {
  it("is a fresh pending message, the first or after another", () => {
    const first = pending();
    expect(isNewSend(undefined, first)).toBe(true);
    expect(isNewSend(first, pending({ text: "again", after: 5 }))).toBe(true);
  });

  it("is not the same one, its note or failure later, or its end", () => {
    const sent = pending();
    expect(isNewSend(sent, sent)).toBe(false);
    expect(isNewSend(sent, { ...sent, note: "Not sent: can't reach Emberhut", failed: true })).toBe(false);
    expect(isNewSend(sent, { ...sent, note: "Taking a while" })).toBe(false);
    expect(isNewSend(sent, undefined)).toBe(false);
    expect(isNewSend(undefined, undefined)).toBe(false);
  });

  it("is a failed one sent again", () => {
    const failed = pending({ note: "Not sent: can't reach Emberhut", failed: true });
    expect(isNewSend(failed, pending())).toBe(true);
  });
});
