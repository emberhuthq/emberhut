import type { ClaudeAccount, Machine } from "@/api";
import type { IconName } from "@/ui";
import { percent } from "@/format/size";
import { parseInstant } from "@/format/time";
import { contrast, mix } from "@/theme/colors";

// What a hut's card says (a hut is a computer the assistant works on, a machine of GET /api/machines): its kind, whether
// it is online or asleep and since when, its sessions and resources, and each of its accounts with the room left.
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

/** The kind's icon: a laptop for one with a battery, else a desktop's screen, a rack or a cloud. */
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

/** What an asleep hut's head says under its kind, quietly: being asleep isn't a fault. */
export const asleepNote = "Won't take new sessions until it wakes";

/** How long ago, short: "just now", "5 min ago", "2 h ago", "3 d ago". */
export function ago(seconds: number): string {
  const s = Math.max(0, Math.round(seconds));
  if (s < 60) return "just now";
  if (s < 3600) return `${Math.floor(s / 60)} min ago`;
  if (s < 86_400) return `${Math.floor(s / 3600)} h ago`;
  return `${Math.floor(s / 86_400)} d ago`;
}

/** How long since its last report: from its last sample's time, else the age the server gave. */
function silentFor(machine: Machine, now: Date): number | null {
  const seen = parseInstant(machine.seen);
  return seen ? (now.getTime() - seen.getTime()) / 1000 : (machine.age ?? null);
}

/** The pill: "Online", "Asleep · 2 h ago". */
export function pillLine(machine: Machine, now: Date = new Date()): string {
  const presence = presenceOf(machine);
  const word = presenceWord[presence];
  if (presence === "online" || presence === "notYet") return word;
  const seconds = silentFor(machine, now);
  return seconds === null ? word : `${word} · ${ago(seconds)}`;
}

/** "2 open · 1 working · 2 queued", "None". */
export function sessionsLine(machine: Machine): string {
  const open = machine.sessions.length;
  const working = machine.sessions.filter((s) => s.running).length;
  const queued = machine.runner?.queued ?? 0;
  if (open === 0 && queued === 0) return "None";
  return [`${open} open`, working > 0 ? `${working} working` : null, queued > 0 ? `${queued} queued` : null].filter(Boolean).join(" · ");
}

const GB = 1024 ** 3;
/** A GPU busier than this share isn't idle. */
const idle = 0.03;

/** In whole numbers: "CPU 12% · RAM 18 / 64 GB · GPU idle"; null when its runner measures none of them. */
export function resourcesLine(machine: Machine): string | null {
  const parts: string[] = [];
  if (machine.cpu?.usage !== null && machine.cpu?.usage !== undefined) parts.push(`CPU ${percent(machine.cpu.usage)}`);
  if (machine.memory) {
    const { total, available } = machine.memory;
    parts.push(`RAM ${Math.round((total - available) / GB)} / ${Math.round(total / GB)} GB`);
  }
  const utils = machine.gpus.map((g) => g.util).filter((u): u is number => u !== null && u !== undefined);
  if (utils.length > 0) {
    const busiest = Math.max(...utils);
    const gpu = machine.gpus.length > 1 ? "GPUs" : "GPU";
    parts.push(busiest < idle ? `${gpu} idle` : `${gpu} ${percent(busiest)}`);
  }
  return parts.length > 0 ? parts.join(" · ") : null;
}

/** How old an asleep hut's figures are: "as of 2 h ago"; null for a hut that is there, or an unknown age. */
export function asOf(machine: Machine, now: Date = new Date()): string | null {
  if (presenceOf(machine) !== "asleep") return null;
  const seconds = silentFor(machine, now);
  return seconds === null ? null : `as of ${ago(seconds)}`;
}

/** "Updated just now", "Updated 3 min ago". */
export function updatedLine(fetched: Date, now: Date = new Date()): string {
  const seconds = (now.getTime() - fetched.getTime()) / 1000;
  return seconds < 60 ? "Updated just now" : `Updated ${ago(seconds)}`;
}

/** The runner an account without one is kept on (the server's own word for it). */
const linuxRunner = "runner";

/** The accounts kept on the hut, in the server's order. */
export function accountsOn(accounts: readonly ClaudeAccount[] | null | undefined, machine: Machine): ClaudeAccount[] {
  return (accounts ?? []).filter((a) => (a.machine ?? linuxRunner) === machine.id);
}

/** fine: over 40% left. low: 16–40%. out: 15% or less, or it can't take work now. unknown: its use isn't known. */
export type RoomTone = "fine" | "low" | "out" | "unknown";

