/**
 * The title a rename sends for what's typed over a task titled `current`: trimmed; null when it is the task's title
 * already (nothing to change). An empty one goes too: the server says why it won't take it.
 */
export function newTitle(current: string, typed: string): string | null {
  const title = typed.trim();
  return title === current.trim() ? null : title;
}

/** What the page says when the rename didn't happen: the server's reason (an empty or too long title, no such task). */
export function renameFailure(cause: unknown): string {
  return `Not renamed: ${cause instanceof Error && cause.message !== "" ? cause.message : "unknown"}`;
}
