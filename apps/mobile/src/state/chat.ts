import { createStore } from "zustand/vanilla";
import type { ChatMsg, EmberhutApi } from "@/api";

/** The server's page size for /api/history and the state's chat. */
export const pageSize = 60;

/**
 * A page of a room's chat joined to what is held: the page's copies win. What's held before the page stays only when
 * nothing can be missing between them (they overlap, or the page isn't full); after a full page with a gap, the page
 * starts over. What's held after the page stays: the server's chat only grows.
 */
export function join(held: readonly ChatMsg[], page: readonly ChatMsg[]): ChatMsg[] {
  const first = page[0]?.id;
  const last = page[page.length - 1]?.id;
  if (first === undefined || last === undefined) return [...held];
  const touches = page.length < pageSize || held.some((m) => m.id >= first);
  return [...(touches ? held.filter((m) => m.id < first) : []), ...page, ...held.filter((m) => m.id > last)];
}

export interface ChatState {
  /** Each room's chat as this device holds it, oldest first. */
  chats: Record<number, ChatMsg[]>;
  /** The room's first message is held: "Show earlier" has nothing more. */
  complete: Record<number, boolean>;
  /** Joins a page (the state's chat, a history page) into the room's chat. */
  hold: (room: number, page: readonly ChatMsg[]) => void;
  /** What a room got since the last message held (none held: its newest page). */
  catchUp: (room: number) => Promise<void>;
  /** The 60 before the oldest held; a second call while one runs waits on the same one. */
  loadEarlier: (room: number) => Promise<void>;
}

export function createChatStore(api: EmberhutApi) {
  const loading = new Map<number, Promise<void>>();
  return createStore<ChatState>()((set, get) => ({
    chats: {},
    complete: {},

    hold(room, page) {
      const held = get().chats[room] ?? [];
      const next = join(held, page);
      if (next.length === held.length && next.every((m, i) => m === held[i])) return;
      // The page started the chat over after a gap: what's before it is unknown again.
      const oldest = held[0]?.id;
      const restarted = oldest !== undefined && !next.some((m) => m.id === oldest);
      set((s) => ({ chats: { ...s.chats, [room]: next }, complete: restarted ? { ...s.complete, [room]: false } : s.complete }));
    },

    async catchUp(room) {
      const after = get().chats[room]?.at(-1)?.id ?? 0;
      const page = await api.chat.history(room, { after });
      get().hold(room, page.chat);
    },

    loadEarlier(room) {
      const running = loading.get(room);
      if (running) return running;
      const load = (async () => {
        const before = get().chats[room]?.[0]?.id;
        if (before === undefined || get().complete[room]) return;
        const page = await api.chat.history(room, { before });
        set((s) => {
          const held = s.chats[room] ?? [];
          // Only what is still older than the oldest held: the chat may have moved while the page came.
          const first = held[0]?.id ?? before;
          const older = page.chat.filter((m) => m.id < first);
          return { chats: { ...s.chats, [room]: [...older, ...held] }, complete: { ...s.complete, [room]: page.chat.length < pageSize } };
        });
      })().finally(() => loading.delete(room));
      loading.set(room, load);
      return load;
    },
  }));
}
