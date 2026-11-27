import { createStore, type StoreApi } from "zustand/vanilla";
import type { HealthCheck, Slot } from "@/api";
import { inBackground } from "@/log";
import type { SessionState } from "./session";

/** While the app is in front and on its server, the server's health is checked this often. */
export const checkEveryMs = 15_000;
/** This many failed checks in a row and the app goes to the fallback server. */
export const failuresToFallBack = 3;
/** At launch, a server that doesn't answer is asked again this many times, this far apart, before the app falls back. */
export const launchRetries = 2;
export const launchRetryMs = 1_000;

export interface FallbackState {
  /** Which server requests go to now. */
  using: Slot;
  /** Health checks of the server that failed in a row. */
  failures: number;
  /** Try again is asking the server. */
  checking: boolean;
  /** Try again found the server still not answering. */
  stillDown: boolean;
  /** At launch: the server is checked, asked again twice if it doesn't answer, and the app falls back if it never does. */
  launch: () => Promise<void>;
  /** One health check of the server (every 15 s while the app is in front); the third failure in a row falls back. */
  check: () => Promise<void>;
  /** Asks the server again; goes back to it only if it answers. True when the app is on its server. */
  tryAgain: () => Promise<boolean>;
  /** The app came to the front (checks start) or went away (they stop). */
  foreground: (active: boolean) => void;
}

export interface FallbackDeps {
  session: StoreApi<SessionState>;
  health: HealthCheck;
  /** Called after the server in use changed: what was fetched from the other one is dropped there. */
  onSwitch?: (using: Slot) => void;
  sleep?: (ms: number) => Promise<void>;
  timers?: { setInterval: typeof setInterval; clearInterval: typeof clearInterval };
}

/** The app can go to its fallback server: it is signed in to both. */
export const canFallBack = (s: SessionState): boolean =>
  s.status === "signedIn" && s.serverUrl !== null && s.fallback !== null && s.fallback.signedIn;

const wait = (ms: number) => new Promise<void>((resolve) => setTimeout(resolve, ms));

/**
 * The fallback server: while the app is on its server and has a fallback, the server's `GET /health` is checked every
 * 15 s in the foreground; after three failures in a row (or at launch, when it doesn't answer after two quick retries)
 * requests go to the fallback server, with its own session. The app never goes back by itself: Try again checks the
 * server and goes back only if it answers. A launch always starts on the server.
 */
export function createFallbackStore({ session, health, onSwitch, sleep = wait, timers = globalThis }: FallbackDeps) {
  let timer: ReturnType<typeof setInterval> | null = null;
  // One check at a time: a slow one (a server that doesn't answer takes the whole timeout) isn't overtaken.
  let busy = false;

  const switchTo = (using: Slot) => {
    if (store.getState().using === using) return;
    store.setState({ using, failures: 0, stillDown: false, checking: false });
    onSwitch?.(using);
  };

  /** Runs `work` unless a check is running; checks only while the app is on its server and can fall back. */
  async function exclusive(work: (url: string) => Promise<void>): Promise<void> {
    const s = session.getState();
    if (busy || store.getState().using !== "server" || !canFallBack(s) || s.serverUrl === null) return;
    busy = true;
    try {
      await work(s.serverUrl);
    } finally {
      busy = false;
    }
  }

  /** The answer counts only if the app is still on the same server, with a fallback. */
  const stillOn = (url: string) =>
    store.getState().using === "server" && session.getState().serverUrl === url && canFallBack(session.getState());

  const store = createStore<FallbackState>()((set, get) => ({
    using: "server",
    failures: 0,
    checking: false,
    stillDown: false,

    launch: () =>
      exclusive(async (url) => {
        for (let attempt = 0; attempt <= launchRetries; attempt++) {
          if (await health(url)) {
            set({ failures: 0 });
            return;
          }
          if (!stillOn(url)) return;
          if (attempt < launchRetries) await sleep(launchRetryMs);
        }
        if (stillOn(url)) switchTo("fallback");
      }),

    check: () =>
      exclusive(async (url) => {
        const answered = await health(url);
        if (!stillOn(url)) return;
        if (answered) {
          if (get().failures !== 0) set({ failures: 0 });
          return;
        }
        const failures = get().failures + 1;
        if (failures >= failuresToFallBack) switchTo("fallback");
        else set({ failures });
      }),

    async tryAgain() {
      const url = session.getState().serverUrl;
      if (get().using === "server") return true;
      if (url === null || get().checking) return false;
      set({ checking: true, stillDown: false });
      const answered = await health(url);
      if (get().using !== "fallback") return get().using === "server";
      if (answered) switchTo("server");
      else set({ checking: false, stillDown: true });
      return answered;
    },

    foreground(active) {
      if (active && timer === null) {
        timer = timers.setInterval(() => inBackground(get().check(), "fallback: check"), checkEveryMs);
      } else if (!active && timer !== null) {
        timers.clearInterval(timer);
        timer = null;
        // Failures count in a row while in front: a phone put away starts over.
        set({ failures: 0 });
      }
    },
  }));

  session.subscribe((now, before) => {
    // Launched (the kept session read) on a server with a fallback: is the server there?
    if (before.status === "starting" && now.status === "signedIn") inBackground(store.getState().launch(), "fallback: launch");
    // Signed out, the fallback removed or signed out, or another server paired: back to the server.
    if (store.getState().using === "fallback" && (!canFallBack(now) || now.serverUrl !== before.serverUrl)) switchTo("server");
  });

  return store;
}
