import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { entries, groupName, groups, groupStatus, nothingKnown, pathOf } from "./entries";
import { search, suggestions } from "./find";

/** The list both apps check theirs against. */
const shared = JSON.parse(readFileSync(fileURLToPath(new URL("../../../../fixtures/settings-entries.json", import.meta.url)), "utf8")) as {
  groups: string[];
  entries: { id: string }[];
};

const first = (query: string) => search(query, nothingKnown).rows[0]?.entry.id;
const titles = (now: typeof nothingKnown) => suggestions(now).map((row) => row.title);

describe("the settings' list", () => {
  it("has the desktop's ids and groups, in its order", () => {
    expect(entries.map((e) => e.id)).toEqual(shared.entries.map((e) => e.id));
    expect(groups.map((g) => groupName[g])).toEqual(shared.groups);
  });

  it("says where every setting lives", () => {
    for (const entry of entries) expect(pathOf(entry).startsWith(groupName[entry.group])).toBe(true);
    expect(new Set(entries.map((e) => e.id)).size).toBe(entries.length);
  });
});

describe("Your ember's status line", () => {
  it("says what it is doing, not that it is on duty", () => {
    const on = { ...nothingKnown, onDuty: true };
    expect(groupStatus("ember", on)).toBe("Ready");
    expect(groupStatus("ember", { ...on, working: true })).toBe("Working on 1 thing");
    expect(groupStatus("ember", { ...on, jobs: 2 })).toBe("Working on 2 things");
    expect(groupStatus("ember", nothingKnown)).toBe("Off duty");
  });
});

describe("search", () => {
  it("finds a setting by its other names", () => {
    for (const [query, id] of [
      ["MCP", "tools"],
      ["model", "thinks-with"],
      ["pause", "on-duty"],
      ["theme", "appearance-mode"],
      ["dark mode", "appearance-mode"],
      ["QR", "pair"],
      ["forget", "about-you"],
      ["memory", "about-you"],
      ["hide agents", "show-agents"],
      ["agents below rooms", "agents-place"],
    ] as const) {
      expect(first(query), query).toBe(id);
    }
  });

  it("ranks title matches first and marks them", () => {
    const { rows } = search("dar", nothingKnown);
    expect(rows.slice(0, 2).map((r) => r.entry.id)).toEqual(["appearance-mode", "dark-room"]);
    expect(rows[0]?.marks).toEqual([[0, 3]]);
    expect(rows[1]?.icon).not.toBe(rows[0]?.icon);
    expect(first("ROOMS archived")).toBe("archived-rooms");
  });

  it("offers the closest few when nothing matches", () => {
    const found = search("darkk", nothingKnown);
    expect(found.nothing).toBe(true);
    expect(found.rows[0]?.entry.id).toBe("appearance-mode");
    expect(search("zzzz", nothingKnown).rows).toEqual([]);
  });

  it("says how the agents sit in the rooms list now", () => {
    const line = (query: string, now: typeof nothingKnown) => search(query, now).rows[0]?.line;
    expect(line("hide agents", nothingKnown)).toBe("Your agents' rooms, in the rooms list");
    expect(line("hide agents", { ...nothingKnown, showAgents: false })).toBe("Hidden · open them from here");
    expect(line("agents below rooms", nothingKnown)).toBe("Below rooms, after paused ones");
    expect(line("agents below rooms", { ...nothingKnown, agentsPlace: "above" })).toBe("Before paused rooms");
  });

  it("finds the account only in builds with it", () => {
    expect(search("account", nothingKnown).rows.map((r) => r.entry.id)).not.toContain("account");
    expect(first("account")).not.toBe("account");
    expect(search("account", { ...nothingKnown, hasAccount: true }).rows[0]?.entry.id).toBe("account");
  });
});

describe("suggestions", () => {
  it("put lessons waiting first", () => {
    expect(suggestions({ ...nothingKnown, lessons: 3 })[0]?.title).toBe("Review 3 lessons");
    expect(titles(nothingKnown).some((t) => t.startsWith("Review"))).toBe(false);
  });

  it("suggest what needs a look, and never what is fine", () => {
    expect(titles({ ...nothingKnown, sourcesOn: 1 }).some((t) => t.startsWith("Fix") || t.startsWith("Reconnect"))).toBe(false);
    expect(titles({ ...nothingKnown, failingTools: ["linear"], sourcesToFix: ["Gmail"] }).slice(0, 2)).toEqual([
      "Reconnect linear",
      "Fix Gmail",
    ]);
  });

  it("offer the opposite of what shows, and pairing by what is paired", () => {
    expect(titles({ ...nothingKnown, dark: true, onDuty: true })).toEqual(expect.arrayContaining(["Switch to light", "Go off duty"]));
    expect(titles(nothingKnown)).toEqual(expect.arrayContaining(["Switch to dark", "Go on duty"]));
    expect(titles({ ...nothingKnown, signedIn: true })).toContain("Pair a computer");
    expect(titles({ ...nothingKnown, signedIn: true, otherDevices: true })).toContain("Add a phone or a hut");
  });

  it("show the sun for Switch to light, the moon for Switch to dark", () => {
    const icon = (now: typeof nothingKnown, title: string) => suggestions(now).find((row) => row.title === title)?.icon;
    expect(icon({ ...nothingKnown, dark: true }, "Switch to light")).toBe("sunny-outline");
    expect(icon(nothingKnown, "Switch to dark")).toBe("moon-outline");
  });

  it("are never more than seven", () => {
    const busy = { ...nothingKnown, lessons: 2, failingTools: ["a"], sourcesToFix: ["b"], signedIn: true };
    expect(suggestions(busy)).toHaveLength(7);
  });
});
