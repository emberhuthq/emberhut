import { describe, expect, it } from "vitest";
import type { Fact, FactsPage } from "@/api";
import { factGroups, trialsShown, withChange } from "./facts";

function fact(id: string, category: string, fields: Partial<Fact> = {}): Fact {
  return { id, category, title: id, text: `About ${id}`, source: "chat", updated: "2026-11-20T10:00:00Z", ...fields };
}

describe("About you", () => {
  it("groups facts in the reference client's category order, any other category after, and searches titles and texts", () => {
    const facts = [
      fact("gym", "health"),
      fact("sara", "people"),
      fact("car", "vehicles"),
      fact("rent", "money", { text: "Paid on the 1st" }),
    ];
    expect(factGroups(facts, "").map((g) => [g.name, g.facts.map((f) => f.id)])).toEqual([
      ["People", ["sara"]],
      ["Money", ["rent"]],
      ["Health & habits", ["gym"]],
      ["Vehicles", ["car"]],
    ]);
    expect(factGroups(facts, "PAID").map((g) => g.key)).toEqual(["money"]);
    expect(trialsShown([{ id: 1, text: "Reply in English", at: "", ends: "", said: "" }], "arabic")).toEqual([]);
  });

  it("shows a fix, a forget, a wake switch and a dropped rule before the server answers", () => {
    const page: FactsPage = {
      facts: [fact("people/sara", "people"), fact("work/role", "work")],
      trials: [{ id: 4, text: "Shorter answers", at: "", ends: "", said: "" }],
    };
    const fixed = withChange(page, { factSet: { id: "work/role", category: "work", title: "role", text: "Founder" } });
    expect(fixed.facts.find((f) => f.id === "work/role")).toMatchObject({ text: "Founder", source: "you corrected it" });
    expect(withChange(page, { personSet: { id: "people/sara", important: true } }).facts[0]?.important).toBe(true);
    expect(withChange(page, { factDrop: "people/sara" }).facts.map((f) => f.id)).toEqual(["work/role"]);
    expect(withChange(page, { trialDrop: 4 }).trials).toEqual([]);
  });
});
