import { describe, expect, it, vi } from "vitest";
import { AuthSession, StorageError, type AuthEvent } from "@/api";
import { fakeApi, flakyStorage, json, memoryStorage, tokens } from "@/test/fakes";

const signedIn = () => memoryStorage({ "emberhut.refresh": "refresh-0", "emberhut.install": "install-a", "emberhut.device": "7" });

describe("AuthSession", () => {
  it("renews an expired access token with the refresh token and install id, and keeps the rotated refresh token", async () => {
    const { server, storage, clock, auth } = fakeApi({ storage: signedIn() });
    let n = 0;
    server.on("POST", "/api/auth/refresh", () => json(200, tokens(++n, clock.now)));

    expect(await auth.accessToken()).toBe("access-1");
    expect(server.to("/api/auth/refresh")[0]?.body).toEqual({ refresh: "refresh-0", install: "install-a" });
    expect(storage.items.get("emberhut.refresh")).toBe("refresh-1");

    // Valid for an hour: no renewal until it is nearly out, then the new refresh token goes.
    clock.now += 30 * 60_000;
    expect(await auth.accessToken()).toBe("access-1");
    clock.now += 29.5 * 60_000;
    expect(await auth.accessToken()).toBe("access-2");
    expect(server.to("/api/auth/refresh").map((r) => (r.body as { refresh: string }).refresh)).toEqual(["refresh-0", "refresh-1"]);
  });

  it("shares one renewal between concurrent calls", async () => {
    const { server, clock, auth, api } = fakeApi({ storage: signedIn() });
    let release!: () => void;
    const gate = new Promise<void>((resolve) => (release = resolve));
    server.on("POST", "/api/auth/refresh", async () => {
      await gate;
      return json(200, tokens(1, clock.now));
    });
    server.on("GET", "/api/facts", () => json(200, { facts: [] }));

    const calls = Promise.all([api.settings.facts(), api.settings.facts(), auth.accessToken(), api.settings.facts()]);
    await new Promise((resolve) => setTimeout(resolve, 10));
    release();
    await calls;

    expect(server.to("/api/auth/refresh")).toHaveLength(1);
    expect(server.to("/api/facts").map((r) => r.headers.authorization)).toEqual(["Bearer access-1", "Bearer access-1", "Bearer access-1"]);
  });

  it("signs out when the server refuses the refresh token", async () => {
    const { server, storage, auth } = fakeApi({ storage: signedIn() });
    const events: AuthEvent[] = [];
    auth.subscribe((e) => events.push(e));
    server.on("POST", "/api/auth/refresh", () => json(401, { ok: false, status: 401, error: "Sign in again." }));

    expect(await auth.accessToken()).toBeNull();
    expect(auth.signedIn).toBe(false);
    expect(storage.items.has("emberhut.refresh")).toBe(false);
    expect(storage.items.get("emberhut.install")).toBe("install-a");
    expect(events).toEqual([{ kind: "signedOut", reason: "expired" }]);
  });

  it("keeps the refresh token when the gate in front of the refresh route says the device isn't let in", async () => {
    const { server, storage, clock, auth } = fakeApi({ storage: signedIn() });
    const events: AuthEvent[] = [];
    auth.subscribe((e) => events.push(e));
    let gate = true;
    server.on("POST", "/api/auth/refresh", () => (gate ? json(401, { error: "not Emberhut's device" }) : json(200, tokens(1, clock.now))));

    expect(await auth.accessToken()).toBeNull();
    expect(auth.signedIn).toBe(true);
    expect(storage.items.get("emberhut.refresh")).toBe("refresh-0");
    expect(events).toEqual([{ kind: "refused" }]);
    gate = false;
    expect(await auth.accessToken()).toBe("access-1");
  });

  it("finishes a renewal a caller gave up on, so the rotated refresh token is kept", async () => {
    const { server, storage, clock, auth, api } = fakeApi({ storage: signedIn() });
    let release!: () => void;
    const gate = new Promise<void>((resolve) => (release = resolve));
    let n = 0;
    server.on("POST", "/api/auth/refresh", async () => {
      await gate;
      return json(200, tokens(++n, clock.now));
    });
    server.on("GET", "/api/tasks/5", () => json(200, { id: 5 }));

    const controller = new AbortController();
    const abandoned = api.tasks.get(5, controller.signal).catch((e: unknown) => e);
    await new Promise((resolve) => setTimeout(resolve, 10));
    controller.abort();
    release();
    await abandoned;

    expect(storage.items.get("emberhut.refresh")).toBe("refresh-1");
    await api.tasks.get(5);
    expect(server.to("/api/auth/refresh")).toHaveLength(1);
    expect(server.to("/api/tasks/5").at(-1)?.headers.authorization).toBe("Bearer access-1");
    clock.now += 60 * 60_000;
    await auth.accessToken();
    expect(server.to("/api/auth/refresh").map((r) => (r.body as { refresh: string }).refresh)).toEqual(["refresh-0", "refresh-1"]);
  });

  it("gives a renewal a long limit of its own, so a dead connection can't hold every call", async () => {
    vi.useFakeTimers();
    try {
      const { server, auth } = fakeApi({ storage: signedIn() });
      let signal: AbortSignal | undefined;
      server.on("POST", "/api/auth/refresh", (request) => {
        signal = request.signal;
        return new Promise<Response>((_, reject) =>
          signal?.addEventListener("abort", () => reject(new DOMException("Aborted", "AbortError"))),
        );
      });
      const call = auth.accessToken();
      await vi.advanceTimersByTimeAsync(60_000);
      expect(signal?.aborted).toBe(false);
      await vi.advanceTimersByTimeAsync(60_000);
      expect(await call).toBeNull();
      expect(auth.signedIn).toBe(true);
    } finally {
      vi.useRealTimers();
    }
  });

  it("keeps the refresh token when the server can't be reached, and tries again on the next call", async () => {
    const { server, clock, auth } = fakeApi({ storage: signedIn() });
    let up = false;
    server.on("POST", "/api/auth/refresh", () => {
      if (!up) throw new TypeError("Network request failed");
      return json(200, tokens(1, clock.now));
    });

    expect(await auth.accessToken()).toBeNull();
    expect(auth.signedIn).toBe(true);
    up = true;
    expect(await auth.accessToken()).toBe("access-1");
  });

  it("pairs with a one-time code and this install's identity", async () => {
    const { server, storage, clock, auth } = fakeApi();
    const events: AuthEvent[] = [];
    auth.subscribe((e) => events.push(e));
    server.on("POST", "/api/auth/pair", () => json(200, tokens(1, clock.now)));

    expect(await auth.pair(" 4F7K-92QX ", { name: "Pixel 8", model: "Pixel 8" })).toEqual({ ok: true });
    expect(server.to("/api/auth/pair")[0]?.body).toEqual({ code: "4F7K-92QX", install: "install-1", name: "Pixel 8", model: "Pixel 8" });
    expect(storage.items.get("emberhut.refresh")).toBe("refresh-1");
    expect(storage.items.get("emberhut.install")).toBe("install-1");
    expect(auth.device).toBe(7);
    expect(events).toEqual([{ kind: "signedIn", device: 7 }]);
    expect(await auth.accessToken()).toBe("access-1");
    expect(server.to("/api/auth/refresh")).toHaveLength(0);
  });

  it("reports a refused code with the server's reason", async () => {
    const { server, auth } = fakeApi();
    server.on("POST", "/api/auth/pair", () => json(400, { ok: false, status: 400, error: "That code has expired." }));
    expect(await auth.pair("OLD", { name: "Pixel", model: "Pixel" })).toEqual({ ok: false, error: "That code has expired." });
    expect(auth.signedIn).toBe(false);
  });

  it("makes the install id once and keeps it", async () => {
    const { storage, auth } = fakeApi();
    const first = await auth.installId();
    expect(await auth.installId()).toBe(first);
    expect(storage.items.get("emberhut.install")).toBe(first);
  });

  it("signs out locally and forgets the session but not the install", async () => {
    const { auth, storage } = fakeApi({ storage: signedIn() });
    await auth.load();
    await auth.signOut();
    expect(auth.signedIn).toBe(false);
    expect([...storage.items.keys()]).toEqual(["emberhut.install"]);
  });

  it("reads secure storage again after a failed read, and says what failed", async () => {
    const { storage, state } = flakyStorage({ "emberhut.refresh": "refresh-0", "emberhut.install": "install-a" });
    const { server, clock, auth } = fakeApi({ storage });
    server.on("POST", "/api/auth/refresh", () => json(200, tokens(1, clock.now)));
    await expect(auth.accessToken()).rejects.toBeInstanceOf(StorageError);
    await expect(auth.load()).rejects.toThrow("Couldn't read this phone's saved sign-in");
    state.failing = false;
    expect(await auth.accessToken()).toBe("access-1");
  });

  it("keeps a renewed session in memory when the Keychain won't take it, and logs why", async () => {
    const { storage, state } = flakyStorage({ "emberhut.refresh": "refresh-0", "emberhut.install": "install-a" });
    state.failing = false;
    const logged: string[] = [];
    const { server, clock } = fakeApi();
    const auth = new AuthSession({
      baseUrl: () => "http://emberhut.test:18990",
      fetch: server.fetch,
      storage,
      newId: () => "install-b",
      now: () => clock.now,
      log: (where) => logged.push(where),
    });
    server.on("POST", "/api/auth/refresh", () => json(200, tokens(1, clock.now)));
    await auth.load();
    state.failing = true;
    expect(await auth.accessToken()).toBe("access-1");
    expect(auth.signedIn).toBe(true);
    expect(logged).toEqual(["auth: keeping the renewed sign-in"]);
  });

  it("reads again after a storage that throws before it ever awaits", async () => {
    const storage = memoryStorage({ "emberhut.refresh": "refresh-0", "emberhut.install": "install-a" });
    let broken = true;
    const getItem = storage.getItem;
    storage.getItem = (key) => {
      if (broken) throw new Error("thrown at once");
      return getItem(key);
    };
    const { auth } = fakeApi({ storage });
    await expect(auth.load()).rejects.toBeInstanceOf(StorageError);
    broken = false;
    await auth.load();
    expect(auth.signedIn).toBe(true);
  });
});
