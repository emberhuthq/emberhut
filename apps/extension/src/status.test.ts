import { describe, expect, it } from "vitest";
import { refusal, type Answer } from "./status";

describe("the worker's answers", () => {
  it("tells a refusal from a status whose link has an error of its own", () => {
    expect(refusal({ refused: "That pairing offer is gone" })).toBe("That pairing offer is gone");
    const status = { paired: false, error: "", version: "0.1.0" } as unknown as Answer;
    expect(refusal(status)).toBeNull();
    expect(refusal({ ...status, error: "Can't reach the server" })).toBeNull();
  });
});
