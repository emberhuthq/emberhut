import { describe, expect, it } from "vitest";
import type { ThemePage } from "@/api";
import { builtInTheme } from "@/theme/builtIn";
import { lookCards, lookChange, shownOf, tintChange, tintChoices, tintOf } from "./look";

const paper = builtInTheme.presets[0]!;

/** The built-in page with every preset `ids` names (paper's colours, the accent its own), choosing `preset`. */
function page(ids: string[], choice: Partial<ThemePage["choice"]> = {}): ThemePage {
  const presets = ids.map((id, n) => ({
    ...paper,
    id,
    name: id[0]!.toUpperCase() + id.slice(1),
    light: { ...paper.light, accent: `#00000${n}` },
  }));
  return { ...builtInTheme, presets, choice: { ...builtInTheme.choice, ...choice } };
}

describe("your ember's look", () => {
  it("draws a card for every preset the server lists, in its order, one it has never heard of too", () => {
    const cards = lookCards(page(["paper", "graphite", "teal", "calm"]));
    expect(cards.map((c) => c.id)).toEqual(["paper", "graphite", "teal", "calm"]);
    expect(cards[3]?.name).toBe("Calm");
    expect(cards[2]?.light).toMatchObject({
      paper: paper.light.background,
      sheet: paper.light.surface,
      ink: paper.light.text,
      accent: "#000002",
    });
    expect(cards[2]?.dark).toMatchObject({ paper: paper.dark.background, sheet: paper.dark.surface, accent: paper.dark.accent });
    expect(cards.filter((c) => c.isDefault).map((c) => c.id)).toEqual(["paper"]);
  });

  it("ticks the preset in use, and none under a custom colour", () => {
    const ticked = (p: ThemePage) =>
      lookCards(p)
        .filter((c) => c.selected)
        .map((c) => c.id);
    expect(ticked(page(["paper", "teal"], { preset: "teal" }))).toEqual(["teal"]);
    expect(ticked(page(["paper", "teal"], { preset: "teal", accent: "#3366FF" }))).toEqual([]);
  });

  it("picks a preset in its own colour, as Appearance does, and a tint alone", () => {
    expect(lookChange("teal")).toEqual({ preset: "teal", accent: null });
    expect(tintChange("coral")).toEqual({ emberTint: "coral" });
  });

  it("lists the server's tints, the one in use marked, and none from a server before tints", () => {
    const tints = tintChoices(page(["paper"], { emberTint: "blue" }));
    expect(tints.map((t) => t.id)).toEqual(["ember", "amber", "coral", "blue"]);
    expect(tints.filter((t) => t.selected).map((t) => t.id)).toEqual(["blue"]);
    expect(tints.find((t) => t.isDefault)?.id).toBe("ember");
    const early = tintChoices(page(["paper"], { emberTint: "rose" }));
    expect(early.filter((t) => t.selected).map((t) => t.id)).toEqual(["ember"]);
    const { tints: _gone, ...older } = page(["paper"]);
    expect(tintChoices(older)).toEqual([]);
  });

  it("draws the pages in the appearance chosen, and both for System", () => {
    expect(shownOf("light")).toBe("light");
    expect(shownOf("dark")).toBe("dark");
    expect(shownOf("system")).toBe("both");
  });

  it("draws the ember in its own colours until a tint is chosen", () => {
    expect(tintOf(undefined)).toBe("ember");
    expect(tintOf({})).toBe("ember");
    expect(tintOf({ emberTint: "amber" })).toBe("amber");
  });
});
