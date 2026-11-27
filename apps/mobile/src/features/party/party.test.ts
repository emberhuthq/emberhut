import { describe, expect, it } from "vitest";
import type { Party, PartyColor, PartyMessage, PartyModel, PartyPersona } from "@/api";
import { contrast, readableOn } from "@/theme/colors";
import {
  addPersona,
  applyPartyEvent,
  canStart,
  emptyParty,
  estimate,
  meter,
  modelName,
  notInCast,
  partyCost,
  partyTokens,
  personaColor,
  resumeLabel,
  savePersona,
  shortRole,
  startingCast,
  statusWord,
  stepTurns,
} from "./party";

const models: PartyModel[] = [
  { id: "claude-opus-5-5", name: "Opus 5.5", in: 4, out: 20, note: "Best talker" },
  { id: "claude-sonnet-5-5", name: "Sonnet 5.5", in: 2, out: 10, note: "Half the price" },
  { id: "claude-haiku-5-5", name: "Haiku 5.5", in: 0.1, out: 0.5, note: "Fast and cheap" },
];

const preset = (id: string, name: string, model = ""): PartyPersona => ({ id, name, role: `${name}. Who they are.`, model });

const presets = [
  preset("optimist", "Nora"),
  preset("skeptic", "Victor"),
  preset("engineer", "Sam"),
  preset("historian", "Lena"),
  preset("devil", "Omar"),
  preset("comedian", "Jules"),
];

const party = (extra: Partial<Party> = {}): Party => ({
  id: 3,
  title: "Four-day weeks",
  topic: "Should Emberhut's team work four days?",
  status: "running",
  why: null,
  personas: [
    { id: "p1", name: "Nora", role: "An optimist.", model: "claude-opus-5-5", color: "blue" },
    { id: "p2", name: "Victor", role: "A skeptic.", model: "claude-haiku-5-5", color: "orange" },
  ],
  turns: 5,
  maxTurns: 12,
  tokens: 23_400,
  maxTokens: 150_000,
  cost: 0.1234,
  created: "2026-11-25T10:00:00Z",
  updated: "2026-11-25T10:05:00Z",
  speaking: null,
  last: null,
  ...extra,
});

const line = (id: number, extra: Partial<PartyMessage> = {}): PartyMessage => ({
  id,
  party: 3,
  who: "p1",
  name: "Nora",
  kind: "turn",
  text: `Line ${id}`,
  model: "claude-opus-5-5",
  tokens: 300,
  cost: 0.01,
  cut: false,
  at: "2026-11-25T10:01:00Z",
  ...extra,
});

describe("the numbers", () => {
  it("writes tokens short", () => {
    expect(partyTokens(950)).toBe("950");
    expect(partyTokens(23_400)).toBe("23.4k");
    expect(partyTokens(150_000)).toBe("150k");
    expect(partyTokens(1_000_000)).toBe("1M");
    expect(partyTokens(1_250_000)).toBe("1.3M");
  });

  it("writes the cost in dollars, under a cent as <$0.01", () => {
    expect(partyCost(0)).toBe("$0.00");
    expect(partyCost(0.004)).toBe("<$0.01");
    expect(partyCost(0.1234)).toBe("$0.12");
    expect(partyCost(2.5)).toBe("$2.50");
  });

  it("says where a party stands", () => {
    expect(meter(party())).toBe("Turn 5 of 12 · 23.4k of 150k tokens · $0.12");
    expect(statusWord("running")).toBe("Talking");
    expect(statusWord("stopped")).toBe("Stopped");
    expect(resumeLabel("paused")).toBe("Resume");
    expect(resumeLabel("done")).toBe("Continue");
  });

  it("names models and colours personas", () => {
    expect(modelName(models, "claude-sonnet-5-5")).toBe("Sonnet 5.5");
    expect(modelName(models, "claude-next-6")).toBe("next-6");
    expect(personaColor("green", "light")).toBe("#34C759");
    expect(personaColor("green", "dark")).toBe("#30D158");
    expect(personaColor("", "light")).toBe("#8E8E93");
  });

  it("puts each persona's initial in white or near-black, whichever reads 4.5:1 on its colour", () => {
    const colors: (PartyColor | "")[] = ["blue", "orange", "green", "purple", "pink", ""];
    for (const scheme of ["light", "dark"] as const) {
      for (const color of colors) {
        const fill = personaColor(color, scheme);
        expect(contrast(readableOn(fill), fill), `${color} ${scheme}`).toBeGreaterThanOrEqual(4.5);
      }
    }
    expect(readableOn("#34C759")).toBe("#0B0D10");
    expect(readableOn("#5856D6")).toBe("#FFFFFF");
  });
});

