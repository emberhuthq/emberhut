/**
 * What you call your ember ("Pip", "Juniper"): one name on every device, kept by the server per person (`POST
 * /api/ember/name`) and carried in the state as `emberName`. Without one it is "Your ember". The rules here are the
 * server's (`emberhut_api::ember_name::clean_name`), so a name it would refuse is refused before it is sent.
 */

import { ApiError, type EmberName, type EmberNameReply, type EmberhutState } from "@/api";

/** What the ember is called when you gave it no name. */
export const DEFAULT_EMBER_NAME = "Your ember";

/** The most characters a name may have. */
export const EMBER_NAME_MAX = 24;

/** The default, never changed. */
export const defaultEmberName: EmberName = { name: DEFAULT_EMBER_NAME, custom: false, at: null };

/** Your own name for the ember in a state; undefined while it is "Your ember" (or the server is older). */
export function customEmberName(state: Pick<EmberhutState, "emberName"> | undefined): string | undefined {
  const ember = state?.emberName;
  return ember?.custom ? ember.name : undefined;
}

/** What the ember is called in a state: your name for it, or "Your ember". */
export function emberName(state: Pick<EmberhutState, "emberName"> | undefined): string {
  return customEmberName(state) ?? DEFAULT_EMBER_NAME;
}

/** What the ember is called mid-sentence ("Message Juniper", "What your ember knows"): your name for it, or "your ember". */
export function emberCalled(state: Pick<EmberhutState, "emberName"> | undefined): string {
  return customEmberName(state) ?? DEFAULT_EMBER_NAME.toLowerCase();
}

/** A name as the server keeps it (null: the default), or why it wouldn't take it. */
export type CleanName = { ok: true; name: string | null } | { ok: false; error: string };

// Unicode's White_Space, as Rust's `trim` and `char::is_whitespace` take it (JavaScript's `trim` also takes the BOM).
const SPACE = "\\t\\n\\v\\f\\r \\u0085\\u00A0\\u1680\\u2000-\\u200A\\u2028\\u2029\\u202F\\u205F\\u3000";
const edges = new RegExp(`^[${SPACE}]+|[${SPACE}]+$`, "gu");
// What keeps a name from being one line: control characters, and the line and paragraph separators.
const breaks = /[\p{Cc}\u2028\u2029]/u;
// The characters that draw nothing or turn text around: zero-width spaces and joiners, direction marks, the BOM.
const invisible = /[\u200B-\u200F\u202A-\u202E\u2060-\u2069\uFEFF]/u;
// A run of the spaces left once breaks are refused: it becomes one plain space.
const spaces = /[ \u00A0\u1680\u2000-\u200A\u202F\u205F\u3000]+/gu;
const ZWJ = "\u200D";

/** `text` without the white space at its ends, as the server trims it. */
const trimmed = (text: string) => text.replace(edges, "");

/** `text` trimmed with each run of spaces inside it made one. */
const collapsed = (text: string) => trimmed(text).replace(spaces, " ");

/** Whether a mark, a variation selector, a skin tone, a tag or a joiner, which belong to the character before them. */
const extending = /^[\p{M}\uFE0E\uFE0F\u{1F3FB}-\u{1F3FF}\u{E0020}-\u{E007F}\u200D]$/u;
const pictographic = /^\p{Extended_Pictographic}$/u;
const regional = /^[\u{1F1E6}-\u{1F1FF}]$/u;

/**
 * `text`'s characters as people see them, for an engine without `Intl.Segmenter` (Hermes may lack it): code points,
 * with marks, variation selectors, skin tones and tags joined to the one before, an emoji joined to the one before it by
 * a joiner, and the two regional letters of a flag together. Close to Unicode's rules for names, not all of them.
 */
export function fallbackGraphemes(text: string): string[] {
  const clusters: string[][] = [];
  for (const point of text) {
    const last = clusters[clusters.length - 1];
    const before = last?.[last.length - 1];
    const joins =
      last !== undefined &&
      (extending.test(point) ||
        (before === ZWJ && pictographic.test(last[0] ?? "") && pictographic.test(point)) ||
        (regional.test(point) && last.length === 1 && regional.test(before ?? "")));
    if (joins) last.push(point);
    else clusters.push([point]);
  }
  return clusters.map((cluster) => cluster.join(""));
}

const segmenter = typeof Intl.Segmenter === "function" ? new Intl.Segmenter(undefined, { granularity: "grapheme" }) : null;

/** `text`'s characters as people see them: an emoji (with its joiners, skin tone or flag letters) is one. */
export function graphemes(text: string): string[] {
  return segmenter ? Array.from(segmenter.segment(text), (part) => part.segment) : fallbackGraphemes(text);
}

/** How many characters a name typed has, as it would be kept (trimmed, spaces made one), counted as people see them. */
export function emberNameLength(text: string): number {
  return graphemes(collapsed(text)).length;
}

/** `name` as it fits `most` characters: itself, or its first ones and an ellipsis. */
export function shortEmberName(name: string, most: number): string {
  const parts = graphemes(name);
  return parts.length <= most
    ? name
    : `${parts
        .slice(0, Math.max(most - 1, 1))
        .join("")
        .trimEnd()}…`;
}

