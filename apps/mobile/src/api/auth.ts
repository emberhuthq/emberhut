import { checkMachine, notTheMachine, type MachineKey } from "@/connect/machineId";
import { reasonOf, StorageError } from "./errors";
import { buildUrl, readJson, timeoutSignal, type Fetch } from "./fetch";
import type { PairRequest, Tokens, Unauthorized } from "./types";

/** Where the device's refresh token, install id and device number live: expo-secure-store in the app. */
export interface SecureStorage {
  getItem(key: string): Promise<string | null>;
  setItem(key: string, value: string): Promise<void>;
  deleteItem(key: string): Promise<void>;
}

export type AuthEvent =
  | { kind: "signedIn"; device: number | null }
  /** expired: the server refused the refresh token; required: the server enforces sign-in and this device has none. */
  | { kind: "signedOut"; reason: "expired" | "required" | "local" }
  /** The server let nothing through: this phone is not on the server's network as its owner. */
  | { kind: "refused" }
  /** A request got through again after a refusal. */
  | { kind: "reachable" };

export type PairOutcome = { ok: true } | { ok: false; error: string };

/** Where and with what a pairing is made; by default at the kept server, with no machine named. */
export interface PairOptions {
  /** The server to pair at; the kept one when left out. */
  url?: string;
  /** The machine id the answer must prove (a link's, or a machine picked from the account). */
  expected?: string | null;
  /**
   * Called once the answer is accepted and before any of its tokens are held: where the caller saves the new server's
   * address, so no token is ever sent to the old one. A failure ends the pairing.
   */
  commit?: () => Promise<void>;
}

/** The machine this phone paired with: its id and key, pinned, and the address it was paired at. */
export interface KeptMachine extends MachineKey {
  url: string;
}

/** A session as secure storage keeps it, to move from one server's place to the other's (src/api/server.ts, Slot). */
export interface KeptSession {
  refresh: string;
  device: number | null;
  machine: KeptMachine | null;
}

export interface AuthSessionOptions {
  /** The server, as it is now: it's typed in at pairing. */
  baseUrl: () => string;
  fetch: Fetch;
  storage: SecureStorage;
  newId: () => string;
  now?: () => number;
  /** Where a failure with no caller to tell goes (the app's log); dropped when unset. */
  log?: (where: string, cause: unknown) => void;
  /**
   * The prefix of the keys this session is kept under (slotPrefix in src/api/server.ts): this phone's server's by default.
   * The install id is the phone's own, one for every server.
   */
  prefix?: string;
}

const sessionKeys = (prefix = "emberhut.") => ({
  refresh: `${prefix}refresh`,
  install: "emberhut.install",
  device: `${prefix}device`,
  machine: `${prefix}machine`,
});
/** Renew this long before the access token runs out. */
const margin = 60_000;
/** When the server gives no expiry: a bit under its one hour. */
const fallbackLife = 3_000_000;
/** A renewal waits this long for the server: past a slow loop, short of a connection that died. */
const renewalTimeoutMs = 120_000;

/**
 * This device's session: a short-lived access token in memory, and a refresh token in secure storage that the server
 * turns over on every use. Only one renewal is ever in flight: a second call sending the same refresh token, or a renewal
 * abandoned after the server already turned it over, would sign the device out and make it pair again as a new device.
 * So a renewal never takes a caller's signal; every caller waits on the same one.
 */
export class AuthSession {
  private access: string | null = null;
  private accessUntil = 0;
  private refresh: string | null = null;
  private install: string | null = null;
  private renewing: Promise<void> | null = null;
  private refused = false;
  private loaded: Promise<void> | null = null;
  private readonly listeners = new Set<(event: AuthEvent) => void>();
  private readonly now: () => number;
  private readonly keys: ReturnType<typeof sessionKeys>;
  device: number | null = null;
  /** The machine's key, pinned at pairing; null before, or after pairing with a server that predates machine keys. */
  machine: KeptMachine | null = null;

