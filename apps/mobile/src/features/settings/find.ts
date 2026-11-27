/**
 * Settings' search and suggestions over the one list of entries, as the desktop's (`store/settings/search.rs`,
 * `suggest.rs`): words in any order, case-insensitive, found in a title, line, place or other names; title matches first,
 * then everyday settings, then the list's order; with nothing found, the closest few. The suggestions are plain verbs
 * worked out from how things stand, never what is already done. Neither changes a setting: a row only opens where it lives.
 */

import type { IconName } from "@/ui";
import { entries, groupShown, lineOf, pathOf, type Entry, type Now } from "./entries";

/** A row of the front door's list: the setting it opens, its words, and the parts of its title to mark. */
export interface Row {
  entry: Entry;
  title: string;
  line: string;
  /** Its icon: its setting's, or one for what it does (a sun for Switch to light). */
  icon: IconName;
  /** [start, end) of each match in the title. */
  marks: [number, number][];
}

/** How many of the closest settings show when nothing matches. */
const closestCount = 3;

/** The shortest start of a word that still counts as a near match. */
const near = 3;

/** Where `word` starts in `text`, and whether at a word's start; null if it doesn't. */
function find(text: string, word: string): { at: number; start: boolean } | null {
  const lower = text.toLowerCase();
  let first: { at: number; start: boolean } | null = null;
  for (let at = lower.indexOf(word); at >= 0; at = lower.indexOf(word, at + 1)) {
    const start = at === 0 || !/[\p{L}\p{N}]/u.test(lower[at - 1] ?? "");
    if (start) return { at, start };
    first ??= { at, start };
  }
  return first;
}

/** How well `word` matches `entry`: 0 for not at all. */
function score(entry: Entry, line: string, word: string): number {
  const inTitle = find(entry.title, word);
  const title = inTitle ? (inTitle.start ? 12 : 9) : 0;
  const other = Math.max(0, ...entry.synonyms.map((s) => find(s, word)).map((f) => (f ? (f.start ? 6 : 4) : 0)));
  const rest = Math.max(0, ...[line, pathOf(entry)].map((t) => find(t, word)).map((f) => (f ? (f.start ? 2 : 1) : 0)));
  return Math.max(title, other, rest);
}

function row(entry: Entry, line: string, words: string[]): Row {
  const marks = words
    .map((w) => [find(entry.title, w), w.length] as const)
    .filter((m): m is [{ at: number; start: boolean }, number] => m[0] !== null)
    .map(([f, length]): [number, number] => [f.at, f.at + length])
    .sort((a, b) => a[0] - b[0]);
  return { entry, title: entry.title, line, marks, icon: entry.icon };
}

/** What a search found: the settings matching every word, best first, or else the closest few (`nothing` true). */
export function search(query: string, now: Now): { rows: Row[]; nothing: boolean } {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean);
  if (words.length === 0) return { rows: [], nothing: false };
  const ranked: { key: number[]; row: Row }[] = [];
  const closest: { key: number[]; row: Row }[] = [];
  entries.forEach((entry, order) => {
    if (!groupShown(entry.group, now)) return;
    const line = lineOf(entry, now);
    const scores = words.map((w) => score(entry, line, w));
    const technical = entry.technical ? 1 : 0;
    if (scores.every((s) => s > 0)) {
      const inTitle = words.every((w) => find(entry.title, w) !== null) ? 0 : 1;
      const total = scores.reduce((a, b) => a + b, 0);
      ranked.push({ key: [inTitle, -total, technical, order], row: row(entry, line, words) });
      return;
    }
    const partial = words
      .map((w) => {
        for (let n = w.length; n >= near; n--) {
          const s = score(entry, line, w.slice(0, n));
          if (s > 0) return s;
        }
        return 0;
      })
      .reduce((a, b) => a + b, 0);
    if (partial > 0) closest.push({ key: [-partial, technical, order], row: row(entry, line, words) });
  });
  const byKey = (a: { key: number[] }, b: { key: number[] }) => {
    for (let i = 0; i < a.key.length; i++) if (a.key[i] !== b.key[i]) return (a.key[i] ?? 0) - (b.key[i] ?? 0);
    return 0;
  };
  if (ranked.length > 0) return { rows: ranked.sort(byKey).map((r) => r.row), nothing: false };
  return {
    rows: closest
      .sort(byKey)
      .slice(0, closestCount)
      .map((r) => r.row),
    nothing: true,
  };
}

/** The most rows Suggested shows. */
export const mostSuggested = 7;

const counted = (count: number, one: string) => `${count} ${one}${count === 1 ? "" : "s"}`;

/** What to suggest now, what needs a look first, at most [`mostSuggested`]. */
export function suggestions(now: Now): Row[] {
  const rows: [string, string, string][] = [];
  if (now.lessons > 0)
    rows.push(["lessons", `Review ${counted(now.lessons, "lesson")}`, "Your ember is trying them out: keep or drop each"]);
  const tool = now.failingTools[0];
  if (tool) rows.push(["tools", `Reconnect ${tool}`, "It can't connect, so your ember can't use it"]);
  const source = now.sourcesToFix[0];
  if (source) rows.push(["sources", `Fix ${source}`, "It needs a look before your ember can read it"]);
  if (now.signedIn) {
    rows.push(
      now.otherDevices
        ? ["pair", "Add a phone or computer", "One code, works once for 10 minutes"]
        : ["pair", "Pair a computer", "One QR code, works once for 10 minutes"],
    );
  }
  rows.push(["thinks-with", "Change what it thinks with", now.provider ?? "Claude, OpenAI, DeepSeek, OpenRouter, Ollama…"]);
  rows.push([
    "appearance-mode",
    now.dark ? "Switch to light" : "Switch to dark",
    `Now: ${now.appearance === "system" ? "as your phone is" : now.appearance}`,
  ]);
  rows.push(
    now.onDuty
      ? ["on-duty", "Go off duty", "Stops working on its own. Still answers when you message it."]
      : ["on-duty", "Go on duty", "Starts working on its own again"],
  );
  rows.push([
    "about-you",
    "See what your ember knows about you",
    now.facts === undefined ? "Fix or forget any of it" : `${counted(now.facts, "thing")} · fix or forget any`,
  ]);
  return rows.slice(0, mostSuggested).flatMap(([id, title, line]) => {
    const entry = entries.find((e) => e.id === id);
    // The setting's icon is a moon; switching to light shows the sun it brings.
    const icon = title === "Switch to light" ? "sunny-outline" : entry?.icon;
    return entry && icon ? [{ entry, title, line, marks: [], icon }] : [];
  });
}