/** Whether a character in `cluster` hides or turns text: a joiner counts only when it joins something after it. */
function hides(cluster: string): boolean {
  const points = [...cluster];
  return points.some((point, i) => invisible.test(point) && !(point === ZWJ && i < points.length - 1));
}

/**
 * `text` as the server keeps it: trimmed, each run of spaces inside made one; null when blank or the default's own words
 * (in any case); refused when not one line of plain text, or over 24 characters as people see them.
 */
export function cleanEmberName(text: string): CleanName {
  const name = trimmed(text);
  if (name === "") return { ok: true, name: null };
  if (breaks.test(name) || graphemes(name).some(hides)) return { ok: false, error: "One line of plain text" };
  const one = name.replace(spaces, " ");
  if (one.toLowerCase() === DEFAULT_EMBER_NAME.toLowerCase()) return { ok: true, name: null };
  if (graphemes(one).length > EMBER_NAME_MAX) return { ok: false, error: `${EMBER_NAME_MAX} characters at most` };
  return { ok: true, name: one };
}

/** From how many characters typed the field shows its count. */
export const EMBER_NAME_COUNT_FROM = 20;

/** Whether the name field takes `text`: no edit makes it longer than a name may be. */
export function takesEdit(text: string): boolean {
  return emberNameLength(text) <= EMBER_NAME_MAX;
}

/** The field's count once near the most (`21/24`); null before. */
export function nameCount(text: string): string | null {
  const count = emberNameLength(text);
  return count >= EMBER_NAME_COUNT_FROM ? `${count}/${EMBER_NAME_MAX}` : null;
}

/** What leaving the name field does: nothing changed (it closes), a rename to send, or a reason it stays open. */
export type Leaving = { kind: "close" } | { kind: "rename"; typed: string } | { kind: "stay"; error: string };

/** Leaving the field holding `typed` while the name is `custom` (undefined: the default). Blank means the default, never an error. */
export function leaving(typed: string, custom: string | undefined): Leaving {
  const clean = cleanEmberName(typed);
  if (!clean.ok) return { kind: "stay", error: clean.error };
  return clean.name === (custom ?? null) ? { kind: "close" } : { kind: "rename", typed };
}

/** The server's reason without the field's name it starts with (`name: 24 characters at most`). */
function reasonOf(cause: unknown): string {
  if (cause instanceof ApiError && cause.unreachable) return "Can't reach your ember";
  const message = cause instanceof Error ? cause.message : "no reason given";
  const words = message.replace(/^name:\s*/u, "");
  return words.charAt(0).toUpperCase() + words.slice(1);
}

/** What the rename flow reaches: the name now on screen and the server, its own state, the request. */
export interface RenameIo {
  /** The name on screen now. */
  current: () => EmberName | undefined;
  /** Puts `name` everywhere the ember is named. */
  show: (name: EmberName) => void;
  /** Sends the rename. */
  send: (name: string | null) => Promise<EmberNameReply>;
}

/** How a rename went: the name kept, or why not (and the name went back). */
export type RenameOutcome = { ok: true; name: EmberName } | { ok: false; error: string };

/**
 * Renames the ember to `typed` (blank: back to "Your ember"): shown at once, then sent; a name the server refuses (or
 * one that can't reach it) goes back to the one before, with the reason. A name the rules refuse is never sent.
 */
export async function renameEmber(typed: string, io: RenameIo): Promise<RenameOutcome> {
  const clean = cleanEmberName(typed);
  if (!clean.ok) return { ok: false, error: clean.error };
  const before = io.current() ?? defaultEmberName;
  io.show(clean.name === null ? { ...defaultEmberName, at: before.at } : { name: clean.name, custom: true, at: before.at });
  try {
    const { ok: _ok, error: _error, ...kept } = await io.send(clean.name);
    io.show(kept);
    return { ok: true, name: kept };
  } catch (cause) {
    io.show(before);
    return { ok: false, error: reasonOf(cause) };
  }
}

/** The chat's title: an agent's name in its room, the ember's name (yours for it, or "Your ember") in Main, else the room's. */
export function chatTitle(room: { main: boolean; name?: string }, agent: string | undefined, custom: string | undefined): string {
  if (agent !== undefined) return agent;
  if (room.main) return custom ?? DEFAULT_EMBER_NAME;
  return room.name ?? DEFAULT_EMBER_NAME;
}

/** The longest the ember's name is in the message box's prompt before it is cut with an ellipsis. */
const PROMPT_NAME = 16;

/** The message box's prompt: `Message <agent>` in an agent's room, else `Message <ember>`, `… in <room>` in a project's. */
export function composerPrompt(agent: string | undefined, project: string | undefined, called: string): string {
  if (agent !== undefined) return `Message ${agent}`;
  const name = shortEmberName(called, PROMPT_NAME);
  return project === undefined ? `Message ${name}` : `Message ${name} in ${project}`;
}
