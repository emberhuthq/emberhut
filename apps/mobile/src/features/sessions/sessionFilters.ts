import type { AgentNow, RunnerSession, ExternalThread } from "@/api";
import { machineLabel, noMachineNames, type MachineNames } from "@/features/machines/machineNames";
import type { SessionFilter } from "@/state/prefs";

export type { SessionFilter };

// The Sessions tab's Worker and Environment filters. Worker:
// who runs it. Environment: the machine and the account. Several picks in one add up, the filters narrow each other, and
// each pick's count is what it would show with the other filters as they are. None picked: all.

/** What the filters read off a row: who runs it, its machine, its account (by name: one account on two machines is one). */
export interface SessionFacts {
  kind: string;
  machine: string | null;
  account: string | null;
}

/** The Worker filter's key for external threads. Picks are saved with the Sessions filter on the device, so it keeps its spelling. */
export const EXTERNAL_THREAD_KIND = "t3";

export const workerKinds: readonly { key: string; name: string }[] = [
  { key: "worker", name: "Emberhut worker" },
  { key: "session", name: "Runner session" },
  { key: "lead", name: "Ultra lead" },
  { key: "helper", name: "Helper agents" },
  { key: "direct", name: "Your sessions" },
  { key: EXTERNAL_THREAD_KIND, name: "External thread" },
];

/** Older servers send no kind: a lead agent's name says it (Worker, Session, Lead), every other row is a helper. */
export function agentFacts(a: AgentNow): SessionFacts {
  const kind = a.kind ?? (a.lead ? (a.name === "Worker" ? "worker" : a.name === "Lead" ? "lead" : "session") : "helper");
  return { kind, machine: a.machine ?? null, account: a.accountName ?? a.account ?? null };
}
export const sessionFacts = (s: RunnerSession): SessionFacts => ({
  kind: s.direct ? "direct" : "session",
  machine: s.machine,
  account: s.accountName ?? s.account ?? null,
});
/** An external thread's machine is its own id (its app's, not a runner's): the server's word for where it runs. */
export const externalThreadFacts = (t: ExternalThread): SessionFacts => ({ kind: EXTERNAL_THREAD_KIND, machine: t.machine, account: null });

type Part = "worker" | "machine" | "account";
const some = (l: string[] | null | undefined): l is string[] => !!l && l.length > 0;

/** A row by its facts; skip leaves one filter out (a menu's own counts ignore its own picks). */
export function keeps(f: SessionFilter, x: SessionFacts, skip?: Part): boolean {
  if (skip !== "worker" && some(f.workers) && !f.workers.includes(x.kind)) return false;
  if (skip !== "machine" && some(f.machines) && !f.machines.includes(x.machine ?? "")) return false;
  if (skip !== "account" && some(f.accounts) && !f.accounts.includes(x.account ?? "")) return false;
  return true;
}

export const isFiltered = (f: SessionFilter): boolean => some(f.workers) || some(f.machines) || some(f.accounts);

export interface Pick {
  key: string;
  label: string;
  count: number;
}

export interface SessionPicks {
  workers: Pick[];
  machines: Pick[];
  accounts: Pick[];
}

/**
 * Each kind, machine and account these rows have (or that is picked), with its count. Machines go in the server's order
 * (GET /api/machines) by its names, then any it doesn't list by id.
 */
export function sessionPicks(f: SessionFilter, rows: readonly SessionFacts[], names: MachineNames = noMachineNames): SessionPicks {
  const n = (part: Part, hit: (x: SessionFacts) => boolean) => rows.filter((x) => keeps(f, x, part) && hit(x)).length;
  const workers = workerKinds
    .map((k) => ({ key: k.key, label: k.name, count: n("worker", (x) => x.kind === k.key) }))
    .filter((p) => p.count > 0 || f.workers?.includes(p.key));
  const ms = new Set([...rows.flatMap((x) => (x.machine ? [x.machine] : [])), ...(f.machines ?? [])]);
  const order = [...names.keys()];
  const machines = [...order.filter((m) => ms.has(m)), ...[...ms].filter((m) => !names.has(m)).sort()].map((m) => ({
    key: m,
    label: machineLabel(names, m),
    count: n("machine", (x) => x.machine === m),
  }));
  const as = new Set([...rows.flatMap((x) => (x.account ? [x.account] : [])), ...(f.accounts ?? [])]);
  const accounts = [...as].sort().map((a) => ({ key: a, label: a, count: n("account", (x) => x.account === a) }));
  return { workers, machines, accounts };
}

/** One pick on or off; the last one off clears the filter. */
export function toggle(list: string[] | null | undefined, key: string): string[] | null {
  const l = list ?? [];
  const next = l.includes(key) ? l.filter((k) => k !== key) : [...l, key];
  return next.length > 0 ? next : null;
}

/** A chip's words: its name with nothing picked, else what's picked ("Studio, Bedrock", "Lead +2"). */
export function chipLabel(name: string, picked: readonly Pick[]): string {
  if (picked.length === 0) return name;
  return picked.length <= 2 ? picked.map((p) => p.label).join(", ") : `${picked[0]!.label} +${picked.length - 1}`;
}
