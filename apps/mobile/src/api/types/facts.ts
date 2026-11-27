import type { Instant } from "./common";

/** Something Emberhut knows about you (its context folder). */
export interface Fact {
  /** "people/x". */
  id: string;
  category: string;
  title: string;
  text: string;
  source: string;
  updated: Instant;
  /** People only. */
  now?: string;
  nowAt?: Instant;
  important?: boolean;
  chats?: string[];
  /** The project it belongs to; missing means shared. */
  project?: string;
}

export interface Lesson {
  id: number;
  text: string;
  at: Instant;
}

/** A lesson from your corrections Emberhut is trying out: in its prompt until `ends` unless you drop it. */
export interface TrialRule {
  id: number;
  text: string;
  at: Instant;
  ends: Instant;
  /** What you said that it came from. */
  said: string;
}

/** GET /api/facts. */
export interface FactsPage {
  facts: Fact[];
  lessons?: Lesson[];
  trials?: TrialRule[];
}