  constructor(private readonly options: AuthSessionOptions) {
    this.now = options.now ?? Date.now;
    this.keys = sessionKeys(options.prefix);
  }

  /** Reads what secure storage holds, once; a read that fails throws StorageError, and the next call reads again. */
  load(): Promise<void> {
    if (this.loaded) return this.loaded;
    const loading = this.read();
    this.loaded = loading;
    // Forgotten from outside the read, so a storage that throws before its first await is read again too.
    loading.catch(() => {
      if (this.loaded === loading) this.loaded = null;
    });
    return loading;
  }

  /** Forgets what is held in memory and reads secure storage again. */
  reload(): Promise<void> {
    this.dropTokens();
    this.machine = null;
    this.loaded = null;
    return this.load();
  }

  private async read(): Promise<void> {
    const { storage } = this.options;
    try {
      const [refresh, install, device, machine] = await Promise.all([
        storage.getItem(this.keys.refresh),
        storage.getItem(this.keys.install),
        storage.getItem(this.keys.device),
        storage.getItem(this.keys.machine),
      ]);
      this.refresh = refresh;
      this.install = install;
      this.device = device === null ? null : Number(device);
      this.machine = keptMachineFrom(machine);
    } catch (cause) {
      throw new StorageError("Couldn't read this phone's saved sign-in", { cause });
    }
  }

  get signedIn(): boolean {
    return this.refresh !== null;
  }

  subscribe(listener: (event: AuthEvent) => void): () => void {
    this.listeners.add(listener);
    return () => void this.listeners.delete(listener);
  }

  /** The access token as it is now, for URLs a player loads itself; it may be out of date. */
  currentAccess(): string | null {
    return this.access;
  }

  /** A valid access token, renewed first when it is (nearly) out; null without a session or when renewal failed. */
  async accessToken(): Promise<string | null> {
    await this.load();
    if (this.access !== null && this.now() + margin < this.accessUntil) return this.access;
    if (this.refresh === null) return null;
    this.renewing ??= this.renew().finally(() => {
      this.renewing = null;
    });
    await this.renewing;
    return this.access !== null && this.now() < this.accessUntil ? this.access : null;
  }

  /** The install id the server keeps one device row for: made once, kept across launches. */
  async installId(): Promise<string> {
    await this.load();
    if (this.install === null) {
      const made = this.options.newId();
      try {
        await this.options.storage.setItem(this.keys.install, made);
      } catch (cause) {
        throw new StorageError("Couldn't save this phone's install id", { cause });
      }
      this.install = made;
    }
    return this.install;
  }

  /**
   * Redeems a one-time pairing code from Emberhut for this device's session. The answer's machine key must give its id, and
   * that id must be `expected` when given (a link's, or a machine picked from the account). Pairing again at the address
   * a key was pinned for expects that key's machine. With neither, the first key seen is pinned (trust on first use).
   */
  async pair(code: string, device: { name: string; model: string }, options: PairOptions = {}): Promise<PairOutcome> {
    const url = options.url ?? this.options.baseUrl();
    let install: string;
    try {
      install = await this.installId();
    } catch (cause) {
      return { ok: false, error: storageReason(cause) };
    }
    const request: PairRequest = { code: code.trim(), install, ...device };
    let status: number;
    let body: unknown;
    try {
      ({ status, body } = await this.post("/api/auth/pair", request, 30_000, url));
    } catch {
      return { ok: false, error: "Can't reach Emberhut" };
    }
    const tokens = body as Tokens | undefined;
    // The first gate's 401 (not on the server's network) carries no `ok`; the pairing's own refusals of a code do.
    if (status !== 401 || tokens?.ok === undefined) this.noteResponse(status, body);
    if (status !== 200 || tokens?.ok !== true || !tokens.access) return { ok: false, error: reasonOf(body) ?? `HTTP ${status}` };
    const kept = this.machine?.url === url ? this.machine.id : null;
    const machine = checkMachine(tokens.machine, options.expected ?? kept);
    // Not the machine meant: its session is never kept, so nothing is sent to it again.
    if (!machine.ok) return { ok: false, error: notTheMachine };
    try {
      await options.commit?.();
    } catch {
      return { ok: false, error: "Couldn't keep the server's address on this phone. Try again." };
    }
    try {
      await this.keep(tokens);
      // The key is pinned only with the session it came with: a key kept without its tokens would pin a machine this
      // phone never paired with.
      await this.keepMachine(machine.machine === null ? null : { ...machine.machine, url });
    } catch (cause) {
      // Not kept, not signed in: a session that would be gone at the next launch isn't one. Pairing again with the same
      // install replaces the server's row for it.
      await this.forget().catch((failure: unknown) => this.options.log?.("auth: forgetting a half-kept sign-in", failure));
      return { ok: false, error: storageReason(cause) };
    }
    this.emit({ kind: "signedIn", device: this.device });
    return { ok: true };
  }

