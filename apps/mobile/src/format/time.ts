import type { Day, Instant } from "@/api";

const dayMs = 86_400_000;

/** A calendar day ("2026-11-24", or the start of an instant) as local midnight; null when it isn't one. */
export function parseDay(day: Day | null | undefined): Date | null {
  const match = /^(\d{4})-(\d{2})-(\d{2})/.exec(day ?? "");
  if (!match) return null;
  const [, year, month, date] = match.map(Number) as [number, number, number, number];
  return new Date(year, month - 1, date);
}

function midnight(at: Date): Date {
  return new Date(at.getFullYear(), at.getMonth(), at.getDate());
}

/** Calendar days from today to `day`'s date, whatever its time: 0 today, 1 tomorrow, -2 two days ago. */
export function daysFromToday(day: Date, now: Date = new Date()): number {
  // Rounded: a day across a clock change is 23 or 25 hours.
  return Math.round((midnight(day).getTime() - midnight(now).getTime()) / dayMs);
}

export function parseInstant(at: Instant | null | undefined): Date | null {
  if (!at) return null;
  const time = Date.parse(at);
  return Number.isNaN(time) ? null : new Date(time);
}

/** "just now", "5 min ago", "3 h ago", "yesterday", "4 days ago", "12 Nov"; "in 20 min" ahead. */
export function relative(at: Date, now: Date = new Date()): string {
  const seconds = Math.round((at.getTime() - now.getTime()) / 1000);
  const ahead = seconds > 0;
  const s = Math.abs(seconds);
  const say = (text: string) => (ahead ? `in ${text}` : `${text} ago`);
  if (s < 45) return "just now";
  if (s < 3_600) return say(`${Math.max(1, Math.round(s / 60))} min`);
  if (s < 86_400) return say(`${Math.round(s / 3_600)} h`);
  const days = Math.abs(daysFromToday(at, now));
  if (days === 1) return ahead ? "tomorrow" : "yesterday";
  if (days < 7) return say(`${days} days`);
  return shortDate(at);
}

/** "12 Nov". */
export function shortDate(at: Date): string {
  return at.toLocaleDateString("en-GB", { day: "numeric", month: "short" });
}

/** "Mon". */
export function weekday(at: Date): string {
  return at.toLocaleDateString("en-GB", { weekday: "short" });
}

/** "14:05". */
function clock(at: Date): string {
  return at.toLocaleTimeString("en-GB", { hour: "2-digit", minute: "2-digit" });
}

/** "Mon 14:05". */
export function weekdayClock(at: Date): string {
  return `${weekday(at)} ${clock(at)}`;
}

/** How long between two times: "40 s", "12 min", "2 h 5 min". */
export function span(from: Date, to: Date): string {
  const s = Math.max(1, Math.round((to.getTime() - from.getTime()) / 1000));
  if (s < 60) return `${s} s`;
  const minutes = Math.round(s / 60);
  if (minutes < 60) return `${minutes} min`;
  const rest = minutes % 60;
  return rest === 0 ? `${Math.floor(minutes / 60)} h` : `${Math.floor(minutes / 60)} h ${rest} min`;
}

/** A day as a deadline reads: "3d late", "Today", "Tomorrow", "Fri", "3 Nov". */
export function deadlineText(day: Date, now: Date = new Date()): string {
  const days = daysFromToday(day, now);
  if (days < 0) return `${-days}d late`;
  if (days === 0) return "Today";
  if (days === 1) return "Tomorrow";
  return days < 7 ? weekday(day) : shortDate(day);
}
