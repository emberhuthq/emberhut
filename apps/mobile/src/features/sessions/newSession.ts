import type { ClaudeAccount, FolderRoots } from "@/api";

/** The folder the session opens in: the one you picked, else the last one used, else your first project. */
export function sessionFolder(picked: string, roots: FolderRoots | undefined): string {
  return picked || roots?.recent[0]?.path || roots?.projects[0]?.path || "";
}

/** An existing branch is picked from the folder's own branches: only then are they asked for. */
export const wantsBranches = (worktree: boolean, existing: boolean, folder: string): boolean => worktree && existing && folder !== "";

/** Your projects' folders first, then where sessions started lately: six at most, once each. */
export function quickFolders(roots: FolderRoots | undefined): { path: string; name: string }[] {
  const seen = new Set<string>();
  const all = [...(roots?.projects ?? []), ...(roots?.recent ?? [])];
  return all
    .filter((f) => !seen.has(f.path) && seen.add(f.path))
    .slice(0, 6)
    .map((f) => ({ path: f.path, name: f.name || (f.path.split("/").at(-1) ?? f.path) }));
}

/**
 * The accounts a session on `machine` can run on: the server's own list for that machine (an account it ties to no
 * machine goes anywhere), without the ones it keeps for its own work (factory) or turned off.
 */
export const accountsFor = (accounts: readonly ClaudeAccount[], machine: string): ClaudeAccount[] =>
  accounts.filter((a) => (!a.machine || a.machine === machine) && a.scope !== "factory" && a.enabled !== false);

/** The machine a new session goes to: the one picked while the server still offers it, else the server's first. */
export function sessionMachine(picked: string | null, offered: readonly { id: string }[]): string | null {
  return offered.find((m) => m.id === picked)?.id ?? offered[0]?.id ?? null;
}

/** An account with its room: this week's and the 5 hours' use, out until when, or not logged in. */
export function accountLine(a: ClaudeAccount, now = Date.now()): string {
  const pct = (x: number | null | undefined) => `${Math.round(x ?? 0)}%`;
  const until = a.limitedUntil ? Date.parse(a.limitedUntil) : NaN;
  const room =
    a.loggedIn === false
      ? "not logged in"
      : !Number.isNaN(until) && until > now
        ? `out until ${new Date(until).toLocaleTimeString("en-GB", { hour: "2-digit", minute: "2-digit" })}`
        : `${pct(a.used7d)} week · ${pct(a.used5h)} 5 h`;
  // An account kept for one kind of work says so ("work": that work only).
  const only = a.scope && a.scope !== "any" ? ` · ${a.scope.charAt(0).toUpperCase()}${a.scope.slice(1)} only` : "";
  return `${a.name} · ${room}${only}`;
}
