import { afterEach, describe, expect, it, vi } from "vitest";
import {
  AuthSession,
  createApi,
  createDialect,
  createHealthCheck,
  createServerAddress,
  createServerHttp,
  parseDialect,
  slotPrefix,
  type Fetch,
} from "@/api";
import { checkEveryMs, createFallbackStore, failuresToFallBack } from "@/state/fallback";
import { createSessionStore } from "@/state/session";
import { json, memoryStorage, tokens } from "@/test/fakes";

const old = "http://192.168.1.10:18990";
const fresh = "http://192.168.1.11:18990";
const device = { name: "Ada's iPhone", model: "iPhone" };
const start = Date.parse("2026-11-24T10:00:00Z");
// The build's dialect: the old server spells the owner "person".
const dialect = createDialect(parseDialect(JSON.stringify({ words: [["person", "owner"]] })));

interface Request {
  url: URL;
  method: string;
  headers: Record<string, string>;
  body: unknown;
}

/** Stand-in servers by origin: each answers its own routes, and one that is down answers nothing at all. */
function network() {
  const sent: Request[] = [];
  const routes = new Map<string, (request: Request) => Response>();
  const down = new Set<string>();
  const fetch: Fetch = async (url, init) => {
    const headers = Object.fromEntries(new Headers(init.headers).entries());
    const body = typeof init.body === "string" ? JSON.parse(init.body) : init.body;
    const request = { url: new URL(url), method: init.method ?? "GET", headers, body };
    sent.push(request);
    if (down.has(request.url.origin)) throw new TypeError("Network request failed");
    const route = routes.get(`${request.url.origin} ${request.method} ${request.url.pathname}`);
    return route ? route(request) : json(404, { error: "not found" });
  };
  return {
    fetch,
    sent,
    down,
    on(origin: string, method: string, path: string, answer: (request: Request) => Response) {
      routes.set(`${origin} ${method} ${path}`, answer);
    },
    to: (origin: string, path: string) => sent.filter((r) => r.url.origin === origin && r.url.pathname === path),
  };
}

/** A phone as the app builds it (services.ts), on stand-in servers: `old` and `fresh` answer /health and pairing. */
function phone(kept: Record<string, string> = {}) {
  const net = network();
  for (const [origin, n] of [
    [old, 1],
    [fresh, 2],
  ] as const) {
    net.on(origin, "GET", "/health", () => new Response("ok"));
    net.on(origin, "POST", "/api/auth/pair", () => json(200, { ...tokens(n, start), device: n * 10 }));
    // A renewal turns the refresh token over and keeps the device.
    net.on(origin, "POST", "/api/auth/refresh", () => json(200, { ...tokens(n, start), refresh: `renewed-${n}`, device: undefined }));
    net.on(origin, "POST", "/api/settings", () => json(200, { ok: true }));
  }
  const storage = memoryStorage(kept);
  let ids = 0;
  const session = (prefix: string) => {
    const address = createServerAddress(storage, prefix);
    const auth = new AuthSession({
      baseUrl: () => address.url ?? "http://server.invalid",
      fetch: net.fetch,
      storage,
      newId: () => `install-${++ids}`,
      now: () => start,
      prefix,
    });
    return { address, auth };
  };
  const places = { server: session(slotPrefix.server), fallback: session(slotPrefix.fallback) };
  const switches: string[] = [];
  const http = createServerHttp({ places, using: () => fallback.getState().using, fetch: net.fetch, dialect });
  const api = createApi(http.inUse);
  const store = createSessionStore({
    auth: places.server.auth,
    api,
    address: places.server.address,
    device,
    fallback: places.fallback,
    revoke: (slot, id) => createApi(http.own[slot]).devices.revoke(id),
  });
  const fallback = createFallbackStore({
    session: store,
    health: createHealthCheck(net.fetch),
    onSwitch: (using) => switches.push(using),
    sleep: async () => undefined,
  });
  return { net, storage, places, http, api, store, fallback, switches };
}

