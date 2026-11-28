import { createStore } from "zustand/vanilla";
import {
  checkServerAddress,
  firstReachable,
  type AuthEvent,
  type AuthSession,
  type Reach,
  type ServerAddress,
  type Slot,
  type EmberhutApi,
} from "@/api";
import type { PairingLink } from "@/connect/pairingLink";

/**
 * What a pairing is for:
 * - server: this phone's server, replacing the one it had (pairing as it always was);
 * - switch: a new server for this phone, the one it had kept as its fallback server, with its session;
 * - fallback: the fallback server, the one the app goes back to when its server stops answering.
 */
export type PairRole = "server" | "switch" | "fallback";

/** The server this phone goes back to when its own stops answering (src/state/fallback.ts), with its own session. */
export interface FallbackServer {
  url: string;
  /** It speaks this build's wire dialect (ServerAddress.speaksDialect). */
  speaksDialect: boolean;
  /** Its session is kept: false once it signed this phone out, until it is paired again. */
  signedIn: boolean;
  /** The id its key proved at pairing; null without one. */
  machineId: string | null;
}

/** A machine picked from the account's list: the next pairing must prove it is this one. */
export interface PinnedMachine {
  id: string;
  name: string;
}

export interface SessionState {
  /** starting: secure storage not read yet. */
  status: "starting" | "signedIn" | "signedOut";
  /** The server enforces sign-in: the app can't be used without pairing. */
  required: boolean;
  /** The server's address, as you typed it at pairing; null until then. */
  serverUrl: string | null;
  /** Not on the server's network as its owner: the server lets nothing through. */
  refused: boolean;
  /** You said not now (only while the server doesn't enforce it). */
  later: boolean;
  device: number | null;
  /** What this device calls itself until you name it while pairing. */
  deviceName: string;
  pairing: boolean;
  pairError: string | null;
  /** Why the saved sign-in couldn't be read at start (secure storage failed); the sign-in screen offers a retry. */
  startError: string | null;
  /** The id of the machine this phone paired with, from the key its answer proved; null without one. */
  machineId: string | null;
  /** The machine the next pairing must be, when it was picked from the account; null for any. */
  pin: PinnedMachine | null;
  /** The server speaks this build's wire dialect (ServerAddress.speaksDialect). */
  speaksDialect: boolean;
  /** The fallback server, or null without one. */
  fallback: FallbackServer | null;
  /** Reads the kept server and sign-in, then asks the server whether it enforces sign-in. Never rejects. */
  start: () => Promise<void>;
  /**
   * Redeems a pairing code; `name` is how the server's device list will show this one, `server` the address you typed
   * (kept from then on; the one already kept when you give none), `role` what the server is to be.
   */
  pair: (code: string, name?: string, server?: string, role?: PairRole) => Promise<boolean>;
  /**
   * Pairs from a pairing link (scanned or pasted): over the first of its routes that answers, and only with the machine
   * whose id the link names.
   */
  pairLink: (link: PairingLink, name?: string, role?: PairRole) => Promise<boolean>;
  /** Whether the server (or the fallback server) speaks this build's wire dialect. */
  setSpeaksDialect: (slot: Slot, on: boolean) => Promise<void>;
  /** Signs out of the fallback server (best effort) and forgets it here. */
  removeFallback: () => Promise<void>;
  /**
   * Pins the machine the next pairing must be (a machine picked from the account), or unpins it. Pinning one asks for
   * pairing now: a Not now said before no longer holds.
   */
  pinMachine: (pin: PinnedMachine | null) => void;
  /** Signs this device out on the server (best effort), then forgets it here. */
  signOut: () => Promise<void>;
  /** Not now: only once a server is kept (there's nothing to show without one) and while it doesn't enforce sign-in. */
  signInLater: () => void;
}

export interface SessionDeps {
  auth: AuthSession;
  api: EmberhutApi;
  /** Where the server is: auth and api send to it. */
  address: ServerAddress;
  device: { name: string; model: string };
  /** Whether the machine answers at a route: a link's routes are probed with it. Without it no route answers. */
  reach?: Reach;
  /** The fallback server's place: its address and its own session. Without it the app has no fallback server. */
  fallback?: { auth: AuthSession; address: ServerAddress };
  /** Revokes a device on one server, with that server's session (POST /api/settings authRevoke). */
  revoke?: (slot: Slot, device: number) => Promise<unknown>;
}

