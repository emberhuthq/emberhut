import { isServerPath, type MessageRef } from "@/api";

/** What a chip reads: "<task key> · its title", "PR #76 · owner/repo"; short enough that two share a row. */
export function refText(ref: MessageRef): string {
  if (!ref.title || ref.kind === "card") return ref.label;
  const title = ref.title.length > 34 ? `${ref.title.slice(0, 33).trimEnd()}…` : ref.title;
  return `${ref.label} · ${title}`;
}

export type RefTarget =
  { open: "task"; id: number } | { open: "file"; path: string; name: string } | { open: "link"; url: string } | { open: "none" };

/** Where a tap on a chip goes: a task's page, a report or file on the server in the viewer, a PR or card in the browser. */
export function refTarget(ref: MessageRef): RefTarget {
  if (ref.kind === "task" && ref.task !== undefined) return { open: "task", id: ref.task };
  const url = ref.url ?? "";
  if (isServerPath(url)) return { open: "file", path: url, name: ref.name ?? ref.title ?? "report.html" };
  if (url.startsWith("https://")) return { open: "link", url };
  return { open: "none" };
}

/** How many of an answer's chips show before "+N": a few on a line or two, the rest a tap away. */
export const chipsAtFirst = 3;

/** The chips on show: all once expanded, else the first few, with how many wait behind "+N" (none when that's one). */
export function shownRefs<T>(refs: readonly T[], expanded: boolean, first = chipsAtFirst): { shown: readonly T[]; more: number } {
  // Hiding one chip behind a "+1" chip saves nothing.
  if (expanded || refs.length <= first + 1) return { shown: refs, more: 0 };
  return { shown: refs.slice(0, first), more: refs.length - first };
}
