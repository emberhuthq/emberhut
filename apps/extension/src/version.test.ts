import { describe, expect, it } from "vitest";
import { isTried, newer, shouldReload } from "./version";

describe("versions and the reload", () => {
  it("compares part by part", () => {
    expect(newer("0.6.10", "0.6.4")).toBe(true);
    expect(newer("1.0", "1.0.0")).toBe(false);
    expect(newer("2", "1.9")).toBe(true);
    expect(newer("0.1.0", "0.2.0")).toBe(false);
  });

  it("reloads for a newer folder, never while it holds a tab", () => {
    expect(shouldReload("0.2.0", "0.1.0", 0, null)).toBe(true);
    expect(shouldReload("0.2.0", "0.1.0", 1, null)).toBe(false);
    expect(shouldReload("0.1.0", "0.1.0", 0, null)).toBe(false);
    expect(shouldReload(null, "0.1.0", 0, null)).toBe(false);
  });

  it("never tries a version twice: a reload that came back still older doesn't loop", () => {
    const tried = { version: "0.2.0", at: 1_000 };
    // Reloaded for 0.2.0 and still running 0.1.0 (another folder): never again for 0.2.0, nor for an older one.
    expect(shouldReload("0.2.0", "0.1.0", 0, tried)).toBe(false);
    expect(shouldReload("0.1.5", "0.1.0", 0, tried)).toBe(false);
    // A newer version gets its one try.
    expect(shouldReload("0.3.0", "0.1.0", 0, tried)).toBe(true);
    expect(isTried(tried)).toBe(true);
    expect(isTried({ version: 1 })).toBe(false);
  });
});
