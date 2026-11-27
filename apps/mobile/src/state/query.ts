import { QueryClient } from "@tanstack/react-query";
import type { ApprovalsQuery, RunnerMachine, TasksQuery } from "@/api";

/** Server data: one cache for the app, importable outside React. */
export const queryClient = new QueryClient({
  defaultOptions: {
    queries: { staleTime: 10_000, retry: 2, retryDelay: (failures) => Math.min(1_000 * 2 ** failures, 4_000) },
  },
});

export const queryKeys = {
  allStates: ["state"] as const,
  state: (room: number) => ["state", room] as const,
  task: (id: number) => ["task", id] as const,
  tasks: (query: TasksQuery) => ["tasks", query] as const,
  facts: ["facts"] as const,
  switches: ["switches"] as const,
  environments: ["environments"] as const,
  machines: ["machines"] as const,
  /** The machines without their history: names, and whether they take new work. */
  machineSummary: ["machines", "summary"] as const,
  parties: ["parties"] as const,
  /** The emberhut.com account's machines, from its directory. */
  accountMachines: ["account", "machines"] as const,
  agents: ["agents"] as const,
  /** What the agent thinks with: the saved model provider, never its key. */
  agentProvider: ["agentProvider"] as const,
  /** Every approvals list and request: what an `approvals` event or a decision refreshes. */
  allApprovals: ["approvals"] as const,
  approvals: (query: ApprovalsQuery) => ["approvals", "list", query] as const,
  approval: (id: number) => ["approvals", "one", id] as const,
  /** A custom accent's tokens and contrast, nothing saved. */
  themePreview: (hex: string) => ["themePreview", hex] as const,
  transcript: (session: string, agent?: string) => ["transcript", session, agent ?? null] as const,
  folders: (machine: RunnerMachine, path?: string) => ["folders", machine, path ?? null] as const,
  branches: (machine: RunnerMachine, cwd: string) => ["branches", machine, cwd] as const,
  browserSession: (device: string) => ["browserSession", device] as const,
  /** A server file's text: a report's page, a note. */
  fileText: (path: string) => ["fileText", path] as const,
};
