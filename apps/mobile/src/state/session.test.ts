import { describe, expect, it } from "vitest";
import { createSessionStore, needsSignIn } from "@/state/session";
import { createServerAddress } from "@/api";
import { fakeApi, fixedAddress, flakyStorage, json, memoryStorage, tokens } from "@/test/fakes";

const device = { name: "Ada's Pixel", model: "Pixel 8" };

describe("session store", () => {
  it("starts signed out without a refresh token, and pairs", async () => {
    const { server, clock, auth, api } = fakeApi();
    server.on("POST", "/api/auth/pair", () => json(200, tokens(1, clock.now)));
    const store = createSessionStore({ auth, api, address: fixedAddress(), device });
    expect(store.getState().status).toBe("starting");
    await store.getState().start();
    expect(store.getState().status).toBe("signedOut");
    expect(needsSignIn(store.getState())).toBe(true);

    expect(await store.getState().pair("ABCD")).toBe(true);
    expect(store.getState()).toMatchObject({ status: "signedIn", device: 7, pairError: null });
    expect(server.to("/api/auth/pair")[0]?.body).toMatchObject(device);
  });

  it("asks nothing before it has a server, then keeps the one you type when you pair", async () => {
    const { server, clock, auth, api } = fakeApi();
    server.on("POST", "/api/auth/pair", () => json(200, tokens(1, clock.now)));
    const storage = memoryStorage();
    const store = createSessionStore({ auth, api, address: createServerAddress(storage), device });
    await store.getState().start();
    expect(store.getState()).toMatchObject({ status: "signedOut", serverUrl: null });
    expect(server.to("/api/auth/status")).toEqual([]);

    expect(await store.getState().pair("ABCD", undefined, "not a server")).toBe(false);
    expect(store.getState().pairError).toMatch(/isn't a server address/);
    expect(await store.getState().pair("ABCD", undefined, "http://my-server:18990")).toBe(false);
    expect(store.getState()).toMatchObject({ pairing: false, serverUrl: null, pairError: expect.stringMatching(/Plain http only/) });
    expect(await store.getState().pair("ABCD", undefined, "my-server:18990/")).toBe(true);
    expect(store.getState().serverUrl).toBe("https://my-server:18990");
    expect(await createServerAddress(storage).load()).toBe("https://my-server:18990");
  });

  it("asks the server whether sign-in is required, so an enforcing server offers no Not now", async () => {
    const { server, auth, api } = fakeApi();
    server.on("GET", "/api/auth/status", () => json(200, { mode: "enforce", bound: true, devices: [], signedIn: false }));
    const store = createSessionStore({ auth, api, address: fixedAddress(), device });
    await store.getState().start();
    expect(store.getState()).toMatchObject({ status: "signedOut", required: true });
    store.getState().signInLater();
    expect(needsSignIn(store.getState())).toBe(true);
  });

  it("keeps the reason a code was refused", async () => {
    const { server, auth, api } = fakeApi();
    server.on("POST", "/api/auth/pair", () => json(403, { ok: false, error: "Wrong code." }));
    const store = createSessionStore({ auth, api, address: fixedAddress(), device });
    expect(await store.getState().pair("NOPE")).toBe(false);
    expect(store.getState()).toMatchObject({ pairing: false, pairError: "Wrong code." });
  });

  it("lets you wait with sign-in only while the server doesn't require it", async () => {
    const { server, auth, api } = fakeApi();
    const store = createSessionStore({ auth, api, address: fixedAddress(), device });
    await store.getState().start();
    store.getState().signInLater();
    expect(needsSignIn(store.getState())).toBe(false);

    server.on("GET", "/api/facts", () => json(401, { signIn: true }));
    await api.settings.facts().catch(() => undefined);
    expect(store.getState().required).toBe(true);
    expect(needsSignIn(store.getState())).toBe(true);
    store.getState().signInLater();
    expect(needsSignIn(store.getState())).toBe(true);
  });

  it("signs this device out on the server, then here", async () => {
    const storage = memoryStorage({ "emberhut.refresh": "refresh-0", "emberhut.install": "i", "emberhut.device": "7" });
    const { server, clock, auth, api } = fakeApi({ storage });
    server.on("POST", "/api/auth/refresh", () => json(200, tokens(1, clock.now)));
    server.on("POST", "/api/settings", () => json(200, { ok: true }));
    const store = createSessionStore({ auth, api, address: fixedAddress(), device });
    await store.getState().start();
    expect(store.getState().status).toBe("signedIn");

    await store.getState().signOut();
    expect(server.to("/api/settings")[0]?.body).toEqual({ authRevoke: 7 });
    expect(store.getState()).toMatchObject({ status: "signedOut", required: false, device: null });
  });

  it("notes a device the server doesn't let in, until a request gets through", async () => {
    const { server, auth, api } = fakeApi();
    const store = createSessionStore({ auth, api, address: fixedAddress(), device });
    server.on("GET", "/api/facts", () => json(401, { error: "not Emberhut's device" }));
    await api.settings.facts().catch(() => undefined);
    expect(store.getState().refused).toBe(true);
    server.on("GET", "/api/facts", () => json(200, { facts: [] }));
    await api.settings.facts();
    expect(store.getState().refused).toBe(false);
  });

  it("shows sign-in with the reason when secure storage can't be read, and reads again on retry", async () => {
    const { storage, state } = flakyStorage({
      "emberhut.server": "https://emberhut.test:18990",
      "emberhut.refresh": "refresh-0",
      "emberhut.install": "i",
    });
    const { server, clock, auth, api } = fakeApi({ storage });
    server.on("POST", "/api/auth/refresh", () => json(200, tokens(1, clock.now)));
    const store = createSessionStore({ auth, api, address: createServerAddress(storage), device });
    await expect(store.getState().start()).resolves.toBeUndefined();
    expect(store.getState()).toMatchObject({ status: "signedOut", startError: "Couldn't read this phone's saved sign-in." });
    expect(needsSignIn(store.getState())).toBe(true);
    expect(storage.items.get("emberhut.refresh")).toBe("refresh-0"); // nothing deleted: it may open later

    state.failing = false;
    await store.getState().start();
    expect(store.getState()).toMatchObject({ status: "signedIn", startError: null, serverUrl: "https://emberhut.test:18990" });
  });

  it("never sticks on pairing when the address can't be kept, and holds no tokens then", async () => {
    // Only the address's storage fails: the answer is accepted, then the address can't be saved.
    const { storage, state } = flakyStorage();
    const { server, clock, auth, api } = fakeApi();
    server.on("POST", "/api/auth/pair", () => json(200, tokens(1, clock.now)));
    const store = createSessionStore({ auth, api, address: createServerAddress(storage), device });
    expect(await store.getState().pair("ABCD", undefined, "my-server:18990")).toBe(false);
    expect(store.getState()).toMatchObject({ pairing: false, pairError: "Couldn't keep the server's address on this phone. Try again." });
    expect(auth.signedIn).toBe(false);

    state.failing = false;
    expect(await store.getState().pair("ABCD", undefined, "my-server:18990")).toBe(true);
    expect(store.getState()).toMatchObject({ status: "signedIn", pairing: false, pairError: null });
  });

  it("never sticks on pairing when the install id or the tokens can't be kept", async () => {
    const { storage, state } = flakyStorage();
    const { server, clock, auth, api } = fakeApi({ storage });
    server.on("POST", "/api/auth/pair", () => json(200, tokens(1, clock.now)));
    const store = createSessionStore({ auth, api, address: fixedAddress(), device });
    expect(await store.getState().pair("ABCD")).toBe(false);
    expect(store.getState()).toMatchObject({ pairing: false, pairError: "Couldn't read this phone's saved sign-in. Try again." });
    expect(server.to("/api/auth/pair")).toEqual([]);

    // The install id is kept, then the Keychain fails again before the tokens are: not signed in, nothing half kept.
    state.failing = false;
    await auth.installId();
    state.failing = true;
    expect(await store.getState().pair("ABCD")).toBe(false);
    expect(store.getState()).toMatchObject({ status: "starting", pairing: false });
    expect(store.getState().pairError).toBe("Couldn't save the sign-in on this phone. Try again.");
    expect(auth.signedIn).toBe(false);
  });

  it("offers no Not now before there is a server: there would be nothing to show", async () => {
    const { auth, api } = fakeApi();
    const store = createSessionStore({ auth, api, address: createServerAddress(memoryStorage()), device });
    await store.getState().start();
    store.getState().signInLater();
    expect(store.getState().later).toBe(false);
    expect(needsSignIn(store.getState())).toBe(true);
  });
});