export function createSessionStore({ auth, api, address, device, reach = async () => "none", fallback, revoke }: SessionDeps) {
  const revokeOn = revoke ?? ((_slot: Slot, id: number) => api.devices.revoke(id));

  /** The fallback server as the store shows it. */
  const fallbackNow = (): FallbackServer | null =>
    fallback?.address.url
      ? {
          url: fallback.address.url,
          speaksDialect: fallback.address.speaksDialect,
          signedIn: fallback.auth.signedIn,
          machineId: fallback.auth.machine?.id ?? null,
        }
      : null;

  /** Why a pairing for `role` at `url` can't start, or null. */
  function refusal(role: PairRole, url: string): string | null {
    if (role === "server") return null;
    if (fallback === undefined) return "This app has no fallback server.";
    if (url === address.url) return "That is this phone's server already.";
    if (role === "switch" && (address.url === null || !auth.signedIn)) return "Pair this phone with its server first.";
    return null;
  }

  /**
   * Redeems the code at `url`. The address is saved only once the answer is accepted, before its tokens are held: a
   * refused pairing (a wrong code, another machine) leaves the kept addresses as they were.
   */
  async function pairAt(code: string, name: string | undefined, url: string, expected: string | null, role: PairRole): Promise<boolean> {
    const named = { ...device, name: name?.trim() || device.name };
    let outcome;
    if (role === "fallback" && fallback !== undefined) {
      const commit = async () => {
        if (url !== fallback.address.url) await fallback.address.save(url);
      };
      outcome = await fallback.auth.pair(code, named, { url, expected, commit });
    } else {
      const commit = async () => {
        if (role === "switch" && fallback !== undefined && address.url !== null) {
          // The server this phone had, and its session as it is now, become the fallback's; the new server's tokens
          // replace the session only after this.
          const kept = await auth.snapshot();
          if (kept === null) throw new Error("No session to keep");
          await fallback.auth.adopt(kept);
          await fallback.address.save(address.url);
          await fallback.address.setSpeaksDialect(address.speaksDialect);
          // A server paired to take over from one that is kept as the fallback is a new one: it speaks the app's words.
          await address.setSpeaksDialect(false);
        }
        if (url === address.url) return;
        await address.save(url);
        store.setState({ serverUrl: url });
      };
      outcome = await auth.pair(code, named, { url, expected, commit });
    }
    store.setState(
      outcome.ok
        ? { pairError: null, pin: null, fallback: fallbackNow(), speaksDialect: address.speaksDialect }
        : { pairError: outcome.error, fallback: fallbackNow() },
    );
    return outcome.ok;
  }

  const store = createStore<SessionState>()((set, get) => ({
    status: "starting",
    required: false,
    serverUrl: null,
    refused: false,
    later: false,
    device: null,
    deviceName: device.name,
    pairing: false,
    pairError: null,
    startError: null,
    machineId: null,
    pin: null,
    speaksDialect: true,
    fallback: null,

    async start() {
      set({ startError: null });
      try {
        set({ serverUrl: await address.load(), speaksDialect: address.speaksDialect });
        if (address.url !== null) await auth.load();
        if (fallback !== undefined && (await fallback.address.load()) !== null) await fallback.auth.load();
        set({ fallback: fallbackNow() });
      } catch {
        // The Keychain before the first unlock, a Keystore entry it can't decrypt: sign-in shows why, and Try again reads
        // again (nothing is deleted: a locked Keychain opens later, and pairing replaces whatever is unreadable).
        set({ status: "signedOut", startError: "Couldn't read this phone's saved sign-in." });
        return;
      }
      if (address.url === null) {
        set({ status: "signedOut" });
        return;
      }
      set({ status: auth.signedIn ? "signedIn" : "signedOut", device: auth.device, machineId: auth.machine?.id ?? null });
      // Under enforce, a phone without a session goes straight to sign-in, with no "Not now".
      const server = await api.auth.status().catch(() => null);
      if (server) set({ required: server.mode === "enforce" && !server.signedIn });
    },

    async pair(code, name, server, role = "server") {
      if (get().pairing) return false;
      const check = server === undefined ? null : checkServerAddress(server);
      if (check !== null && !check.ok) {
        set({ pairError: check.error });
        return false;
      }
      const url = check?.url ?? address.url;
      if (url === null) {
        set({ pairError: "Type the server's address first." });
        return false;
      }
      const refused = refusal(role, url);
      if (refused !== null) {
        set({ pairError: refused });
        return false;
      }
      set({ pairing: true, pairError: null });
      try {
        return await pairAt(code, name, url, get().pin?.id ?? null, role);
      } finally {
        set({ pairing: false });
      }
    },

    async pairLink(link, name, role = "server") {
      if (get().pairing) return false;
      const pin = get().pin;
      if (pin !== null && pin.id !== link.id) {
        set({ pairError: `That pairing link is for another hut, not ${pin.name}.` });
        return false;
      }
      const routes = directRoutes(link.routes);
      if (routes.length === 0) {
        set({ pairError: "None of the link's addresses can be reached from this phone. Type the server's address and the code instead." });
        return false;
      }
      set({ pairing: true, pairError: null });
      try {
        const url = await firstReachable(routes, link.id, reach);
        if (url === null) {
          set({ pairError: "Can't reach the hut at any address in the link. Is this phone on its network?" });
          return false;
        }
        const refused = refusal(role, url);
        if (refused !== null) {
          set({ pairError: refused });
          return false;
        }
        return await pairAt(link.code, name, url, link.id, role);
      } finally {
        set({ pairing: false });
      }
    },

    pinMachine(pin) {
      set(pin === null ? { pin, pairError: null } : { pin, pairError: null, later: false });
    },

    async signOut() {
      // Signed out of Emberhut: of the fallback server too, so no session is left on this phone.
      await get().removeFallback();
      const id = auth.device;
      if (id !== null) await revokeOn("server", id).catch(() => undefined);
      await auth.signOut();
    },

    async setSpeaksDialect(slot, on) {
      if (slot === "server") {
        await address.setSpeaksDialect(on);
      } else {
        if (!fallback?.address.url) return;
        await fallback.address.setSpeaksDialect(on);
      }
      set({ speaksDialect: address.speaksDialect, fallback: fallbackNow() });
    },

    async removeFallback() {
      if (fallback === undefined || fallback.address.url === null) return;
      const id = fallback.auth.device;
      if (id !== null && fallback.auth.signedIn) await revokeOn("fallback", id).catch(() => undefined);
      await fallback.auth.clear();
      await fallback.address.clear();
      set({ fallback: null });
    },

    signInLater() {
      if (!get().required && get().serverUrl !== null) set({ later: true });
    },
  }));

  auth.subscribe((event: AuthEvent) => {
    switch (event.kind) {
      case "signedIn":
        api.state.reset();
        store.setState({
          status: "signedIn",
          device: event.device,
          machineId: auth.machine?.id ?? null,
          required: false,
          later: false,
          startError: null,
        });
        break;
      case "signedOut":
        api.state.reset();
        store.setState((s) => ({
          status: "signedOut",
          device: null,
          required: event.reason === "required" || (event.reason === "expired" && s.required),
        }));
        break;
      case "refused":
        store.setState({ refused: true });
        break;
      case "reachable":
        store.setState({ refused: false });
        break;
    }
  });

  // The fallback server signing this phone out ends only its session: the app goes on with its server.
  fallback?.auth.subscribe((event) => {
    if (event.kind === "signedIn" || event.kind === "signedOut") store.setState({ fallback: fallbackNow() });
  });

  return store;
}

/**
 * A link's routes this phone can pair over, in order and each once: http(s) addresses that pass the typed address's rule
 * (https, or plain http only to a private address). A relay route carries a sealed session, which this app doesn't open
 * yet, so it is left out.
 */
export function directRoutes(routes: readonly string[]): string[] {
  const usable = routes.flatMap((route) => {
    if (!/^https?:\/\//i.test(route)) return [];
    const check = checkServerAddress(route);
    return check.ok ? [check.url] : [];
  });
  return [...new Set(usable)];
}

/** The sign-in screen shows before anything else. */
export const needsSignIn = (s: SessionState): boolean => s.status === "signedOut" && (s.required || !s.later);