  /**
   * The session as it is now, once a renewal in flight has settled (a renewal turns the refresh token over); null without
   * one. For moving it to the other server's place (`adopt`).
   */
  async snapshot(): Promise<KeptSession | null> {
    await this.load();
    await this.renewing;
    return this.refresh === null ? null : { refresh: this.refresh, device: this.device, machine: this.machine };
  }

  /** Keeps `session` as this one, in secure storage and then in memory: a session moved here from the other server's place. */
  async adopt(session: KeptSession): Promise<void> {
    const { storage } = this.options;
    try {
      await storage.setItem(this.keys.refresh, session.refresh);
      if (session.device === null) await storage.deleteItem(this.keys.device);
      else await storage.setItem(this.keys.device, String(session.device));
      if (session.machine === null) await storage.deleteItem(this.keys.machine);
      else await storage.setItem(this.keys.machine, JSON.stringify(session.machine));
    } catch (cause) {
      throw new StorageError("Couldn't save the sign-in on this phone", { cause });
    }
    this.access = null;
    this.accessUntil = 0;
    this.refresh = session.refresh;
    this.device = session.device;
    this.machine = session.machine;
  }

  /** Forgets the session and the machine's pinned key, with no event: a fallback server that was removed. */
  async clear(): Promise<void> {
    await this.forget();
    await this.options.storage.deleteItem(this.keys.machine);
    this.machine = null;
  }

  /** Forgets the session on this device. Revoking it on the server first is the caller's (POST /api/settings authRevoke). */
  async signOut(): Promise<void> {
    await this.forget();
    this.emit({ kind: "signedOut", reason: "local" });
  }

  /** Every answer from the server passes here: a 401 tells a missing session from a device that isn't let in at all. */
  noteResponse(status: number, body: unknown): void {
    if (status !== 401) {
      if (this.refused) {
        this.refused = false;
        this.emit({ kind: "reachable" });
      }
      return;
    }
    if ((body as Unauthorized | undefined)?.signIn === true) {
      // With a refresh token in hand, the access token was only stale: the next call renews it.
      if (this.refresh !== null) {
        this.access = null;
        this.accessUntil = 0;
        return;
      }
      this.emit({ kind: "signedOut", reason: "required" });
      return;
    }
    if (!this.refused) {
      this.refused = true;
      this.emit({ kind: "refused" });
    }
  }

