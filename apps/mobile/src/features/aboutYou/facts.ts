import type { Fact, FactsPage, SettingsChange, TrialRule } from "@/api";
import type { IconName } from "@/ui";

/** The facts' categories in the reference client's order; any other the server has comes after, under its own name. */
const categories: readonly { key: string; name: string; icon: IconName }[] = [
  { key: "people", name: "People", icon: "people" },
  { key: "work", name: "Work", icon: "briefcase" },
  { key: "money", name: "Money", icon: "cash" },
  { key: "preferences", name: "How you like things", icon: "options" },
  { key: "health", name: "Health & habits", icon: "heart" },
  { key: "home", name: "Home & family", icon: "home" },
  { key: "projects", name: "Projects", icon: "folder" },
];

export interface FactGroup {
  key: string;
  name: string;
  icon: IconName;
  facts: Fact[];
}

/** What you searched for, in the title or the text, any case. */
const matches = (query: string, ...texts: string[]) => {
  const q = query.trim().toLowerCase();
  return q === "" || texts.some((t) => t.toLowerCase().includes(q));
};

export function factGroups(facts: readonly Fact[], query: string): FactGroup[] {
  const shown = facts.filter((f) => matches(query, f.title, f.text));
  const known = new Set(categories.map((c) => c.key));
  const others = [...new Set(shown.map((f) => f.category).filter((c) => !known.has(c)))].map((key) => ({
    key,
    name: key.charAt(0).toUpperCase() + key.slice(1),
    icon: "document-text" as IconName,
  }));
  return [...categories, ...others]
    .map((c) => ({ ...c, facts: shown.filter((f) => f.category === c.key) }))
    .filter((g) => g.facts.length > 0);
}

export function trialsShown(trials: readonly TrialRule[], query: string): TrialRule[] {
  return trials.filter((t) => matches(query, t.text));
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
  const trials = change.trialDrop === undefined ? page.trials : page.trials?.filter((t) => t.id !== change.trialDrop);
  return { ...page, facts, trials };
}
