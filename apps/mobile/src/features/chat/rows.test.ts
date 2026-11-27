import { describe, expect, it } from "vitest";
import type { ChatMsg } from "@/api";
import { buildRows, eventLines, eventSummary, quoteOf } from "./rows";

const at = (minutes: number) => new Date(Date.UTC(2026, 10, 24, 9, minutes)).toISOString();
const m = (id: number, minutes: number | null, extra: Partial<ChatMsg> = {}): ChatMsg => ({
  id,
  mine: false,
  text: `m${id}`,
  time: "",
  at: minutes === null ? null : at(minutes),
  ...extra,
});

describe("buildRows", () => {
  it("folds a run of events into one row keyed by its first", () => {
    const rows = buildRows([m(1, 0), m(2, 1, { event: true, text: "a" }), m(3, 2, { event: true, text: "b" }), m(4, 3)]);
    expect(rows.map((r) => r.key)).toEqual(["1", "2", "4"]);
    expect(rows[1]).toMatchObject({ kind: "events", text: "a\nb", ids: [2, 3] });
  });

  it("stamps the first row and any after a 15-minute gap", () => {
    const rows = buildRows([m(1, 0), m(2, 10), m(3, 26), m(4, null), m(5, 30)]);
    expect(rows.map((r) => r.stamp !== null)).toEqual([true, false, true, false, false]);
  });
});

describe("events", () => {
  it("reads the server's lines and counts their kinds", () => {
    const lines = eventLines("- [checkin] look at the build\n- [note] a\nnot a line\n- [note] b\n- [new_mail] c\n- [body] d\n- [body] e");
    expect(lines).toHaveLength(6);
    expect(eventSummary(lines)).toBe("1 check-in · 2 notes · 1 new mail · 2 bodies");
  });
});

describe("quoteOf", () => {
  it("keeps a short preview and the first picture", () => {
    const pic = { name: "a.png", url: "/uploads/a.png", kind: "image" as const };
    const q = quoteOf(m(9, 0, { text: "x".repeat(300), mine: true, files: [{ name: "f.pdf", url: "/u/f.pdf", kind: "file" }, pic] }));
    expect(q).toMatchObject({ id: 9, mine: true, file: pic });
    expect(q.text).toHaveLength(160);
  });
});