  private async renew(): Promise<void> {
    const refresh = this.refresh;
    if (refresh === null) return;
    let status: number;
    let body: unknown;
    try {
      // A long limit, never a caller's: one that fired after the server turned the token over would keep the old one, and
      // the next renewal would be refused. Android's HTTP client has no timeout of its own, so without one a connection
      // that died mid-renewal would hold every request behind it.
      ({ status, body } = await this.post("/api/auth/refresh", { refresh, install: await this.installId() }, renewalTimeoutMs));
    } catch {
      return; // unreachable: the refresh token stays, the next call tries again
    }
    const tokens = body as Tokens | undefined;
    if (status === 200 && tokens?.ok === true && tokens.access) {
      // Held in memory either way: this launch goes on with the turned-over token even when the Keychain won't keep it.
      await this.keep(tokens).catch((cause: unknown) => this.options.log?.("auth: keeping the renewed sign-in", cause));
      return;
    }
    // Only the refresh route's own refusal ({ ok: false }) ends the session. The gate in front of it answers 401 without
    // `ok` for a device it doesn't let in (a network check that failed for a moment): the refresh token stays.
    if (status === 401 && tokens?.ok === false) {
      await this.forget();
      this.emit({ kind: "signedOut", reason: "expired" });
      return;
    }
    this.noteResponse(status, body);
  }

  /** Holds the new tokens; throws StorageError when the refresh token can't be kept (it's held in memory all the same). */
  private async keep(tokens: Tokens): Promise<void> {
    const { storage } = this.options;
    this.access = tokens.access ?? null;
    const expires = tokens.expires ? Date.parse(tokens.expires) : NaN;
    this.accessUntil = Number.isNaN(expires) ? this.now() + fallbackLife : expires;
    if (tokens.refresh) this.refresh = tokens.refresh;
    if (tokens.device !== undefined) this.device = tokens.device;
    try {
      if (tokens.refresh) await storage.setItem(this.keys.refresh, tokens.refresh);
      if (tokens.device !== undefined) await storage.setItem(this.keys.device, String(tokens.device));
    } catch (cause) {
      throw new StorageError("Couldn't save the sign-in on this phone", { cause });
    }
  }

  /** Pins the machine's key (or forgets it, for a server that has none); held only once secure storage has it. */
  private async keepMachine(machine: KeptMachine | null): Promise<void> {
    try {
      if (machine === null) await this.options.storage.deleteItem(this.keys.machine);
      else await this.options.storage.setItem(this.keys.machine, JSON.stringify(machine));
    } catch (cause) {
      throw new StorageError("Couldn't save the sign-in on this phone", { cause });
    }
    this.machine = machine;
  }

  private dropTokens(): void {
    this.access = null;
    this.accessUntil = 0;
    this.refresh = null;
    this.device = null;
  }

  private async forget(): Promise<void> {
    this.dropTokens();
    await Promise.all([this.options.storage.deleteItem(this.keys.refresh), this.options.storage.deleteItem(this.keys.device)]);
  }

  private async post(
    path: string,
    json: unknown,
    timeoutMs = 30_000,
    baseUrl = this.options.baseUrl(),
  ): Promise<{ status: number; body: unknown }> {
    const limit = timeoutSignal(timeoutMs);
    try {
      const response = await this.options.fetch(buildUrl(baseUrl, path), {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(json),
        signal: limit.signal,
      });
      return { status: response.status, body: await readJson(response) };
    } finally {
      limit.done();
    }
  }

  private emit(event: AuthEvent): void {
    for (const listener of this.listeners) listener(event);
  }
}

/** The kept machine as secure storage holds it, or null when there is none (or it can't be read as one). */
function keptMachineFrom(text: string | null): KeptMachine | null {
  if (text === null) return null;
  try {
    const value = JSON.parse(text) as Partial<KeptMachine> | null;
    return typeof value?.id === "string" && typeof value.publicKey === "string" && typeof value.url === "string"
      ? { id: value.id, publicKey: value.publicKey, url: value.url }
      : null;
  } catch {
    return null;
  }
}

/** A storage failure in words for the sign-in screen. */
const storageReason = (cause: unknown): string =>
  `${cause instanceof StorageError ? cause.message : "Couldn't use this phone's secure storage"}. Try again.`;