/** A phone paired with the old server (it speaks the build's dialect, by default) that has the new one as its server. */
async function switched() {
  const p = phone({ "emberhut.server": old, "emberhut.refresh": "refresh-old", "emberhut.device": "5", "emberhut.install": "i" });
  await p.store.getState().start();
  await p.fallback.getState().launch(); // the launch check run, as the app's start runs it
  expect(await p.store.getState().pair("ABCD-EFGH", undefined, fresh, "switch")).toBe(true);
  return p;
}

describe("pairing a new server and keeping the old one as the fallback", () => {
  it("moves the old server and its session to the fallback's place, and pairs at the new one", async () => {
    const { net, storage, store } = await switched();
    expect(net.to(fresh, "/api/auth/pair")).toHaveLength(1);
    expect(net.to(old, "/api/auth/pair")).toEqual([]);
    expect(store.getState()).toMatchObject({
      status: "signedIn",
      serverUrl: fresh,
      device: 20,
      speaksDialect: false,
      fallback: { url: old, speaksDialect: true, signedIn: true },
    });
    expect(Object.fromEntries(storage.items)).toMatchObject({
      "emberhut.server": fresh,
      "emberhut.refresh": "refresh-2",
      "emberhut.device": "20",
      "emberhut.dialect": "off",
      "emberhut.fallback.server": old,
      "emberhut.fallback.refresh": "renewed-1", // as it was after the launch's renewal
      "emberhut.fallback.device": "5",
      "emberhut.install": "i",
    });
    expect(storage.items.has("emberhut.fallback.dialect")).toBe(false);
  });

  it("keeps the old session as it is after a renewal in flight turns it over", async () => {
    const p = phone({ "emberhut.server": old, "emberhut.refresh": "refresh-old", "emberhut.install": "i" });
    p.net.on(old, "POST", "/api/auth/refresh", () => json(200, tokens(9, start)));
    await p.store.getState().start();
    await p.api.http.send("/api/facts"); // renews the old session first
    expect(await p.store.getState().pair("ABCD-EFGH", undefined, fresh, "switch")).toBe(true);
    expect(p.storage.items.get("emberhut.fallback.refresh")).toBe("refresh-9");
  });

  it("reads both places back at the next launch", async () => {
    const { storage } = await switched();
    const next = phone(Object.fromEntries(storage.items));
    await next.store.getState().start();
    expect(next.store.getState()).toMatchObject({
      serverUrl: fresh,
      speaksDialect: false,
      fallback: { url: old, speaksDialect: true, signedIn: true },
    });
  });

  it("leaves both as they were when the new server refuses the code", async () => {
    const p = phone({ "emberhut.server": old, "emberhut.refresh": "refresh-old", "emberhut.install": "i" });
    p.net.on(fresh, "POST", "/api/auth/pair", () => json(401, { ok: false, error: "Wrong code." }));
    await p.store.getState().start();
    expect(await p.store.getState().pair("ABCD-EFGH", undefined, fresh, "switch")).toBe(false);
    expect(p.store.getState()).toMatchObject({ serverUrl: old, fallback: null, pairError: "Wrong code." });
    expect([...p.storage.items.keys()].some((key) => key.startsWith("emberhut.fallback."))).toBe(false);
  });

  it("needs a server with a session to keep, and another address", async () => {
    const empty = phone();
    await empty.store.getState().start();
    expect(await empty.store.getState().pair("ABCD-EFGH", undefined, fresh, "switch")).toBe(false);
    expect(empty.store.getState().pairError).toBe("Pair this phone with its server first.");

    const p = phone({ "emberhut.server": old, "emberhut.refresh": "refresh-old", "emberhut.install": "i" });
    await p.store.getState().start();
    expect(await p.store.getState().pair("ABCD-EFGH", undefined, old, "switch")).toBe(false);
    expect(p.store.getState().pairError).toBe("That is this phone's server already.");
    expect(p.net.to(old, "/api/auth/pair")).toEqual([]);
  });
});

