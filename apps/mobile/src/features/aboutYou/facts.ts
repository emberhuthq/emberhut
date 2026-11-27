import type { Fact, FactsPage, SettingsChange, TrialRule } from "@/api";
import type { IconName } from "@/ui";

/** The page's title, as its header and the way to it name it. */
export const aboutYouTitle = "What your ember knows about you";

/** The search field's placeholder. */
export const searchPlaceholder = "Search what your ember knows";

/** What the page says when the server took a keep but the lesson is still on trial: a server older than `trialKeep`. */
export const cantKeepEarly = "Your server can't keep lessons early yet; it will decide at the weekly review";

/** A keep the server answered but didn't make: the page says so plainly, not as a failure to save. */
export class CantKeepEarly extends Error {
  constructor() {
    super(cantKeepEarly);
    this.name = "CantKeepEarly";
  }
}

/** The line under the page for a change that didn't take: the plain line for a keep the server can't make yet. */
export function changeProblem(error: Error): string {
  return error instanceof CantKeepEarly ? error.message : `Not saved: ${error.message}`;
}

/** The facts' categories in the reference client's order; a category the app doesn't know is listed under Other, last. */
const categories: readonly { key: string; name: string; icon: IconName }[] = [
  { key: "people", name: "People", icon: "people" },
  { key: "work", name: "Work", icon: "briefcase" },
  { key: "money", name: "Money", icon: "cash" },
  { key: "preferences", name: "Preferences", icon: "options" },
  { key: "health", name: "Health & habits", icon: "heart" },
  { key: "home", name: "Home & family", icon: "home" },
  { key: "projects", name: "Projects", icon: "folder" },
];

/** Evidence kept for later: never listed. */
const unlisted = new Set(["proof"]);

/** The key of Other, for the categories the app doesn't know. */
export const otherKey = "other";

const other = { key: otherKey, name: "Other", icon: "document-text" as IconName };

export interface FactGroup {
  key: string;
  name: string;
  icon: IconName;
  facts: Fact[];
}

/** What you searched for, in any of the texts, any case. */
const matches = (query: string, ...texts: string[]) => {
  const q = query.trim().toLowerCase();
  return q === "" || texts.some((t) => t.toLowerCase().includes(q));
};

/** Where a fact is listed: its category's key, Other's for one the app doesn't know, or none for evidence. */
function shelf(fact: Fact): string | null {
  if (unlisted.has(fact.category)) return null;
  return categories.some((c) => c.key === fact.category) ? fact.category : otherKey;
}

/** The facts that match `query` by category, in the page's order; a category with none is left out. */
export function factGroups(facts: readonly Fact[], query: string): FactGroup[] {
  const shown = facts.filter((f) => matches(query, f.title, f.text));
  return [...categories, other].map((c) => ({ ...c, facts: shown.filter((f) => shelf(f) === c.key) })).filter((g) => g.facts.length > 0);
}

export interface CategoryChip {
  /** A category's key; null for All. */
  key: string | null;
  label: string;
  count: number;
}

/** The chips over the list: All, then each category with facts that match `query`, and how many. */
export function categoryChips(facts: readonly Fact[], query: string): CategoryChip[] {
  const groups = factGroups(facts, query);
  const all = groups.reduce((n, g) => n + g.facts.length, 0);
  return [{ key: null, label: "All", count: all }, ...groups.map((g) => ({ key: g.key, label: g.name, count: g.facts.length }))];
}

/** The groups the chosen chip shows: all of them, or its own (all when the search left it empty). */
export function groupsShown(groups: readonly FactGroup[], chip: string | null): FactGroup[] {
  const chosen = groups.filter((g) => g.key === chip);
  return chip === null || chosen.length === 0 ? [...groups] : chosen;
}

/** A lesson in plain words to you: its own line when your ember gave one, else the rule as it follows it. */
export function lessonText(trial: TrialRule): string {
  const line = trial.lesson?.trim();
  return line ? line : trial.text;
}

/** The lessons that match `query`, by what you said and the lesson. */
export function trialsShown(trials: readonly TrialRule[], query: string): TrialRule[] {
  return trials.filter((t) => matches(query, t.said, lessonText(t)));
}

/** The day to decide by: the earliest end of `trials`, as an instant; null when there are none. */
export function decideBy(trials: readonly TrialRule[]): string | null {
  const ends = trials.map((t) => t.ends).filter((end) => !Number.isNaN(Date.parse(end)));
  if (ends.length === 0) return null;
  return ends.reduce((a, b) => (Date.parse(b) < Date.parse(a) ? b : a));
}

/** "2 lessons to review", "1 lesson to review". */
export function lessonsToReview(count: number): string {
  return `${count} ${count === 1 ? "lesson" : "lessons"} to review`;
}

/** Whether lesson `id` is still on trial in `page`: after a keep the server answered, a server that can't keep early. */
export function stillOnTrial(page: FactsPage, id: number): boolean {
  return (page.trials ?? []).some((t) => t.id === id);
}

/** The facts page as it will be once the server took a change: what you fix shows at once. */
export function withChange(page: FactsPage, change: SettingsChange): FactsPage {
  let facts = page.facts;
  if (change.factSet) {
    const set = change.factSet;
    facts = facts.map((f) => (f.id === set.id ? { ...f, text: set.text, source: "you corrected it" } : f));
  }
  if (change.personSet) {
    const person = change.personSet;
    facts = facts.map((f) => (f.id === person.id ? { ...f, important: person.important } : f));
  }
  if (change.factDrop) facts = facts.filter((f) => f.id !== change.factDrop);
  const ended = [change.trialDrop, change.trialKeep].filter((id) => id !== undefined);
  const trials = ended.length === 0 ? page.trials : page.trials?.filter((t) => !ended.includes(t.id));
  return { ...page, facts, trials };
}
