import { createStore } from "zustand/vanilla";
import type { EmberhutApi, SettingsChange } from "@/api";
import { inBackground } from "@/log";

/** How long a tap on a task waits, undoable, before it is sent. */
export const undoMs = 4_000;
/** How long a refusal stays on screen. */
export const failedMs = 6_000;

export interface Toast {
  id: number;
  task: number;
  text: string;
  /** A closing answer (No, Not now, Leave it). */
  warn: boolean;
  /** The held request is on its way. */
  sending: boolean;
  failed: boolean;
}

export interface ActionsState {
  toast: Toast | null;
  /** Tasks acted on: off every list until the server agrees or you undo it. */
  gone: ReadonlySet<number>;
  sending: ReadonlySet<number>;
  /** Holds a change to a task for `undoMs`, the task gone from the lists meanwhile; the action before it goes now. */
  act: (task: number, text: string, change: SettingsChange, failure: string, warn?: boolean) => void;
  undo: () => void;
  /** Sends what is held now (the app going to the background, a new action). */
  flush: () => void;
  dismiss: () => void;
  /** Your answer to an ask. "Leave it", "Not now" and "No" close the task on the server. */
  decide: (task: number, choice: string) => void;
  markDone: (task: number, done?: boolean) => void;
  /** Not now: out of the way for a week. */
  later: (task: number, now?: Date) => void;
  archive: (task: number) => void;
  unarchive: (task: number) => void;
}

const closing = /^(leave it|not now|no)$/i;

export interface ActionsOptions {
  /** After the server took a change: refresh what shows it. */
  onApplied?: () => void;
}

/**
 * A tap on a task shows at once: the task leaves every list, a toast offers Undo, and only after the undo window is the
 * change sent. Held rather than sent and reversed, because an answer can't be taken back once Emberhut has it. A refusal
 * brings the task back with the server's reason.
 */
export function createActionsStore(api: EmberhutApi, options: ActionsOptions = {}) {
  const held = new Map<number, { change: SettingsChange; failure: string }>();
  let nextId = 1;

  return createStore<ActionsState>()((set, get) => {
    const without = (items: ReadonlySet<number>, task: number) => new Set([...items].filter((t) => t !== task));

    async function send(toast: Toast): Promise<void> {
      const item = held.get(toast.id);
      if (!item) return;
      held.delete(toast.id);
      set((s) => ({
        sending: new Set(s.sending).add(toast.task),
        toast: s.toast?.id === toast.id ? { ...s.toast, sending: true } : s.toast,
      }));
      let error: string | null = null;
      try {
        await api.settings.apply(item.change);
      } catch (cause) {
        error = cause instanceof Error ? cause.message : "no reason given";
      }
      if (error === null) options.onApplied?.();
      const failed: Toast | null =
        error === null
          ? null
          : { id: nextId++, task: toast.task, text: `${item.failure}: ${error}`, warn: false, sending: false, failed: true };
      set((s) => ({
        gone: without(s.gone, toast.task),
        sending: without(s.sending, toast.task),
        toast: failed ?? (s.toast?.id === toast.id ? null : s.toast),
      }));
      if (failed) setTimeout(() => get().toast?.id === failed.id && set({ toast: null }), failedMs);
    }

    function act(task: number, text: string, change: SettingsChange, failure: string, warn = false): void {
      if (get().gone.has(task)) return;
      get().flush();
      const toast: Toast = { id: nextId++, task, text, warn, sending: false, failed: false };
      held.set(toast.id, { change, failure });
      set((s) => ({ gone: new Set(s.gone).add(task), toast }));
      setTimeout(() => inBackground(send(toast), "actions: send"), undoMs);
    }

    return {
      toast: null,
      gone: new Set(),
      sending: new Set(),
      act,

      undo() {
        const toast = get().toast;
        if (!toast || !held.delete(toast.id)) return;
        set((s) => ({ gone: without(s.gone, toast.task), toast: null }));
      },

      flush() {
        const toast = get().toast;
        if (toast && held.has(toast.id)) inBackground(send(toast), "actions: send");
      },

      dismiss() {
        if (get().toast?.failed) set({ toast: null });
      },

      decide(task, choice) {
        const closes = closing.test(choice.trim());
        const text = closes
          ? `${choice}: task closed`
          : choice === "Done"
            ? "Marked done"
            : choice.toLowerCase() === "approve"
              ? "Approved"
              : `Answered: ${choice}`;
        act(task, text, { taskDecide: task, choice }, "Answer not sent", closes);
      },

      markDone(task, done = true) {
        act(task, done ? "Marked done" : "Not done", { todoDone: task, done }, "Not marked done");
      },

      later(task, now = new Date()) {
        const week = new Date(now.getFullYear(), now.getMonth(), now.getDate() + 7);
        act(task, "Moved to next week", { todoPark: task, until: week.toISOString() }, "Not moved");
      },

      archive(task) {
        act(task, "Archived", { taskArchive: task }, "Not archived");
      },

      unarchive(task) {
        act(task, "Back in to do", { taskUnarchive: task }, "Not unarchived");
      },
    };
  });
}