describe("pairing a fallback server", () => {
  it("pairs it in its own place and leaves the server as it is", async () => {
    const p = phone({ "emberhut.server": fresh, "emberhut.refresh": "refresh-fresh", "emberhut.install": "i" });
    await p.store.getState().start();
    expect(await p.store.getState().pair("ABCD-EFGH", undefined, old, "fallback")).toBe(true);
    expect(p.store.getState()).toMatchObject({ serverUrl: fresh, fallback: { url: old, signedIn: true, speaksDialect: true } });
    expect(p.storage.items.get("emberhut.refresh")).toBe("renewed-2");
    expect(p.storage.items.get("emberhut.fallback.refresh")).toBe("refresh-1");
    expect(p.net.to(old, "/api/auth/pair")[0]?.body).toMatchObject({ install: "i", name: device.name });
  });

  it("marks which server speaks the build's dialect", async () => {
    const { storage, store } = await switched();
    await store.getState().setSpeaksDialect("fallback", false);
    await store.getState().setSpeaksDialect("server", true);
    expect(store.getState()).toMatchObject({ speaksDialect: true, fallback: { speaksDialect: false } });
    expect(storage.items.get("emberhut.fallback.dialect")).toBe("off");
    expect(storage.items.has("emberhut.dialect")).toBe(false);
  });

  it("is removed with its session, revoked there; signing out removes it too", async () => {
    const p = await switched();
    await p.store.getState().removeFallback();
    expect(p.net.to(old, "/api/settings")[0]?.body).toEqual({ authRevoke: 5 });
    expect(p.store.getState().fallback).toBeNull();
    expect([...p.storage.items.keys()].some((key) => key.startsWith("emberhut.fallback."))).toBe(false);

    const q = await switched();
    await q.store.getState().signOut();
    expect(q.net.to(old, "/api/settings")[0]?.body).toEqual({ authRevoke: 5 });
    expect(q.net.to(fresh, "/api/settings")[0]?.body).toEqual({ authRevoke: 20 });
    expect(q.store.getState()).toMatchObject({ status: "signedOut", fallback: null });
  });
});

describe("going to the fallback server", () => {
  afterEach(() => vi.useRealTimers());

  it("goes after three failed health checks in a row, and requests then go there with its session and words", async () => {
    const p = await switched();
    // Only the old server speaks the build's dialect: the new one's "person" is a word like any other.
    p.net.on(fresh, "GET", "/api/facts", () => json(200, { facts: [], who: "person" }));
    p.net.on(old, "GET", "/api/facts", () => json(200, { facts: [], who: "person" }));
    expect(await p.api.http.get("/api/facts")).toEqual({ facts: [], who: "person" });

    p.net.down.add(fresh);
    for (let n = 1; n < failuresToFallBack; n++) {
      await p.fallback.getState().check();
      expect(p.fallback.getState()).toMatchObject({ using: "server", failures: n });
    }
    await p.fallback.getState().check();
    expect(p.fallback.getState()).toMatchObject({ using: "fallback", failures: 0 });
    expect(p.switches).toEqual(["fallback"]);

    expect(await p.api.http.get("/api/facts")).toEqual({ facts: [], who: "owner" });
    expect(p.net.to(old, "/api/facts")[0]?.headers.authorization).toBe("Bearer access-1");
  });

  it("starts counting again after a check that answers", async () => {
    const p = await switched();
    p.net.down.add(fresh);
    await p.fallback.getState().check();
    await p.fallback.getState().check();
    p.net.down.delete(fresh);
    await p.fallback.getState().check();
    p.net.down.add(fresh);
    await p.fallback.getState().check();
    expect(p.fallback.getState()).toMatchObject({ using: "server", failures: 1 });
  });

  it("counts a server error or no answer in time as a failure", async () => {
    vi.useFakeTimers();
    const net = network();
    net.on(fresh, "GET", "/health", () => json(503, {}));
    expect(await createHealthCheck(net.fetch)(fresh)).toBe(false);
    const hanging: Fetch = (_url, init) =>
      new Promise((_resolve, reject) => init.signal?.addEventListener("abort", () => reject(new Error("aborted"))));
    const answer = createHealthCheck(hanging, 4_000)(fresh);
    await vi.advanceTimersByTimeAsync(4_000);
    expect(await answer).toBe(false);
  });

  it("at launch, goes when the server doesn't answer after two quick retries", async () => {
    const { storage } = await switched();
    const p = phone(Object.fromEntries(storage.items));
    p.net.down.add(fresh);
    await p.store.getState().start();
    await vi.waitFor(() => expect(p.fallback.getState().using).toBe("fallback"));
    expect(p.net.to(fresh, "/health")).toHaveLength(3);
  });

  it("at launch, stays when the server answers a retry", async () => {
    const { storage } = await switched();
    const p = phone(Object.fromEntries(storage.items));
    let calls = 0;
    p.net.on(fresh, "GET", "/health", () => (++calls < 2 ? json(503, {}) : new Response("ok")));
    await p.store.getState().start();
    await vi.waitFor(() => expect(calls).toBe(2));
    await p.fallback.getState().launch();
    expect(p.fallback.getState().using).toBe("server");
  });

  it("never checks without a fallback server, or once it signed this phone out", async () => {
    const p = phone({ "emberhut.server": fresh, "emberhut.refresh": "refresh-fresh", "emberhut.install": "i" });
    p.net.down.add(fresh);
    await p.store.getState().start();
    for (let n = 0; n < 5; n++) await p.fallback.getState().check();
    expect(p.net.to(fresh, "/health")).toEqual([]);
    expect(p.fallback.getState().using).toBe("server");

    const q = await switched();
    q.net.on(old, "POST", "/api/auth/refresh", () => json(401, { ok: false }));
    await q.http.own.fallback.send("/api/facts");
    expect(q.store.getState()).toMatchObject({ status: "signedIn", fallback: { url: old, signedIn: false } });
    q.net.down.add(fresh);
    for (let n = 0; n < 5; n++) await q.fallback.getState().check();
    expect(q.fallback.getState().using).toBe("server");
  });

  it("checks every 15 s only while the app is in front", async () => {
    vi.useFakeTimers();
    const p = await switched();
    p.net.down.add(fresh);
    p.fallback.getState().foreground(true);
    await vi.advanceTimersByTimeAsync(checkEveryMs * 2);
    expect(p.net.to(fresh, "/health")).toHaveLength(2);
    p.fallback.getState().foreground(false);
    await vi.advanceTimersByTimeAsync(checkEveryMs * 5);
    expect(p.net.to(fresh, "/health")).toHaveLength(2);
    expect(p.fallback.getState()).toMatchObject({ using: "server", failures: 0 });
    p.fallback.getState().foreground(true);
    await vi.advanceTimersByTimeAsync(checkEveryMs * 3);
    expect(p.fallback.getState().using).toBe("fallback");
    p.fallback.getState().foreground(false);
  });
});

