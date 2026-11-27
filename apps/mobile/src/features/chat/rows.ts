import type { ChatMsg, ChatQuote } from "@/api";

// The chat as rows: each message on its own, every run of the server's event lines folded into one, a time over a row
// when 15 minutes passed since the one before (as Messages does).

export type Row =
  | { kind: "message"; key: string; msg: ChatMsg; stamp: Date | null }
  | { kind: "events"; key: string; text: string; ids: number[]; stamp: Date | null };

const gapMs = 15 * 60_000;

export function buildRows(chat: readonly ChatMsg[]): Row[] {
  const out: Row[] = [];
  let last: number | null = null;
  for (const m of chat) {
    const at = m.at ? Date.parse(m.at) : NaN;
    const prev = out.at(-1);
    if (m.event && prev?.kind === "events") {
      out[out.length - 1] = { ...prev, text: `${prev.text}\n${m.text}`, ids: [...prev.ids, m.id] };
    } else {
      const stamp = Number.isNaN(at) || (last !== null && at - last <= gapMs) ? null : new Date(at);
      out.push(
        m.event
          ? { kind: "events", key: String(m.id), text: m.text, ids: [m.id], stamp }
          : { kind: "message", key: String(m.id), msg: m, stamp },
      );
    }
    if (!Number.isNaN(at)) last = at;
  }
  return out;
}

export interface EventLine {
  kind: string;
  text: string;
}

/** The server writes what it told Emberhut as "- [kind] text" lines. */
export function eventLines(text: string): EventLine[] {
  return text.split("\n").flatMap((raw) => {
    const m = /^- \[(\w+)\] (.*)$/.exec(raw);
    return m ? [{ kind: m[1]!, text: m[2]! }] : [];
  });
}

/** "checkin" reads "check-in", "new_mail" "new mail". */
const eventName = (kind: string): string => (kind === "checkin" ? "check-in" : kind.replace(/_/g, " "));

/** "1 check-in · 2 notes": each kind once, in the order they came, counted. */
export function eventSummary(lines: readonly EventLine[]): string {
  const counts = new Map<string, number>();
  for (const l of lines) counts.set(eventName(l.kind), (counts.get(eventName(l.kind)) ?? 0) + 1);
  return [...counts]
    .map(([name, n]) =>
      n === 1 || name.endsWith("s") ? `${n} ${name}` : name.endsWith("y") ? `${n} ${name.slice(0, -1)}ies` : `${n} ${name}s`,
    )
    .join(" · ");
}

/** A message as a reply quotes it: who, when, a short preview, its first picture or video. */
export function quoteOf(m: ChatMsg): ChatQuote {
  return { id: m.id, mine: m.mine, at: m.at ?? null, text: m.text.slice(0, 160), file: m.files?.find((f) => f.kind !== "file") ?? null };
}
