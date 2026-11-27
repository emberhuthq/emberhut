import { describe, expect, it } from "vitest";
import { taskKey } from "@/api";
import { refTarget, refText, shownRefs } from "./refs";

describe("refs", () => {
  it("reads a chip short", () => {
    expect(refText({ kind: "task", label: taskKey(3744), title: "مقهى: why the weekend menu sold out (post-mortem)", task: 3744 })).toBe(
      `${taskKey(3744)} · مقهى: why the weekend menu sold o…`,
    );
    expect(refText({ kind: "pr", label: "PR #412", title: "acme/garden" })).toBe("PR #412 · acme/garden");
    expect(refText({ kind: "card", label: "CC card", title: "bc349c528b" })).toBe("CC card");
  });

  it("opens each kind where it lives", () => {
    expect(refTarget({ kind: "task", label: taskKey(1), task: 1 })).toEqual({ open: "task", id: 1 });
    expect(refTarget({ kind: "report", label: "Report", url: "/api/artifacts/8218", name: "pm.html" })).toEqual({
      open: "file",
      path: "/api/artifacts/8218",
      name: "pm.html",
    });
    expect(refTarget({ kind: "pr", label: "PR #4", url: "https://github.com/a/b/pull/4" })).toEqual({
      open: "link",
      url: "https://github.com/a/b/pull/4",
    });
    expect(refTarget({ kind: "ticket", label: "CAL-1", url: "http://insecure" })).toEqual({ open: "none" });
    // An address that reads as another server is never fetched with the session.
    expect(refTarget({ kind: "report", label: "Report", url: "//evil.example/r.html" })).toEqual({ open: "none" });
    expect(refTarget({ kind: "file", label: "File", url: "/\\evil.example/r.html" })).toEqual({ open: "none" });
  });
});

describe("shownRefs", () => {
  const seven = ["a", "b", "c", "d", "e", "f", "g"];
  it("shows the first three and how many wait, until expanded", () => {
    expect(shownRefs(seven, false)).toEqual({ shown: ["a", "b", "c"], more: 4 });
    expect(shownRefs(seven, true)).toEqual({ shown: seven, more: 0 });
  });
  it("never hides a single chip behind +1", () => {
    expect(shownRefs(["a", "b", "c", "d"], false)).toEqual({ shown: ["a", "b", "c", "d"], more: 0 });
    expect(shownRefs(["a"], false)).toEqual({ shown: ["a"], more: 0 });
  });
});
