import { describe, expect, it } from "vitest";
import type { Fact, FactsPage, TrialRule } from "@/api";
import { learningFold } from "./learningFold";
import {
  CantKeepEarly,
  cantKeepEarly,
  categoryChips,
  changeProblem,
  decideBy,
  factGroups,
  groupsShown,
  lessonText,
  lessonsToReview,
  stillOnTrial,
  toConfirm,
  trialsShown,
  whyIKnow,
  withChange,
} from "./facts";

function fact(id: string, category: string, fields: Partial<Fact> = {}): Fact {
  return { id, category, title: id, text: `About ${id}`, source: "chat", updated: "2026-11-20T10:00:00Z", ...fields };
}

function trial(id: number, fields: Partial<TrialRule> = {}): TrialRule {
  return { id, text: "Reply in the owner's language.", at: "2026-11-10T10:00:00Z", ends: "2026-11-24T10:00:00Z", said: "", ...fields };
}

describe("What your ember knows about you", () => {
  it("groups facts in the page's category order, Other last, leaves evidence out, and searches titles and texts", () => {
    const facts = [
      fact("gym", "health"),
      fact("sara", "people"),
      fact("car", "vehicles"),
      fact("kept", "proof"),
      fact("rent", "money", { text: "Paid on the 1st" }),
      fact("mornings", "preferences"),
    ];
    expect(factGroups(facts, "").map((g) => [g.name, g.facts.map((f) => f.id)])).toEqual([
      ["People", ["sara"]],
      ["Money", ["rent"]],
      ["Preferences", ["mornings"]],
      ["Health & habits", ["gym"]],
      ["Other", ["car"]],
    ]);
    expect(factGroups(facts, "PAID").map((g) => g.key)).toEqual(["money"]);
  });

  it("has a chip for All and each category with facts, and a chip narrows the list to its category", () => {
    const facts = [fact("sara", "people"), fact("omar", "people"), fact("desk", "work", { text: "At home" })];
    expect(categoryChips(facts, "")).toEqual([
      { key: null, label: "All", count: 3 },
      { key: "people", label: "People", count: 2 },
      { key: "work", label: "Work", count: 1 },
    ]);
    expect(categoryChips(facts, "home")).toEqual([
      { key: null, label: "All", count: 1 },
      { key: "work", label: "Work", count: 1 },
    ]);
    const groups = factGroups(facts, "");
    expect(groupsShown(groups, "work").map((g) => g.key)).toEqual(["work"]);
    expect(groupsShown(groups, null).map((g) => g.key)).toEqual(["people", "work"]);
    expect(
      groupsShown(factGroups(facts, "home"), "people").map((g) => g.key),
      "a chip the search emptied reads as All",
    ).toEqual(["work"]);
  });

  it("words a lesson to you, finds it by what you said, and decides by the earliest end", () => {
    const worded = trial(4, { said: "Which file?", lesson: "I say which file a change is in.", ends: "2026-12-04T10:00:00Z" });
    const plain = trial(5, { said: "Answer in English" });
    expect(lessonText(worded)).toBe("I say which file a change is in.");
    expect(lessonText(plain), "no line of its own: the rule").toBe("Reply in the owner's language.");
    expect(lessonText(trial(6, { lesson: "  " }))).toBe("Reply in the owner's language.");
    expect(trialsShown([worded, plain], "english").map((t) => t.id)).toEqual([5]);
    expect(trialsShown([worded, plain], "which file").map((t) => t.id)).toEqual([4]);
    expect(decideBy([worded, plain])).toBe("2026-11-24T10:00:00Z");
    expect(decideBy([])).toBeNull();
    expect([lessonsToReview(1), lessonsToReview(2)]).toEqual(["1 lesson to review", "2 lessons to review"]);
  });

  it("shows a fix, a forget, a wake switch, a kept and a dropped lesson before the server answers", () => {
    const page: FactsPage = {
      facts: [fact("people/sara", "people"), fact("work/role", "work")],
      trials: [trial(4), trial(5)],
    };
    const fixed = withChange(page, { factSet: { id: "work/role", category: "work", title: "role", text: "Founder" } });
    expect(fixed.facts.find((f) => f.id === "work/role")).toMatchObject({ text: "Founder", source: "you corrected it" });
    expect(withChange(page, { personSet: { id: "people/sara", important: true } }).facts[0]?.important).toBe(true);
    expect(withChange(page, { factDrop: "people/sara" }).facts.map((f) => f.id)).toEqual(["work/role"]);
    expect(withChange(page, { trialDrop: 4 }).trials?.map((t) => t.id)).toEqual([5]);
    expect(withChange(page, { trialKeep: 5 }).trials?.map((t) => t.id)).toEqual([4]);
  });

  it("says plainly when the server can't keep a lesson early", () => {
    const page: FactsPage = { facts: [], trials: [trial(4)] };
    expect(stillOnTrial(page, 4), "an older server left it on trial").toBe(true);
    expect(stillOnTrial({ facts: [] }, 4)).toBe(false);
    expect(changeProblem(new CantKeepEarly())).toBe(cantKeepEarly);
    expect(changeProblem(new Error("no such fact"))).toBe("Not saved: no such fact");
  });

  it("keeps Learning from you folded until opened, then as left", () => {
    expect(learningFold.getState().open).toBe(false);
    learningFold.getState().toggle();
    expect(learningFold.getState().open).toBe(true);
    learningFold.getState().toggle();
    expect(learningFold.getState().open).toBe(false);
  });

  it("lists the facts that wait for you apart, and says why your ember knows each", () => {
    const facts = [
      fact("sara", "people", { origin: "assistant", from: "chat #12" }),
      fact("orders", "preferences", { pending: true, origin: "message", from: "chat #40" }),
      fact("kept", "proof", { pending: true }),
    ];
    expect(factGroups(facts, "").flatMap((g) => g.facts.map((f) => f.id))).toEqual(["sara"]);
    expect(toConfirm(facts, "").map((f) => f.id)).toEqual(["orders"]);
    expect(toConfirm(facts, "nothing like it")).toEqual([]);
    expect(whyIKnow(facts[0]!)).toBe("You told your ember · chat #12");
    expect(whyIKnow(facts[1]!)).toBe("From someone else's message · chat #40");
    expect(whyIKnow(fact("old", "home"))).toBeNull();
  });

  it("shows a confirm, a pin and a forget everywhere at once", () => {
    const page: FactsPage = {
      facts: [fact("orders", "preferences", { pending: true }), fact("sara", "people"), fact("rent", "money")],
    };
    const confirmed = withChange(page, { factConfirm: "orders" });
    expect(confirmed.facts.find((f) => f.id === "orders")?.pending).toBe(false);
    const pinned = withChange(page, { factProfile: { id: "sara", profile: "pin" } });
    expect(pinned.facts.find((f) => f.id === "sara")?.profile).toBe("pin");
    expect(withChange(pinned, { factProfile: { id: "sara", profile: null } }).facts.find((f) => f.id === "sara")?.profile).toBeUndefined();
    expect(withChange(page, { factForget: { id: "rent", pieces: [3] } }).facts.map((f) => f.id)).toEqual(["orders", "sara"]);
  });
});