describe("going back to the server", () => {
  async function onFallback() {
    const p = await switched();
    p.net.down.add(fresh);
    for (let n = 0; n < failuresToFallBack; n++) await p.fallback.getState().check();
    expect(p.fallback.getState().using).toBe("fallback");
    return p;
  }

  it("never happens by itself, even once the server answers again", async () => {
    const p = await onFallback();
    p.net.down.delete(fresh);
    const checked = p.net.to(fresh, "/health").length;
    for (let n = 0; n < 5; n++) await p.fallback.getState().check();
    expect(p.fallback.getState().using).toBe("fallback");
    expect(p.net.to(fresh, "/health")).toHaveLength(checked);
  });

  it("Try again stays on the fallback while the server doesn't answer, and goes back once it does", async () => {
    const p = await onFallback();
    expect(await p.fallback.getState().tryAgain()).toBe(false);
    expect(p.fallback.getState()).toMatchObject({ using: "fallback", stillDown: true, checking: false });

    p.net.down.delete(fresh);
    expect(await p.fallback.getState().tryAgain()).toBe(true);
    expect(p.fallback.getState()).toMatchObject({ using: "server", stillDown: false, failures: 0 });
    expect(p.switches).toEqual(["fallback", "server"]);
    p.net.on(fresh, "GET", "/api/facts", () => json(200, { facts: [] }));
    await p.api.http.get("/api/facts");
    expect(p.net.to(fresh, "/api/facts")[0]?.headers.authorization).toBe("Bearer access-2");
  });

  it("happens when the fallback server is removed, or the phone signs out", async () => {
    const p = await onFallback();
    await p.store.getState().removeFallback();
    expect(p.fallback.getState().using).toBe("server");

    const q = await onFallback();
    await q.store.getState().signOut();
    expect(q.fallback.getState().using).toBe("server");
  });
});