describe("picking the cast", () => {
  it("starts with the optimist, the skeptic and the engineer, on Opus", () => {
    const cast = startingCast(presets);
    expect(cast.map((p) => p.name)).toEqual(["Nora", "Victor", "Sam"]);
    expect(cast.every((p) => p.model === "claude-opus-5-5")).toBe(true);
  });

  it("adds up to five, each with an id of its own", () => {
    let cast = startingCast(presets);
    cast = addPersona(cast, presets[0]!);
    expect(cast.map((p) => p.id)).toEqual(["optimist", "skeptic", "engineer", "optimist-2"]);
    cast = addPersona(addPersona(cast, presets[3]!), presets[4]!);
    expect(cast).toHaveLength(5);
  });

  it("saves an edited persona in its place, a new one at the end", () => {
    const cast = startingCast(presets);
    const edited = savePersona(cast, { ...cast[1]!, role: "  Grumpy.  ", model: "claude-haiku-5-5" });
    expect(edited[1]).toMatchObject({ id: "skeptic", role: "Grumpy.", model: "claude-haiku-5-5" });
    const more = savePersona(cast, { id: "new", name: " Ada ", role: "A mathematician.", model: "claude-sonnet-5-5" });
    expect(more.at(-1)).toMatchObject({ id: "new", name: "Ada" });
  });

  it("offers only those not in the cast yet", () => {
    expect(notInCast(presets, startingCast(presets)).map((p) => p.name)).toEqual(["Lena", "Omar", "Jules"]);
  });

  it("starts with a topic and 2 to 5 personas", () => {
    const cast = startingCast(presets);
    expect(canStart("AI and jobs", cast)).toBe(true);
    expect(canStart("  ", cast)).toBe(false);
    expect(canStart("AI and jobs", cast.slice(0, 1))).toBe(false);
  });

  it("keeps the turn cap between 2 and 60", () => {
    expect(stepTurns(12, 2)).toBe(14);
    expect(stepTurns(2, -2)).toBe(2);
    expect(stepTurns(60, 2)).toBe(60);
  });

  it("estimates the most it can cost at the dearest model in the cast", () => {
    const cast = startingCast(presets);
    expect(estimate(cast, 150_000, models)).toBeCloseTo(1.08); // 150k × (0.8 × $4 + 0.2 × $20) / M
    expect(estimate([{ ...cast[0]!, model: "claude-haiku-5-5" }], 1_000_000, models)).toBeCloseTo(0.18);
  });

  it("shortens a role to its first sentence", () => {
    expect(shortRole("A skeptic. You poke holes.")).toBe("A skeptic");
    expect(shortRole("x".repeat(50))).toBe(`${"x".repeat(40)}…`);
  });
});

describe("the live talk", () => {
  it("takes the snapshot, then each change", () => {
    let v = applyPartyEvent(emptyParty, { type: "snapshot", party: party(), messages: [line(1)] });
    expect(v.messages.map((m) => m.id)).toEqual([1]);
    v = applyPartyEvent(v, { type: "party", party: party({ turns: 6, status: "paused" }) });
    expect(v.party).toMatchObject({ turns: 6, status: "paused" });
  });

  it("grows the speaking bubble, and ends it when that persona's line is in", () => {
    let v = applyPartyEvent(emptyParty, { type: "snapshot", party: party(), messages: [] });
    v = applyPartyEvent(v, { type: "speaking", speaking: { who: "p2", name: "Victor", text: "I doubt" } });
    v = applyPartyEvent(v, { type: "speaking", speaking: { who: "p2", name: "Victor", text: "I doubt it" } });
    expect(v.party?.speaking?.text).toBe("I doubt it");
    v = applyPartyEvent(v, { type: "message", message: line(2, { kind: "say", who: "owner", name: "Emberhut" }) });
    expect(v.party?.speaking?.who).toBe("p2");
    v = applyPartyEvent(v, { type: "message", message: line(3, { who: "p2", name: "Victor", text: "I doubt it." }) });
    expect(v.party?.speaking).toBeNull();
    expect(v.messages.map((m) => m.id)).toEqual([2, 3]);
  });

  it("never adds a line twice", () => {
    const v = applyPartyEvent(emptyParty, { type: "snapshot", party: party(), messages: [line(1), line(2)] });
    expect(applyPartyEvent(v, { type: "message", message: line(2) })).toBe(v);
  });

  it("waits for a party before it shows who speaks", () => {
    expect(applyPartyEvent(emptyParty, { type: "speaking", speaking: { who: "p1", name: "Nora", text: "" } })).toBe(emptyParty);
  });
});
