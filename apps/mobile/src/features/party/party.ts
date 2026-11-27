import type { Party, PartyColor, PartyLiveEvent, PartyMessage, PartyModel, PartyPage, PartyPersona, PartyStatus } from "@/api";

// Party's words, numbers and the live talk as the page holds it.

export const olderServerText = "Emberhut's server is on older code: Party comes with its next update";

/** A cast is 2 to 5. */
export const minCast = 2;
export const maxCast = 5;

/** The turn cap's stepper: 2 to 60, by 2. */
export const turnCaps = { min: 2, max: 60, step: 2 } as const;

export const tokenCaps = [50_000, 150_000, 500_000, 1_000_000] as const;

/** What a new party starts with: the optimist, the skeptic and the engineer. */
export const startingPresets = ["optimist", "skeptic", "engineer"] as const;

export const defaultModel = "claude-opus-5-5";

/** "950", "23.4k", "1.0M". */
export function partyTokens(n: number): string {
  if (n >= 1_000_000) return `${trim(n / 1_000_000)}M`;
  if (n >= 1000) return `${trim(n / 1000)}k`;
  return String(n);
}

// One decimal, without a ".0" on a round cap: "150k", "1M", "23.4k".
const trim = (x: number) => {
  const s = x.toFixed(1);
  return s.endsWith(".0") ? s.slice(0, -2) : s;
};

/** "$0.12"; under a cent, "<$0.01". */
export function partyCost(usd: number): string {
  return usd > 0 && usd < 0.01 ? "<$0.01" : `$${usd.toFixed(2)}`;
}

export function statusWord(status: PartyStatus): string {
  return { running: "Talking", paused: "Paused", done: "Done", stopped: "Stopped" }[status] ?? status;
}

/** The status dot's tone: talking green, paused orange, ended grey. */
export function statusTone(status: PartyStatus): "done" | "warning" | "tertiary" {
  return status === "running" ? "done" : status === "paused" ? "warning" : "tertiary";
}

export const castLine = (party: Party): string => party.personas.map((p) => p.name).join(", ");

/** "Turn 5 of 12 · 23.4k of 150k tokens · $0.12". */
export function meter(party: Party): string {
  return `Turn ${party.turns} of ${party.maxTurns} · ${partyTokens(party.tokens)} of ${partyTokens(party.maxTokens)} tokens · ${partyCost(party.cost)}`;
}

/** The header's play button: a paused party resumes; an ended one carries on for more turns. */
export function resumeLabel(status: PartyStatus): string {
  return status === "paused" ? "Resume" : "Continue";
}

export const canStop = (status: PartyStatus): boolean => status === "running" || status === "paused";

// iOS's system colours, as each scheme has them; grey for a persona without one.
const personaColors: Record<PartyColor, { light: string; dark: string }> = {
  blue: { light: "#007AFF", dark: "#0A84FF" },
  orange: { light: "#FF9500", dark: "#FF9F0A" },
  green: { light: "#34C759", dark: "#30D158" },
  purple: { light: "#AF52DE", dark: "#BF5AF2" },
  pink: { light: "#FF2D55", dark: "#FF375F" },
};

export function personaColor(color: string | undefined, scheme: "light" | "dark"): string {
  const c = personaColors[color as PartyColor] as { light: string; dark: string } | undefined;
  return c ? c[scheme] : "#8E8E93";
}

export const personaOf = (party: Party | null, who: string): PartyPersona | undefined => party?.personas.find((p) => p.id === who);

export const initial = (name: string): string => name.trim().charAt(0).toUpperCase() || "?";

/** A model's name ("Opus 5.5"), or its id without "claude-" when the server's list doesn't have it. */
export function modelName(models: PartyModel[], id: string): string {
  return models.find((m) => m.id === id)?.name ?? id.replace(/^claude-/, "");
}

/** A role's first sentence, 40 characters at most: "An optimist", "A skeptic". */
export function shortRole(role: string): string {
  const first = role.split(".")[0] ?? role;
  return first.length > 40 ? `${first.slice(0, 40)}…` : first;
}

// ── The cast being picked ──

/** An id not yet in the cast: `base`, then `base-2`, `base-3`… */
export function freeId(cast: PartyPersona[], base: string): string {
  const taken = new Set(cast.map((p) => p.id));
  if (!taken.has(base)) return base;
  let n = 2;
  while (taken.has(`${base}-${n}`)) n++;
  return `${base}-${n}`;
}

/** The cast with this persona (a preset, one of your own, a new one) added, while there's room for one more. */
export function addPersona(cast: PartyPersona[], p: PartyPersona): PartyPersona[] {
  if (cast.length >= maxCast) return cast;
  return [...cast, { id: freeId(cast, p.id), name: p.name, role: p.role, model: p.model || defaultModel }];
}

/** The persona saved from the editor: in its place, or added. */
export function savePersona(cast: PartyPersona[], p: PartyPersona): PartyPersona[] {
  const named = { ...p, name: p.name.trim(), role: p.role.trim() };
  return cast.some((c) => c.id === p.id) ? cast.map((c) => (c.id === p.id ? named : c)) : addPersona(cast, named);
}

export function startingCast(presets: PartyPersona[]): PartyPersona[] {
  return startingPresets.reduce<PartyPersona[]>((cast, id) => {
    const p = presets.find((x) => x.id === id);
    return p ? addPersona(cast, p) : cast;
  }, []);
}

/** The presets or your own personas not already in the cast (by name). */
export const notInCast = (people: PartyPersona[], cast: PartyPersona[]): PartyPersona[] =>
  people.filter((p) => !cast.some((c) => c.name === p.name));

export const canStart = (topic: string, cast: PartyPersona[]): boolean =>
  topic.trim() !== "" && cast.length >= minCast && cast.length <= maxCast;

export const stepTurns = (n: number, by: number): number => Math.min(turnCaps.max, Math.max(turnCaps.min, n + by));

/** The most it can cost: the token cap, all at the dearest model in the cast (output priced as a fifth of the tokens). */
export function estimate(cast: PartyPersona[], maxTokens: number, models: PartyModel[]): number {
  const perMillion = Math.max(
    0,
    ...cast.map((p) => {
      const m = models.find((x) => x.id === p.model);
      return m ? 0.8 * m.in + 0.2 * m.out : 0;
    }),
  );
  return (maxTokens * perMillion) / 1_000_000;
}

// ── One party, live ──

export interface PartyView {
  party: Party | null;
  messages: PartyMessage[];
}

export const emptyParty: PartyView = { party: null, messages: [] };

export const loadedParty = (page: PartyPage): PartyView => ({ party: page.party, messages: page.messages });

/**
 * The page moved on by one live event. A line it already holds isn't added twice, and a persona's finished turn ends
 * their live bubble, so their words never show twice.
 */
export function applyPartyEvent(v: PartyView, e: PartyLiveEvent): PartyView {
  switch (e.type) {
    case "snapshot":
      return { party: e.party, messages: e.messages };
    case "party":
      return { ...v, party: e.party };
    case "message": {
      const m = e.message;
      if (v.messages.some((x) => x.id === m.id)) return v;
      const party = v.party && m.kind === "turn" && v.party.speaking?.who === m.who ? { ...v.party, speaking: null } : v.party;
      return { party, messages: [...v.messages, m] };
    }
    case "speaking":
      return v.party ? { ...v, party: { ...v.party, speaking: e.speaking } } : v;
    default:
      return v;
  }
}
