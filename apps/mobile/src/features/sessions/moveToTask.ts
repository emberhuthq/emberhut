import type { Issue } from "@/api";
import { isOpen } from "@/features/tasks/model";

/** The sessions a server can move to another task: a runner's ("r-…") and a worker's ("w-<task>"). */
export const isMovable = (id: string): boolean => /^[rw]-./.test(id);

/**
 * The tasks a session can move to, in the order given: open ones (not done or archived, which the server refuses), not
 * one it already works on, not one put away here; with a query, only those whose key or title has it.
 */
export function moveTargets(tasks: readonly Issue[], current: readonly number[], gone: ReadonlySet<number>, query = ""): Issue[] {
  const words = query.trim().toLowerCase();
  const seen = new Set<number>();
  return tasks.filter((task) => {
    if (seen.has(task.id) || !isOpen(task) || current.includes(task.id) || gone.has(task.id)) return false;
    seen.add(task.id);
    return words === "" || task.key.toLowerCase().includes(words) || task.title.toLowerCase().includes(words);
  });
}

/** What the picker says when the move didn't happen: the server's reason (no such session or task, a closed task). */
export function moveFailure(cause: unknown): string {
  return `Not moved: ${cause instanceof Error && cause.message !== "" ? cause.message : "unknown"}`;
}
