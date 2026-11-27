import { createStore } from "zustand/vanilla";
import type { ChatFile, ChatMsg, ChatQuote, EmberhutApi } from "@/api";
import { ApiError } from "@/api";

/** Your message on its way, until the room's chat has it, with the line under it. */
export interface Pending {
  text: string;
  files: ChatFile[];
  reply: ChatQuote | null;
  /** The newest message held when you sent it: only a newer one of yours can be its copy. */
  after: number;
  /** Under the bubble: null while Emberhut takes it, else what happened. */
  note: string | null;
  /** Not sent: a tap sends it again. */
  failed: boolean;
}

export interface Draft {
  text: string;
  files: ChatFile[];
  reply?: ChatQuote | null;
}

export interface TalkState {
  pending: Record<number, Pending | undefined>;
  send: (room: number, draft: Draft) => Promise<void>;
  retry: (room: number) => Promise<void>;
  /** The stop button: what you sent in the room is no longer waited on. */
  forget: (room: number) => void;
}

export interface TalkDeps {
  api: EmberhutApi;
  /** The room's chat as held now. */
  chat: (room: number) => readonly ChatMsg[];
  /** The send is done: the state is fetched again, so the answer and the end of the pending line come together. */
  onSent: (room: number) => void;
  /** A send can vanish without an error: after this long without its copy, you're told and can send it again. */
  lostMs?: number;
}

export const lostNote = "Not sent: Emberhut never got it · tap to send again";

/** The room's chat has your message: from then on its own copy shows. */
export function delivered(pending: Pending, chat: readonly ChatMsg[]): boolean {
  return chat.some((m) => m.mine && m.id > pending.after && m.text === pending.text);
}

export function createTalkStore({ api, chat, onSent, lostMs = 45_000 }: TalkDeps) {
  const store = createStore<TalkState>()((set, get) => {
    const update = (room: number, sent: Pending, change: Partial<Pending> | null) =>
      set((s) =>
        s.pending[room] !== sent ? s : { pending: { ...s.pending, [room]: change === null ? undefined : { ...sent, ...change } } },
      );

    return {
      pending: {},

      async send(room, draft) {
        const text = draft.text.trim();
        if (text === "" && draft.files.length === 0) return;
        const sent: Pending = {
          text,
          files: draft.files,
          reply: draft.reply ?? null,
          after: chat(room).at(-1)?.id ?? 0,
          note: null,
          failed: false,
        };
        set((s) => ({ pending: { ...s.pending, [room]: sent } }));
        // Never resent by itself: a copy that did arrive late would reach Emberhut twice.
        setTimeout(() => {
          const now = get().pending[room];
          if (now === sent && !delivered(sent, chat(room))) update(room, sent, { note: lostNote, failed: true });
        }, lostMs);
        try {
          const result = await api.chat.talk({ text, files: sent.files, room, replyTo: sent.reply?.id });
          if (!result.started) update(room, sent, null);
        } catch (cause) {
          const reason = cause instanceof ApiError && !cause.unreachable ? cause.message : "can't reach Emberhut";
          update(room, sent, { note: `Not sent: ${reason}`, failed: true });
        }
        onSent(room);
      },

      async retry(room) {
        const last = get().pending[room];
        if (last?.failed) await get().send(room, { text: last.text, files: last.files, reply: last.reply });
      },

      forget(room) {
        if (get().pending[room]) set((s) => ({ pending: { ...s.pending, [room]: undefined } }));
      },
    };
  });
  return store;
}
