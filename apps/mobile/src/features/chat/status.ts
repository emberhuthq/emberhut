import type { EmberhutState } from "@/api";

export interface EmberhutStatus {
  text: string;
  tone: "secondary" | "warning" | "danger";
}

const percent = (x: number) => `${Math.round(x * 100)}%`;

/** "Watching", "Working…", "Paused", with the jobs running and a quota warning once 80% of a window is used. */
export function emberhutStatus(state: EmberhutState | undefined, working: boolean, offline: boolean, now = Date.now()): EmberhutStatus {
  if (state === undefined) return { text: "Connecting…", tone: "secondary" };
  if (offline) return { text: "Offline", tone: "warning" };
  const jobs = (state.tasks ?? []).filter((t) => t.status === "running").length;
  const base = (!state.on ? "Paused" : working ? "Working…" : "Watching") + (jobs > 0 ? ` · ${jobs} job${jobs === 1 ? "" : "s"}` : "");
  const account = state.account;
  const until = account?.limitedUntil ? Date.parse(account.limitedUntil) : NaN;
  if (!Number.isNaN(until) && until > now) {
    const time = new Date(until).toLocaleTimeString("en-GB", { hour: "2-digit", minute: "2-digit" });
    return { text: `Out of quota until ${time}`, tone: "danger" };
  }
  const windows = (
    [
      ["5-hour", account?.used5h ?? 0],
      ["weekly", account?.used7d ?? 0],
    ] as const
  ).filter(([, used]) => used >= 0.8);
  const worst = windows.reduce<(typeof windows)[number] | null>((a, w) => (a === null || w[1] > a[1] ? w : a), null);
  if (worst) return { text: `${base} · ${percent(worst[1])} of ${worst[0]} used`, tone: worst[1] >= 0.95 ? "danger" : "warning" };
  return { text: base, tone: "secondary" };
}
