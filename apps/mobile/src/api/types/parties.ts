import type { Instant, Reply } from "./common";

// Party: 2 to 5 AI personas talking about your topic, turn by turn.

/** running: talking now. paused: by you, or waiting on a cap you can raise. done: reached its end. stopped: by you. */
export type PartyStatus = "running" | "paused" | "done" | "stopped";

export type PartyColor = "blue" | "orange" | "green" | "purple" | "pink";

/** One persona: in a party its id is p1…p5 and it has a colour; a preset's id is its own ("optimist"), its colour empty. */
export interface PartyPersona {
  id: string;
  name: string;
  /** Who they are and how they talk: their system prompt's heart. */
  role: string;
  model: string;
  color?: PartyColor | "";
}

/** A model a persona can talk with, its prices in USD per million tokens. */
export interface PartyModel {
  id: string;
  name: string;
  in: number;
  out: number;
  note: string;
}

/** Who is talking now, and their words so far. */
export interface PartySpeaking {
  who: string;
  name: string;
  text: string;
}

export interface Party {
  id: number;
  title: string;
  topic: string;
  status: PartyStatus;
  /** Why it paused or ended, in words. */
  why: string | null;
  personas: PartyPersona[];
  turns: number;
  maxTurns: number;
  tokens: number;
  maxTokens: number;
  /** USD at list prices. */
  cost: number;
  created: Instant;
  updated: Instant;
  speaking: PartySpeaking | null;
  /** The list's preview: the last line said. */
  last: string | null;
}

export interface PartyMessage {
  id: number;
  party: number;
  /** "owner" (OWNER), or a persona's id. */
  who: string;
  name: string;
  /** turn: a persona's. say: you jumped in. steer: your direction for them all. */
  kind: "turn" | "say" | "steer";
  text: string;
  model: string | null;
  tokens: number;
  cost: number;
  /** Stopped mid-turn. */
  cut: boolean;
  at: Instant;
}

/** GET /api/parties: your parties (newest first), the ready-made personas, your own from past parties, the models. */
export interface PartiesPage {
  parties: Party[];
  presets: PartyPersona[];
  recent: PartyPersona[];
  models: PartyModel[];
  defaults: { maxTurns: number; maxTokens: number; model: string };
}

/** GET /api/parties/<id>: it and its talk. */
export interface PartyPage {
  party: Party;
  messages: PartyMessage[];
}

/** POST /api/parties. */
export interface NewPartyRequest {
  topic: string;
  personas: { name: string; role: string; model: string }[];
  maxTurns?: number;
  maxTokens?: number;
}

export type PartyReply = Reply & { party: Party };

/** GET /api/parties/<id>/live: a snapshot, then each change. */
export type PartyLiveEvent =
  | { type: "snapshot"; party: Party; messages: PartyMessage[] }
  | { type: "party"; party: Party }
  | { type: "message"; message: PartyMessage }
  | { type: "speaking"; speaking: PartySpeaking | null };
