import type { SessionHead, SessionLiveEvent, SessionStep, SessionTranscript } from "@/api";

// A session page's view of its stream: the steps so far, Claude's words and thinking as they're written, the turn's
// tokens, and the head. Each live event moves it on; a step it already has is never added twice.

export interface LiveView {
  head: SessionHead | null;
  steps: SessionStep[];
  /** The last step held: a reconnect asks only for the ones after it. */
  seen: number;
  /** Claude's words in the block being written now. */
  typing: string;
  /** Its thinking being written now (empty: it keeps it to itself); null when it isn't thinking. */
  thinking: string | null;
  thinkingSince: string | null;
  tokens: number | null;
  /** Older steps than these exist and aren't shown. */
  earlier: boolean;
}

export const emptyView: LiveView = {
  head: null,
  steps: [],
  seen: 0,
  typing: "",
  thinking: null,
  thinkingSince: null,
  tokens: null,
  earlier: false,
};

export function loaded(t: SessionTranscript): LiveView {
  return { ...emptyView, head: t.session, steps: t.events, seen: t.events.at(-1)?.seq ?? 0, earlier: t.earlier ?? false };
}

export function applyEvent(v: LiveView, e: SessionLiveEvent, now: () => string = () => new Date().toISOString()): LiveView {
  switch (e.type) {
    case "step": {
      if (e.step.seq <= v.seen) return v;
      const own = !e.step.parent;
      return {
        ...v,
        steps: [...v.steps, e.step],
        seen: e.step.seq,
        typing: own && e.step.kind === "text" ? "" : v.typing,
        thinking: own && e.step.kind === "thinking" ? null : v.thinking,
      };
    }
    case "delta":
      return { ...v, typing: e.start ? e.text : v.typing + e.text };
    case "think":
      return e.start ? { ...v, thinking: e.text, thinkingSince: e.since ?? now() } : { ...v, thinking: (v.thinking ?? "") + e.text };
    case "tokens":
      return { ...v, tokens: e.tokens };
    case "head": {
      const head = e.session;
      if (head.running !== true) return { ...v, head, typing: "", thinking: null, tokens: null };
      return { ...v, head, tokens: head.tokens ? Math.max(v.tokens ?? 0, head.tokens) : v.tokens };
    }
  }
}
