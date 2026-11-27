import { createStore, type StoreApi } from "zustand/vanilla";
import { inBackground } from "@/log";
import type { SessionState } from "@/state/session";

export type PushPermission = "granted" | "denied" | "undetermined";

export interface PermissionState {
  status: PushPermission;
  /** Android still shows its prompt (false once you blocked it, or after your second no). */
  canAskAgain: boolean;
}

/** The phone's side of push: expo-notifications in the app (push/native.ts), a stand-in in the tests. */
export interface PushPlatform {
  /** Why this build can't have push at all (nothing is asked or sent then); unset where it can. */
  unsupported?: string;
  /** Makes Emberhut's notification channel, which has to exist before Android 13 asks for the permission. */
  prepare(): Promise<void>;
  permission(): Promise<PermissionState>;
  /** Shows Android's prompt; true when granted. */
  requestPermission(): Promise<boolean>;
  /** This install's FCM token; throws when the build has no Firebase config. */
  token(): Promise<string>;
  onTokenChange(listener: (token: string) => void): () => void;
  /** Voids this install's FCM token, so a push to it fails and Emberhut drops it; the next token() is a new one. */
  forget(): Promise<void>;
}

export interface PushState {
  /** null until it was looked at, after sign-in. */
  permission: PushPermission | null;
  /** Emberhut has this phone's current token. */
  registered: boolean;
  error: string | null;
  /** The platform's reason there's no push on this phone at all, or null. */
  unsupported: string | null;
  /** Registers this phone's token with Emberhut when signed in and allowed; safe to call again (on launch, from Settings). */
  sync: () => Promise<void>;
}

export interface PushDeps {
  platform: PushPlatform;
  /** POST /api/devices { token, platform: "android" }. */
  register: (token: string) => Promise<unknown>;
  session: StoreApi<SessionState>;
}

/**
 * Push for this phone: after sign-in it asks for the permission (Android 13 and later), then gives Emberhut its FCM token,
 * and the new one whenever FCM turns it over. Nothing is sent while signed out or without the permission.
 */
export function createPushStore({ platform, register, session }: PushDeps) {
  let sent: string | null = null;
  // Asked only while you never answered, and once a launch at most: after a no, Settings says how to turn it on.
  let asked = false;
  let running: Promise<void> | null = null;
  // A sign-out voiding the token: a sign-in right after waits, or it would hand Emberhut the token being voided.
  let forgetting: Promise<void> = Promise.resolve();
  const signedIn = () => session.getState().status === "signedIn";

  // FCM's listener hands over the same token the sync just asked for: one of the two sends it.
  let giving: string | null = null;
  async function give(token: string): Promise<void> {
    if (token === sent || token === giving) return;
    giving = token;
    try {
      await register(token);
      sent = token;
      store.setState({ registered: true, error: null });
    } catch (cause) {
      store.setState({ registered: false, error: cause instanceof Error ? cause.message : "Not registered" });
    } finally {
      if (giving === token) giving = null;
    }
  }

  /** The channel made, then the permission as it is, asked for once when you never answered. */
  async function allowed(): Promise<PushPermission> {
    await platform.prepare();
    const now = await platform.permission();
    if (now.status === "undetermined" && now.canAskAgain && !asked) {
      asked = true;
      return (await platform.requestPermission()) ? "granted" : "denied";
    }
    return now.status;
  }

  async function run(): Promise<void> {
    if (!signedIn() || platform.unsupported) return;
    await forgetting;
    let permission: PushPermission;
    try {
      permission = await allowed();
    } catch (cause) {
      // The channel or the permission API failed (a build without channels, the system's service down): Settings says so.
      store.setState({ registered: false, error: `Push couldn't start: ${cause instanceof Error ? cause.message : "the phone said no"}` });
      return;
    }
    store.setState({ permission });
    if (permission !== "granted") return;
    let token: string;
    try {
      token = await platform.token();
    } catch {
      store.setState({ registered: false, error: "This build has no push set up" });
      return;
    }
    if (signedIn()) await give(token);
  }

  const store = createStore<PushState>()(() => ({
    permission: null,
    registered: false,
    error: null,
    unsupported: platform.unsupported ?? null,
    sync() {
      running ??= run().finally(() => {
        running = null;
      });
      return running;
    },
  }));

  // A phone without push never asks, listens or sends.
  if (platform.unsupported) return store;
  session.subscribe((now, before) => {
    if (now.status === "signedIn" && before.status !== "signedIn") inBackground(store.getState().sync(), "registration: sync");
    if (now.status === "signedOut" && before.status === "signedIn") {
      // Signed out here, however it came about: this phone's token stops working, so no push reaches it signed out.
      sent = null;
      store.setState({ registered: false, error: null });
      forgetting = platform.forget().catch(() => undefined);
    }
  });
  platform.onTokenChange((token) => {
    if (signedIn() && store.getState().permission === "granted") inBackground(give(token), "registration: give");
  });

  return store;
}
