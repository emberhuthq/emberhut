import { describe, expect, it } from "vitest";
import { ApiError, type Approval } from "@/api";
import { queryKeys } from "@/state/query";
import { afterAnswer, canWithdraw, decideOn, OneAtATime, stillAsked } from "./decision";

const approval = (fields: Partial<Approval> = {}): Approval => ({
  id: 7,
  agent: "client-a",
  kind: "payment",
  target: "example-hosting.test",
  summary: "The hosting bill",
  payloadHash: "ab".repeat(32),
  approver: "owner",
  waitsOnOwner: true,
  state: "pending",
  createdAt: "2026-11-27T10:00:00.000Z",
  ...fields,
});

const lists = [queryKeys.allApprovals, queryKeys.allStates];

describe("an answer on the request on screen", () => {
  it("carries the payload hash of exactly that request", () => {
    const onScreen = approval({ payloadHash: "cd".repeat(32) });
    expect(decideOn(onScreen, "approve")).toEqual({ kind: "decide", decision: "approve", payloadHash: "cd".repeat(32) });
    expect(decideOn(onScreen, "reject")).toEqual({ kind: "decide", decision: "reject", payloadHash: "cd".repeat(32) });
  });

  it("holds a question only while the request is still the one asked on, waiting on you", () => {
    const asked = approval();
    expect(stillAsked(asked, approval())).toBe(true);
    expect(stillAsked(asked, approval({ payloadHash: "ef".repeat(32) }))).toBe(false);
    expect(stillAsked(asked, approval({ state: "approved", waitsOnOwner: false }))).toBe(false);
    expect(stillAsked(asked, undefined)).toBe(false);
  });

  it("can be withdrawn while it waits, and once approved until used, never when imported", () => {
    expect(canWithdraw(approval())).toBe(true);
    expect(canWithdraw(approval({ state: "approved" }))).toBe(true);
    expect(canWithdraw(approval({ state: "approved", imported: true }))).toBe(false);
    expect(canWithdraw(approval({ state: "used" }))).toBe(false);
    expect(canWithdraw(approval({ state: "rejected" }))).toBe(false);
  });
});

describe("after an answer", () => {
  it("takes the server's copy and reads the lists and the states again", () => {
    const answered = approval({ state: "approved", waitsOnOwner: false });
    expect(afterAnswer(7, { ok: answered })).toEqual({ set: answered, remove: false, invalidate: lists });
  });

  it("reads the request again on a 409, so the page shows it as it is", () => {
    expect(afterAnswer(7, { error: new ApiError(409, "changed") })).toEqual({
      remove: false,
      invalidate: [queryKeys.approval(7), ...lists],
    });
  });

  it("reads it again when the answer may have been lost on the way", () => {
    expect(afterAnswer(7, { error: new ApiError(0, "Can't reach Emberhut") }).invalidate).toContainEqual(queryKeys.approval(7));
  });

  it("lets go of a request the server doesn't have, so its page says it's gone", () => {
    expect(afterAnswer(7, { error: new ApiError(404, "no such request") })).toEqual({ remove: true, invalidate: lists });
  });

  it("does nothing more for an error of this device's own", () => {
    expect(afterAnswer(7, { error: new Error("boom") })).toEqual({ remove: false, invalidate: [] });
  });
});

describe("one answer at a time", () => {
  it("refuses a second tap while one is out, and takes the next once it is back", () => {
    const turn = new OneAtATime();
    expect(turn.take()).toBe(true);
    expect(turn.held).toBe(true);
    expect(turn.take()).toBe(false);
    turn.release();
    expect(turn.take()).toBe(true);
  });
});
