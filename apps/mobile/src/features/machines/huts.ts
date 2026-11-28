import type { ClaudeAccount, Machine } from "@/api";
import type { IconName } from "@/ui";
import { gbOf, percent } from "@/format/size";
import { parseInstant } from "@/format/time";

// What a hut's card says (a hut is a computer the assistant works on, a machine of GET /api/machines): its kind, whether
// it is online or asleep and when it was last seen, what runs there now, and each of its accounts with the room left.
// The desktop's Huts page says the same (apps/desktop/src/ui/machines/words.rs).

/** What kind of computer a hut is: from the platform its runner reports, a server when its name says so. */
export type HutKind = "mac" | "linux" | "windows" | "server" | "cloud" | "computer";

const serverWords = ["server", "vps", "srv", "nas"];

export function hutKind(machine: Machine): HutKind {
  const platform = (machine.platform ?? "").toLowerCase();
  const named = `${machine.name} ${machine.host ?? ""}`.toLowerCase();
  if (platform === "darwin" || platform === "macos") return "mac";
  if (platform === "win32" || platform === "windows") return "windows";
  if (platform === "cloud") return "cloud";
  if (serverWords.some((w) => named.includes(w))) return "server";
  return platform === "linux" ? "linux" : "computer";
}

export const kindWord: Record<HutKind, string> = {
  mac: "Mac",
  linux: "Linux PC",
  windows: "Windows PC",
  server: "Server",
  cloud: "Cloud",
  computer: "Computer",
};

/** The kind's icon: a laptop for one with a battery. */
export function kindIcon(kind: HutKind, laptop: boolean): IconName {
  if (laptop && (kind === "mac" || kind === "linux" || kind === "windows")) return "laptop-outline";
  if (kind === "server") return "server-outline";
  if (kind === "cloud") return "cloud-outline";
  if (kind === "computer") return "hardware-chip-outline";
  return "desktop-outline";
}

/** Online: its runner reports. Quiet: none for a minute, still counted on. Asleep: none for two. Not yet: never. */
export type Presence = "online" | "quiet" | "asleep" | "notYet";

export function presenceOf(machine: Machine): Presence {
  if (machine.state === "ok" || machine.state === "warn" || machine.state === "low") return "online";
  if (machine.state === "stale") return "quiet";
  if (machine.state === "offline") return "asleep";
  return "notYet";
}

export const presenceWord: Record<Presence, string> = {
  online: "Online",
  quiet: "Quiet",
  asleep: "Asleep",
  notYet: "Not reported yet",
};

/** How long ago, short: "just now", "5 min ago", "2 h ago", "3 d ago". */
export function ago(seconds: number): string {
  const s = Math.max(0, Math.round(seconds));
  if (s < 60) return "just now";
  if (s < 3600) return `${Math.floor(s / 60)} min ago`;
  if (s < 86_400) return `${Math.floor(s / 3600)} h ago`;
  return `${Math.floor(s / 86_400)} d ago`;
}

/** "Online", "Asleep · last seen 2 h ago". */
export function presenceLine(machine: Machine, now: Date = new Date()): string {
  const presence = presenceOf(machine);
  const word = presenceWord[presence];
  if (presence === "online" || presence === "notYet") return word;
  const seen = parseInstant(machine.seen);
  const seconds = seen ? (now.getTime() - seen.getTime()) / 1000 : machine.age;
  return seconds === null || seconds === undefined ? word : `${word} · last seen ${ago(seconds)}`;
}

/** "3 sessions · 1 running · 2 queued", "Nothing running". */
export function runningLine(machine: Machine): string {
  const sessions = machine.sessions.length;
  const running = machine.sessions.filter((s) => s.running).length;
  const queued = machine.runner?.queued ?? 0;
  if (sessions === 0 && queued === 0) return "Nothing running";
  return [
    sessions === 1 ? "1 session" : `${sessions} sessions`,
    running > 0 ? `${running} running` : null,
    queued > 0 ? `${queued} queued` : null,
  ]
    .filter(Boolean)
    .join(" · ");
}

/** "CPU 12% · Memory 18.0 of 64.0 GB · 1 GPU"; null when its runner measures none of them. */
export function resourcesLine(machine: Machine): string | null {
  const parts: string[] = [];
  if (machine.cpu?.usage !== null && machine.cpu?.usage !== undefined) parts.push(`CPU ${percent(machine.cpu.usage)}`);
  if (machine.memory) parts.push(`Memory ${gbOf(machine.memory.total - machine.memory.available, machine.memory.total)}`);
  if (machine.gpus.length === 1) parts.push("1 GPU");
  else if (machine.gpus.length > 1) parts.push(`${machine.gpus.length} GPUs`);
  return parts.length > 0 ? parts.join(" · ") : null;
}

/** The runner an account without one is kept on (the server's own word for it). */
const linuxRunner = "runner";

/** The accounts kept on the hut, in the server's order. */
export function accountsOn(accounts: readonly ClaudeAccount[] | null | undefined, machine: Machine): ClaudeAccount[] {
  return (accounts ?? []).filter((a) => (a.machine ?? linuxRunner) === machine.id);
}

/** fine: plenty left. low: little. out: it can't take work now. unknown: its use isn't known. */
export type RoomTone = "fine" | "low" | "out" | "unknown";

export interface Room {
  /** On the row's right: "72% left", "Not checked yet", "Out until 18:00". */
  value: string;
  /** Under it: "5-hour 28% used · week 10% used", "5-hour not reported · week 40% used"; "" when nothing more to say. */
  detail: string;
  /** The room left, 0…1, for its bar; null when it isn't known. */
  share: number | null;
  tone: RoomTone;
}

const lowRoom = 20;

const clock = (at: Date) => `${String(at.getHours()).padStart(2, "0")}:${String(at.getMinutes()).padStart(2, "0")}`;

/**
 * An account's room: 100 less the higher of its known 5-hour and week use. An unknown 5-hour use is said as not reported,
 * never read as 0% used; an account with no use known at all hasn't been checked yet (never 100% left).
 */
export function room(account: ClaudeAccount, now: Date = new Date()): Room {
  const h5 = account.used5h ?? null;
  const week = account.used7d ?? null;
  const used = (label: string, n: number | null) => (n === null ? `${label} not reported` : `${label} ${Math.round(n)}% used`);
  const detail = h5 === null && week === null ? "" : `${used("5-hour", h5)} · ${used("week", week)}`;
  const said = (value: string, tone: RoomTone): Room => ({ value, detail, share: null, tone });
  if (account.loggedIn === false) return said("Not logged in", "out");
  if (account.enabled === false) return said("Off", "out");
  const until = parseInstant(account.limitedUntil);
  if (until && until > now) return said(`Out until ${clock(until)}`, "out");
  const known = [h5, week].filter((n): n is number => n !== null);
  if (known.length === 0) return said("Not checked yet", "unknown");
  const left = Math.min(Math.max(100 - Math.max(...known), 0), 100);
  return {
    value: `${Math.round(left)}% left`,
    detail,
    share: left / 100,
    tone: left <= 0 ? "out" : left < lowRoom ? "low" : "fine",
  };
}

/** The line over the cards: "2 huts · 1 asleep", "1 hut". */
export function countLine(machines: readonly Machine[]): string {
  const asleep = machines.filter((m) => presenceOf(m) === "asleep").length;
  const huts = machines.length === 1 ? "1 hut" : `${machines.length} huts`;
  return asleep === 0 ? huts : `${huts} · ${asleep} asleep`;
}