export interface Room {
  /** On the row's right: "72% left", "60% left · week only", "9% left · back in 1 h 20 m", "Not checked yet". */
  value: string;
  /** Under it, the same way round: "5-hour 72% left · week 90% left", "5-hour not reported · week 60% left". */
  detail: string;
  /** The room left, 0…1, for its bar; null when it isn't known. */
  share: number | null;
  tone: RoomTone;
}

const redRoom = 15;
const amberRoom = 40;

const clock = (at: Date) => `${String(at.getHours()).padStart(2, "0")}:${String(at.getMinutes()).padStart(2, "0")}`;

/** "back in 20 m", "back in 1 h 20 m", "back in 3 d". */
function backIn(at: Date, now: Date): string {
  const minutes = Math.max(1, Math.round((at.getTime() - now.getTime()) / 60_000));
  if (minutes < 60) return `back in ${minutes} m`;
  if (minutes < 1440) return minutes % 60 === 0 ? `back in ${minutes / 60} h` : `back in ${Math.floor(minutes / 60)} h ${minutes % 60} m`;
  return `back in ${Math.floor(minutes / 1440)} d`;
}

/**
 * An account's room: 100 less the higher of its known 5-hour and week use. An unknown 5-hour use is said as not reported
 * and the room as from the week only, never read as 0% used; an account with no use known at all hasn't been checked yet
 * (never 100% left). Low room says when the fuller window starts again, when known. `asleep`: its hut is asleep, so the
 * line says how old the figures are.
 */
export function room(account: ClaudeAccount, now: Date = new Date(), asleep = false): Room {
  const h5 = account.used5h ?? null;
  const week = account.used7d ?? null;
  const left = (label: string, n: number | null) =>
    n === null ? `${label} not reported` : `${label} ${Math.round(Math.min(Math.max(100 - n, 0), 100))}% left`;
  const lines = h5 === null && week === null ? [] : [left("5-hour", h5), left("week", week)];
  const checked = parseInstant(account.checked);
  if (asleep && checked) lines.push(`checked ${ago((now.getTime() - checked.getTime()) / 1000)}`);
  const detail = lines.join(" · ");
  const said = (value: string, tone: RoomTone): Room => ({ value, detail, share: null, tone });
  if (account.loggedIn === false) return said("Not logged in", "out");
  if (account.enabled === false) return said("Off", "out");
  const until = parseInstant(account.limitedUntil);
  if (until && until > now) return said(`Out until ${clock(until)}`, "out");
  if (h5 === null && week === null) return said("Not checked yet", "unknown");
  // The fuller window sets the room, and says when it starts again.
  const weekFuller = h5 === null || (week !== null && week > h5);
  const most = (weekFuller ? week : h5) ?? 0;
  const resets = parseInstant(weekFuller ? account.resets7d : account.resets5h);
  const roomLeft = Math.min(Math.max(100 - most, 0), 100);
  const tone: RoomTone = roomLeft <= redRoom ? "out" : roomLeft <= amberRoom ? "low" : "fine";
  const value = [
    `${Math.round(roomLeft)}% left`,
    h5 === null ? "week only" : null,
    resets && resets > now && tone !== "fine" ? backIn(resets, now) : null,
  ]
    .filter(Boolean)
    .join(" · ");
  return { value, detail, share: roomLeft / 100, tone };
}

/** The line over the cards: "2 huts · 1 asleep", "1 hut". */
export function countLine(machines: readonly Machine[]): string {
  const asleep = machines.filter((m) => presenceOf(m) === "asleep").length;
  const huts = machines.length === 1 ? "1 hut" : `${machines.length} huts`;
  return asleep === 0 ? huts : `${huts} · ${asleep} asleep`;
}

/** A soft tint of `ink` over `ground` for a pill, and `ink` taken darker (lighter on a dark ground) until it reads on it at 4.5:1. */
export function tinted(ink: string, ground: string): { fill: string; text: string } {
  const fill = mix(ink, ground, 0.15);
  const toward = contrast(fill, "#000000") > contrast(fill, "#FFFFFF") ? "#000000" : "#FFFFFF";
  let text = ink;
  for (let share = 0.05; contrast(text, fill) < 4.5 && share <= 1; share += 0.05) text = mix(toward, ink, share);
  return { fill, text };
}

/** A bar's track on `ground`, and its outline: the muted ink mixed in until the edge stands out at 3:1. */
export function track(muted: string, ground: string): { fill: string; line: string } {
  let line = mix(muted, ground, 0.5);
  for (let share = 0.55; contrast(line, ground) < 3 && share <= 1; share += 0.05) line = mix(muted, ground, share);
  return { fill: mix(muted, ground, 0.12), line };
}
